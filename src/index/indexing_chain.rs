//! Per-segment indexing pipeline ported from `org.apache.lucene.index`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`FieldInvertState`] | `FieldInvertState` |
//! | [`DefaultIndexingChain`] | `IndexingChain` |
//! | `PerField` (private) | `IndexingChain.PerField` |
//! | [`EmptyNormsProducer`] | the `NormsProducer` the chain passes to the codec |
//!
//! The chain turns a [`Document`] into buffered postings and a stored-fields
//! stream: it registers each field in the segment's [`FieldInfosBuilder`], runs
//! the analysis pipeline, validates positions and offsets, feeds every token to
//! [`FreqProxTermsWriter`], and hands every stored value to the
//! [`StoredFieldsConsumer`]. At flush time it streams the buffered postings
//! through the codec's postings format, producing the `.doc`, `.pos`, `.pay`,
//! `.tim`, `.tip` and `.tmd` files of the segment, and finishes the
//! stored-fields stream, producing `.fdt`, `.fdx` and `.fdm`.
//!
//! # Java to Rust adaptations
//!
//! * **Field lookup is a map, not a hand-rolled hash table.** Lucene keeps
//!   `PerField` objects in an open-addressed `PerField[] fieldHash` with its own
//!   `rehash()`, because it wants to avoid `HashMap.Entry` allocations per
//!   segment. A `HashMap<String, usize>` into a `Vec<PerField>` costs the same
//!   lookup and lets the borrow checker prove that the per-field state and the
//!   shared byte pool are disjoint.
//! * **Attribute handles are resolved per field instance, not cached in
//!   `FieldInvertState`.** Lucene caches five `Attribute` references in
//!   `setAttributeSource`. Rucene's `AttributeSource` hands out `Ref<'_, T>`
//!   guards borrowed from the stream, which cannot be stored next to the stream
//!   they borrow from, so this port resolves which attributes are present once
//!   per field instance and reads their values per token.
//! * **Flush reports its result instead of mutating shared state.** Lucene lets
//!   `FreqProxTermsWriter.applyDeletes` write straight into
//!   `SegmentWriteState.liveDocs` / `.delCountOnFlush`. Here the chain returns
//!   an [`IndexingChainFlushResult`] and the DWPT applies it, so no caller holds
//!   a mutable alias of the flush state.
//!
//! # Scope
//!
//! Lucene's `IndexingChain` also drives doc values, points, vectors, norms and
//! term vectors. Those consumers are separate ports; this chain implements the
//! inverted-index and stored-fields paths, so a flushed segment currently
//! contains its postings files and its stored-fields files and nothing else.
//!
//! Lucene picks `SortingStoredFieldsConsumer` when the segment has an index
//! sort. Index sorting is a separate port, so this chain always uses the plain
//! [`StoredFieldsConsumer`]; [`DefaultIndexingChain::bind_segment`] is the one
//! place that choice will be made.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::analysis::tokenattributes::{
    BytesTermAttributeImpl, CharTermAttributeImpl, OffsetAttribute, OffsetAttributeImpl,
    PackedTokenAttributeImpl, PayloadAttribute, PayloadAttributeImpl, PositionIncrementAttribute,
    PositionIncrementAttributeImpl, TermFrequencyAttribute, TermFrequencyAttributeImpl,
    TermToBytesRefAttribute,
};
use crate::analysis::Analyzer;
use crate::codecs::postings::{NormsProducer, NumericDocValues};
use crate::codecs::state::SegmentWriteState;
use crate::document::{Document, InvertableType, StoredValueType};
use crate::error::{LuceneError, Result};
use crate::index::documents_writer::{
    IndexingChain, IndexingChainFlushResult, IndexingChainFlushState, SharedIndexingScratch,
    MAX_STORED_STRING_LENGTH,
};
use crate::index::field_infos::FieldInfosBuilder;
use crate::index::freq_prox_terms_writer::{FreqProxTermsWriter, InvertedToken};
use crate::index::index_writer_config::LiveIndexWriterConfig;
use crate::index::stored_fields_consumer::StoredFieldsConsumer;
use crate::index::{FieldInfo, IndexOptions, IndexableField, SegmentInfo};
use crate::store::TrackingDirectoryWrapper;
use crate::util::{AttributeSource, BytesRef, InfoStream};

/// Highest position a token may occupy.
///
/// Equivalent to `IndexWriter.MAX_POSITION`.
pub const MAX_POSITION: i32 = i32::MAX - 128;

// ---------------------------------------------------------------------------
// FieldInvertState
// ---------------------------------------------------------------------------

/// Statistics gathered while one field of one document is inverted.
///
/// Equivalent to `org.apache.lucene.index.FieldInvertState`. The values survive
/// across the instances of a multi-valued field and are reset when the field is
/// first seen in a document.
#[derive(Debug, Clone)]
pub struct FieldInvertState {
    index_created_version_major: i32,
    name: String,
    index_options: IndexOptions,
    position: i32,
    length: i32,
    num_overlap: i32,
    offset: i32,
    max_term_frequency: i32,
    unique_term_count: i32,
    last_start_offset: i32,
    last_position: i32,
}

impl FieldInvertState {
    /// Creates the state of a field, with every statistic at zero.
    pub fn new(
        index_created_version_major: i32,
        name: String,
        index_options: IndexOptions,
    ) -> Self {
        Self {
            index_created_version_major,
            name,
            index_options,
            position: 0,
            length: 0,
            num_overlap: 0,
            offset: 0,
            max_term_frequency: 0,
            unique_term_count: 0,
            last_start_offset: 0,
            last_position: 0,
        }
    }

    /// Re-initialises every statistic for a new document.
    ///
    /// Equivalent to `FieldInvertState.reset()`. `position` starts at `-1` so
    /// that the first token's position increment of one lands on position zero.
    pub fn reset(&mut self) {
        self.position = -1;
        self.length = 0;
        self.num_overlap = 0;
        self.offset = 0;
        self.max_term_frequency = 0;
        self.unique_term_count = 0;
        self.last_start_offset = 0;
        self.last_position = 0;
    }

    /// Returns the field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the index options the field is inverted with.
    pub fn index_options(&self) -> IndexOptions {
        self.index_options
    }

    /// Returns the major version that created the index.
    pub fn index_created_version_major(&self) -> i32 {
        self.index_created_version_major
    }

    /// Returns the position of the last processed token.
    pub fn position(&self) -> i32 {
        self.position
    }

    /// Returns the number of tokens in the field, counting custom term
    /// frequencies.
    pub fn length(&self) -> i32 {
        self.length
    }

    /// Overrides the token count.
    pub fn set_length(&mut self, length: i32) {
        self.length = length;
    }

    /// Returns the number of tokens whose position increment was zero.
    pub fn num_overlap(&self) -> i32 {
        self.num_overlap
    }

    /// Overrides the overlap count.
    pub fn set_num_overlap(&mut self, num_overlap: i32) {
        self.num_overlap = num_overlap;
    }

    /// Returns the end offset of the last processed token.
    pub fn offset(&self) -> i32 {
        self.offset
    }

    /// Moves the running position to `position`.
    ///
    /// Java mutates the package-private `FieldInvertState.position` field
    /// directly from `IndexingChain`; Rust needs an explicit setter for the
    /// per-field writers and their tests, which live in another module.
    pub fn set_position(&mut self, position: i32) {
        self.position = position;
    }

    /// Moves the running offset accumulator to `offset`.
    ///
    /// See [`Self::set_position`] for why this setter exists.
    pub fn set_offset(&mut self, offset: i32) {
        self.offset = offset;
    }

    /// Returns the highest frequency any single term reached in this field.
    pub fn max_term_frequency(&self) -> i32 {
        self.max_term_frequency
    }

    /// Overrides the highest per-term frequency.
    pub fn set_max_term_frequency(&mut self, max_term_frequency: i32) {
        self.max_term_frequency = max_term_frequency;
    }

    /// Returns the number of distinct terms seen in this field.
    pub fn unique_term_count(&self) -> i32 {
        self.unique_term_count
    }

    /// Records one more distinct term.
    pub fn increment_unique_term_count(&mut self) {
        self.unique_term_count += 1;
    }

    /// Returns the start offset of the last processed token.
    pub fn last_start_offset(&self) -> i32 {
        self.last_start_offset
    }

    /// Returns the position of the previous token.
    pub fn last_position(&self) -> i32 {
        self.last_position
    }
}

// ---------------------------------------------------------------------------
// Norms
// ---------------------------------------------------------------------------

/// Norms source handed to the postings format while norms are not written.
///
/// The Lucene 10.4 postings format reads norms only to compute the impact
/// blocks that accompany skip data. Rucene's `Lucene104PostingsWriter` treats
/// every norm as one (see its module documentation), so this producer reports
/// the same constant and never allocates.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyNormsProducer;

/// Numeric doc values that report a norm of one for every document.
#[derive(Debug, Default, Clone, Copy)]
struct ConstantNorms;

impl NumericDocValues for ConstantNorms {
    fn get(&self, _doc_id: i32) -> Result<i64> {
        Ok(1)
    }
}

impl NormsProducer for EmptyNormsProducer {
    fn get_norms(&self, _field_info: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        Ok(Box::new(ConstantNorms))
    }
}

// ---------------------------------------------------------------------------
// PerField
// ---------------------------------------------------------------------------

/// Per-field state kept for the whole segment.
///
/// Equivalent to `IndexingChain.PerField`, restricted to the inverted-index
/// path.
#[derive(Debug)]
struct PerField {
    field_info: FieldInfo,
    invert_state: FieldInvertState,
    /// Index of this field's writer inside [`FreqProxTermsWriter`], or `None`
    /// for a field that is stored but not indexed.
    ///
    /// Lucene creates the per-field writer in `PerField.setInvertState()`,
    /// which `initializeFieldInfo` only calls for indexed fields.
    writer_index: Option<usize>,
    /// Generation of the document this field was last seen in.
    field_gen: i64,
    /// `true` until the first instance of this field is inverted in the current
    /// document; multi-valued fields see it `false` from the second value on.
    first: bool,
}

/// Which attributes a field instance's token stream actually carries.
///
/// Resolved once per field instance, exactly where Lucene caches its
/// `Attribute` references in `FieldInvertState.setAttributeSource`.
#[derive(Debug, Default, Clone, Copy)]
struct TokenAttributeLayout {
    packed: bool,
    bytes_term: bool,
    char_term: bool,
    position_increment: bool,
    offset: bool,
    payload: bool,
    term_freq: bool,
}

impl TokenAttributeLayout {
    fn resolve(source: &AttributeSource) -> Self {
        Self {
            packed: source.has_attribute::<PackedTokenAttributeImpl>(),
            bytes_term: source.has_attribute::<BytesTermAttributeImpl>(),
            char_term: source.has_attribute::<CharTermAttributeImpl>(),
            position_increment: source.has_attribute::<PositionIncrementAttributeImpl>(),
            offset: source.has_attribute::<OffsetAttributeImpl>(),
            payload: source.has_attribute::<PayloadAttributeImpl>(),
            term_freq: source.has_attribute::<TermFrequencyAttributeImpl>(),
        }
    }
}

/// One token as read from the attributes of a token stream.
#[derive(Debug)]
struct RawToken {
    term: BytesRef,
    position_increment: i32,
    start_offset: i32,
    end_offset: i32,
    payload: Option<BytesRef>,
    term_freq: i32,
    has_term_freq_attribute: bool,
}

/// Reads the current token off `source`.
///
/// Lucene reaches for `TermToBytesRefAttribute`, `PositionIncrementAttribute`,
/// `OffsetAttribute`, `PayloadAttribute` and `TermFrequencyAttribute`; the two
/// former are added to the stream if missing, which is why the defaults here
/// (increment one, offsets zero, frequency one) match the attribute defaults.
fn read_token(source: &AttributeSource, layout: &TokenAttributeLayout) -> Result<RawToken> {
    let term = if layout.packed {
        source
            .get_attribute::<PackedTokenAttributeImpl>()
            .expect("INVARIANT: presence checked in TokenAttributeLayout::resolve")
            .get_bytes_ref()
    } else if layout.bytes_term {
        source
            .get_attribute::<BytesTermAttributeImpl>()
            .expect("INVARIANT: presence checked in TokenAttributeLayout::resolve")
            .get_bytes_ref()
    } else if layout.char_term {
        source
            .get_attribute::<CharTermAttributeImpl>()
            .expect("INVARIANT: presence checked in TokenAttributeLayout::resolve")
            .get_bytes_ref()
    } else {
        return Err(LuceneError::IllegalArgument(
            "token stream carries no term attribute".to_string(),
        ));
    };

    let position_increment = if layout.packed {
        source
            .get_attribute::<PackedTokenAttributeImpl>()
            .expect("INVARIANT: presence checked above")
            .get_position_increment()
    } else if layout.position_increment {
        source
            .get_attribute::<PositionIncrementAttributeImpl>()
            .expect("INVARIANT: presence checked above")
            .get_position_increment()
    } else {
        1
    };

    let (start_offset, end_offset) = if layout.packed {
        let attribute = source
            .get_attribute::<PackedTokenAttributeImpl>()
            .expect("INVARIANT: presence checked above");
        (attribute.start_offset(), attribute.end_offset())
    } else if layout.offset {
        let attribute = source
            .get_attribute::<OffsetAttributeImpl>()
            .expect("INVARIANT: presence checked above");
        (attribute.start_offset(), attribute.end_offset())
    } else {
        (0, 0)
    };

    let payload = if layout.payload {
        source
            .get_attribute::<PayloadAttributeImpl>()
            .expect("INVARIANT: presence checked above")
            .get_payload()
            .cloned()
    } else {
        None
    };

    let term_freq = if layout.term_freq {
        source
            .get_attribute::<TermFrequencyAttributeImpl>()
            .expect("INVARIANT: presence checked above")
            .get_term_frequency()
    } else {
        1
    };

    Ok(RawToken {
        term,
        position_increment,
        start_offset,
        end_offset,
        payload,
        term_freq,
        has_term_freq_attribute: layout.term_freq,
    })
}

/// Reads the end-of-stream position increment and end offset.
fn read_stream_end(source: &AttributeSource, layout: &TokenAttributeLayout) -> (i32, i32) {
    let position_increment = if layout.packed {
        source
            .get_attribute::<PackedTokenAttributeImpl>()
            .map(|attribute| attribute.get_position_increment())
            .unwrap_or(0)
    } else if layout.position_increment {
        source
            .get_attribute::<PositionIncrementAttributeImpl>()
            .map(|attribute| attribute.get_position_increment())
            .unwrap_or(0)
    } else {
        0
    };
    let end_offset = if layout.packed {
        source
            .get_attribute::<PackedTokenAttributeImpl>()
            .map(|attribute| attribute.end_offset())
            .unwrap_or(0)
    } else if layout.offset {
        source
            .get_attribute::<OffsetAttributeImpl>()
            .map(|attribute| attribute.end_offset())
            .unwrap_or(0)
    } else {
        0
    };
    (position_increment, end_offset)
}

// ---------------------------------------------------------------------------
// DefaultIndexingChain
// ---------------------------------------------------------------------------

/// The inverted-index pipeline of one `DocumentsWriterPerThread`.
///
/// Equivalent to `org.apache.lucene.index.IndexingChain`.
#[derive(Debug)]
pub struct DefaultIndexingChain {
    config: Arc<LiveIndexWriterConfig>,
    bytes_used: Arc<AtomicI64>,
    scratch: SharedIndexingScratch,
    terms_writer: FreqProxTermsWriter,
    per_fields: Vec<PerField>,
    per_field_index: HashMap<String, usize>,
    next_field_gen: i64,
    aborting_error: Option<LuceneError>,
    /// The stored-fields half of the chain, present once the chain knows which
    /// segment it writes. Equivalent to `IndexingChain.storedFieldsConsumer`,
    /// which Lucene builds in the constructor because it already has the
    /// directory and the `SegmentInfo` there.
    stored_fields_consumer: Option<StoredFieldsConsumer>,
}

impl DefaultIndexingChain {
    /// Creates a chain bound to the given live configuration.
    pub fn new(config: Arc<LiveIndexWriterConfig>) -> Self {
        let bytes_used = Arc::new(AtomicI64::new(0));
        Self {
            scratch: SharedIndexingScratch::new(Arc::clone(&bytes_used)),
            terms_writer: FreqProxTermsWriter::new(Arc::clone(&bytes_used)),
            config,
            bytes_used,
            per_fields: Vec::new(),
            per_field_index: HashMap::new(),
            next_field_gen: 0,
            aborting_error: None,
            stored_fields_consumer: None,
        }
    }

    /// Creates a chain already bound to the segment it will write.
    ///
    /// This is the shape of Lucene's `IndexingChain` constructor, which takes
    /// the directory and the `SegmentInfo` up front. Prefer it whenever the
    /// segment is known at construction time; the `DocumentsWriterPerThread`
    /// cannot, because it builds both after the chain, and calls
    /// [`IndexingChain::bind_segment`] instead.
    pub fn new_for_segment(
        config: Arc<LiveIndexWriterConfig>,
        directory: Arc<TrackingDirectoryWrapper>,
        segment_info: &SegmentInfo,
    ) -> Result<Self> {
        let mut chain = Self::new(config);
        <Self as IndexingChain>::bind_segment(&mut chain, directory, segment_info)?;
        Ok(chain)
    }

    /// Returns the stored-fields consumer, if the chain is bound to a segment.
    pub fn stored_fields_consumer(&self) -> Option<&StoredFieldsConsumer> {
        self.stored_fields_consumer.as_ref()
    }

    /// Opens the stored-fields frame of `doc_id`.
    ///
    /// Equivalent to `IndexingChain.startStoredFields(int)`: a failure here
    /// leaves the stored-fields stream misaligned, so it aborts the whole
    /// segment rather than just the document.
    fn start_stored_fields(&mut self, doc_id: i32) -> Result<()> {
        let Some(consumer) = self.stored_fields_consumer.as_mut() else {
            return Ok(());
        };
        consumer.start_document(doc_id).inspect_err(|error| {
            self.aborting_error = Some(LuceneError::CorruptIndex(format!(
                "the stored fields of segment may be corrupt after doc {doc_id}: {error}"
            )));
        })
    }

    /// Closes the stored-fields frame of the current document.
    ///
    /// Equivalent to `IndexingChain.finishStoredFields()`.
    fn finish_stored_fields(&mut self) -> Result<()> {
        let Some(consumer) = self.stored_fields_consumer.as_mut() else {
            return Ok(());
        };
        consumer.finish_document().inspect_err(|error| {
            self.aborting_error = Some(LuceneError::CorruptIndex(format!(
                "the stored fields of segment may be corrupt: {error}"
            )));
        })
    }

    /// Validates and writes the stored value of one field instance.
    ///
    /// Equivalent to the `fieldType.stored()` half of
    /// `IndexingChain.invertAndStore`. The two validation failures are
    /// document-level — the document is rejected and indexing continues — while
    /// a failure inside the consumer is aborting, because it may have left the
    /// stored-fields stream half written.
    fn store_field(&mut self, per_field: usize, field: &dyn IndexableField) -> Result<()> {
        // Aborting is deliberate, and it is where Lucene ends up too, one step
        // later. `IndexableField.storedValue()` declares no `IOException`
        // (`IndexableField.java:77`) and `IndexingChain.java:1422` calls it
        // *outside* the try/catch that reaches `onAbortingException`
        // (`:1434-1437`) — because at that point Java has read nothing: its
        // `StoredFieldDataInput` is still a live cursor, drained later inside
        // `StoredFieldsWriter.writeField`, which *is* inside that try/catch.
        // This port drains the cursor eagerly in `stored_value()`, so the same
        // read failure surfaces one call earlier and must be routed to the same
        // place: a half-read stored value leaves the segment untrustworthy.
        let value = field.stored_value().inspect_err(|error| {
            self.aborting_error = Some(LuceneError::CorruptIndex(format!(
                "the stored value of field \"{}\" could not be read: {error}",
                field.name()
            )));
        })?;
        let Some(value) = value else {
            // Lucene reaches this when `Field.storedValue()` returns null.
            return Err(LuceneError::IllegalArgument(
                "Cannot store a null value".to_string(),
            ));
        };
        if value.value_type() == StoredValueType::STRING {
            let text = value
                .string_value()
                .expect("INVARIANT: the value was just typed as STRING");
            // `String.length()` counts UTF-16 code units and is O(1) in Java.
            // A string never has more UTF-16 units than UTF-8 bytes, so the
            // cheap byte test settles every realistic case; the exact count
            // only runs for the few hundred megabytes above the limit.
            if text.len() > MAX_STORED_STRING_LENGTH {
                let length = text.encode_utf16().count();
                if length > MAX_STORED_STRING_LENGTH {
                    return Err(LuceneError::IllegalArgument(format!(
                        "stored field \"{}\" is too large ({length} characters) to store",
                        field.name()
                    )));
                }
            }
        }
        let info = self.per_fields[per_field].field_info.clone();
        let Some(consumer) = self.stored_fields_consumer.as_mut() else {
            return Err(LuceneError::IllegalState(format!(
                "the indexing chain is not bound to a segment, so the stored field \"{}\" \
                 cannot be written; call bind_segment first",
                field.name()
            )));
        };
        consumer.write_field(&info, value).inspect_err(|error| {
            self.aborting_error = Some(LuceneError::CorruptIndex(format!(
                "the stored fields of segment may be corrupt after field \"{}\": {error}",
                field.name()
            )));
        })
    }

    /// Returns the shared scratch buffers of this chain.
    pub fn scratch(&mut self) -> &mut SharedIndexingScratch {
        &mut self.scratch
    }

    /// Returns the inversion statistics of `field`, if it was indexed.
    ///
    /// The values are those of the last document in which the field appeared,
    /// which is exactly what `IndexingChain` keeps in its `PerField`.
    pub fn field_invert_state(&self, field: &str) -> Option<&FieldInvertState> {
        self.per_field_index
            .get(field)
            .map(|index| &self.per_fields[*index].invert_state)
    }

    /// Returns the number of distinct terms buffered for `field`.
    pub fn field_term_count(&self, field: &str) -> i32 {
        self.terms_writer
            .field_by_name(field)
            .map_or(0, |writer| writer.num_terms())
    }

    /// Registers `field_info` and returns the index of its [`PerField`].
    fn get_or_add_per_field(&mut self, field_info: &FieldInfo) -> usize {
        if let Some(index) = self.per_field_index.get(&field_info.name) {
            return *index;
        }
        let writer_index = if field_info.index_options == IndexOptions::NONE {
            None
        } else {
            Some(self.terms_writer.add_field(field_info))
        };
        let invert_state = FieldInvertState::new(
            self.config.index_created_version_major(),
            field_info.name.clone(),
            field_info.index_options,
        );
        let index = self.per_fields.len();
        self.per_fields.push(PerField {
            field_info: field_info.clone(),
            invert_state,
            writer_index,
            field_gen: -1,
            first: true,
        });
        self.per_field_index.insert(field_info.name.clone(), index);
        index
    }

    /// Builds the [`FieldInfo`] a document field implies.
    ///
    /// Equivalent to `IndexingChain.initializeFieldInfo` restricted to the
    /// attributes the inverted-index path needs.
    fn describe_field(
        field: &dyn IndexableField,
        field_infos: &FieldInfosBuilder,
    ) -> Result<FieldInfo> {
        let field_type = field.field_type();
        let name = field.name().to_string();
        let soft_deletes = field_infos.soft_deletes_field_name() == Some(name.as_str());
        let parent = field_infos.parent_field_name() == Some(name.as_str());
        FieldInfo::new_full(
            name,
            -1,
            field_type.store_term_vectors(),
            field_type.omit_norms(),
            false,
            field_type.index_options(),
            field_type.doc_values_type(),
            field_type.doc_values_skip_index_type(),
            -1,
            field_type.attributes().clone(),
            field_type.point_dimension_count(),
            field_type.point_index_dimension_count(),
            field_type.point_num_bytes(),
            field_type.vector_dimension(),
            field_type.vector_encoding(),
            field_type.vector_similarity_function(),
            soft_deletes,
            parent,
        )
    }

    /// Inverts one instance of one field.
    ///
    /// Equivalent to `IndexingChain.PerField.invert`. It takes its collaborators
    /// explicitly so that the borrow checker can see that the per-field state
    /// and the shared byte pool are disjoint.
    fn invert(
        analyzer: &dyn Analyzer,
        info_stream: &dyn InfoStream,
        terms_writer: &mut FreqProxTermsWriter,
        per_field: &mut PerField,
        doc_id: i32,
        field: &dyn IndexableField,
        first: bool,
    ) -> Result<()> {
        debug_assert!(field
            .field_type()
            .index_options()
            .subsumes(IndexOptions::DOCS));
        if first {
            per_field.invert_state.reset();
        }
        match field.invertable_type() {
            Some(InvertableType::BINARY) => {
                Self::invert_term(terms_writer, per_field, doc_id, field)
            }
            Some(InvertableType::TOKEN_STREAM) => Self::invert_token_stream(
                analyzer,
                info_stream,
                terms_writer,
                per_field,
                doc_id,
                field,
            ),
            None => Err(LuceneError::IllegalArgument(format!(
                "field \"{}\" is not indexed but reached the inverter",
                field.name()
            ))),
        }
    }

    /// Inverts a field that indexes its binary value as a single term.
    ///
    /// Equivalent to `IndexingChain.PerField.invertTerm`.
    fn invert_term(
        terms_writer: &mut FreqProxTermsWriter,
        per_field: &mut PerField,
        doc_id: i32,
        field: &dyn IndexableField,
    ) -> Result<()> {
        let binary_value = field.binary_value().ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "Field {} returns TERM for invertableType() and null for binaryValue(), which is illegal",
                field.name()
            ))
        })?;
        let field_type = field.field_type();
        if field_type.tokenized()
            || field_type
                .index_options()
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            || field_type.store_term_vector_positions()
            || field_type.store_term_vector_offsets()
            || field_type.store_term_vector_payloads()
        {
            return Err(LuceneError::IllegalArgument(format!(
                "Fields that are tokenized or index proximity data must produce a non-null TokenStream, but {} did not",
                field.name()
            )));
        }

        per_field.invert_state.position += 1;
        // Lucene 10.5.0 increments `length` twice here — once before and once
        // after `TermsHashPerField.start` — so a single-valued binary field
        // reports a length of two. Verified against
        // `lucene/core/src/java/org/apache/lucene/index/IndexingChain.java`
        // lines 2074-2078 at tag `releases/lucene/10.5.0`. Functional parity
        // requires reproducing it: the value reaches user code through
        // `Similarity.computeNorm(FieldInvertState)`.
        per_field.invert_state.length += 2;

        let token = InvertedToken {
            start_offset: 0,
            end_offset: 0,
            payload: None,
            term_freq: 1,
            has_term_freq_attribute: false,
        };
        let writer_index = per_field.writer_index.ok_or_else(|| {
            LuceneError::IllegalState(format!("field \"{}\" has no postings writer", field.name()))
        })?;
        let (pool, writer) = terms_writer.pool_and_field(writer_index);
        writer.add(
            pool,
            &mut per_field.invert_state,
            binary_value.slice(),
            doc_id,
            &token,
        )
    }

    /// Inverts a field through its token stream.
    ///
    /// Equivalent to `IndexingChain.PerField.invertTokenStream`.
    fn invert_token_stream(
        analyzer: &dyn Analyzer,
        info_stream: &dyn InfoStream,
        terms_writer: &mut FreqProxTermsWriter,
        per_field: &mut PerField,
        doc_id: i32,
        field: &dyn IndexableField,
    ) -> Result<()> {
        let analyzed = field.field_type().tokenized();
        let is_term_doc = per_field.field_info.is_term_doc_field();
        let mut stream = field.token_stream(analyzer, None);

        let outcome = (|| -> Result<()> {
            stream.reset()?;
            let layout = TokenAttributeLayout::resolve(stream.attribute_source());

            while stream.increment_token()? {
                let token = read_token(stream.attribute_source(), &layout)?;
                let state = &mut per_field.invert_state;

                let position_increment = token.position_increment;
                state.position += position_increment;
                if state.position < state.last_position {
                    return Err(LuceneError::IllegalArgument(if position_increment == 0 {
                        format!(
                            "first position increment must be > 0 (got 0) for field '{}'",
                            field.name()
                        )
                    } else if position_increment < 0 {
                        format!(
                            "position increment must be >= 0 (got {position_increment}) for field '{}'",
                            field.name()
                        )
                    } else {
                        format!(
                            "position overflowed Integer.MAX_VALUE (got posIncr={position_increment} lastPosition={} position={}) for field '{}'",
                            state.last_position,
                            state.position,
                            field.name()
                        )
                    }));
                }
                if state.position > MAX_POSITION {
                    return Err(LuceneError::IllegalArgument(format!(
                        "position {} is too large for field '{}': max allowed position is {MAX_POSITION}",
                        state.position,
                        field.name()
                    )));
                }
                state.last_position = state.position;
                if position_increment == 0 {
                    state.num_overlap += 1;
                }

                let start_offset = state.offset + token.start_offset;
                let end_offset = state.offset + token.end_offset;
                if start_offset < state.last_start_offset || end_offset < start_offset {
                    return Err(LuceneError::IllegalArgument(format!(
                        "startOffset must be non-negative, and endOffset must be >= startOffset, and offsets must not go backwards startOffset={start_offset},endOffset={end_offset},lastStartOffset={} for field '{}'",
                        state.last_start_offset,
                        field.name()
                    )));
                }
                state.last_start_offset = start_offset;

                let counted = if is_term_doc { 1 } else { token.term_freq };
                state.length = state.length.checked_add(counted).ok_or_else(|| {
                    LuceneError::IllegalArgument(format!(
                        "too many tokens for field \"{}\"",
                        field.name()
                    ))
                })?;

                let inverted = InvertedToken {
                    start_offset: token.start_offset,
                    end_offset: token.end_offset,
                    payload: token.payload.as_ref().map(BytesRef::slice),
                    term_freq: token.term_freq,
                    has_term_freq_attribute: token.has_term_freq_attribute,
                };
                let writer_index = per_field.writer_index.ok_or_else(|| {
                    LuceneError::IllegalState(format!(
                        "field \"{}\" has no postings writer",
                        field.name()
                    ))
                })?;
                let (pool, writer) = terms_writer.pool_and_field(writer_index);
                writer.add(
                    pool,
                    &mut per_field.invert_state,
                    token.term.slice(),
                    doc_id,
                    &inverted,
                )?;
            }

            stream.end()?;
            let (end_position_increment, end_offset) =
                read_stream_end(stream.attribute_source(), &layout);
            per_field.invert_state.position += end_position_increment;
            per_field.invert_state.offset += end_offset;
            Ok(())
        })();

        let close_outcome = stream.close();
        if outcome.is_err() && info_stream.is_enabled("DW") {
            info_stream.message(
                "DW",
                &format!(
                    "An exception was thrown while processing field {}",
                    per_field.field_info.name
                ),
            );
        }
        outcome?;
        close_outcome?;

        if analyzed {
            per_field.invert_state.position +=
                analyzer.get_position_increment_gap(&per_field.field_info.name);
            per_field.invert_state.offset += analyzer.get_offset_gap(&per_field.field_info.name);
        }
        Ok(())
    }
}

impl IndexingChain for DefaultIndexingChain {
    fn process_document(
        &mut self,
        doc_id: i32,
        doc: &Document,
        _is_last_doc: bool,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<()> {
        let analyzer = self.config.analyzer_arc();
        let info_stream = self.config.info_stream();
        let field_gen = self.next_field_gen;
        self.next_field_gen += 1;

        // Two passes, as in Lucene: every instance of a multi-valued field must
        // be inverted together, because the analyzer may reuse one TokenStream
        // across fields.
        let mut doc_fields: Vec<usize> = Vec::with_capacity(doc.get_fields().len());
        for field in doc.get_fields() {
            let described = Self::describe_field(field.as_ref(), field_infos)?;
            let registered = field_infos.add(&described)?.clone();
            let index = self.get_or_add_per_field(&registered);
            if self.per_fields[index].field_gen != field_gen {
                self.per_fields[index].field_gen = field_gen;
                self.per_fields[index].first = true;
            }
            doc_fields.push(index);
        }

        // Lucene opens a stored-fields frame for every document, before any
        // field is processed, and closes it in a `finally` block: a document
        // that stores nothing still occupies its own frame, so the stream stays
        // aligned with the doc ids.
        self.start_stored_fields(doc_id)?;

        let mut indexed: Vec<usize> = Vec::new();
        let mut outcome: Result<()> = Ok(());
        for (field, index) in doc.get_fields().iter().zip(doc_fields.iter().copied()) {
            // `IndexingChain.invertAndStore` inverts first and stores second,
            // for every field instance, in document order.
            if field.field_type().index_options() != IndexOptions::NONE {
                let first = self.per_fields[index].first;
                if first {
                    self.per_fields[index].first = false;
                    indexed.push(index);
                }
                let result = Self::invert(
                    analyzer.as_ref(),
                    info_stream.as_ref(),
                    &mut self.terms_writer,
                    &mut self.per_fields[index],
                    doc_id,
                    field.as_ref(),
                    first,
                );
                if let Err(error) = result {
                    // Lucene distinguishes a document-level problem (the
                    // document is dropped, indexing continues) from a corrupt
                    // terms hash (the whole DWPT must be aborted). Only the
                    // validation errors raised above and by the per-field
                    // writer are recoverable.
                    if !matches!(
                        error,
                        LuceneError::IllegalArgument(_) | LuceneError::IllegalState(_)
                    ) {
                        self.aborting_error = Some(LuceneError::CorruptIndex(format!(
                            "indexing chain buffers may be corrupt after field \"{}\": {error}",
                            field.name()
                        )));
                    }
                    outcome = Err(error);
                    break;
                }
            }

            if field.field_type().stored() {
                if let Err(error) = self.store_field(index, field.as_ref()) {
                    outcome = Err(error);
                    break;
                }
            }
        }

        // `FreqProxTermsWriterPerField.finish` records that the field stores
        // payloads, which the field infos must carry into the segment.
        for index in indexed {
            let per_field = &self.per_fields[index];
            let Some(writer_index) = per_field.writer_index else {
                continue;
            };
            if self.terms_writer.field(writer_index).saw_payloads() {
                if let Some(info) = field_infos.field_info_mut(&per_field.field_info.name) {
                    info.set_store_payloads();
                }
            }
        }

        // Lucene's `finally` closes the frame unless an aborting exception was
        // already recorded, in which case the whole segment is discarded and
        // the stream no longer matters.
        if self.aborting_error.is_none() {
            self.finish_stored_fields()?;
        }
        outcome
    }

    fn bind_segment(
        &mut self,
        directory: Arc<TrackingDirectoryWrapper>,
        segment_info: &SegmentInfo,
    ) -> Result<()> {
        if self.stored_fields_consumer.is_some() {
            return Err(LuceneError::IllegalState(format!(
                "the indexing chain is already bound to segment {}",
                segment_info.name
            )));
        }
        // Lucene chooses `SortingStoredFieldsConsumer` here when the segment
        // has an index sort; index sorting is a separate port, so the plain
        // consumer is the only option for now.
        self.stored_fields_consumer = Some(StoredFieldsConsumer::new(
            self.config.codec(),
            directory,
            segment_info.clone(),
        ));
        Ok(())
    }

    fn abort(&mut self) {
        // Lucene runs `storedFieldsConsumer.abort()` inside a try-with-resources
        // whose finalizer aborts the terms hash, so both release their files
        // even when one of them throws.
        if let Some(consumer) = self.stored_fields_consumer.as_mut() {
            consumer.abort();
        }
        self.terms_writer.abort();
        self.per_fields.clear();
        self.per_field_index.clear();
        self.bytes_used.store(0, Ordering::Release);
    }

    fn ram_bytes_used(&self) -> i64 {
        self.bytes_used.load(Ordering::Acquire)
            + self
                .stored_fields_consumer
                .as_ref()
                .map_or(0, StoredFieldsConsumer::ram_bytes_used)
    }

    fn flush(&mut self, state: &IndexingChainFlushState<'_>) -> Result<IndexingChainFlushResult> {
        let seg_updates = crate::codecs::stub::BufferedUpdates;
        let mut write_state = SegmentWriteState::new(
            state.info_stream,
            state.directory,
            state.segment_info,
            state.field_infos,
            &seg_updates,
            state.context,
        );
        write_state.live_docs = state.live_docs.cloned();
        write_state.del_count_on_flush = state.del_count_on_flush;

        // Lucene finishes the stored fields before the postings; the two write
        // different files, so only the order of the calls is reproduced here.
        if let Some(consumer) = self.stored_fields_consumer.as_mut() {
            consumer.finish(state.segment_info.max_doc()?)?;
            consumer.flush(state.segment_info)?;
        }

        let norms = EmptyNormsProducer;
        self.terms_writer
            .flush(&mut write_state, state.delete_terms, &norms)?;

        if state.info_stream.is_enabled("DWPT") {
            state.info_stream.message(
                "DWPT",
                &format!(
                    "flushed postings and stored fields for segment {} ({} fields, {} deleted docs)",
                    state.segment_info.name,
                    state.field_infos.size(),
                    write_state.del_count_on_flush
                ),
            );
        }

        self.per_fields.clear();
        self.per_field_index.clear();
        self.bytes_used.store(0, Ordering::Release);

        Ok(IndexingChainFlushResult {
            live_docs: write_state.live_docs,
            del_count_on_flush: write_state.del_count_on_flush,
        })
    }

    fn take_aborting_error(&mut self) -> Option<LuceneError> {
        self.aborting_error.take()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::analysis::tokenattributes::{
        CharTermAttribute, PayloadAttributeImpl as PayloadImpl,
    };
    use crate::analysis::{default_token_attribute_factory, StandardAnalyzer, TokenStream};
    use crate::codecs::{register_codec, Codec, Lucene104Codec};
    use crate::document::{Field, FieldType, IntField, Store, StoredField, StringField, TextField};
    use crate::index::documents_writer::TermDelete;
    use crate::index::field_infos::FieldNumbers;
    use crate::index::{SegmentInfo, Term};
    use crate::store::{
        flush_io_context, ByteBuffersDirectory, Directory, FlushInfo, TrackingDirectoryWrapper,
    };
    use crate::util::{NoOutputInfoStream, Version};

    // -- Scripted token stream --------------------------------------------

    /// One scripted token.
    #[derive(Debug, Clone)]
    struct Tok {
        term: String,
        position_increment: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<Vec<u8>>,
    }

    impl Tok {
        fn new(term: &str, position_increment: i32, start: i32, end: i32) -> Self {
            Self {
                term: term.to_string(),
                position_increment,
                start_offset: start,
                end_offset: end,
                payload: None,
            }
        }

        fn with_payload(mut self, payload: &[u8]) -> Self {
            self.payload = Some(payload.to_vec());
            self
        }
    }

    /// Emits a fixed list of tokens so the tests never depend on an analyzer.
    #[derive(Debug)]
    struct ScriptedTokenStream {
        source: AttributeSource,
        tokens: Vec<Tok>,
        upto: usize,
        final_offset: i32,
        with_payload_attribute: bool,
    }

    impl ScriptedTokenStream {
        fn new(tokens: Vec<Tok>) -> Self {
            let with_payload_attribute = tokens.iter().any(|token| token.payload.is_some());
            let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
            source
                .add_attribute::<PackedTokenAttributeImpl>()
                .expect("packed token attribute");
            if with_payload_attribute {
                source.add_attribute_impl_instance(Box::new(PayloadImpl::new()));
            }
            Self {
                source,
                tokens,
                upto: 0,
                final_offset: 0,
                with_payload_attribute,
            }
        }
    }

    impl TokenStream for ScriptedTokenStream {
        fn increment_token(&mut self) -> Result<bool> {
            if self.upto == self.tokens.len() {
                return Ok(false);
            }
            self.source.clear_attributes();
            let token = self.tokens[self.upto].clone();
            self.upto += 1;
            {
                let mut packed = self
                    .source
                    .get_attribute_mut::<PackedTokenAttributeImpl>()
                    .expect("packed token attribute");
                packed.append_string(&token.term);
                packed.set_position_increment(token.position_increment);
                packed.set_offset(token.start_offset, token.end_offset);
            }
            if self.with_payload_attribute {
                let mut payload = self
                    .source
                    .get_attribute_mut::<PayloadImpl>()
                    .expect("payload attribute");
                payload.set_payload(token.payload.clone().map(BytesRef::new));
            }
            self.final_offset = token.end_offset;
            Ok(true)
        }

        fn reset(&mut self) -> Result<()> {
            self.upto = 0;
            self.final_offset = 0;
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            self.source.end_attributes();
            let mut packed = self
                .source
                .get_attribute_mut::<PackedTokenAttributeImpl>()
                .expect("packed token attribute");
            packed.set_offset(self.final_offset, self.final_offset);
            Ok(())
        }

        fn attribute_source(&self) -> &AttributeSource {
            &self.source
        }

        fn attribute_source_mut(&mut self) -> &mut AttributeSource {
            &mut self.source
        }
    }

    // -- Fixtures ----------------------------------------------------------

    fn ensure_codec() -> Arc<dyn Codec> {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        crate::codecs::default_codec().expect("Lucene104 codec is registered")
    }

    fn config() -> Arc<LiveIndexWriterConfig> {
        ensure_codec();
        Arc::new(LiveIndexWriterConfig::new(
            Arc::new(StandardAnalyzer::new()),
        ))
    }

    fn field_type(options: IndexOptions) -> FieldType {
        let mut field_type = FieldType::new();
        field_type.set_tokenized(true).expect("tokenized");
        field_type.set_omit_norms(true).expect("omit norms");
        field_type
            .set_index_options(options)
            .expect("index options");
        field_type.freeze();
        field_type
    }

    fn scripted_field(
        name: &str,
        options: IndexOptions,
        tokens: Vec<Tok>,
    ) -> Box<dyn IndexableField> {
        let stream: Rc<RefCell<dyn TokenStream>> =
            Rc::new(RefCell::new(ScriptedTokenStream::new(tokens)));
        Box::new(
            Field::new_with_token_stream(name, stream, field_type(options))
                .expect("token stream field"),
        )
    }

    fn builder() -> FieldInfosBuilder {
        FieldInfosBuilder::new(Arc::new(
            FieldNumbers::new(None, None).expect("field numbers"),
        ))
    }

    // -- FieldInvertState --------------------------------------------------

    #[test]
    fn reset_puts_position_before_the_first_token() {
        let mut state = FieldInvertState::new(10, "body".to_string(), IndexOptions::DOCS);
        state.set_length(7);
        state.set_num_overlap(3);
        state.set_max_term_frequency(9);
        state.increment_unique_term_count();
        state.reset();
        assert_eq!(state.position(), -1, "a first increment of one lands on 0");
        assert_eq!(state.length(), 0);
        assert_eq!(state.num_overlap(), 0);
        assert_eq!(state.offset(), 0);
        assert_eq!(state.max_term_frequency(), 0);
        assert_eq!(state.unique_term_count(), 0);
        assert_eq!(state.last_start_offset(), 0);
        assert_eq!(state.last_position(), 0);
    }

    #[test]
    fn invert_state_statistics_match_the_java_definitions() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            vec![
                Tok::new("the", 1, 0, 3),
                Tok::new("quick", 1, 4, 9),
                // A synonym stacked on "quick": position increment zero.
                Tok::new("fast", 0, 4, 9),
                Tok::new("the", 1, 10, 13),
                Tok::new("fox", 1, 14, 17),
            ],
        ));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");

        let state = chain.field_invert_state("body").expect("invert state");
        assert_eq!(state.length(), 5, "one per token, including the overlap");
        assert_eq!(state.num_overlap(), 1, "one token had a zero increment");
        assert_eq!(state.unique_term_count(), 4, "the, quick, fast, fox");
        assert_eq!(state.max_term_frequency(), 2, "\"the\" occurs twice");
        assert_eq!(
            state.position(),
            3,
            "the stacked synonym does not advance, and end() adds a zero increment"
        );
        assert_eq!(
            state.offset(),
            18,
            "end() advances the accumulator to 17, then the offset gap adds one"
        );
        assert_eq!(state.name(), "body");
        assert_eq!(
            state.index_options(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
        );
    }

    #[test]
    fn a_multi_valued_field_accumulates_statistics_across_its_values() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        for _ in 0..2 {
            document.add(scripted_field(
                "body",
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
                vec![Tok::new("alpha", 1, 0, 5), Tok::new("beta", 1, 6, 10)],
            ));
        }
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");

        let state = chain.field_invert_state("body").expect("invert state");
        assert_eq!(state.length(), 4, "both values are counted");
        assert_eq!(state.unique_term_count(), 2, "alpha and beta");
        assert_eq!(state.max_term_frequency(), 2);
        // The default analyzer gaps are 0 (positions) and 1 (offsets); each
        // value contributes its final offset of 10 plus the gap, so the
        // accumulator ends at 2 * (10 + 1).
        assert_eq!(state.offset(), 22);
    }

    #[test]
    fn a_binary_field_reports_the_length_lucene_reports() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(Box::new(
            StringField::new("id", "abc".to_string(), Store::NO).expect("string field"),
        ));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");

        let state = chain.field_invert_state("id").expect("invert state");
        // Lucene 10.5.0 increments `length` twice in `IndexingChain.invertTerm`;
        // see the comment in `DefaultIndexingChain::invert_term`. This test
        // pins the upstream behaviour so a future "cleanup" cannot silently
        // change what `Similarity.computeNorm` observes.
        assert_eq!(state.length(), 2);
        assert_eq!(state.position(), 0);
        assert_eq!(state.unique_term_count(), 1);
        assert_eq!(chain.field_term_count("id"), 1);
    }

    // -- Validation --------------------------------------------------------

    #[test]
    fn a_negative_first_position_increment_is_rejected() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            vec![Tok::new("alpha", 0, 0, 5)],
        ));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("must be rejected");
        assert!(
            format!("{error}").contains("first position increment must be > 0"),
            "{error}"
        );
        assert!(
            chain.take_aborting_error().is_none(),
            "a bad document must not poison the whole DWPT"
        );
    }

    #[test]
    fn offsets_going_backwards_are_rejected() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            vec![Tok::new("alpha", 1, 10, 15), Tok::new("beta", 1, 4, 9)],
        ));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("must be rejected");
        assert!(
            format!("{error}").contains("offsets must not go backwards"),
            "{error}"
        );
    }

    // -- Field infos -------------------------------------------------------

    #[test]
    fn a_field_that_saw_a_payload_is_marked_in_the_field_infos() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            vec![Tok::new("alpha", 1, 0, 5).with_payload(&[7, 7])],
        ));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");
        assert!(
            field_infos
                .field_info("body")
                .expect("field info")
                .has_payloads(),
            "storePayloads must reach the segment's field infos"
        );
    }

    #[test]
    fn a_field_without_payloads_is_not_marked() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");
        assert!(!field_infos
            .field_info("body")
            .expect("field info")
            .has_payloads());
    }

    #[test]
    fn field_numbers_are_assigned_in_first_seen_order_and_reused() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut first = Document::new();
        first.add(scripted_field(
            "alpha",
            IndexOptions::DOCS,
            vec![Tok::new("a", 1, 0, 1)],
        ));
        first.add(scripted_field(
            "beta",
            IndexOptions::DOCS,
            vec![Tok::new("b", 1, 0, 1)],
        ));
        chain
            .process_document(0, &first, true, &mut field_infos)
            .expect("doc 0");

        let mut second = Document::new();
        second.add(scripted_field(
            "beta",
            IndexOptions::DOCS,
            vec![Tok::new("c", 1, 0, 1)],
        ));
        chain
            .process_document(1, &second, true, &mut field_infos)
            .expect("doc 1");

        assert_eq!(
            field_infos.field_info("alpha").unwrap().get_field_number(),
            0
        );
        assert_eq!(
            field_infos.field_info("beta").unwrap().get_field_number(),
            1
        );
    }

    #[test]
    fn a_document_without_indexed_fields_buffers_nothing() {
        let (mut chain, _tracking, _info) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(Box::new(
            TextField::new("body", "ignored".to_string(), Store::YES).expect("text field"),
        ));
        // A stored-only field: index options NONE.
        let mut stored_type = FieldType::new();
        stored_type.set_stored(true).expect("stored");
        stored_type.freeze();
        let mut only_stored = Document::new();
        only_stored.add(Box::new(
            Field::new("meta", "value".to_string(), stored_type).expect("stored field"),
        ));
        chain
            .process_document(0, &only_stored, true, &mut field_infos)
            .expect("process document");
        assert_eq!(chain.field_term_count("meta"), 0);
        assert_eq!(
            chain
                .field_invert_state("meta")
                .expect("a per-field entry exists for every field")
                .length(),
            0,
            "a field that is not indexed is never inverted"
        );
        drop(document);
    }

    // -- Flush -------------------------------------------------------------

    /// Builds a chain bound to a fresh single-segment in-memory directory.
    ///
    /// Mirrors what the `DocumentsWriterPerThread` does: the tracking wrapper
    /// the chain writes through is the one whose file list becomes the
    /// segment's, and the `SegmentInfo` handed to `flush` is a *different*
    /// object from the one the chain was bound to.
    fn bound_chain(
        max_doc: i32,
    ) -> (
        DefaultIndexingChain,
        Arc<TrackingDirectoryWrapper>,
        SegmentInfo,
    ) {
        let codec = ensure_codec();
        // Reading the segment back goes through the same tracking wrapper, so
        // there is exactly one underlying directory.
        let tracking = Arc::new(TrackingDirectoryWrapper::new(Box::new(
            ByteBuffersDirectory::new(),
        )));
        let directory: Arc<dyn Directory> = Arc::clone(&tracking) as Arc<dyn Directory>;
        let make_info = |max_doc| {
            SegmentInfo::new(
                Arc::clone(&directory),
                Version::LATEST,
                Some(Version::LATEST),
                "_0".to_string(),
                max_doc,
                false,
                false,
                Arc::clone(&codec),
                HashMap::new(),
                [7u8; 16],
                HashMap::new(),
                Default::default(),
            )
            .expect("segment info")
        };
        // The DWPT binds the chain while `maxDoc` is still unset.
        let indexing_info = make_info(-1);
        let chain =
            DefaultIndexingChain::new_for_segment(config(), Arc::clone(&tracking), &indexing_info)
                .expect("bind segment");
        (chain, tracking, make_info(max_doc))
    }

    /// Indexes `documents` and flushes them into a fresh in-memory directory.
    fn flush_documents(
        documents: Vec<Document>,
        delete_terms: &[TermDelete],
    ) -> (Vec<String>, IndexingChainFlushResult) {
        let (files, result, _, _) = flush_documents_with_segment(documents, delete_terms);
        (files, result)
    }

    /// Like [`flush_documents`], but also returns the flushed segment and the
    /// directory it lives in, so a test can read the segment back.
    fn flush_documents_with_segment(
        documents: Vec<Document>,
        delete_terms: &[TermDelete],
    ) -> (
        Vec<String>,
        IndexingChainFlushResult,
        SegmentInfo,
        Arc<TrackingDirectoryWrapper>,
    ) {
        let max_doc = documents.len() as i32;
        let (mut chain, tracking, segment_info) = bound_chain(max_doc);
        let mut field_infos = builder();
        for (doc_id, document) in documents.iter().enumerate() {
            chain
                .process_document(doc_id as i32, document, true, &mut field_infos)
                .expect("process document");
        }
        let finished = field_infos.finish().expect("field infos");

        let info_stream = NoOutputInfoStream;
        let context = flush_io_context(FlushInfo::new(max_doc, 0));
        let state = IndexingChainFlushState {
            info_stream: &info_stream,
            directory: &tracking,
            segment_info: &segment_info,
            field_infos: &finished,
            context: context.as_ref(),
            live_docs: None,
            del_count_on_flush: 0,
            delete_terms,
        };
        let result = chain.flush(&state).expect("flush");
        let mut files: Vec<String> = tracking.get_created_files().into_iter().collect();
        files.sort();
        (files, result, segment_info, tracking)
    }

    #[test]
    fn flushing_writes_the_postings_files_of_the_segment() {
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            vec![Tok::new("alpha", 1, 0, 5), Tok::new("beta", 1, 6, 10)],
        ));
        let (files, _) = flush_documents(vec![document], &[]);
        for extension in ["doc", "pos", "tim", "tip", "tmd"] {
            assert!(
                files
                    .iter()
                    .any(|name| name.ends_with(&format!(".{extension}"))),
                "the flush must create a .{extension} file, got {files:?}"
            );
        }
    }

    #[test]
    fn flushing_a_segment_without_postings_writes_only_the_stored_fields() {
        let mut stored_type = FieldType::new();
        stored_type.set_stored(true).expect("stored");
        stored_type.freeze();
        let mut document = Document::new();
        document.add(Box::new(
            Field::new("meta", "value".to_string(), stored_type).expect("stored field"),
        ));
        let (files, result) = flush_documents(vec![document], &[]);
        let mut extensions: Vec<String> = files
            .iter()
            .filter_map(|name| name.rsplit_once('.').map(|(_, ext)| ext.to_string()))
            .collect();
        extensions.sort();
        assert_eq!(
            extensions,
            vec!["fdm".to_string(), "fdt".to_string(), "fdx".to_string()],
            "a stored-only document produces the stored-fields files and no postings: {files:?}"
        );
        assert_eq!(result.del_count_on_flush, 0);
        assert!(result.live_docs.is_none());
    }

    #[test]
    fn flushing_applies_the_segment_private_delete_terms() {
        let mut documents = Vec::new();
        for index in 0..4 {
            let mut document = Document::new();
            document.add(scripted_field(
                "body",
                IndexOptions::DOCS,
                vec![Tok::new("alpha", 1, 0, 5)],
            ));
            document.add(scripted_field(
                "id",
                IndexOptions::DOCS,
                vec![Tok::new(&format!("id{index}"), 1, 0, 3)],
            ));
            documents.push(document);
        }
        let deletes = vec![TermDelete {
            term: Term::new("body", BytesRef::new(b"alpha".to_vec())),
            doc_id_upto: 2,
        }];
        let (_, result) = flush_documents(documents, &deletes);
        assert_eq!(
            result.del_count_on_flush, 2,
            "docs 0 and 1 are below docIDUpto"
        );
        let live_docs = result.live_docs.expect("live docs");
        assert!(!live_docs.get(0) && !live_docs.get(1));
        assert!(live_docs.get(2) && live_docs.get(3));
    }

    // -- Stored fields ------------------------------------------------------

    /// Builds a stored-and-indexed text field.
    fn stored_text_field(name: &str, value: &str) -> Box<dyn crate::index::IndexableField> {
        Box::new(TextField::new(name, value.to_string(), Store::YES).expect("text field"))
    }

    /// Reads every document of the flushed segment back as a `Document`.
    fn read_back(
        segment_info: &SegmentInfo,
        directory: &TrackingDirectoryWrapper,
        field_infos: &crate::index::FieldInfos,
    ) -> Vec<Vec<(String, String)>> {
        let reader = ensure_codec()
            .stored_fields_format()
            .fields_reader(
                directory,
                segment_info,
                field_infos,
                &*crate::store::DEFAULT_IO_CONTEXT,
            )
            .expect("stored fields reader");
        (0..segment_info.max_doc().expect("max doc"))
            .map(|doc_id| {
                let mut visitor = crate::document::DocumentStoredFieldVisitor::new();
                reader.document(doc_id, &mut visitor).expect("document");
                visitor
                    .into_document()
                    .into_fields()
                    .iter()
                    .map(|field| {
                        (
                            field.name().to_string(),
                            field
                                .string_value()
                                .or_else(|| field.numeric_value().map(|v| v.to_string()))
                                .unwrap_or_else(|| {
                                    format!("{:?}", field.binary_value().expect("a value"))
                                }),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn stored_fields_written_by_the_chain_come_back_from_the_segment() {
        let mut documents = Vec::new();
        for index in 0..3 {
            let mut document = Document::new();
            document.add(stored_text_field("title", &format!("title {index}")));
            document.add(Box::new(IntField::new("num", index, Store::YES)));
            document.add(Box::new(
                StoredField::new_bytes("blob", BytesRef::new(vec![index as u8, 0xFF]))
                    .expect("stored bytes"),
            ));
            documents.push(document);
        }

        let max_doc = documents.len() as i32;
        let (mut chain, tracking, segment_info) = bound_chain(max_doc);
        let mut field_infos = builder();
        for (doc_id, document) in documents.iter().enumerate() {
            chain
                .process_document(doc_id as i32, document, true, &mut field_infos)
                .expect("process document");
        }
        let finished = field_infos.finish().expect("field infos");
        let info_stream = NoOutputInfoStream;
        let context = flush_io_context(FlushInfo::new(max_doc, 0));
        let state = IndexingChainFlushState {
            info_stream: &info_stream,
            directory: &tracking,
            segment_info: &segment_info,
            field_infos: &finished,
            context: context.as_ref(),
            live_docs: None,
            del_count_on_flush: 0,
            delete_terms: &[],
        };
        chain.flush(&state).expect("flush");

        let documents = read_back(&segment_info, &tracking, &finished);
        assert_eq!(
            documents,
            vec![
                vec![
                    ("title".to_string(), "title 0".to_string()),
                    ("num".to_string(), "0".to_string()),
                    (
                        "blob".to_string(),
                        format!("{:?}", BytesRef::new(vec![0, 0xFF]))
                    ),
                ],
                vec![
                    ("title".to_string(), "title 1".to_string()),
                    ("num".to_string(), "1".to_string()),
                    (
                        "blob".to_string(),
                        format!("{:?}", BytesRef::new(vec![1, 0xFF]))
                    ),
                ],
                vec![
                    ("title".to_string(), "title 2".to_string()),
                    ("num".to_string(), "2".to_string()),
                    (
                        "blob".to_string(),
                        format!("{:?}", BytesRef::new(vec![2, 0xFF]))
                    ),
                ],
            ],
            "the stored values must come back in the order the document declared them"
        );
    }

    #[test]
    fn a_document_that_stores_nothing_keeps_its_doc_id() {
        let mut documents = Vec::new();
        for index in 0..4 {
            let mut document = Document::new();
            if index % 2 == 0 {
                document.add(stored_text_field("title", &format!("doc {index}")));
            } else {
                // Indexed but not stored: the frame must still be written.
                document.add(scripted_field(
                    "body",
                    IndexOptions::DOCS,
                    vec![Tok::new("alpha", 1, 0, 5)],
                ));
            }
            documents.push(document);
        }
        let max_doc = documents.len() as i32;
        let (mut chain, tracking, segment_info) = bound_chain(max_doc);
        let mut field_infos = builder();
        for (doc_id, document) in documents.iter().enumerate() {
            chain
                .process_document(doc_id as i32, document, true, &mut field_infos)
                .expect("process document");
        }
        let finished = field_infos.finish().expect("field infos");
        let info_stream = NoOutputInfoStream;
        let context = flush_io_context(FlushInfo::new(max_doc, 0));
        let state = IndexingChainFlushState {
            info_stream: &info_stream,
            directory: &tracking,
            segment_info: &segment_info,
            field_infos: &finished,
            context: context.as_ref(),
            live_docs: None,
            del_count_on_flush: 0,
            delete_terms: &[],
        };
        chain.flush(&state).expect("flush");

        assert_eq!(
            read_back(&segment_info, &tracking, &finished),
            vec![
                vec![("title".to_string(), "doc 0".to_string())],
                Vec::new(),
                vec![("title".to_string(), "doc 2".to_string())],
                Vec::new(),
            ]
        );
    }

    #[test]
    fn a_flush_writes_the_stored_field_files_even_when_nothing_was_stored() {
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        let (files, _) = flush_documents(vec![document], &[]);
        for extension in ["fdt", "fdx", "fdm"] {
            assert!(
                files
                    .iter()
                    .any(|name| name.ends_with(&format!(".{extension}"))),
                "Lucene frames every document, so a segment always has a .{extension}: {files:?}"
            );
        }
    }

    #[test]
    fn the_stored_field_files_are_released_on_abort() {
        let (mut chain, _tracking, _info) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(stored_text_field("title", "doomed"));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");
        assert!(
            chain.stored_fields_consumer().expect("bound").has_writer(),
            "indexing a document opens the stored-fields writer"
        );
        chain.abort();
        assert!(
            !chain.stored_fields_consumer().expect("bound").has_writer(),
            "abort must release the stored-fields writer"
        );
    }

    #[test]
    fn binding_a_second_segment_is_rejected() {
        let (mut chain, tracking, info) = bound_chain(1);
        let error = IndexingChain::bind_segment(&mut chain, tracking, &info)
            .expect_err("a chain writes exactly one segment");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn an_unbound_chain_refuses_to_drop_a_stored_field() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(stored_text_field("title", "value"));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("silently dropping stored data would corrupt the segment");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn aborting_releases_every_accounted_byte() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        for doc_id in 0..20 {
            let mut document = Document::new();
            document.add(scripted_field(
                "body",
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                vec![Tok::new(&format!("term{doc_id}"), 1, 0, 5)],
            ));
            chain
                .process_document(doc_id, &document, true, &mut field_infos)
                .expect("process document");
        }
        assert!(chain.ram_bytes_used() > 0);
        chain.abort();
        assert_eq!(chain.ram_bytes_used(), 0);
        assert!(chain.field_invert_state("body").is_none());
    }
}
