//! Per-segment indexing pipeline ported from `org.apache.lucene.index`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`FieldInvertState`] | `FieldInvertState` |
//! | [`DefaultIndexingChain`] | `IndexingChain` |
//! | `PerField` (private) | `IndexingChain.PerField` |
//! | [`EmptyNormsProducer`] | the `NormsProducer` the chain passes to the codec |
//!
//! The chain's `EmptyNormsProducer` is a
//! [`NormsProducer`](crate::codecs::norms::NormsProducer) that reports a norm
//! of one for every document; see its documentation for the divergence.
//!
//! The chain turns a [`Document`] into buffered postings, a stored-fields
//! stream and a term-vectors stream: it registers each field in the segment's
//! [`FieldInfosBuilder`], runs the analysis pipeline, validates positions and
//! offsets, feeds every token to [`FreqProxTermsWriter`] and — for the fields
//! that ask for them — to the [`TermVectorsConsumer`], and hands every stored
//! value to the [`StoredFieldsConsumer`]. At flush time it streams the buffered
//! postings through the codec's postings format, producing the `.doc`, `.pos`,
//! `.pay`, `.tim`, `.tip` and `.tmd` files of the segment, finishes the
//! stored-fields stream, producing `.fdt`, `.fdx` and `.fdm`, and finishes the
//! term-vectors stream, producing `.tvd`, `.tvx` and `.tvm`.
//!
//! # The two term hashes
//!
//! Lucene chains `FreqProxTermsWriter` and `TermVectorsConsumer` as two
//! `TermsHash` instances and lets `TermsHashPerField.add` forward each token
//! from the first to the second. This port makes the chain explicit: the token
//! loop feeds [`FreqProxTermsWriter`], takes back the pool offset the token was
//! interned at, and forwards that offset to the [`TermVectorsConsumer`] when the
//! field asked for vectors — which is precisely the `nextPerField.add(int, int)`
//! call Lucene makes.
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
//! Lucene's `IndexingChain` also drives doc values, points, vectors and norms.
//! Those consumers are separate ports; this chain implements the
//! inverted-index, stored-fields and term-vectors paths, so a flushed segment
//! currently contains its postings files, its stored-fields files and — when a
//! field asked for them — its term-vectors files, and nothing else.
//!
//! Lucene picks `SortingStoredFieldsConsumer` and `SortingTermVectorsConsumer`
//! when the segment has an index sort. Index sorting is a separate port, so this
//! chain always uses the plain [`StoredFieldsConsumer`] and
//! [`TermVectorsConsumer`]; [`DefaultIndexingChain::bind_segment`] is the one
//! place both choices will be made.

#![deny(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crate::analysis::tokenattributes::{
    BytesTermAttributeImpl, CharTermAttributeImpl, OffsetAttribute, OffsetAttributeImpl,
    PackedTokenAttributeImpl, PayloadAttribute, PayloadAttributeImpl, PositionIncrementAttribute,
    PositionIncrementAttributeImpl, TermFrequencyAttribute, TermFrequencyAttributeImpl,
    TermToBytesRefAttribute,
};
use crate::analysis::Analyzer;
use crate::analysis::TokenStream;
use crate::codecs::doc_values::DocValuesConsumer;
use crate::codecs::knn_vectors::FieldVectorWriter;
use crate::codecs::norms::NormsProducer;
use crate::codecs::points::PointsWriter;
use crate::codecs::state::SegmentWriteState;
use crate::document::column::{
    create_column_field_adapter, BinaryColumn, ColumnBatch, ColumnFieldAdapter, ColumnValidation,
    Density, DictionaryColumn, LongColumn, NumericKind, TypedColumn, VectorColumn,
};
use crate::document::{Document, InvertableType, NumericDocValuesField, StoredValueType};
use crate::error::{LuceneError, Result};
use crate::index::doc_values_writer::DocValuesWriter;
use crate::index::documents_writer::{
    IndexingChain, IndexingChainFlushResult, IndexingChainFlushState, SharedIndexingScratch,
    MAX_STORED_STRING_LENGTH,
};
use crate::index::field_infos::FieldInfosBuilder;
use crate::index::freq_prox_terms_writer::{FreqProxTermsWriter, InvertedToken};
use crate::index::index_sorter::{Sorter, SorterDocMap};
use crate::index::index_writer_config::LiveIndexWriterConfig;
use crate::index::norms_writer::NormValuesWriter;
use crate::index::point_values_writer::PointValuesWriter;
use crate::index::stored_fields_consumer::StoredFieldsConsumer;
use crate::index::term_vectors_consumer::TermVectorsConsumer;
use crate::index::vector_values_consumer::VectorValuesConsumer;
use crate::index::{
    DocValuesType, FieldInfo, IndexOptions, IndexableField, IndexableFieldType, SegmentInfo,
};
use crate::index::{NumericValue, StoredValue};
use crate::search::doc_values_iteration::{
    binary_as_iterator, numeric_as_iterator, sorted_as_iterator, sorted_numeric_as_iterator,
    sorted_set_as_iterator,
};
use crate::search::Similarity;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::TrackingDirectoryWrapper;
use crate::util::byte_block_pool::MAX_TERM_LENGTH;
use crate::util::java_collections::JavaStringMap;
use crate::util::NumericUtils;
use crate::util::{Accountable, AttributeSource, BytesRef, InfoStream};

/// Highest position a token may occupy.
///
/// Equivalent to `IndexWriter.MAX_POSITION`.
pub const MAX_POSITION: i32 = i32::MAX - 128;

/// Computes the hash bucket Lucene's open `PerField[] fieldHash` uses for
/// `name`, exactly `fieldName.hashCode() & hashMask`: Java hashes the string's
/// UTF-16 code units with `h = 31*h + unit`, in 32-bit wrapping arithmetic.
fn java_string_hashcode(name: &str) -> i32 {
    name.encode_utf16().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}

/// Replays Lucene's `PerField[] fieldHash` over the per-field state and
/// returns the per-field indices in the order the flush loops visit them.
///
/// `IndexingChain` starts with two buckets (`fieldHash = new PerField[2],
/// hashMask = 1`), chains a new field at the *head* of its bucket
/// (`IndexingChain.java:1466-1467`), doubles the table once
/// `totalFieldCount >= fieldHash.length / 2` and rehashes by walking the old
/// buckets in order, head to tail, re-inserting each field at the head of its
/// new bucket — which reverses the within-bucket order
/// (`IndexingChain.java:546-566`).
///
/// Both `writeDocValues` (`IndexingChain.java:439-497`) and `writePoints`
/// (`IndexingChain.java:396-435`) walk that same table, buckets in order and
/// each chain head to tail, so one function serves both. The resulting
/// sequence fixes the order of the per-field entries inside the `.dvm` and the
/// `.kdm`. `writeNorms` is different: it iterates the field infos, so norms
/// are in field-number order.
fn field_hash_flush_order(fields: &[PerField]) -> Vec<usize> {
    let names: Vec<&str> = fields
        .iter()
        .map(|per_field| per_field.field_info.name.as_str())
        .collect();
    field_hash_order(&names)
}

/// The core of [`field_hash_flush_order`], over the field names alone.
///
/// Split out so the order can be asserted directly: it depends on nothing but
/// the names and the sequence they were first seen in, and getting it wrong
/// reorders the metadata entries of the `.dvm` without changing a single value.
fn field_hash_order(names: &[&str]) -> Vec<usize> {
    let mut table: Vec<Vec<usize>> = vec![Vec::new(); 2];
    let mut total_field_count = 0usize;
    for (index, name) in names.iter().enumerate() {
        // `hashMask` is re-read from the live table on every insert, because
        // `rehash` replaces both the array and the mask
        // (`IndexingChain.java:565-566`). Holding the initial mask across a
        // growth would bucket every later field as if the table were still two
        // wide, which changes the order the entries reach the `.dvm`.
        let mask = table.len() - 1;
        table[java_string_hashcode(name) as u32 as usize & mask].insert(0, index);
        total_field_count += 1;
        // At most 50% load factor: grow *after* the insert, as Lucene does.
        if total_field_count >= table.len() / 2 {
            table = rehash(&table, names);
        }
    }
    table.into_iter().flatten().collect()
}

/// Doubles the field-hash table and re-inserts every field into its new bucket.
///
/// Equivalent to `IndexingChain.rehash()` (`IndexingChain.java:546-566`): the
/// old buckets are walked in order, head to tail, and each field is chained at
/// the *head* of its new bucket, which reverses the within-bucket order. Java
/// mutates `fieldHash` and `hashMask` in place; here the new table is returned
/// and its mask is read back off its length, which is the same pair of values.
fn rehash(table: &[Vec<usize>], names: &[&str]) -> Vec<Vec<usize>> {
    let new_hash_size = table.len() * 2;
    debug_assert!(new_hash_size > table.len());
    let mut new_hash_array: Vec<Vec<usize>> = vec![Vec::new(); new_hash_size];
    let new_hash_mask = new_hash_size - 1;
    for chain in table {
        for &index in chain {
            new_hash_array[java_string_hashcode(names[index]) as u32 as usize & new_hash_mask]
                .insert(0, index);
        }
    }
    new_hash_array
}

/// How many bytes of an over-long term are shown in the error message.
///
/// Equivalent to the literal `30` both
/// `ArrayUtil.copyOfSubArray(bigTerm.bytes, bigTerm.offset, bigTerm.offset + 30)`
/// calls in `IndexingChain` use.
const IMMENSE_TERM_PREFIX: usize = 30;

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

/// Norms source the flush hands to the postings format.
///
/// The Lucene 10.4 postings format reads norms only to compute the impact
/// blocks that accompany skip data. Java hands the flush a producer over the
/// norms it has just written (`IndexingChain.java:361-370`); this one reports
/// a norm of one for every document instead, so a flushed segment's impacts
/// for a field with norms differ from Lucene's (see the call site in
/// `flush`).
#[derive(Debug, Clone, Copy)]
pub struct EmptyNormsProducer {
    max_doc: i32,
}

impl EmptyNormsProducer {
    /// Creates the producer for a segment of `max_doc` documents.
    pub fn new(max_doc: i32) -> Self {
        Self { max_doc }
    }
}

/// Dense norms that report a norm of one for every document.
#[derive(Debug, Clone, Copy)]
struct ConstantNorms {
    max_doc: i32,
    doc: i32,
}

impl DocIdSetIterator for ConstantNorms {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = if target >= self.max_doc {
            NO_MORE_DOCS
        } else {
            target
        };
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        i64::from(self.max_doc)
    }
}

impl crate::index::doc_values::DocValuesIterator for ConstantNorms {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        Ok(target < self.max_doc)
    }
}

impl crate::index::NumericDocValues for ConstantNorms {
    fn long_value(&self) -> Result<i64> {
        Ok(1)
    }
}

impl NormsProducer for EmptyNormsProducer {
    fn get_norms(
        &self,
        _field_info: &FieldInfo,
    ) -> Result<Box<dyn crate::index::NumericDocValues>> {
        Ok(Box::new(ConstantNorms {
            max_doc: self.max_doc,
            doc: -1,
        }))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(*self))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
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
    /// Index of this field's table inside the [`TermVectorsConsumer`], or
    /// `None` for a field that is not indexed or for a chain that is not bound
    /// to a segment yet.
    ///
    /// Lucene creates it in the same `PerField.setInvertState()` call, through
    /// `TermsHash.addField`, which forwards to the next hash of the chain.
    vectors_index: Option<usize>,
    /// Whether the current field instance's tokens must also reach the
    /// term-vectors consumer. Equivalent to `TermsHashPerField.doNextCall`,
    /// which Lucene sets from the return value of the next hash's `start`.
    do_vectors: bool,
    /// Generation of the document this field was last seen in.
    field_gen: i64,
    /// `true` until the first instance of this field is inverted in the current
    /// document; multi-valued fields see it `false` from the second value on.
    first: bool,
    /// Buffered norms of this field, or `None` when the field omits norms or is
    /// not indexed.
    ///
    /// Lucene creates it in `PerField.setInvertState()`, guarded by
    /// `fieldInfo.omitsNorms() == false` (`IndexingChain.java:1837-1841`), and
    /// keeps it for the whole segment.
    norms: Option<NormValuesWriter>,
    /// Buffered doc values of this field, or `None` when the field declares no
    /// doc-values type.
    ///
    /// Lucene creates it in `PerField.setFieldInfo()` — the `DocValuesType`
    /// switch of `initializeFieldInfo` (`IndexingChain.java:1351-1369`) — and
    /// keeps it for the whole segment.
    doc_values: Option<DocValuesWriter>,
    /// Buffered point values of this field, or `None` when the field declares
    /// no point dimensions.
    ///
    /// Lucene creates it in `initializeFieldInfo` when
    /// `fi.getPointDimensionCount() != 0` (`IndexingChain.java:1372-1374`) and
    /// keeps it for the whole segment.
    point_values: Option<PointValuesWriter>,
    /// The codec's per-field vectors writer, or `None` when the field declares
    /// no vector dimensions.
    ///
    /// Equivalent to `IndexingChain.PerField.knnFieldVectorsWriter`, which
    /// Lucene obtains from `vectorValuesConsumer.addField(fi)` in
    /// `initializeFieldInfo` (`IndexingChain.java:1375-1382`). Unlike the norms,
    /// doc-values and point buffers beside it, this one is **not** a buffer this
    /// crate owns: it is a handle into the codec's vectors writer, which does
    /// the buffering itself.
    knn_field_vectors_writer: Option<FieldVectorWriter>,
    /// Which indexing features the columns of the current batch have claimed
    /// for this field.
    ///
    /// Equivalent to `IndexingChain.PerField.columnFeatures`, which
    /// `processBatch` uses to refuse two columns of one field claiming the same
    /// feature.
    column_features: u8,
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
    /// The term-vectors half of the chain, present once the chain knows which
    /// segment it writes. Equivalent to `IndexingChain.termVectorsWriter`,
    /// which Lucene builds in the constructor because it already has the
    /// directory and the `SegmentInfo` there.
    term_vectors_consumer: Option<TermVectorsConsumer>,
    /// The KNN-vectors half of the chain, present once the chain knows which
    /// segment it writes. Equivalent to `IndexingChain.vectorValuesConsumer`,
    /// which Lucene builds in the constructor (`IndexingChain.java:134-135`).
    vector_values_consumer: Option<VectorValuesConsumer>,
    /// The reserved field that marks the last document of a block, present when
    /// `IndexWriterConfig.setParentField` names one.
    ///
    /// Equivalent to `IndexingChain.parentField`, which Lucene builds in the
    /// constructor as `new NumericDocValuesField(parentFieldName, -1)`
    /// (`IndexingChain.java:164`).
    parent_field: Option<NumericDocValuesField>,
    /// Index into [`per_fields`](Self::per_fields) of the parent field's
    /// per-field state, created the first time a document actually needs it.
    ///
    /// Equivalent to `IndexingChain.parentPf`.
    parent_pf: Option<usize>,
}

/// Presents a column's name and field type as an
/// [`IndexableField`](crate::index::IndexableField), so that `describe_field`
/// can build the column's `FieldInfo`.
///
/// Java's `processBatch` reads `column.name()` and `column.fieldType()`
/// straight off the column and hands them to `initializeFieldInfo`, which takes
/// a `PerField`. This port's `describe_field` takes an `IndexableField`, as
/// `process_document` gives it, so the two members are carried in one.
struct ColumnFieldTypeCarrier {
    name: String,
    field_type: Arc<dyn IndexableFieldType>,
}

impl std::fmt::Debug for ColumnFieldTypeCarrier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnFieldTypeCarrier")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for ColumnFieldTypeCarrier {
    /// Port-only carrier with no Lucene counterpart: Java passes the column's
    /// name and field type as two arguments, so there is no object whose
    /// `toString()` could be reproduced. It renders in the inherited
    /// `Field.toString()` shape with an empty value, which is what a `Field`
    /// carrying no `fieldsData` renders as.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}<{}:>", self.field_type, self.name)
    }
}

impl IndexableField for ColumnFieldTypeCarrier {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.field_type.as_ref()
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        unreachable!("INVARIANT: the carrier only ever answers name() and field_type()")
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn std::io::Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

impl DefaultIndexingChain {
    /// Creates a chain bound to the given live configuration.
    pub fn new(config: Arc<LiveIndexWriterConfig>) -> Self {
        let bytes_used = Arc::new(AtomicI64::new(0));
        // `IndexingChain.java:163-167` also builds the parent field's `PerField`
        // and seeds its schema here; this port defers that to the first document
        // — see `get_or_add_parent_per_field`.
        let parent_field = config
            .parent_field()
            .map(|name| NumericDocValuesField::new(name, -1));
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
            term_vectors_consumer: None,
            vector_values_consumer: None,
            parent_field,
            parent_pf: None,
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

    /// Returns the term-vectors consumer, if the chain is bound to a segment.
    pub fn term_vectors_consumer(&self) -> Option<&TermVectorsConsumer> {
        self.term_vectors_consumer.as_ref()
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
            self.on_aborting_exception(LuceneError::CorruptIndex(format!(
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
            self.on_aborting_exception(LuceneError::CorruptIndex(format!(
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
            self.on_aborting_exception(LuceneError::CorruptIndex(format!(
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
            self.on_aborting_exception(LuceneError::CorruptIndex(format!(
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
    ///
    /// This is the body of `IndexingChain.initializeFieldInfo`
    /// (`IndexingChain.java:1302-1384`) that runs once per field per segment:
    /// Lucene reaches it from the first pass of `processDocument`, for every
    /// field it has not seen in this segment yet, in **document field order**.
    ///
    /// # Errors
    ///
    /// Returns whatever
    /// [`VectorValuesConsumer::add_field`](crate::index::vector_values_consumer::VectorValuesConsumer::add_field)
    /// raises for a field that declares vector dimensions. Java wraps exactly
    /// that one call in `try { .. } catch (Throwable th) { onAbortingException(th); throw th; }`
    /// (`IndexingChain.java:1375-1381`): it is the only part of
    /// `initializeFieldInfo` that can leave a half-written file behind, because
    /// it is the only part that opens one.
    /// Records the failure that makes the whole segment untrustworthy.
    ///
    /// Equivalent to `IndexingChain.onAbortingException(Throwable)`, which sets
    /// `hasHitAbortingException` and hands the cause to the consumer the
    /// `DocumentsWriterPerThread` installed. This port keeps the first such
    /// failure and reports it at the end of the document, which is where the
    /// caller can act on it.
    fn on_aborting_exception(&mut self, error: LuceneError) {
        if self.aborting_error.is_none() {
            self.aborting_error = Some(error);
        }
    }

    /// Fails when a field the index sort names is indexed with the wrong
    /// doc-values type.
    ///
    /// Equivalent to `IndexingChain.validateIndexSortDVType(Sort, String, DocValuesType)`.
    /// Java asks each sorter for a comparator over a reader that answers only
    /// for the field being validated and throws from the accessor of the wrong
    /// kind; the check that reaches is exactly the one made directly here — the
    /// sort field's type must agree with the doc-values type the field carries.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when a sort field cannot sort an
    /// index at all, and [`LuceneError::IllegalArgument`] when it names this
    /// field with the wrong doc-values type.
    fn validate_index_sort_dv_type(
        index_sort: &crate::search::Sort,
        field_to_validate: &str,
        dv_type: DocValuesType,
    ) -> Result<()> {
        for sort_field in index_sort.fields() {
            if sort_field.get_index_sorter().is_none() {
                return Err(LuceneError::IllegalState(format!(
                    "Cannot sort index with sort order {sort_field:?}"
                )));
            }
            if sort_field.field() != Some(field_to_validate) {
                continue;
            }
            let expected = match sort_field.field_type() {
                crate::search::SortFieldType::String => DocValuesType::SORTED,
                crate::search::SortFieldType::Int
                | crate::search::SortFieldType::Long
                | crate::search::SortFieldType::Float
                | crate::search::SortFieldType::Double => DocValuesType::NUMERIC,
                _ => continue,
            };
            if dv_type != expected {
                return Err(LuceneError::IllegalArgument(format!(
                    "SortField {sort_field:?} expected field [{field_to_validate}] to be \
                     {expected:?} but it is [{dv_type:?}]"
                )));
            }
        }
        Ok(())
    }

    /// Returns a leaf reader over the doc values buffered so far, which is all
    /// the index sorter has to compare at flush time.
    ///
    /// Equivalent to `IndexingChain.getDocValuesLeafReader()`, whose anonymous
    /// `DocValuesLeafReader` answers each of the five doc-values kinds from the
    /// per-field writer, and `null` for a field that either does not exist or
    /// carries a different kind.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a writer's buffers raises.
    fn get_doc_values_leaf_reader(&self, max_doc: i32) -> Result<BufferedDocValuesReader> {
        let mut producers = HashMap::new();
        for per_field in &self.per_fields {
            let Some(writer) = per_field.doc_values.as_ref() else {
                continue;
            };
            producers.insert(
                per_field.field_info.name.clone(),
                (per_field.field_info.clone(), writer.get_doc_values()?),
            );
        }
        Ok(BufferedDocValuesReader { max_doc, producers })
    }

    /// Returns the document map that puts this segment in the configured index
    /// sort, or `None` when there is no sort or the documents already satisfy
    /// it.
    ///
    /// Equivalent to `IndexingChain.maybeSortSegment(SegmentWriteState)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java also wraps every comparator so
    /// that a segment holding document blocks compares each block by its parent
    /// document, and refuses a blocked segment that names no parent field. This
    /// port compares documents directly; the blocked case is refused outright
    /// rather than sorted wrongly, which is the safe half of Lucene's
    /// behaviour and never produces a differently ordered segment.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] when the sort names a
    /// field no [`IndexSorter`](crate::index::index_sorter::IndexSorter) can
    /// read, and [`LuceneError::CorruptIndex`] for a blocked segment.
    fn maybe_sort_segment(
        &self,
        segment_info: &SegmentInfo,
        field_infos: &crate::index::FieldInfos,
    ) -> Result<Option<SorterDocMap>> {
        let index_sort = segment_info.index_sort();
        // **Divergence from Lucene 10.5.0.** `SegmentInfo.getIndexSort()`
        // returns `null` for an unsorted segment; this port carries a `Sort`
        // there and encodes "unsorted" as the relevance sort, which is what
        // `Sort::default()` builds. An empty sort means the same thing.
        if index_sort.fields().is_empty() || *index_sort == crate::search::Sort::default() {
            return Ok(None);
        }
        if segment_info.get_has_blocks() {
            return Err(LuceneError::CorruptIndex(format!(
                "segment {} has document blocks and uses index sorting, which this port cannot \
                 order: the parent field is {}",
                segment_info.name,
                field_infos.get_parent_field().unwrap_or("not set")
            )));
        }

        let max_doc = segment_info.max_doc()?;
        let doc_values_reader = self.get_doc_values_leaf_reader(max_doc)?;
        let mut comparators = Vec::with_capacity(index_sort.fields().len());
        for sort_field in index_sort.fields() {
            let sorter = sort_field.get_index_sorter().ok_or_else(|| {
                LuceneError::UnsupportedOperation(format!(
                    "Cannot sort index using sort field {sort_field:?}"
                ))
            })?;
            comparators.push(sorter.get_doc_comparator(&doc_values_reader, max_doc)?);
        }
        // `Sorter.sort` answers `None` when the documents are already in order,
        // which is what spares an unsorted rewrite of a segment that needs none.
        Ok(Sorter::sort_multi(max_doc, comparators))
    }

    fn get_or_add_per_field(&mut self, field_info: &FieldInfo) -> Result<usize> {
        if let Some(index) = self.per_field_index.get(&field_info.name) {
            return Ok(*index);
        }
        let indexed = field_info.index_options != IndexOptions::NONE;
        let writer_index = indexed.then(|| self.terms_writer.add_field(field_info));
        // `PerField.setInvertState()` builds *both* per-field consumers through
        // `TermsHash.addField`, and tells the term-vectors consumer that the
        // segment has vectors as soon as a field says so
        // (`IndexingChain.java:1836-1845`).
        let vectors_index = match self.term_vectors_consumer.as_mut() {
            Some(consumer) if indexed => {
                let index = consumer.add_field(field_info);
                if field_info.has_term_vectors() {
                    consumer.set_has_vectors();
                }
                Some(index)
            }
            _ => None,
        };
        let invert_state = FieldInvertState::new(
            self.config.index_created_version_major(),
            field_info.name.clone(),
            field_info.index_options,
        );
        // `PerField.setInvertState()` creates the norms buffer for every
        // indexed field that does not omit norms, *before* any document is
        // seen: "Even if no documents actually succeed in setting a norm, we
        // still write norms for this segment"
        // (`IndexingChain.java:1837-1841`). `has_norms()` is exactly Java's
        // `indexOptions != NONE && omitNorms == false`.
        let norms = field_info
            .has_norms()
            .then(|| NormValuesWriter::new(field_info.clone(), Arc::clone(&self.bytes_used)));
        // `initializeFieldInfo`'s `DocValuesType` switch creates the doc-values
        // writer of every field that declares one, indexed or not
        // (`IndexingChain.java:1351-1369`); the `NONE` arm creates nothing and
        // the `default` arm is unreachable for a valid [`DocValuesType`].
        // Java validates the field against the index sort here, the first time
        // it is seen with a doc-values type: a sort that names a field indexed
        // with the wrong kind cannot order the segment, and finding that out at
        // flush would waste the whole segment
        // (`IndexingChain.java:1345-1350`).
        if field_info.doc_values_type != DocValuesType::NONE {
            if let Some(index_sort) = self.config.index_sort() {
                Self::validate_index_sort_dv_type(
                    index_sort,
                    &field_info.name,
                    field_info.doc_values_type,
                )?;
            }
        }
        let doc_values = match field_info.doc_values_type {
            DocValuesType::NONE => None,
            _ => Some(DocValuesWriter::new(
                field_info.clone(),
                Arc::clone(&self.bytes_used),
            )),
        };
        // `initializeFieldInfo` creates the point-values writer for every field
        // that declares point dimensions, indexed or not
        // (`IndexingChain.java:1372-1374`).
        let point_values = if field_info.point_dimension_count != 0 {
            Some(PointValuesWriter::new(
                field_info.clone(),
                Arc::clone(&self.bytes_used),
            ))
        } else {
            None
        };
        // `initializeFieldInfo` ends by asking the vectors consumer for this
        // field's writer, which creates the codec's vectors writer — and with
        // it the segment's vector files — the first time any field asks
        // (`IndexingChain.java:1375-1382`). A failure here is aborting: the
        // consumer may have created and half-written a `.vec`, `.vemf`, `.vex`
        // or `.vem` that only `abort()` can remove.
        let knn_field_vectors_writer = if field_info.vector_dimension != 0 {
            let Some(consumer) = self.vector_values_consumer.as_mut() else {
                // Without a bound segment there is nowhere to write vectors.
                // The term-vectors path raises the same kind of error for the
                // same reason.
                return Err(LuceneError::IllegalState(format!(
                    "the indexing chain is not bound to a segment, so the vectors of field \"{}\" cannot be written",
                    field_info.name
                )));
            };
            match consumer.add_field(field_info) {
                Ok(writer) => Some(writer),
                Err(error) => {
                    self.on_aborting_exception(LuceneError::CorruptIndex(format!(
                        "the vectors of segment may be corrupt after field \"{}\": {error}",
                        field_info.name
                    )));
                    return Err(error);
                }
            }
        } else {
            None
        };
        let index = self.per_fields.len();
        self.per_fields.push(PerField {
            field_info: field_info.clone(),
            invert_state,
            writer_index,
            vectors_index,
            do_vectors: false,
            field_gen: -1,
            first: true,
            norms,
            doc_values,
            point_values,
            knn_field_vectors_writer,
            column_features: 0,
        });
        self.per_field_index.insert(field_info.name.clone(), index);
        Ok(index)
    }

    /// Returns the per-field state of the reserved parent field, creating it
    /// the first time the segment needs it.
    ///
    /// Equivalent to the tail of the `IndexingChain` constructor
    /// (`IndexingChain.java:163-167`) together with
    /// `IndexingChain.initializeFieldInfo(parentPf)`, which Java runs from
    /// `initAndValidateFields` or `processParentFieldForColumnBatch` the first
    /// time a document in this segment needs the field.
    ///
    /// Java can create the `PerField` in the constructor because its
    /// `getOrAddPerField` only needs the field *name* there and leaves
    /// `pf.fieldInfo` null until a document arrives. This port's
    /// [`get_or_add_per_field`](Self::get_or_add_per_field) registers the
    /// `FieldInfo` and builds every per-field writer in one step, so it needs
    /// the segment's [`FieldInfosBuilder`], which the constructor does not
    /// have. Deferring the whole step to the first document that needs the
    /// field keeps the segment's field infos identical: Java also registers the
    /// parent field only when `initializeFieldInfo(parentPf)` runs, so a
    /// segment that never saw a document block never records it.
    ///
    /// # Errors
    ///
    /// Propagates whatever registering the field info raises.
    fn get_or_add_parent_per_field(
        &mut self,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<Option<usize>> {
        if let Some(index) = self.parent_pf {
            return Ok(Some(index));
        }
        let codec = self.config.codec();
        let described = match self.parent_field.as_ref() {
            Some(parent_field) => Self::describe_field(parent_field, field_infos, codec.as_ref())?,
            None => return Ok(None),
        };
        let registered = field_infos.add(&described)?;
        let index = self.get_or_add_per_field(registered)?;
        self.parent_pf = Some(index);
        Ok(Some(index))
    }

    /// Rejects a document field that carries the reserved parent field's name.
    ///
    /// Equivalent to the `pf == parentPf` check of
    /// `IndexingChain.processDocument` (`IndexingChain.java:622-626`) and
    /// `IndexingChain.processBatch` (`IndexingChain.java:750-753`). Java
    /// compares the `PerField` returned for the name against `parentPf`, which
    /// it created up front; this port compares the names, which selects exactly
    /// the same fields and — unlike the identity test — does not force the
    /// parent field's `FieldInfo` to be registered in a segment that has no
    /// document block.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `field_name` is the
    /// configured parent field.
    fn reject_reserved_parent_field(&self, field_name: &str) -> Result<()> {
        if let Some(parent_field) = self.parent_field.as_ref() {
            if parent_field.name() == field_name {
                return Err(LuceneError::IllegalArgument(format!(
                    "\"{field_name}\" is a reserved field and should not be added to any document"
                )));
            }
        }
        Ok(())
    }

    /// Indexes one instance of one field, and reports whether it is the first
    /// instance of an indexed field with postings in this document.
    ///
    /// Equivalent to `IndexingChain.processField(int, IndexableField,
    /// PerField)`.
    ///
    /// The per-field loop of [`process_document`](IndexingChain::process_document)
    /// keeps the same four steps written out rather than calling this method,
    /// because it also has to decide, for each of them, whether a failure is a
    /// document-level problem or one that leaves the segment's buffers corrupt;
    /// Java makes that decision inside the chain, through the
    /// `abortingExceptionConsumer` its collaborators call. This method is the
    /// plain form, used where no such classification applies.
    ///
    /// # Errors
    ///
    /// Propagates whatever inverting, storing or buffering the field raises.
    fn process_field(
        &mut self,
        doc_id: i32,
        field: &dyn IndexableField,
        pf: usize,
        analyzer: &dyn Analyzer,
        info_stream: &dyn InfoStream,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<bool> {
        let indexed_field =
            self.invert_and_store(doc_id, field, pf, analyzer, info_stream, field_infos)?;
        let field_type_dv = field.field_type().doc_values_type();
        if field_type_dv != DocValuesType::NONE {
            self.per_fields[pf]
                .doc_values
                .as_mut()
                .expect(
                    "INVARIANT: every field with a doc-values type gets a writer in get_or_add_per_field",
                )
                .add_value(doc_id, field)?;
        }
        if field.field_type().point_dimension_count() != 0 {
            match field.binary_value() {
                Some(value) => self.per_fields[pf]
                    .point_values
                    .as_mut()
                    .expect(
                        "INVARIANT: every field with point dimensions gets a writer in get_or_add_per_field",
                    )
                    .add_packed_value(doc_id, &value)?,
                None => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field=\"{}\": point value must not be null",
                        field.name()
                    )))
                }
            }
        }
        if field.field_type().vector_dimension() != 0 {
            Self::index_vector_value(doc_id, &mut self.per_fields[pf], field)?;
        }
        Ok(indexed_field)
    }

    /// Writes the parent-field value of every document of a column batch.
    ///
    /// Equivalent to `IndexingChain.processParentFieldForColumnBatch(int,
    /// int)`. Each document of a batch is an individual document rather than
    /// part of a block, so every one of them is its own parent.
    ///
    /// # Errors
    ///
    /// Propagates whatever registering the field info or buffering the values
    /// raises.
    fn process_parent_field_for_column_batch(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<()> {
        let Some(parent_pf) = self.get_or_add_parent_per_field(field_infos)? else {
            return Ok(());
        };
        let value = match self
            .parent_field
            .as_ref()
            .expect("INVARIANT: the per-field only exists when the parent field does")
            .numeric_value()
        {
            Some(NumericValue::Long(value)) => value,
            other => {
                return Err(LuceneError::IllegalState(format!(
                    "the parent field must carry a long value, but it carries {other:?}"
                )))
            }
        };
        match self.per_fields[parent_pf].doc_values.as_mut() {
            Some(DocValuesWriter::Numeric(writer)) => {
                writer.add_repeat_values(base_doc_id, value, num_docs)
            }
            _ => Err(LuceneError::IllegalState(
                "the parent field must be a numeric doc-values field".to_string(),
            )),
        }
    }

    /// Builds the [`FieldInfo`] a document field implies.
    ///
    /// Equivalent to `IndexingChain.initializeFieldInfo` restricted to the
    /// attributes the inverted-index path needs.
    fn describe_field(
        field: &dyn IndexableField,
        field_infos: &FieldInfosBuilder,
        codec: &dyn crate::codecs::Codec,
    ) -> Result<FieldInfo> {
        let field_type = field.field_type();
        let name = field.name().to_string();
        if field_type.index_options() == IndexOptions::NONE {
            Self::verify_un_indexed_field_type(&name, field_type)?;
        }
        // `initializeFieldInfo` asks the codec how many dimensions it can take
        // for this field name and rejects anything larger, *before* the
        // `FieldInfo` is built and registered (`IndexingChain.java:1316-1321`).
        // Java runs it once per field per segment and this runs it once per
        // field instance; the predicate reads only the field type, which is
        // fixed for a given instance, and a multi-valued field whose instances
        // disagree is rejected by the schema check either way, so the two
        // reject exactly the same documents.
        if field_type.vector_dimension() != 0 {
            Self::validate_max_vector_dimension(
                &name,
                field_type.vector_dimension(),
                codec.knn_vectors_format().get_max_dimensions(&name),
            )?;
        }
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
            JavaStringMap::Hash(field_type.attributes().clone()),
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

    /// Rejects a vector field whose dimension count the codec cannot store.
    ///
    /// Equivalent to `IndexingChain.validateMaxVectorDimension`
    /// (`IndexingChain.java:1552-1562`); the message is Java's, verbatim.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `vector_dim` exceeds
    /// `max_vector_dim`.
    fn validate_max_vector_dimension(
        field_name: &str,
        vector_dim: i32,
        max_vector_dim: i32,
    ) -> Result<()> {
        if vector_dim > max_vector_dim {
            return Err(LuceneError::IllegalArgument(format!(
                "Field [{field_name}] vector's dimensions must be <= [{max_vector_dim}]; got {vector_dim}"
            )));
        }
        Ok(())
    }

    /// Rejects a field type that asks for term vectors without being indexed.
    ///
    /// Equivalent to `IndexingChain.verifyUnIndexedFieldType(String,
    /// IndexableFieldType)`, which `updateDocFieldSchema` runs for every field
    /// whose index options are `NONE`. Without it the four flags would be
    /// silently dropped when the `FieldInfo` is built, and a caller asking for
    /// term vectors on a stored-only field would never learn that it got none.
    fn verify_un_indexed_field_type(name: &str, field_type: &dyn IndexableFieldType) -> Result<()> {
        for (requested, what) in [
            (field_type.store_term_vectors(), "term vectors"),
            (
                field_type.store_term_vector_positions(),
                "term vector positions",
            ),
            (
                field_type.store_term_vector_offsets(),
                "term vector offsets",
            ),
            (
                field_type.store_term_vector_payloads(),
                "term vector payloads",
            ),
        ] {
            if requested {
                return Err(LuceneError::IllegalArgument(format!(
                    "cannot store {what} for a field that is not indexed (field=\"{name}\")"
                )));
            }
        }
        Ok(())
    }

    /// Inverts one instance of one field.
    ///
    /// Equivalent to `IndexingChain.PerField.invert`. It takes its collaborators
    /// explicitly so that the borrow checker can see that the per-field state
    /// and the shared byte pool are disjoint.
    #[allow(clippy::too_many_arguments)]
    fn invert(
        analyzer: &dyn Analyzer,
        info_stream: &dyn InfoStream,
        terms_writer: &mut FreqProxTermsWriter,
        term_vectors: Option<&mut TermVectorsConsumer>,
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
            Some(InvertableType::BINARY) => Self::invert_term(
                info_stream,
                terms_writer,
                term_vectors,
                per_field,
                doc_id,
                field,
                first,
            ),
            Some(InvertableType::TOKEN_STREAM) => Self::invert_token_stream(
                analyzer,
                info_stream,
                terms_writer,
                term_vectors,
                per_field,
                doc_id,
                field,
                first,
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
    #[allow(clippy::too_many_arguments)]
    fn invert_term(
        info_stream: &dyn InfoStream,
        terms_writer: &mut FreqProxTermsWriter,
        mut term_vectors: Option<&mut TermVectorsConsumer>,
        per_field: &mut PerField,
        doc_id: i32,
        field: &dyn IndexableField,
        first: bool,
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
        Self::start_vectors(term_vectors.as_deref_mut(), per_field, field, first)?;
        let (pool, writer) = terms_writer.pool_and_field(writer_index);
        let text_start = writer
            .add(
                pool,
                &mut per_field.invert_state,
                binary_value.slice(),
                doc_id,
                &token,
            )
            .map_err(|error| {
                Self::map_add_error(
                    info_stream,
                    field.name(),
                    binary_value.slice(),
                    false,
                    error,
                )
            })?;
        Self::add_to_vectors(term_vectors, per_field, text_start, &token)
    }

    /// Wraps the error an over-long term raises with the field name and a
    /// readable prefix of the offending bytes.
    ///
    /// Equivalent to the two `catch (MaxBytesLengthExceededException e)` blocks
    /// of `IndexingChain`: `invertTokenStream` (`:2003-2021`), whose message
    /// names the UTF-8 encoding and asks the operator to correct the analyzer,
    /// and `invertTerm` (`:2079-2095`), whose message names the length. Both
    /// render the first thirty bytes with `Arrays.toString`, i.e. as signed
    /// decimals, and both log the message to the `IW` info stream before
    /// throwing. Without the wrapping the operator sees only
    /// `bytes can be at most 32766 in length; got 40000`, which names neither
    /// the field nor the term.
    fn immense_term_error(
        info_stream: &dyn InfoStream,
        field_name: &str,
        term: &[u8],
        analyzed: bool,
        original: &LuceneError,
    ) -> LuceneError {
        // Java appends `e.getMessage()`, the bare text, not the exception's
        // `toString()`. Rucene's `Display` prefixes every variant with its kind,
        // so the carried string is used directly to keep the two messages
        // identical.
        let original = match original {
            LuceneError::IllegalArgument(message) => message.clone(),
            other => other.to_string(),
        };
        let prefix: Vec<String> = term
            .iter()
            .take(IMMENSE_TERM_PREFIX)
            .map(|byte| (*byte as i8).to_string())
            .collect();
        let prefix = format!("[{}]", prefix.join(", "));
        let message = if analyzed {
            format!(
                "Document contains at least one immense term in field=\"{field_name}\" (whose \
                 UTF8 encoding is longer than the max length {MAX_TERM_LENGTH}), all of which \
                 were skipped.  Please correct the analyzer to not produce such terms.  The \
                 prefix of the first immense term is: '{prefix}...', original message: {original}"
            )
        } else {
            format!(
                "Document contains at least one immense term in field=\"{field_name}\" (whose \
                 length is longer than the max length {MAX_TERM_LENGTH}), all of which were \
                 skipped. The prefix of the first immense term is: '{prefix}...'"
            )
        };
        if info_stream.is_enabled("IW") {
            info_stream.message("IW", &format!("ERROR: {message}"));
        }
        LuceneError::IllegalArgument(message)
    }

    /// Rejects a term above [`MAX_TERM_LENGTH`] with Lucene's message.
    ///
    /// The per-field writer reports the same condition as
    /// `MaxBytesLengthExceededException` does — `bytes can be at most … in
    /// length` — but the useful message is assembled one level up, where the
    /// field name is known.
    fn map_add_error(
        info_stream: &dyn InfoStream,
        field_name: &str,
        term: &[u8],
        analyzed: bool,
        error: LuceneError,
    ) -> LuceneError {
        if term.len() > MAX_TERM_LENGTH {
            return Self::immense_term_error(info_stream, field_name, term, analyzed, &error);
        }
        error
    }

    /// Opens one field instance on the term-vectors consumer and records
    /// whether its tokens must be forwarded to it.
    ///
    /// Equivalent to the `termsHashPerField.start(field, first)` call, whose
    /// result Lucene keeps in `TermsHashPerField.doNextCall`.
    ///
    /// # Errors
    ///
    /// Propagates the field-type validation errors of
    /// `TermVectorsConsumerPerField.start`, which are document-level failures.
    fn start_vectors(
        term_vectors: Option<&mut TermVectorsConsumer>,
        per_field: &mut PerField,
        field: &dyn IndexableField,
        first: bool,
    ) -> Result<()> {
        per_field.do_vectors = false;
        let Some(consumer) = term_vectors else {
            // Without a bound segment there is nowhere to write vectors, so a
            // field asking for them must not be silently dropped.
            if field.field_type().store_term_vectors() {
                return Err(LuceneError::IllegalState(format!(
                    "the indexing chain is not bound to a segment, so the term vectors of \
                     field \"{}\" cannot be written; call bind_segment first",
                    field.name()
                )));
            }
            return Ok(());
        };
        let Some(index) = per_field.vectors_index else {
            return Ok(());
        };
        per_field.do_vectors = consumer.start_field(index, field, first)?;
        Ok(())
    }

    /// Forwards one token to the term-vectors consumer.
    ///
    /// Equivalent to the `nextPerField.add(postingsArray.textStarts[termID],
    /// docID)` call `TermsHashPerField.add(BytesRef, int)` makes when
    /// `doNextCall` is set.
    ///
    /// # Errors
    ///
    /// Propagates the errors of `TermVectorsConsumerPerField.add`.
    fn add_to_vectors(
        term_vectors: Option<&mut TermVectorsConsumer>,
        per_field: &PerField,
        text_start: i32,
        token: &InvertedToken<'_>,
    ) -> Result<()> {
        if !per_field.do_vectors {
            return Ok(());
        }
        let (Some(consumer), Some(index)) = (term_vectors, per_field.vectors_index) else {
            return Ok(());
        };
        consumer.add(index, &per_field.invert_state, text_start, token)
    }

    /// Inverts a field through its token stream.
    ///
    /// Equivalent to `IndexingChain.PerField.invertTokenStream`.
    #[allow(clippy::too_many_arguments)]
    fn invert_token_stream(
        analyzer: &dyn Analyzer,
        info_stream: &dyn InfoStream,
        terms_writer: &mut FreqProxTermsWriter,
        term_vectors: Option<&mut TermVectorsConsumer>,
        per_field: &mut PerField,
        doc_id: i32,
        field: &dyn IndexableField,
        first: bool,
    ) -> Result<()> {
        let analyzed = field.field_type().tokenized();
        let is_term_doc = per_field.field_info.is_term_doc_field();
        let mut stream = field.token_stream(analyzer, None);
        let mut term_vectors = term_vectors;

        let outcome = (|| -> Result<()> {
            stream.reset()?;
            let layout = TokenAttributeLayout::resolve(stream.attribute_source());
            // Lucene calls `termsHashPerField.start(field, first)` here, right
            // after `stream.reset()` and `invertState.setAttributeSource`
            // (`IndexingChain.java:1908-1912`).
            Self::start_vectors(term_vectors.as_deref_mut(), per_field, field, first)?;

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
                let text_start = writer
                    .add(
                        pool,
                        &mut per_field.invert_state,
                        token.term.slice(),
                        doc_id,
                        &inverted,
                    )
                    .map_err(|error| {
                        Self::map_add_error(
                            info_stream,
                            field.name(),
                            token.term.slice(),
                            true,
                            error,
                        )
                    })?;
                Self::add_to_vectors(
                    term_vectors.as_deref_mut(),
                    per_field,
                    text_start,
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
    /// Writes the buffered norms of every field of the segment.
    ///
    /// Equivalent to `IndexingChain.writeNorms(SegmentWriteState,
    /// Sorter.DocMap)` (`IndexingChain.java:503-532`), without the doc map;
    /// see [`crate::index::norms_writer`].
    ///
    /// Nothing at all is written — not even the two empty files — when no field
    /// of the segment has norms, which is what `state.fieldInfos.hasNorms()`
    /// guards. The fields are visited in field-number order, because that is
    /// the order `FieldInfos` iterates in and the `.nvm` entries follow it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when a field that should have
    /// norms has no buffer, which would mean the chain never saw the field.
    fn write_norms(&mut self, state: &SegmentWriteState<'_>, max_doc: i32) -> Result<()> {
        if !state.field_infos.has_norms() {
            return Ok(());
        }
        let mut consumer = self.config.codec().norms_format().norms_consumer(state)?;
        // Java's `finally` closes the consumer whether or not a field threw, so
        // that the two files never leak. The write outcome is held and the
        // close runs either way.
        let outcome = (|| -> Result<()> {
            for info in state.field_infos.iter() {
                // Java re-reads `omitsNorms()` and `getIndexOptions()` from the
                // *final* field info rather than trusting the one the writer
                // was created with; `has_norms()` is exactly that conjunction.
                if !info.has_norms() {
                    continue;
                }
                let index = *self.per_field_index.get(&info.name).ok_or_else(|| {
                    LuceneError::IllegalState(format!(
                        "field \"{}\" has norms but was never seen by the indexing chain",
                        info.name
                    ))
                })?;
                let norms = self.per_fields[index].norms.as_mut().ok_or_else(|| {
                    LuceneError::IllegalState(format!(
                        "field \"{}\" has norms but no norms buffer",
                        info.name
                    ))
                })?;
                norms.finish(max_doc);
                norms.flush(consumer.as_mut())?;
            }
            Ok(())
        })();
        let close_outcome = consumer.close();
        outcome?;
        close_outcome
    }

    /// Writes the buffered doc values of the segment through the codec's
    /// doc-values format, producing the `.dvd` and `.dvm` files.
    ///
    /// Equivalent to `IndexingChain.writeDocValues`
    /// (`IndexingChain.java:439-497`). Lucene walks its open `PerField[]
    /// fieldHash` bucket by bucket, newest instance first within a bucket, and
    /// that *table order* — not field-number order — decides the order of the
    /// field entries inside the `.dvm`. This port stores its fields in a
    /// [`Vec`], so the layout is reproduced by replaying the table's insert
    /// and rehash rules over the field names in registration order.
    ///
    /// Like Java, the consumer is created lazily at the first field that has
    /// values and closed once after the last one, whether or not a field
    /// threw. The "BUG" guards below are ports of Lucene's `AssertionError`
    /// checks: they detect a per-field state inconsistent with the field
    /// infos the segment will carry.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] for a per-field state that
    /// disagrees with the segment's field infos, and propagates whatever the
    /// format's consumer raises while writing the fields.
    fn write_doc_values(&mut self, state: &SegmentWriteState<'_>) -> Result<()> {
        if !state.field_infos.has_doc_values() {
            return Ok(());
        }
        // `fieldHash` starts at two buckets, chains newest-first and doubles
        // once the table is half full, reversing every chain it moves; the
        // flush walks buckets in order and each chain head to tail.
        let mut consumer: Option<Box<dyn DocValuesConsumer + '_>> = None;
        let outcome: Result<()> = (|| {
            for index in field_hash_flush_order(&self.per_fields) {
                let per_field = &mut self.per_fields[index];
                let Some(writer) = per_field.doc_values.as_mut() else {
                    if per_field.field_info.doc_values_type != DocValuesType::NONE {
                        return Err(LuceneError::IllegalState(format!(
                            "segment={}: field=\"{}\" has docValues but did not write them",
                            state.segment_info.name, per_field.field_info.name
                        )));
                    }
                    continue;
                };
                if per_field.field_info.doc_values_type == DocValuesType::NONE {
                    return Err(LuceneError::IllegalState(format!(
                        "segment={}: field=\"{}\" has no docValues but wrote them",
                        state.segment_info.name, per_field.field_info.name
                    )));
                }
                if consumer.is_none() {
                    // Lazy init, exactly as `writeDocValues` does.
                    consumer = Some(
                        self.config
                            .codec()
                            .doc_values_format()
                            .fields_consumer(state)?,
                    );
                }
                let consumer = consumer
                    .as_mut()
                    .expect("INVARIANT: the consumer was just created if it was missing");
                writer.flush(&mut **consumer)?;
                // Java nulls `perField.docValuesWriter` here; this port drops
                // the whole per-field table at the end of `flush` instead.
            }
            Ok(())
        })();
        let close_outcome = match consumer.as_mut() {
            Some(consumer) => consumer.close(),
            None => Ok(()),
        };
        // Lucene re-checks the field infos after the `finally` that closes the
        // consumer: writing doc values without the segment declaring them, or
        // declaring them without writing, is a bug in the chain.
        let wrote_values = consumer.is_some();
        if !state.field_infos.has_doc_values() {
            if wrote_values {
                return Err(LuceneError::IllegalState(format!(
                    "segment={}: fieldInfos has no docValues but wrote them",
                    state.segment_info.name
                )));
            }
        } else if !wrote_values {
            return Err(LuceneError::IllegalState(format!(
                "segment={}: fieldInfos has docValues but did not write them",
                state.segment_info.name
            )));
        }
        outcome?;
        close_outcome
    }

    /// Writes the buffered point values of the segment through the codec's
    /// points format, producing the `.kdd`, `.kdi` and `.kdm` files.
    ///
    /// Equivalent to `IndexingChain.writePoints`
    /// (`IndexingChain.java:396-435`). Like `writeDocValues` it walks Lucene's
    /// open `PerField[] fieldHash` bucket by bucket, newest instance first
    /// within a bucket, and that *table order* — not field-number order —
    /// decides the order of the per-field entries inside the `.kdm`; see
    /// [`field_hash_flush_order`].
    ///
    /// The writer is created lazily at the first field that declares points,
    /// `finish()` is called once after the last field, and the writer is closed
    /// whether or not a field threw — which is what Lucene's `success` flag and
    /// its `IOUtils.close` / `closeWhileHandlingException` pair express.
    ///
    /// Java skips a field whose writer exists but whose field info reports no
    /// dimensions ("We could have initialized pointValuesWriter, but failed to
    /// write even a single doc"), and clears the writer either way. This port
    /// drops the whole per-field table at the end of `flush` instead.
    ///
    /// Java also throws `IllegalStateException` for a field that declares
    /// points when `codec.pointsFormat()` returns `null`
    /// (`IndexingChain.java:409-415`). That state cannot exist here:
    /// `Codec::points_format` returns `&dyn PointsFormat`, not an `Option`,
    /// so every codec this crate can hold has one. The check is therefore not
    /// ported, and no [`LuceneError::IllegalState`] can come from this method.
    ///
    /// # Errors
    ///
    /// Propagates whatever the points format raises while creating the writer,
    /// flushing a field, finishing or closing.
    fn write_points(&mut self, state: &SegmentWriteState<'_>) -> Result<()> {
        let mut writer: Option<Box<dyn PointsWriter>> = None;
        let outcome: Result<()> = (|| {
            for index in field_hash_flush_order(&self.per_fields) {
                let per_field = &mut self.per_fields[index];
                let Some(points) = per_field.point_values.as_mut() else {
                    continue;
                };
                // Lucene re-reads the *field info*, not the writer: a field
                // that was registered with points but never got a value can
                // still end the segment with no dimensions.
                if per_field.field_info.point_dimension_count == 0 {
                    continue;
                }
                if writer.is_none() {
                    // Lazy init, exactly as `writePoints` does.
                    let codec = self.config.codec();
                    writer = Some(codec.points_format().fields_writer(state)?);
                }
                let writer = writer
                    .as_mut()
                    .expect("INVARIANT: the writer was just created if it was missing");
                points.flush(&mut **writer)?;
            }
            if let Some(writer) = writer.as_mut() {
                writer.finish()?;
            }
            Ok(())
        })();
        let close_outcome = match writer.as_mut() {
            Some(writer) => writer.close(),
            None => Ok(()),
        };
        outcome?;
        close_outcome
    }

    /// Hands one field instance's vector to the codec's per-field writer.
    ///
    /// Equivalent to `IndexingChain.indexVectorValue`
    /// (`IndexingChain.java:1694-1707`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field carries no
    /// vector of the encoding its type declares — the state Java reaches as a
    /// `ClassCastException` — and propagates whatever the codec's field writer
    /// raises, which is how a document that repeats a vector field, or that
    /// arrives out of doc-id order, is rejected.
    fn index_vector_value(
        doc_id: i32,
        per_field: &mut PerField,
        field: &dyn IndexableField,
    ) -> Result<()> {
        let name = field.name();
        let Some(writer) = per_field.knn_field_vectors_writer.as_mut() else {
            return Err(LuceneError::IllegalState(format!(
                "field=\"{name}\" declares vector dimensions but has no vectors writer"
            )));
        };
        match writer {
            FieldVectorWriter::Float(writer) => {
                let Some(value) = field.float_vector_value() else {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field=\"{name}\" is indexed with FLOAT32 vectors but carries no float vector value"
                    )));
                };
                let value = value.to_vec();
                writer.add_value(doc_id, value)
            }
            FieldVectorWriter::Byte(writer) => {
                let Some(value) = field.byte_vector_value() else {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field=\"{name}\" is indexed with BYTE vectors but carries no byte vector value"
                    )));
                };
                let value = value.to_vec();
                writer.add_value(doc_id, value)
            }
        }
    }

    /// Buffers the norm of one field of one document.
    ///
    /// Equivalent to the norms half of `IndexingChain.PerField.finish(int)`
    /// (`IndexingChain.java:1853-1869`).
    ///
    /// A field that appeared in the document but produced no tokens gets a norm
    /// of `0` without consulting the similarity — that is how the reader tells
    /// "present but empty" from "absent". For every other field the similarity
    /// decides, and a similarity that answers `0` for a non-empty field is
    /// rejected, because `0` is reserved for the empty case and would make the
    /// two indistinguishable.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the similarity returns `0`
    /// for a non-empty field, and propagates whatever
    /// [`Similarity::compute_norm`] and [`NormValuesWriter::add_value`] raise.
    fn finish_norms(
        similarity: &dyn Similarity,
        per_field: &mut PerField,
        doc_id: i32,
    ) -> Result<()> {
        // `fieldInfo.omitsNorms() == false` in Java; a field with no norms
        // writer is exactly a field that omits them or is not indexed, and
        // `finish` is only reached for indexed fields.
        let Some(norms) = per_field.norms.as_mut() else {
            return Ok(());
        };
        let norm_value = if per_field.invert_state.length() == 0 {
            0
        } else {
            let value = similarity.compute_norm(&per_field.invert_state)?;
            if value == 0 {
                return Err(LuceneError::IllegalState(format!(
                    "Similarity {similarity:?} return 0 for non-empty field"
                )));
            }
            value
        };
        norms.add_value(doc_id, norm_value)
    }

    /// Feeds the row-oriented features — stored fields and term inversion — of
    /// every column that has one.
    ///
    /// Equivalent to `IndexingChain.processRowColumns(int, int,
    /// Iterable<Column>)`. The outer loop frames every batch-local document
    /// id, so the stored-fields stream and the terms hash stay aligned with the
    /// reserved document ids even for a document with no row-oriented value;
    /// the column cursors themselves stay sparse.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a cursor returns a
    /// document id out of order or outside the batch, and propagates whatever
    /// inverting or storing a value raises.
    fn process_row_columns(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        columns: &[TypedColumn<'_>],
        doc_fields: &[usize],
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<()> {
        // Collect the row-oriented columns. The per-fields come from the
        // validation pass's cache, indexed by the column's original position,
        // so `row_pf_indices` records that position for each row-mode column.
        let mut adapters: Vec<Box<dyn ColumnFieldAdapter + '_>> = Vec::new();
        let mut heads: Vec<i32> = Vec::new();
        let mut row_pf_indices: Vec<usize> = Vec::new();
        let mut has_inverted = false;
        let mut has_stored = false;

        for (original_idx, column) in columns.iter().enumerate() {
            let field_type = Arc::clone(column.column().field_type());
            if !field_type.stored() && field_type.index_options() == IndexOptions::NONE {
                continue;
            }
            let mut adapter = create_column_field_adapter(match column {
                TypedColumn::Long(c) => TypedColumn::Long(*c),
                TypedColumn::Binary(c) => TypedColumn::Binary(*c),
                TypedColumn::Dictionary(c) => TypedColumn::Dictionary(*c),
                TypedColumn::TokenStream(c) => TypedColumn::TokenStream(*c),
                TypedColumn::VectorFloat(c) => TypedColumn::VectorFloat(*c),
                TypedColumn::VectorByte(c) => TypedColumn::VectorByte(*c),
            })?;
            heads.push(adapter.next_doc()?);
            adapters.push(adapter);
            row_pf_indices.push(original_idx);
            if field_type.index_options() != IndexOptions::NONE {
                has_inverted = true;
            }
            if field_type.stored() {
                has_stored = true;
            }
        }

        let num_row_cols = adapters.len();
        let analyzer = self.config.analyzer_arc();
        let info_stream = self.config.info_stream();
        let similarity = self.config.similarity();

        // Row-dense outer loop: frame every document in `[0, num_docs)`.
        for batch_doc_id in 0..num_docs {
            let seg_doc_id = base_doc_id + batch_doc_id;
            let field_gen = self.next_field_gen;
            self.next_field_gen += 1;
            let mut indexed: Vec<usize> = Vec::new();

            if has_inverted {
                if let Some(consumer) = self.term_vectors_consumer.as_mut() {
                    consumer.start_document();
                }
            }
            if has_stored {
                self.start_stored_fields(seg_doc_id)?;
            }

            let mut outcome: Result<()> = Ok(());
            'columns: for i in 0..num_row_cols {
                let mut head = heads[i];
                if head < batch_doc_id {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Row column \"{}\" returned out-of-order batch doc-id {head}",
                        adapters[i].name()
                    )));
                }
                while head == batch_doc_id {
                    let pf = doc_fields[row_pf_indices[i]];
                    if self.per_fields[pf].field_gen != field_gen {
                        self.per_fields[pf].field_gen = field_gen;
                        self.per_fields[pf].first = true;
                    }
                    match self.invert_and_store(
                        seg_doc_id,
                        adapters[i].as_ref(),
                        pf,
                        analyzer.as_ref(),
                        info_stream.as_ref(),
                        field_infos,
                    ) {
                        Ok(true) => indexed.push(pf),
                        Ok(false) => {}
                        Err(e) => {
                            outcome = Err(e);
                            break 'columns;
                        }
                    }
                    head = adapters[i].next_doc()?;
                }
                heads[i] = head;
            }

            // Lucene's `finally` runs the whole tail only when no aborting
            // exception was recorded, because in that case the segment is
            // discarded and none of it matters.
            if self.aborting_error.is_none() {
                for pf in indexed {
                    if let Err(e) = Self::finish_norms(
                        similarity.as_ref(),
                        &mut self.per_fields[pf],
                        seg_doc_id,
                    ) {
                        if outcome.is_ok() {
                            outcome = Err(e);
                        }
                    }
                    let per_field = &self.per_fields[pf];
                    if let Some(writer_index) = per_field.writer_index {
                        if self.terms_writer.field(writer_index).saw_payloads() {
                            if let Some(info) =
                                field_infos.field_info_mut(&per_field.field_info.name)
                            {
                                info.set_store_payloads();
                            }
                        }
                    }
                    if let (Some(consumer), Some(vectors_index)) =
                        (self.term_vectors_consumer.as_mut(), per_field.vectors_index)
                    {
                        consumer.finish_field(vectors_index);
                    }
                }
                if has_stored {
                    self.finish_stored_fields()?;
                }
                if has_inverted {
                    if let Some(consumer) = self.term_vectors_consumer.as_mut() {
                        if let Err(e) =
                            consumer.finish_document(seg_doc_id, self.terms_writer.pool())
                        {
                            self.on_aborting_exception(LuceneError::CorruptIndex(format!(
                                "the term vectors of segment may be corrupt after doc \
                                 {seg_doc_id}: {e}"
                            )));
                            return Err(e);
                        }
                        for per_field in &self.per_fields {
                            let Some(vectors_index) = per_field.vectors_index else {
                                continue;
                            };
                            if consumer.field(vectors_index).wrote_vectors() {
                                if let Some(info) =
                                    field_infos.field_info_mut(&per_field.field_info.name)
                                {
                                    info.set_store_term_vectors()?;
                                }
                            }
                        }
                    }
                }
            }
            outcome?;
        }

        // Any cursor head left after the outer loop names a document at or past
        // the end of the batch.
        for i in 0..num_row_cols {
            if heads[i] != NO_MORE_DOCS {
                return Err(LuceneError::IllegalArgument(format!(
                    "Row column \"{}\" returned batch doc-id {} which is out of range [0, \
                     {num_docs})",
                    adapters[i].name(),
                    heads[i]
                )));
            }
        }
        Ok(())
    }

    /// Inverts one field instance and stores it, returning whether it was
    /// indexed for the first time in this document.
    ///
    /// Equivalent to `IndexingChain.invertAndStore(int, IndexableField,
    /// PerField)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever inverting or storing the value raises.
    fn invert_and_store(
        &mut self,
        doc_id: i32,
        field: &dyn IndexableField,
        pf: usize,
        analyzer: &dyn Analyzer,
        info_stream: &dyn InfoStream,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<bool> {
        let _ = field_infos;
        let mut indexed_field = false;
        if field.field_type().index_options() != IndexOptions::NONE {
            let first = self.per_fields[pf].first;
            Self::invert(
                analyzer,
                info_stream,
                &mut self.terms_writer,
                self.term_vectors_consumer.as_mut(),
                &mut self.per_fields[pf],
                doc_id,
                field,
                first,
            )?;
            if first {
                self.per_fields[pf].first = false;
                indexed_field = true;
            }
        }
        if field.field_type().stored() {
            self.store_field(pf, field)?;
        }
        Ok(indexed_field)
    }

    /// Feeds a numeric column's doc values and points.
    ///
    /// Equivalent to `IndexingChain.processLongColumn(int, int, LongColumn,
    /// PerField, IndexableFieldType)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a doc-values type a numeric
    /// column cannot fill, and propagates whatever reading the column or
    /// writing a buffer raises.
    fn process_long_column(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        column: &dyn LongColumn,
        pf: usize,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        let dv_type = field_type.doc_values_type();
        let has_points = field_type.point_dimension_count() != 0;

        // Dense fast path: bulk-feed doc values and points from a values cursor.
        if column.column().density() == Density::Dense {
            return self.process_dense_long_column(
                base_doc_id,
                num_docs,
                column,
                pf,
                dv_type,
                has_points,
            );
        }

        let name = column.column().name().to_string();

        // Sparse, doc-values only: per-document tuple-cursor path.
        if !has_points {
            let mut cursor = column.tuples()?;
            loop {
                let batch_doc_id = cursor.next_doc()?;
                if batch_doc_id == NO_MORE_DOCS {
                    break;
                }
                ColumnValidation::check_doc_id(&name, batch_doc_id, num_docs)?;
                let value = cursor.long_value();
                let doc = base_doc_id + batch_doc_id;
                match (dv_type, self.per_fields[pf].doc_values.as_mut()) {
                    (DocValuesType::NUMERIC, Some(DocValuesWriter::Numeric(w))) => {
                        w.add_value(doc, value)?;
                    }
                    (DocValuesType::SORTED_NUMERIC, Some(DocValuesWriter::SortedNumeric(w))) => {
                        w.add_value(doc, value)?;
                    }
                    _ => return Err(Self::incompatible_dv("LongColumn", &name, dv_type)),
                }
            }
            return Ok(());
        }

        // Sparse, with points and optionally numeric doc values.
        let kind = column.numeric_kind();
        let byte_width = match kind {
            NumericKind::Int | NumericKind::Float => 4,
            NumericKind::Long | NumericKind::Double => 8,
        };
        let mut point_scratch = vec![0u8; byte_width];
        let mut cursor = column.tuples()?;
        loop {
            let batch_doc_id = cursor.next_doc()?;
            if batch_doc_id == NO_MORE_DOCS {
                break;
            }
            ColumnValidation::check_doc_id(&name, batch_doc_id, num_docs)?;
            let seg_doc_id = base_doc_id + batch_doc_id;
            let raw = cursor.long_value();
            match dv_type {
                DocValuesType::NONE => {}
                DocValuesType::NUMERIC => match self.per_fields[pf].doc_values.as_mut() {
                    Some(DocValuesWriter::Numeric(w)) => w.add_value(seg_doc_id, raw)?,
                    _ => return Err(Self::incompatible_dv("LongColumn", &name, dv_type)),
                },
                DocValuesType::SORTED_NUMERIC => match self.per_fields[pf].doc_values.as_mut() {
                    Some(DocValuesWriter::SortedNumeric(w)) => w.add_value(seg_doc_id, raw)?,
                    _ => return Err(Self::incompatible_dv("LongColumn", &name, dv_type)),
                },
                _ => return Err(Self::incompatible_dv("LongColumn", &name, dv_type)),
            }
            Self::encode_sortable_point_bytes(raw, kind, &mut point_scratch);
            let value = BytesRef::new(point_scratch.clone());
            let Some(point_writer) = self.per_fields[pf].point_values.as_mut() else {
                return Err(LuceneError::IllegalState(format!(
                    "field=\"{name}\" declares point dimensions but has no points writer"
                )));
            };
            point_writer.add_packed_value(seg_doc_id, &value)?;
        }
        Ok(())
    }

    /// Encodes one numeric value in the sortable-bytes form its point field
    /// uses.
    ///
    /// Equivalent to the private
    /// `IndexingChain.encodeSortablePointBytes(long, LongColumn.NumericKind,
    /// byte[])`.
    fn encode_sortable_point_bytes(raw: i64, kind: NumericKind, scratch: &mut [u8]) {
        match kind {
            NumericKind::Int | NumericKind::Float => {
                NumericUtils::int_to_sortable_bytes(raw as i32, scratch, 0);
            }
            NumericKind::Long | NumericKind::Double => {
                NumericUtils::long_to_sortable_bytes(raw, scratch, 0);
            }
        }
    }

    /// The error Lucene raises for a column whose field declares a doc-values
    /// type that column cannot fill.
    fn incompatible_dv(kind: &str, name: &str, dv_type: DocValuesType) -> LuceneError {
        LuceneError::IllegalArgument(format!(
            "{kind} \"{name}\" has incompatible docValuesType: {dv_type:?}"
        ))
    }

    /// Bulk-feeds the doc values and points of a dense numeric column.
    ///
    /// Equivalent to the private `IndexingChain.processDenseLongColumn(int,
    /// int, LongColumn, PerField, DocValuesType, boolean)`. The doc-values pass
    /// runs first: it transforms the backing values least, so the cursor's
    /// source stays warm in cache for the heavier points pass that follows.
    ///
    /// # Errors
    ///
    /// As [`process_long_column`](Self::process_long_column).
    fn process_dense_long_column(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        column: &dyn LongColumn,
        pf: usize,
        dv_type: DocValuesType,
        has_points: bool,
    ) -> Result<()> {
        let name = column.column().name().to_string();
        if dv_type != DocValuesType::NONE {
            let mut dv_cursor = column.values()?;
            ColumnValidation::check_dense_count(&name, dv_cursor.size(), num_docs)?;
            match (dv_type, self.per_fields[pf].doc_values.as_mut()) {
                (DocValuesType::NUMERIC, Some(DocValuesWriter::Numeric(w))) => {
                    w.add_dense_values(base_doc_id, dv_cursor.as_mut())?;
                }
                (DocValuesType::SORTED_NUMERIC, Some(DocValuesWriter::SortedNumeric(w))) => {
                    w.add_dense_values(base_doc_id, dv_cursor.as_mut())?;
                }
                _ => return Err(Self::incompatible_dv("LongColumn", &name, dv_type)),
            }
        }
        if has_points {
            let mut points_cursor = column.values()?;
            ColumnValidation::check_dense_count(&name, points_cursor.size(), num_docs)?;
            let kind = column.numeric_kind();
            let Some(point_writer) = self.per_fields[pf].point_values.as_mut() else {
                return Err(LuceneError::IllegalState(format!(
                    "field=\"{name}\" declares point dimensions but has no points writer"
                )));
            };
            match kind {
                NumericKind::Int | NumericKind::Float => {
                    point_writer.add_dense_1d_int_values(base_doc_id, points_cursor.as_mut())?;
                }
                NumericKind::Long | NumericKind::Double => {
                    point_writer.add_dense_1d_long_values(base_doc_id, points_cursor.as_mut())?;
                }
            }
        }
        Ok(())
    }

    /// Feeds a binary column's doc values and points.
    ///
    /// Equivalent to `IndexingChain.processBinaryColumn(int, int, BinaryColumn,
    /// PerField, IndexableFieldType)`.
    ///
    /// # Errors
    ///
    /// As [`process_long_column`](Self::process_long_column).
    fn process_binary_column(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        column: &dyn BinaryColumn,
        pf: usize,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        let dv_type = field_type.doc_values_type();
        let has_points = field_type.point_dimension_count() != 0;

        if column.column().density() == Density::Dense {
            return self.process_dense_binary_column(
                base_doc_id,
                num_docs,
                column,
                pf,
                dv_type,
                has_points,
            );
        }

        let name = column.column().name().to_string();
        let mut cursor = column.tuples()?;

        if dv_type == DocValuesType::NONE {
            // Points only: the bytes pass through unchanged, and the caller is
            // responsible for producing sort-encoded bytes of the right length.
            loop {
                let batch_doc_id = cursor.next_doc()?;
                if batch_doc_id == NO_MORE_DOCS {
                    break;
                }
                ColumnValidation::check_doc_id(&name, batch_doc_id, num_docs)?;
                let value = cursor.value();
                let Some(point_writer) = self.per_fields[pf].point_values.as_mut() else {
                    return Err(LuceneError::IllegalState(format!(
                        "field=\"{name}\" declares point dimensions but has no points writer"
                    )));
                };
                point_writer.add_packed_value(base_doc_id + batch_doc_id, &value)?;
            }
            return Ok(());
        }

        loop {
            let batch_doc_id = cursor.next_doc()?;
            if batch_doc_id == NO_MORE_DOCS {
                break;
            }
            ColumnValidation::check_doc_id(&name, batch_doc_id, num_docs)?;
            let seg_doc_id = base_doc_id + batch_doc_id;
            let value = cursor.value();
            match (dv_type, self.per_fields[pf].doc_values.as_mut()) {
                (DocValuesType::BINARY, Some(DocValuesWriter::Binary(w))) => {
                    w.add_value(seg_doc_id, value.slice())?;
                }
                (DocValuesType::SORTED, Some(DocValuesWriter::Sorted(w))) => {
                    w.add_value(seg_doc_id, value.slice())?;
                }
                (DocValuesType::SORTED_SET, Some(DocValuesWriter::SortedSet(w))) => {
                    w.add_value(seg_doc_id, value.slice())?;
                }
                _ => return Err(Self::incompatible_dv("BinaryColumn", &name, dv_type)),
            }
            if has_points {
                let Some(point_writer) = self.per_fields[pf].point_values.as_mut() else {
                    return Err(LuceneError::IllegalState(format!(
                        "field=\"{name}\" declares point dimensions but has no points writer"
                    )));
                };
                point_writer.add_packed_value(seg_doc_id, &value)?;
            }
        }
        Ok(())
    }

    /// Bulk-feeds the doc values and points of a dense binary column.
    ///
    /// Equivalent to the private `IndexingChain.processDenseBinaryColumn(int,
    /// int, BinaryColumn, PerField, DocValuesType, boolean)`.
    ///
    /// # Errors
    ///
    /// As [`process_long_column`](Self::process_long_column).
    fn process_dense_binary_column(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        column: &dyn BinaryColumn,
        pf: usize,
        dv_type: DocValuesType,
        has_points: bool,
    ) -> Result<()> {
        let name = column.column().name().to_string();
        // Doc-values pass first, over a dense values cursor.
        if dv_type != DocValuesType::NONE {
            let mut dv_cursor = column.values()?;
            let size = dv_cursor.size();
            ColumnValidation::check_dense_count(&name, size, num_docs)?;
            for i in 0..size {
                let value = dv_cursor.next_value()?;
                let doc = base_doc_id + i;
                match (dv_type, self.per_fields[pf].doc_values.as_mut()) {
                    (DocValuesType::BINARY, Some(DocValuesWriter::Binary(w))) => {
                        w.add_value(doc, value.slice())?;
                    }
                    (DocValuesType::SORTED, Some(DocValuesWriter::Sorted(w))) => {
                        w.add_value(doc, value.slice())?;
                    }
                    (DocValuesType::SORTED_SET, Some(DocValuesWriter::SortedSet(w))) => {
                        w.add_value(doc, value.slice())?;
                    }
                    _ => return Err(Self::incompatible_dv("BinaryColumn", &name, dv_type)),
                }
            }
        }
        if has_points {
            // Points pass: a fresh dense values cursor, bulk-added.
            let mut pc = column.values()?;
            ColumnValidation::check_dense_count(&name, pc.size(), num_docs)?;
            let Some(point_writer) = self.per_fields[pf].point_values.as_mut() else {
                return Err(LuceneError::IllegalState(format!(
                    "field=\"{name}\" declares point dimensions but has no points writer"
                )));
            };
            point_writer.add_dense_nd_values(base_doc_id, pc.as_mut())?;
        }
        Ok(())
    }

    /// Feeds a dictionary-encoded column's doc values.
    ///
    /// Equivalent to `IndexingChain.processDictionaryColumn(int, int,
    /// DictionaryColumn, PerField, IndexableFieldType)`.
    ///
    /// # Errors
    ///
    /// As [`process_long_column`](Self::process_long_column).
    fn process_dictionary_column(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        column: &dyn DictionaryColumn,
        pf: usize,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        let dv_type = field_type.doc_values_type();
        let dict = column.dictionary();
        let name = column.column().name().to_string();

        match dv_type {
            DocValuesType::SORTED => {
                let dense = column.column().density() == Density::Dense;
                if dense {
                    let mut cursor = column.values()?;
                    ColumnValidation::check_dense_count(&name, cursor.size(), num_docs)?;
                    match self.per_fields[pf].doc_values.as_mut() {
                        Some(DocValuesWriter::Sorted(w)) => {
                            w.add_dense_ordinal_values(base_doc_id, dict, cursor.as_mut())?;
                        }
                        _ => return Err(Self::incompatible_dv("DictionaryColumn", &name, dv_type)),
                    }
                } else {
                    let mut cursor = column.tuples()?;
                    match self.per_fields[pf].doc_values.as_mut() {
                        Some(DocValuesWriter::Sorted(w)) => {
                            w.add_ordinal_tuples(base_doc_id, dict, cursor.as_mut())?;
                        }
                        _ => return Err(Self::incompatible_dv("DictionaryColumn", &name, dv_type)),
                    }
                }
            }
            DocValuesType::SORTED_SET => {
                let mut cursor = column.tuples()?;
                match self.per_fields[pf].doc_values.as_mut() {
                    Some(DocValuesWriter::SortedSet(w)) => {
                        w.add_ordinal_tuples(base_doc_id, dict, cursor.as_mut())?;
                    }
                    _ => return Err(Self::incompatible_dv("DictionaryColumn", &name, dv_type)),
                }
            }
            _ => return Err(Self::incompatible_dv("DictionaryColumn", &name, dv_type)),
        }
        Ok(())
    }

    /// Feeds a float-vector column.
    ///
    /// Equivalent to `IndexingChain.processVectorColumn(int, int, VectorColumn,
    /// PerField, IndexableFieldType)` under `VectorEncoding.FLOAT32`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a document id repeats or
    /// goes backwards, when a vector has the wrong dimension, or when the
    /// column names a document outside the batch.
    fn process_vector_column_float(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        column: &dyn VectorColumn<Vec<f32>>,
        pf: usize,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        let dimension = field_type.vector_dimension();
        let name = column.column().name().to_string();
        let mut cursor = column.tuples()?;
        let mut prev_batch_doc_id = -1;
        let mut consumed = 0;
        loop {
            let batch_doc_id = cursor.next_doc()?;
            if batch_doc_id == NO_MORE_DOCS {
                break;
            }
            ColumnValidation::check_doc_id(&name, batch_doc_id, num_docs)?;
            ColumnValidation::check_vector_doc_id_strictly_increasing(
                &name,
                batch_doc_id,
                prev_batch_doc_id,
            )?;
            let vec = cursor.value();
            ColumnValidation::check_vector_dimension(
                &name,
                vec.len() as i32,
                dimension,
                batch_doc_id,
            )?;
            match self.per_fields[pf].knn_field_vectors_writer.as_mut() {
                Some(FieldVectorWriter::Float(writer)) => {
                    writer.add_value(base_doc_id + batch_doc_id, vec.clone())?;
                }
                _ => {
                    return Err(LuceneError::IllegalState(format!(
                        "field=\"{name}\" is indexed with FLOAT32 vectors but has no float vectors \
                         writer"
                    )))
                }
            }
            prev_batch_doc_id = batch_doc_id;
            consumed += 1;
        }
        if column.column().density() == Density::Dense {
            ColumnValidation::check_dense_count(&name, consumed, num_docs)?;
        }
        Ok(())
    }

    /// Feeds a byte-vector column.
    ///
    /// Equivalent to `IndexingChain.processVectorColumn(int, int, VectorColumn,
    /// PerField, IndexableFieldType)` under `VectorEncoding.BYTE`.
    ///
    /// # Errors
    ///
    /// As [`process_vector_column_float`](Self::process_vector_column_float).
    fn process_vector_column_byte(
        &mut self,
        base_doc_id: i32,
        num_docs: i32,
        column: &dyn VectorColumn<Vec<u8>>,
        pf: usize,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        let dimension = field_type.vector_dimension();
        let name = column.column().name().to_string();
        let mut cursor = column.tuples()?;
        let mut prev_batch_doc_id = -1;
        let mut consumed = 0;
        loop {
            let batch_doc_id = cursor.next_doc()?;
            if batch_doc_id == NO_MORE_DOCS {
                break;
            }
            ColumnValidation::check_doc_id(&name, batch_doc_id, num_docs)?;
            ColumnValidation::check_vector_doc_id_strictly_increasing(
                &name,
                batch_doc_id,
                prev_batch_doc_id,
            )?;
            let vec = cursor.value();
            ColumnValidation::check_vector_dimension(
                &name,
                vec.len() as i32,
                dimension,
                batch_doc_id,
            )?;
            match self.per_fields[pf].knn_field_vectors_writer.as_mut() {
                Some(FieldVectorWriter::Byte(writer)) => {
                    writer.add_value(base_doc_id + batch_doc_id, vec.clone())?;
                }
                _ => {
                    return Err(LuceneError::IllegalState(format!(
                        "field=\"{name}\" is indexed with BYTE vectors but has no byte vectors \
                         writer"
                    )))
                }
            }
            prev_batch_doc_id = batch_doc_id;
            consumed += 1;
        }
        if column.column().density() == Density::Dense {
            ColumnValidation::check_dense_count(&name, consumed, num_docs)?;
        }
        Ok(())
    }
}

impl Accountable for DefaultIndexingChain {
    /// Equivalent to `IndexingChain.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        self.bytes_used.load(Ordering::Acquire)
            + self
                .stored_fields_consumer
                .as_ref()
                .map_or(0, StoredFieldsConsumer::ram_bytes_used)
            + self
                .term_vectors_consumer
                .as_ref()
                .map_or(0, TermVectorsConsumer::ram_bytes_used)
            // `IndexingChain.ramBytesUsed` ends with
            // `vectorValuesConsumer.getAccountable().ramBytesUsed()`
            // (`IndexingChain.java:1720-1724`).
            + self
                .vector_values_consumer
                .as_ref()
                .map_or(0, VectorValuesConsumer::ram_bytes_used)
    }

    /// Returns the three consumers whose bytes
    /// [`ram_bytes_used`](Self::ram_bytes_used) adds to `bytes_used`.
    ///
    /// Equivalent to `IndexingChain.getChildResources()`, which returns
    /// `List.of(storedFieldsConsumer.accountable, termVectorsWriter.accountable,
    /// vectorValuesConsumer.getAccountable())`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's three fields exist from the
    /// moment the chain is constructed and default to
    /// `Accountable.NULL_ACCOUNTABLE`, so the list always has three entries; in
    /// this port the three consumers are bound when the chain is bound to a
    /// segment, so before that this returns fewer entries. Every entry reports
    /// the same bytes Java's counterpart would, and a consumer without a writer
    /// reports zero either way.
    fn child_resources(&self) -> Vec<&dyn Accountable> {
        let mut resources: Vec<&dyn Accountable> = Vec::with_capacity(3);
        if let Some(consumer) = self.stored_fields_consumer.as_ref() {
            resources.push(consumer);
        }
        if let Some(consumer) = self.term_vectors_consumer.as_ref() {
            resources.push(consumer);
        }
        if let Some(consumer) = self.vector_values_consumer.as_ref() {
            resources.push(consumer);
        }
        resources
    }
}

impl IndexingChain for DefaultIndexingChain {
    /// Returns an iterator over the documents of this segment that have a
    /// doc-values value for `field`, or `None` when the field has no doc values
    /// buffered.
    ///
    /// Equivalent to `IndexingChain.getHasDocValues(String)`, which
    /// `DocumentsWriterPerThread.flush` calls **before** flushing so it can
    /// count how many documents the soft-deletes field marks
    /// (`DocumentsWriterPerThread.java:486-499`).
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the buffered doc-values view raises.
    fn get_has_doc_values(&self, field: &str) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let Some(index) = self.per_field_index.get(field) else {
            return Ok(None);
        };
        let per_field = &self.per_fields[*index];
        let Some(writer) = per_field.doc_values.as_ref() else {
            return Ok(None);
        };
        let info = &per_field.field_info;
        if info.doc_values_type == DocValuesType::NONE {
            return Ok(None);
        }
        let producer = writer.get_doc_values()?;
        Ok(Some(match info.doc_values_type {
            DocValuesType::NONE => return Ok(None),
            DocValuesType::NUMERIC => numeric_as_iterator(producer.get_numeric(info)?),
            DocValuesType::BINARY => binary_as_iterator(producer.get_binary(info)?),
            DocValuesType::SORTED => sorted_as_iterator(producer.get_sorted(info)?),
            DocValuesType::SORTED_NUMERIC => {
                sorted_numeric_as_iterator(producer.get_sorted_numeric(info)?)
            }
            DocValuesType::SORTED_SET => sorted_set_as_iterator(producer.get_sorted_set(info)?),
        }))
    }

    /// Indexes a column-oriented batch of documents.
    ///
    /// Equivalent to `IndexingChain.processBatch(int, ColumnBatch)`: the
    /// columns are validated and their fields initialised first, then the
    /// row-oriented features — stored fields and term inversion — are fed
    /// document by document, and finally the column-oriented ones — doc values,
    /// points and vectors — are fed column by column.
    ///
    /// `base_doc_id` is the segment-level document id of the batch's first
    /// document: batch-local document 0 maps to it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a column is invalid, when
    /// two columns of one field claim the same indexing feature, or when a
    /// cursor names a document outside the batch; and propagates whatever
    /// reading a column or writing a buffer raises.
    fn process_batch(
        &mut self,
        base_doc_id: i32,
        column_batch: &dyn ColumnBatch,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<()> {
        let num_docs = column_batch.num_docs();
        let columns = column_batch.columns();
        let mut has_row_columns = false;
        let batch_gen = self.next_field_gen;
        self.next_field_gen += 1;
        let codec = self.config.codec();

        // First pass: validate every column and accumulate each field's
        // schema. A batch may carry more than one column for a field name, to
        // combine distinct features — see `ColumnValidation::feature_mask`.
        let mut doc_fields: Vec<usize> = Vec::with_capacity(columns.len());
        for column in &columns {
            let field_name = column.column().name();
            let field_type = Arc::clone(column.column().field_type());

            ColumnValidation::validate_column_has_indexing_feature(
                field_name,
                field_type.as_ref(),
            )?;
            self.reject_reserved_parent_field(field_name)?;

            match column {
                TypedColumn::Binary(bc) => ColumnValidation::validate_binary_column(
                    field_name,
                    bc.stored_type(),
                    field_type.as_ref(),
                )?,
                TypedColumn::Long(lc) => ColumnValidation::validate_long_column(
                    field_name,
                    lc.numeric_kind(),
                    lc.stored_type(),
                    field_type.as_ref(),
                )?,
                TypedColumn::Dictionary(dc) => ColumnValidation::validate_dictionary_column(
                    field_name,
                    dc.stored_type(),
                    field_type.as_ref(),
                )?,
                TypedColumn::VectorFloat(_) | TypedColumn::VectorByte(_) => {
                    ColumnValidation::validate_vector_column(field_name, field_type.as_ref())?;
                }
                TypedColumn::TokenStream(_) => {
                    ColumnValidation::validate_token_stream_column(
                        field_name,
                        field_type.as_ref(),
                    )?;
                }
            }

            if field_type.stored() || field_type.index_options() != IndexOptions::NONE {
                has_row_columns = true;
            }

            // Java looks the `PerField` up by name and initialises its
            // `FieldInfo` after the loop; this port's `get_or_add_per_field`
            // does both at once, over the `FieldInfo` the builder registers —
            // the same arrangement `process_document` uses.
            let adapter = ColumnFieldTypeCarrier {
                name: field_name.to_string(),
                field_type: Arc::clone(&field_type),
            };
            let described = Self::describe_field(&adapter, field_infos, codec.as_ref())?;
            let registered = field_infos.add(&described)?;
            let index = self.get_or_add_per_field(registered)?;

            let column_features = ColumnValidation::feature_mask(field_type.as_ref());
            if self.per_fields[index].field_gen != batch_gen {
                // The first column for this field name in this batch: start a
                // fresh feature set.
                self.per_fields[index].field_gen = batch_gen;
                self.per_fields[index].column_features = column_features as u8;
            } else {
                // Each indexing feature must come from a single column for a
                // given field name.
                let overlap = i32::from(self.per_fields[index].column_features) & column_features;
                if overlap != 0 {
                    return Err(LuceneError::IllegalArgument(format!(
                        "ColumnBatch has multiple columns for field \"{field_name}\" claiming the \
                         same indexing feature {}; each feature may appear in at most one column.",
                        ColumnValidation::feature_names(overlap)
                    )));
                }
                self.per_fields[index].column_features |= column_features as u8;
            }

            doc_fields.push(index);
        }

        // Every document of a batch is an individual document rather than part
        // of a block, so each one is its own parent
        // (`IndexingChain.java:790-792`).
        self.process_parent_field_for_column_batch(base_doc_id, num_docs, field_infos)?;

        // Row-oriented pass: stored fields and term inversion only.
        if has_row_columns {
            self.process_row_columns(base_doc_id, num_docs, &columns, &doc_fields, field_infos)?;
        }

        // Column-oriented pass: doc values, points and vectors. Each column is
        // asked for a fresh cursor.
        for (col_oriented_idx, column) in columns.iter().enumerate() {
            let field_type = Arc::clone(column.column().field_type());
            if field_type.doc_values_type() == DocValuesType::NONE
                && field_type.point_dimension_count() == 0
                && field_type.vector_dimension() == 0
            {
                // No column-oriented feature.
                continue;
            }
            let pf = doc_fields[col_oriented_idx];
            match column {
                TypedColumn::Long(long_col) => {
                    self.process_long_column(
                        base_doc_id,
                        num_docs,
                        *long_col,
                        pf,
                        field_type.as_ref(),
                    )?;
                }
                TypedColumn::Binary(binary_col) => {
                    self.process_binary_column(
                        base_doc_id,
                        num_docs,
                        *binary_col,
                        pf,
                        field_type.as_ref(),
                    )?;
                }
                TypedColumn::Dictionary(dict_col) => {
                    self.process_dictionary_column(
                        base_doc_id,
                        num_docs,
                        *dict_col,
                        pf,
                        field_type.as_ref(),
                    )?;
                }
                TypedColumn::VectorFloat(vector_col) => {
                    self.process_vector_column_float(
                        base_doc_id,
                        num_docs,
                        *vector_col,
                        pf,
                        field_type.as_ref(),
                    )?;
                }
                TypedColumn::VectorByte(vector_col) => {
                    self.process_vector_column_byte(
                        base_doc_id,
                        num_docs,
                        *vector_col,
                        pf,
                        field_type.as_ref(),
                    )?;
                }
                TypedColumn::TokenStream(tsc) => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Unknown column type: token-stream column \"{}\" has no column-oriented \
                         feature",
                        tsc.column().name()
                    )));
                }
            }
        }
        Ok(())
    }

    fn process_document(
        &mut self,
        doc_id: i32,
        doc: &Document,
        is_last_doc: bool,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<()> {
        let analyzer = self.config.analyzer_arc();
        let info_stream = self.config.info_stream();
        let codec = self.config.codec();
        let field_gen = self.next_field_gen;
        self.next_field_gen += 1;

        // Handle the parent field first, before the document's own fields: only
        // the last document of a block carries it
        // (`IndexingChain.java:607-614`). Java only has to create its
        // `FieldInfo` here, because the `PerField` already exists; this port
        // creates both, which is the same step deferred to the same moment.
        let parent_pf = if is_last_doc {
            self.get_or_add_parent_per_field(field_infos)?
        } else {
            None
        };

        // Two passes, as in Lucene: every instance of a multi-valued field must
        // be inverted together, because the analyzer may reuse one TokenStream
        // across fields.
        let mut doc_fields: Vec<usize> = Vec::with_capacity(doc.get_fields().len());
        for field in doc.get_fields() {
            self.reject_reserved_parent_field(field.name())?;
            let described = Self::describe_field(field.as_ref(), field_infos, codec.as_ref())?;
            // The *live* entry of the builder, not a copy of it. Java's
            // `initializeFieldInfo` hands `pf.setFieldInfo` the very object
            // `fieldInfos.add` returned (`IndexingChain.java:1324-1345`), and
            // the codec writes into it: `PerFieldKnnVectorsFormat.FieldsWriter`
            // stamps `PerFieldKnnVectorsFormat.format` and `.suffix` onto the
            // field info it is given, and the reader refuses a segment whose
            // `.fnm` lacks them. Registering a clone here would leave those
            // attributes on a copy nobody writes out, and produce an index
            // Lucene cannot open.
            let registered = field_infos.add(&described)?;
            let index = self.get_or_add_per_field(registered)?;
            if self.per_fields[index].field_gen != field_gen {
                self.per_fields[index].field_gen = field_gen;
                self.per_fields[index].first = true;
            }
            doc_fields.push(index);
        }

        // `termsHash.startDocument()` runs before the stored-fields frame is
        // opened (`IndexingChain.java:604-605`); for the term-vectors half it
        // only drops whatever the previous document left pending.
        if let Some(consumer) = self.term_vectors_consumer.as_mut() {
            consumer.start_document();
        }

        // Lucene opens a stored-fields frame for every document, before any
        // field is processed, and closes it in a `finally` block: a document
        // that stores nothing still occupies its own frame, so the stream stays
        // aligned with the doc ids.
        self.start_stored_fields(doc_id)?;

        let mut indexed: Vec<usize> = Vec::new();
        let mut outcome: Result<()> = Ok(());

        // 2nd pass — index the parent field first (`IndexingChain.java:650-657`).
        // `parent_field` is currently a `NumericDocValuesField`, so
        // `process_field` always returns `false` here, but the result is checked
        // defensively in case the parent field's representation changes.
        if let Some(parent_pf) = parent_pf {
            let parent_field = self
                .parent_field
                .take()
                .expect("INVARIANT: the per-field only exists when the parent field does");
            let result = self.process_field(
                doc_id,
                &parent_field,
                parent_pf,
                analyzer.as_ref(),
                info_stream.as_ref(),
                field_infos,
            );
            self.parent_field = Some(parent_field);
            match result {
                Ok(true) => indexed.push(parent_pf),
                Ok(false) => {}
                // Java lets the exception leave the `try` block, so the
                // `finally` tail below still closes the stored-fields frame this
                // document opened; recording it as the outcome does the same.
                Err(error) => outcome = Err(error),
            }
        }

        for (field, index) in doc.get_fields().iter().zip(doc_fields.iter().copied()) {
            if outcome.is_err() {
                break;
            }
            // `IndexingChain.invertAndStore` inverts first and stores second,
            // for every field instance, in document order.
            if field.field_type().index_options() != IndexOptions::NONE {
                let first = self.per_fields[index].first;
                let result = Self::invert(
                    analyzer.as_ref(),
                    info_stream.as_ref(),
                    &mut self.terms_writer,
                    self.term_vectors_consumer.as_mut(),
                    &mut self.per_fields[index],
                    doc_id,
                    field.as_ref(),
                    first,
                );
                // `pf.first = false` and `indexedField = true` run *after*
                // `pf.invert(...)` returns normally
                // (`IndexingChain.java:1411-1418`), so a field that threw is
                // never finished: its partial term vector is not queued and its
                // `sawPayloads` flag never reaches the field infos. Marking it
                // first would put bytes on disk for a document both engines
                // then delete.
                if result.is_ok() && first {
                    self.per_fields[index].first = false;
                    indexed.push(index);
                }
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
                        self.on_aborting_exception(LuceneError::CorruptIndex(format!(
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

            // `processField` runs `indexDocValue` for every instance of a
            // field that declares doc values, after the invert-and-store half
            // (`IndexingChain.java:1386-1391`); a validation failure is a
            // document-level problem exactly like the ones `invert` raises.
            if field.field_type().doc_values_type() != DocValuesType::NONE {
                let result = self.per_fields[index].doc_values.as_mut().expect(
                    "INVARIANT: every field with a doc-values type gets a writer in get_or_add_per_field",
                ).add_value(doc_id, field.as_ref());
                if let Err(error) = result {
                    if !matches!(
                        error,
                        LuceneError::IllegalArgument(_) | LuceneError::IllegalState(_)
                    ) {
                        self.on_aborting_exception(LuceneError::CorruptIndex(format!(
                            "indexing chain buffers may be corrupt after field \"{}\": {error}",
                            field.name()
                        )));
                    }
                    outcome = Err(error);
                    break;
                }
            }

            // `processField` then buffers the packed point value of every
            // instance of a field that declares point dimensions
            // (`IndexingChain.java:1393-1395`). Java reads
            // `field.binaryValue()` and lets `addPackedValue` reject a null
            // with "point value must not be null"; here the absent value is an
            // `Option`, and the same message is raised for it.
            if field.field_type().point_dimension_count() != 0 {
                let result = match field.binary_value() {
                    Some(value) => self.per_fields[index]
                        .point_values
                        .as_mut()
                        .expect(
                            "INVARIANT: every field with point dimensions gets a writer in get_or_add_per_field",
                        )
                        .add_packed_value(doc_id, &value),
                    None => Err(LuceneError::IllegalArgument(format!(
                        "field=\"{}\": point value must not be null",
                        field.name()
                    ))),
                };
                if let Err(error) = result {
                    if !matches!(
                        error,
                        LuceneError::IllegalArgument(_) | LuceneError::IllegalState(_)
                    ) {
                        self.on_aborting_exception(LuceneError::CorruptIndex(format!(
                            "indexing chain buffers may be corrupt after field \"{}\": {error}",
                            field.name()
                        )));
                    }
                    outcome = Err(error);
                    break;
                }
            }

            // `processField` ends with `indexVectorValue` for every instance of
            // a field that declares vector dimensions
            // (`IndexingChain.java:1396-1398`). Java switches on the encoding
            // and downcasts the field to `KnnByteVectorField` or
            // `KnnFloatVectorField` to read its array
            // (`IndexingChain.java:1695-1707`); this reads the matching sibling
            // accessor, and an absent value is the `ClassCastException` Java
            // would raise for a field whose type promises a vector its class
            // cannot produce.
            if field.field_type().vector_dimension() != 0 {
                let result =
                    Self::index_vector_value(doc_id, &mut self.per_fields[index], field.as_ref());
                if let Err(error) = result {
                    if !matches!(
                        error,
                        LuceneError::IllegalArgument(_) | LuceneError::IllegalState(_)
                    ) {
                        self.on_aborting_exception(LuceneError::CorruptIndex(format!(
                            "the vectors of segment may be corrupt after field \"{}\": {error}",
                            field.name()
                        )));
                    }
                    outcome = Err(error);
                    break;
                }
            }
        }

        // Lucene's `finally` runs the whole tail — finishing every indexed
        // field, closing the stored-fields frame and finishing the term
        // vectors — only when no aborting exception was recorded, because in
        // that case the whole segment is discarded and none of it matters
        // (`IndexingChain.java:669-686`).
        if self.aborting_error.is_none() {
            // `PerField.finish(docID)`: `FreqProxTermsWriterPerField.finish`
            // records that the field stores payloads, which the field infos
            // must carry into the segment, and
            // `TermVectorsConsumerPerField.finish` queues the field's vector.
            let similarity = self.config.similarity();
            for index in indexed {
                // `PerField.finish(docID)` computes the norm *before*
                // `termsHashPerField.finish()` (`IndexingChain.java:1853-1870`).
                if let Err(error) =
                    Self::finish_norms(similarity.as_ref(), &mut self.per_fields[index], doc_id)
                {
                    // A norm failure is a document-level problem, exactly like
                    // the validation errors `invert` raises: the document is
                    // dropped and indexing continues. Java propagates it out of
                    // its `finally` and so skips the rest of the tail; this port
                    // keeps running it, because the stored-fields stream must
                    // stay aligned with the doc ids for the documents that did
                    // succeed. The error is still returned to the caller, and
                    // the first failure is the one reported.
                    if outcome.is_ok() {
                        outcome = Err(error);
                    }
                }
                let per_field = &self.per_fields[index];
                if let Some(writer_index) = per_field.writer_index {
                    if self.terms_writer.field(writer_index).saw_payloads() {
                        if let Some(info) = field_infos.field_info_mut(&per_field.field_info.name) {
                            info.set_store_payloads();
                        }
                    }
                }
                if let (Some(consumer), Some(vectors_index)) =
                    (self.term_vectors_consumer.as_mut(), per_field.vectors_index)
                {
                    consumer.finish_field(vectors_index);
                }
            }

            self.finish_stored_fields()?;

            // `termsHash.finishDocument(docID)`: a failure here may have left
            // the on-disk term vectors corrupt, so Lucene routes it straight to
            // the aborting-exception consumer.
            if let Some(consumer) = self.term_vectors_consumer.as_mut() {
                if let Err(error) = consumer.finish_document(doc_id, self.terms_writer.pool()) {
                    self.on_aborting_exception(LuceneError::CorruptIndex(format!(
                        "the term vectors of segment may be corrupt after doc {doc_id}: {error}"
                    )));
                    return Err(error);
                }
                // `TermVectorsConsumerPerField.finishDocument` ends with
                // `fieldInfo.setStoreTermVectors()`; the field infos this port
                // writes live in the builder, so the flag is set here instead.
                for per_field in &self.per_fields {
                    let Some(vectors_index) = per_field.vectors_index else {
                        continue;
                    };
                    if consumer.field(vectors_index).wrote_vectors() {
                        if let Some(info) = field_infos.field_info_mut(&per_field.field_info.name) {
                            info.set_store_term_vectors()?;
                        }
                    }
                }
            }
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
        // Lucene chooses `SortingStoredFieldsConsumer` and
        // `SortingTermVectorsConsumer` here when the segment has an index sort;
        // index sorting is a separate port, so the plain consumers are the only
        // option for now.
        self.stored_fields_consumer = Some(StoredFieldsConsumer::new(
            self.config.codec(),
            Arc::clone(&directory) as Arc<dyn crate::store::Directory>,
            segment_info.clone(),
        ));
        self.term_vectors_consumer = Some(TermVectorsConsumer::new(
            self.config.codec(),
            Arc::clone(&directory) as Arc<dyn crate::store::Directory>,
            segment_info.clone(),
            Arc::clone(&self.bytes_used),
        ));
        // `IndexingChain`'s constructor builds the vectors consumer before the
        // stored-fields and term-vectors ones (`IndexingChain.java:134-135`);
        // none of the three writes anything here, so only the objects differ in
        // age.
        self.vector_values_consumer = Some(VectorValuesConsumer::new(
            self.config.codec(),
            directory as Arc<dyn crate::store::Directory>,
            segment_info.clone(),
            self.config.info_stream(),
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
        // `TermsHash.abort()` resets the postings buffers and then aborts the
        // next hash of the chain, which closes the term-vectors writer.
        self.terms_writer.abort();
        if let Some(consumer) = self.term_vectors_consumer.as_mut() {
            consumer.abort();
        }
        // `IndexingChain.abort` closes the vectors writer inside the same
        // finalizer chain (`IndexingChain.java:537-541`), swallowing whatever
        // the close raises: the segment is going away either way.
        if let Some(consumer) = self.vector_values_consumer.as_mut() {
            consumer.abort();
        }
        self.per_fields.clear();
        self.per_field_index.clear();
        self.bytes_used.store(0, Ordering::Release);
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

        // Lucene writes the norms first, then the doc values, then the points,
        // then the stored fields, then `termsHash.flush`, whose `super.flush`
        // finishes the term vectors before the postings format runs
        // (`IndexingChain.java:305`, `:319`, `:326`, `:340-370`,
        // `FreqProxTermsWriter.java:118`). They write different files, so only
        // the order of the calls is reproduced here.
        let max_doc = state.segment_info.max_doc()?;
        // `maybeSortSegment` runs first, before a single file is written: every
        // consumer below needs the map to write its column in the sorted order
        // (`IndexingChain.java:302`).
        let sort_map = self.maybe_sort_segment(state.segment_info, state.field_infos)?;
        self.write_norms(&write_state, max_doc)?;
        self.write_doc_values(&write_state)?;
        self.write_points(&write_state)?;

        // `vectorValuesConsumer.flush(state, sortMap)` runs between the points
        // and the stored fields (`IndexingChain.java:333`).
        if let Some(consumer) = self.vector_values_consumer.as_mut() {
            consumer.flush(max_doc, sort_map.as_ref())?;
        }

        if let Some(consumer) = self.stored_fields_consumer.as_mut() {
            consumer.finish(max_doc)?;
            consumer.flush(state.segment_info)?;
        }

        if let Some(consumer) = self.term_vectors_consumer.as_mut() {
            consumer.flush(max_doc)?;
        }

        // **Divergence from Lucene 10.5.0.** Java hands `termsHash.flush` the
        // merge instance of a real norms producer opened over the files
        // `write_norms` has just written (`IndexingChain.java:361-370`),
        // because its postings writer folds the per-document norm into the
        // impact blocks. This flush still hands it a norm of one for every
        // document, so the impacts of a field with norms differ from Lucene's.
        // A merge reads the merged segment's real norms
        // (`SegmentMerger::merge_terms`).
        let norms = EmptyNormsProducer::new(max_doc);
        self.terms_writer
            .flush(&mut write_state, state.delete_terms, &norms)?;

        // Important to save after asking the consumers to flush, so that a
        // consumer can still alter a `FieldInfo` on its way out — Lucene's
        // `FreqProxTermsWriter` does exactly that with `FieldInfo.storePayloads`
        // (`IndexingChain.java:377-387`). Java takes the codec off the
        // `IndexWriterConfig` and passes `IOContext.DEFAULT` rather than the
        // flush context, and both are reproduced here.
        self.config.codec().field_infos_format().write(
            state.directory,
            state.segment_info,
            "",
            state.field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        )?;

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
            sort_map,
        })
    }

    fn take_aborting_error(&mut self) -> Option<LuceneError> {
        self.aborting_error.take()
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::analysis::tokenattributes::{
        CharTermAttribute, PayloadAttributeImpl as PayloadImpl,
    };
    use crate::analysis::{default_token_attribute_factory, StandardAnalyzer, TokenStream};
    use crate::codecs::{register_codec, Codec, Lucene104Codec};
    use crate::document::{
        Field, FieldType, IntField, KnnFloatVectorField, Store, StoredField, StringField, TextField,
    };
    use crate::index::documents_writer::TermDelete;
    use crate::index::field_infos::FieldNumbers;
    use crate::index::{SegmentInfo, Term, VectorEncoding, VectorSimilarityFunction};
    use crate::search::similarities::{CollectionStatistics, SimScorer, TermStatistics};
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
    fn indexing_chain_child_resources_are_its_three_consumers() {
        let unbound = DefaultIndexingChain::new(config());
        assert!(
            unbound.child_resources().is_empty(),
            "before a segment is bound there is no consumer to report"
        );

        let (chain, _tracking, _info) = bound_chain(1);
        let children = chain.child_resources();
        assert_eq!(
            children.len(),
            3,
            "stored fields, term vectors and vectors, in Java's order"
        );
        let children_bytes: i64 = children.iter().map(|c| c.ram_bytes_used()).sum();
        // `ramBytesUsed` is `bytesUsed` plus exactly these three.
        assert_eq!(
            chain.ram_bytes_used(),
            chain.bytes_used.load(Ordering::Acquire) + children_bytes
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

    // -- Vectors -----------------------------------------------------------

    /// A field type that promises a vector its field cannot produce.
    ///
    /// Java reaches this state as a `ClassCastException` inside
    /// `IndexingChain.indexVectorValue`, which downcasts to
    /// `KnnFloatVectorField`. Rust has no downcast, so the state is reachable
    /// through the trait's default accessors — and the chain has to reject it
    /// rather than index nothing.
    #[derive(Debug)]
    struct VectorlessVectorField {
        name: String,
        field_type: FieldType,
    }

    impl VectorlessVectorField {
        fn new(name: &str, encoding: VectorEncoding) -> Self {
            let mut field_type = FieldType::new();
            field_type
                .set_vector_attributes(3, encoding, VectorSimilarityFunction::EUCLIDEAN)
                .expect("vector attributes");
            field_type.freeze();
            Self {
                name: name.to_string(),
                field_type,
            }
        }
    }

    impl std::fmt::Display for VectorlessVectorField {
        /// Test double; it renders under its own name. `Display` is required
        /// because `IndexableField` carries it as a supertrait, standing in for
        /// the `toString()` every Java field inherits.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("VectorlessVectorField")
        }
    }

    impl IndexableField for VectorlessVectorField {
        fn name(&self) -> &str {
            &self.name
        }
        fn field_type(&self) -> &dyn IndexableFieldType {
            &self.field_type
        }
        fn token_stream(
            &self,
            _analyzer: &dyn Analyzer,
            _reuse: Option<&mut dyn crate::analysis::TokenStream>,
        ) -> Box<dyn crate::analysis::TokenStream> {
            Box::new(crate::analysis::StringTokenStream::new(String::new()).expect("stream"))
        }
        fn binary_value(&self) -> Option<BytesRef> {
            None
        }
        fn string_value(&self) -> Option<String> {
            None
        }
        fn reader_value(&mut self) -> Option<&mut dyn std::io::Read> {
            None
        }
        fn numeric_value(&self) -> Option<crate::document::NumericValue> {
            None
        }
        fn stored_value(&self) -> Result<Option<crate::document::StoredValue>> {
            Ok(None)
        }
        fn invertable_type(&self) -> Option<InvertableType> {
            None
        }
    }

    /// The vector reaches the codec's per-field writer, and the segment gains
    /// the four vector files. This is the whole point of the seam, so it is
    /// asserted before any of the guards around it.
    #[test]
    fn a_vector_field_reaches_the_codec_and_writes_the_segment_files() {
        let (mut chain, tracking, _info) = bound_chain(2);
        let mut field_infos = builder();
        for doc in 0..2 {
            let mut document = Document::new();
            document.add(Box::new(
                KnnFloatVectorField::with_euclidean("v", &[doc as f32 + 1.0, 2.0, 3.0])
                    .expect("float vector field"),
            ));
            chain
                .process_document(doc, &document, true, &mut field_infos)
                .expect("process document");
        }
        let created = tracking.get_created_files();
        for extension in ["vec", "vemf", "vex", "vem"] {
            assert!(
                created
                    .iter()
                    .any(|name| name.ends_with(&format!(".{extension}"))),
                "no .{extension} was created; the chain wrote {created:?}"
            );
        }
        assert!(
            chain.ram_bytes_used() > 0,
            "the buffered vectors must be reported to the segment's RAM total"
        );
    }

    /// The vectors consumer is created lazily, so a chain that never sees a
    /// vector field must leave no vector file behind.
    #[test]
    fn a_chain_without_vector_fields_writes_no_vector_file() {
        let (mut chain, tracking, _info) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS,
            vec![Tok::new("a", 1, 0, 1)],
        ));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");
        for name in tracking.get_created_files() {
            for extension in [".vec", ".vemf", ".vex", ".vem"] {
                assert!(
                    !name.ends_with(extension),
                    "a segment with no vector field wrote {name}"
                );
            }
        }
    }

    /// Lucene rejects a document that offers two values for one vector field,
    /// with a message that names the field. The rejection is document-level:
    /// indexing continues afterwards.
    #[test]
    fn a_document_may_not_repeat_a_vector_field() {
        let (mut chain, _tracking, _info) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(Box::new(
            KnnFloatVectorField::with_euclidean("v", &[1.0, 2.0]).expect("first"),
        ));
        document.add(Box::new(
            KnnFloatVectorField::with_euclidean("v", &[3.0, 4.0]).expect("second"),
        ));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("a repeated vector field must be rejected");
        assert!(
            matches!(&error, LuceneError::IllegalArgument(message)
                if message.contains("appears more than once in this document")
                    && message.contains('v')),
            "unexpected error: {error:?}"
        );
        assert!(
            chain.take_aborting_error().is_none(),
            "a repeated vector field is a document-level problem, not an aborting one"
        );
    }

    /// A dimension count the codec cannot store is rejected before the field
    /// info is built, with Java's message.
    #[test]
    fn a_vector_field_wider_than_the_codec_allows_is_rejected() {
        let (mut chain, _tracking, _info) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        // 1025 is one past `Lucene99HnswVectorsFormat.getMaxDimensions`.
        document.add(Box::new(
            KnnFloatVectorField::with_euclidean("v", &vec![1.0f32; 1025]).expect("wide vector"),
        ));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("an over-wide vector field must be rejected");
        assert!(
            matches!(&error, LuceneError::IllegalArgument(message)
                if message == "Field [v] vector's dimensions must be <= [1024]; got 1025"),
            "unexpected error: {error:?}"
        );
        assert!(
            field_infos.field_info("v").is_none(),
            "a rejected field must not reach the field infos"
        );
    }

    /// Exactly 1024 dimensions is the largest the codec accepts, so the guard
    /// above must not reject it. Without this the guard could be off by one in
    /// the strict direction and nothing would notice.
    #[test]
    fn the_maximum_dimension_count_is_accepted() {
        let (mut chain, _tracking, _info) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(Box::new(
            KnnFloatVectorField::with_euclidean("v", &vec![1.0f32; 1024]).expect("wide vector"),
        ));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("1024 dimensions must be accepted");
    }

    /// A field whose type promises a vector but whose value is absent is the
    /// state Java reaches as a `ClassCastException`.
    #[test]
    fn a_vector_field_without_a_vector_value_is_rejected() {
        for encoding in [VectorEncoding::FLOAT32, VectorEncoding::BYTE] {
            let (mut chain, _tracking, _info) = bound_chain(1);
            let mut field_infos = builder();
            let mut document = Document::new();
            document.add(Box::new(VectorlessVectorField::new("v", encoding)));
            let error = chain
                .process_document(0, &document, true, &mut field_infos)
                .expect_err("a vector field with no value must be rejected");
            assert!(
                matches!(&error, LuceneError::IllegalArgument(message)
                    if message.contains("carries no")),
                "unexpected error for {encoding:?}: {error:?}"
            );
        }
    }

    /// A chain that was never bound to a segment has nowhere to write vectors,
    /// so a vector field must be refused rather than silently dropped.
    #[test]
    fn an_unbound_chain_refuses_a_vector_field() {
        let mut chain = DefaultIndexingChain::new(Arc::new(LiveIndexWriterConfig::new(Arc::new(
            StandardAnalyzer::new(),
        ))));
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(Box::new(
            KnnFloatVectorField::with_euclidean("v", &[1.0, 2.0]).expect("vector field"),
        ));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("an unbound chain must refuse a vector field");
        assert!(
            matches!(&error, LuceneError::IllegalState(message)
                if message.contains("not bound to a segment")),
            "unexpected error: {error:?}"
        );
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

    // -- Norms -------------------------------------------------------------

    /// A field type that keeps norms; the shared [`field_type`] omits them.
    fn norm_field_type(options: IndexOptions) -> FieldType {
        let mut field_type = FieldType::new();
        field_type.set_tokenized(true).expect("tokenized");
        field_type.set_omit_norms(false).expect("keep norms");
        field_type
            .set_index_options(options)
            .expect("index options");
        field_type.freeze();
        field_type
    }

    fn scripted_norm_field(
        name: &str,
        options: IndexOptions,
        tokens: Vec<Tok>,
    ) -> Box<dyn IndexableField> {
        let stream: Rc<RefCell<dyn TokenStream>> =
            Rc::new(RefCell::new(ScriptedTokenStream::new(tokens)));
        Box::new(
            Field::new_with_token_stream(name, stream, norm_field_type(options))
                .expect("token stream field"),
        )
    }

    fn words(count: i32) -> Vec<Tok> {
        (0..count)
            .map(|i| Tok::new(&format!("t{i}"), 1, i * 4, i * 4 + 2))
            .collect()
    }

    /// Flushes `documents` with `config` and reads every norm back out of the
    /// segment the chain wrote, as `(field, doc, value)` in field-number order.
    fn flush_and_read_norms(
        config: Arc<LiveIndexWriterConfig>,
        documents: Vec<Document>,
    ) -> (Vec<String>, Vec<(String, i32, i64)>) {
        let max_doc = documents.len() as i32;
        let codec = ensure_codec();
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
        let indexing_info = make_info(-1);
        let mut chain =
            DefaultIndexingChain::new_for_segment(config, Arc::clone(&tracking), &indexing_info)
                .expect("bind segment");
        let mut field_infos = builder();
        for (doc_id, document) in documents.iter().enumerate() {
            chain
                .process_document(doc_id as i32, document, true, &mut field_infos)
                .expect("process document");
        }
        let finished = field_infos.finish().expect("field infos");
        let segment_info = make_info(max_doc);
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

        let mut files: Vec<String> = tracking.get_created_files().into_iter().collect();
        files.sort();

        let mut norms = Vec::new();
        if finished.has_norms() {
            let read_state = crate::codecs::state::SegmentReadState::new(
                tracking.as_ref(),
                &segment_info,
                &finished,
                &*crate::store::DEFAULT_IO_CONTEXT,
            );
            let producer = codec
                .norms_format()
                .norms_producer(&read_state)
                .expect("norms producer");
            producer.check_integrity().expect("integrity");
            for info in finished.iter().filter(|info| info.has_norms()) {
                let mut values = producer.get_norms(info).expect("norms");
                loop {
                    let doc = values.next_doc().expect("next doc");
                    if doc == crate::search::NO_MORE_DOCS {
                        break;
                    }
                    norms.push((
                        info.name.clone(),
                        doc,
                        values.long_value().expect("long value"),
                    ));
                }
            }
        }
        (files, norms)
    }

    #[test]
    fn a_stored_only_field_cannot_become_a_field_with_norms_mid_segment() {
        // The norms buffer is created once, when the field is first seen, so a
        // field that arrives stored-only and later asks to be indexed would
        // have none. It cannot: the field infos refuse the schema change first,
        // the document is dropped, and the segment flushes without norms — so
        // `write_norms`' "has norms but no norms buffer" guard stays
        // unreachable through the ordinary path.
        let mut stored_type = FieldType::new();
        stored_type.set_stored(true).expect("stored");
        stored_type.freeze();

        let mut first = Document::new();
        first.add(Box::new(
            Field::new("body", "value".to_string(), stored_type).expect("stored field"),
        ));
        let mut second = Document::new();
        second.add(scripted_norm_field(
            "body",
            IndexOptions::DOCS_AND_FREQS,
            words(2),
        ));

        let (mut chain, tracking, segment_info) = bound_chain(2);
        let mut field_infos = builder();
        chain
            .process_document(0, &first, true, &mut field_infos)
            .expect("the stored-only document is fine");
        let error = chain
            .process_document(1, &second, true, &mut field_infos)
            .expect_err("the schema change must be refused");
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("index options")),
            "unexpected error: {error:?}"
        );
        assert!(chain.take_aborting_error().is_none());

        let finished = field_infos.finish().expect("field infos");
        assert!(!finished.has_norms());
        let info_stream = NoOutputInfoStream;
        let context = flush_io_context(FlushInfo::new(2, 0));
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
        chain.flush(&state).expect("the segment still flushes");
    }

    #[test]
    fn a_field_with_norms_writes_the_two_norms_files() {
        let mut document = Document::new();
        document.add(scripted_norm_field(
            "body",
            IndexOptions::DOCS_AND_FREQS,
            words(3),
        ));
        let (files, norms) = flush_and_read_norms(config(), vec![document]);
        for extension in ["nvd", "nvm"] {
            assert!(
                files
                    .iter()
                    .any(|name| name.ends_with(&format!(".{extension}"))),
                "the flush must create a .{extension} file, got {files:?}"
            );
        }
        assert_eq!(norms, vec![("body".to_string(), 0, 3)]);
    }

    #[test]
    fn a_field_that_omits_norms_writes_no_norms_files() {
        // `IndexingChain.writeNorms` is guarded by `fieldInfos.hasNorms()`, so a
        // segment where every field omits them writes neither file.
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS_AND_FREQS,
            words(3),
        ));
        let (files, norms) = flush_and_read_norms(config(), vec![document]);
        for extension in ["nvd", "nvm"] {
            assert!(
                !files
                    .iter()
                    .any(|name| name.ends_with(&format!(".{extension}"))),
                "no .{extension} may be created, got {files:?}"
            );
        }
        assert!(norms.is_empty());
    }

    #[test]
    fn each_document_gets_the_norm_the_similarity_computed() {
        let mut documents = Vec::new();
        for doc in 0..6 {
            let mut document = Document::new();
            document.add(scripted_norm_field(
                "body",
                IndexOptions::DOCS_AND_FREQS,
                words(1 + doc * 5),
            ));
            documents.push(document);
        }
        let (_, norms) = flush_and_read_norms(config(), documents);
        let expected: Vec<(String, i32, i64)> = (0..6)
            .map(|doc| {
                let mut state =
                    FieldInvertState::new(10, "body".to_string(), IndexOptions::DOCS_AND_FREQS);
                state.set_length(1 + doc * 5);
                (
                    "body".to_string(),
                    doc,
                    crate::search::compute_default_norm(&state, true).expect("norm"),
                )
            })
            .collect();
        assert_eq!(norms, expected);
    }

    #[test]
    fn a_document_that_does_not_carry_the_field_has_no_norm() {
        // Absent is a value the format can express, and it must not be confused
        // with a norm of zero.
        let mut documents = Vec::new();
        for doc in 0..6 {
            let mut document = Document::new();
            if doc % 2 == 0 {
                document.add(scripted_norm_field(
                    "body",
                    IndexOptions::DOCS_AND_FREQS,
                    words(1 + doc),
                ));
            }
            // Something must be indexed in every document so the segment keeps
            // its doc ids.
            document.add(scripted_norm_field(
                "title",
                IndexOptions::DOCS_AND_FREQS,
                words(2),
            ));
            documents.push(document);
        }
        let (_, norms) = flush_and_read_norms(config(), documents);
        let body: Vec<i32> = norms
            .iter()
            .filter(|(name, _, _)| name == "body")
            .map(|(_, doc, _)| *doc)
            .collect();
        assert_eq!(body, vec![0, 2, 4]);
        let title: Vec<i32> = norms
            .iter()
            .filter(|(name, _, _)| name == "title")
            .map(|(_, doc, _)| *doc)
            .collect();
        assert_eq!(title, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_field_present_with_no_tokens_gets_a_norm_of_zero() {
        // `PerField.finish` short-circuits to zero when `invertState.length` is
        // zero, without asking the similarity — which is why a similarity that
        // answers zero for a *non-empty* field is a bug.
        let mut documents = Vec::new();
        for doc in 0..3 {
            let mut document = Document::new();
            document.add(scripted_norm_field(
                "body",
                IndexOptions::DOCS_AND_FREQS,
                if doc == 1 { Vec::new() } else { words(4) },
            ));
            documents.push(document);
        }
        let (_, norms) = flush_and_read_norms(config(), documents);
        assert_eq!(
            norms,
            vec![
                ("body".to_string(), 0, 4),
                ("body".to_string(), 1, 0),
                ("body".to_string(), 2, 4),
            ]
        );
    }

    #[test]
    fn a_similarity_that_answers_zero_for_a_non_empty_field_is_rejected() {
        #[derive(Debug)]
        struct ZeroSimilarity;
        impl Similarity for ZeroSimilarity {
            fn java_class_name(&self) -> &'static str {
                "org.apache.lucene.search.similarities.Similarity"
            }

            fn compute_norm(&self, _state: &FieldInvertState) -> Result<i64> {
                Ok(0)
            }

            fn scorer<'a>(
                &'a self,
                boost: f32,
                _collection_stats: &CollectionStatistics,
                _term_stats: &[TermStatistics],
            ) -> Box<dyn SimScorer + 'a> {
                // Java declares `scorer` abstract, so a `Similarity` double has
                // to answer it too. This fixture only exercises `computeNorm`.
                struct ConstantScorer(f32);
                impl SimScorer for ConstantScorer {
                    fn score(&self, _freq: f32, _norm: i64) -> f32 {
                        self.0
                    }
                }
                Box::new(ConstantScorer(boost))
            }
        }

        let mut live = LiveIndexWriterConfig::new(Arc::new(StandardAnalyzer::new()));
        live.set_similarity(Arc::new(ZeroSimilarity));
        ensure_codec();
        let config = Arc::new(live);

        let (mut chain, _, _) = {
            let codec = ensure_codec();
            let tracking = Arc::new(TrackingDirectoryWrapper::new(Box::new(
                ByteBuffersDirectory::new(),
            )));
            let directory: Arc<dyn Directory> = Arc::clone(&tracking) as Arc<dyn Directory>;
            let info = SegmentInfo::new(
                Arc::clone(&directory),
                Version::LATEST,
                Some(Version::LATEST),
                "_0".to_string(),
                -1,
                false,
                false,
                codec,
                HashMap::new(),
                [7u8; 16],
                HashMap::new(),
                Default::default(),
            )
            .expect("segment info");
            let chain = DefaultIndexingChain::new_for_segment(config, Arc::clone(&tracking), &info)
                .expect("bind segment");
            (chain, tracking, info)
        };

        let mut document = Document::new();
        document.add(scripted_norm_field(
            "body",
            IndexOptions::DOCS_AND_FREQS,
            words(3),
        ));
        let mut field_infos = builder();
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("a zero norm for a non-empty field must be refused");
        assert!(
            matches!(error, LuceneError::IllegalState(ref m)
                if m.contains("return 0 for non-empty field")),
            "unexpected error: {error:?}"
        );
        // The failure is document-level, not a reason to throw the segment away.
        assert!(chain.take_aborting_error().is_none());
    }

    #[test]
    fn overlap_tokens_are_discounted_from_the_norm() {
        // Two tokens per position: the default `discountOverlaps` subtracts the
        // ones whose position increment is zero.
        let mut document = Document::new();
        document.add(scripted_norm_field(
            "body",
            IndexOptions::DOCS_AND_FREQS,
            vec![
                Tok::new("a", 1, 0, 1),
                Tok::new("syn", 0, 0, 1),
                Tok::new("b", 1, 2, 3),
                Tok::new("syn2", 0, 2, 3),
            ],
        ));
        let (_, norms) = flush_and_read_norms(config(), vec![document]);
        assert_eq!(norms, vec![("body".to_string(), 0, 2)]);

        let mut live = LiveIndexWriterConfig::new(Arc::new(StandardAnalyzer::new()));
        live.set_similarity(Arc::new(
            crate::search::BM25Similarity::with_discount_overlaps(false),
        ));
        let mut document = Document::new();
        document.add(scripted_norm_field(
            "body",
            IndexOptions::DOCS_AND_FREQS,
            vec![
                Tok::new("a", 1, 0, 1),
                Tok::new("syn", 0, 0, 1),
                Tok::new("b", 1, 2, 3),
                Tok::new("syn2", 0, 2, 3),
            ],
        ));
        let (_, norms) = flush_and_read_norms(Arc::new(live), vec![document]);
        assert_eq!(norms, vec![("body".to_string(), 0, 4)]);
    }

    #[test]
    fn a_multi_valued_field_norms_the_sum_of_its_values() {
        let mut document = Document::new();
        document.add(scripted_norm_field(
            "body",
            IndexOptions::DOCS_AND_FREQS,
            words(3),
        ));
        document.add(scripted_norm_field(
            "body",
            IndexOptions::DOCS_AND_FREQS,
            words(4),
        ));
        let (_, norms) = flush_and_read_norms(config(), vec![document]);
        // One norm for the document, not one per value, and it counts every
        // token of both values.
        assert_eq!(norms, vec![("body".to_string(), 0, 7)]);
    }

    #[test]
    fn a_docs_only_field_norms_its_unique_term_count() {
        let mut document = Document::new();
        document.add(scripted_norm_field(
            "body",
            IndexOptions::DOCS,
            vec![
                Tok::new("a", 1, 0, 1),
                Tok::new("a", 1, 2, 3),
                Tok::new("b", 1, 4, 5),
                Tok::new("a", 1, 6, 7),
            ],
        ));
        let (_, norms) = flush_and_read_norms(config(), vec![document]);
        // Four tokens, two distinct terms.
        assert_eq!(norms, vec![("body".to_string(), 0, 2)]);
    }

    #[test]
    fn norms_are_written_in_field_number_order() {
        // `IndexingChain.writeNorms` iterates the field infos, which
        // `FieldInfos` orders by field number, and the `.nvm` entries follow
        // that order. The names here are deliberately not in lexical order, so
        // a writer or reader that sorted by name instead would disagree.
        let mut document = Document::new();
        for name in ["zeta", "alpha", "mu"] {
            document.add(scripted_norm_field(
                name,
                IndexOptions::DOCS_AND_FREQS,
                words(2),
            ));
        }
        let (_, norms) = flush_and_read_norms(config(), vec![document]);
        let names: Vec<&str> = norms.iter().map(|(name, _, _)| name.as_str()).collect();
        // Field numbers are assigned in first-seen order, so this is 0, 1, 2 —
        // and emphatically not "alpha", "mu", "zeta".
        assert_eq!(names, vec!["zeta", "alpha", "mu"]);
    }

    #[test]
    fn the_norms_buffers_are_reported_to_the_shared_ram_counter() {
        let (mut chain, _, _) = bound_chain(1);
        let empty = chain.ram_bytes_used();
        let mut field_infos = builder();
        for doc in 0..200 {
            let mut document = Document::new();
            document.add(scripted_norm_field(
                "body",
                IndexOptions::DOCS_AND_FREQS,
                words(2),
            ));
            chain
                .process_document(doc, &document, true, &mut field_infos)
                .expect("process document");
        }
        assert!(
            chain.ram_bytes_used() > empty,
            "two hundred buffered norms must be reported"
        );
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
        // The `.fnm` is unconditional: `IndexingChain.flush` writes the field
        // infos on its way out whatever the fields hold
        // (`IndexingChain.java:377-387`), and without it no reader could open
        // the segment. Everything else here is postings, and there are none.
        assert_eq!(
            extensions,
            vec![
                "fdm".to_string(),
                "fdt".to_string(),
                "fdx".to_string(),
                "fnm".to_string()
            ],
            "a stored-only document produces the stored-fields files, the field infos and no postings: {files:?}"
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

    // -- Term vectors ------------------------------------------------------

    /// Builds a frozen field type that indexes and, optionally, stores term
    /// vectors with the requested extras.
    fn tv_field_type(
        options: IndexOptions,
        vectors: bool,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> FieldType {
        let mut field_type = FieldType::new();
        field_type.set_tokenized(true).expect("tokenized");
        field_type.set_omit_norms(true).expect("omit norms");
        field_type
            .set_index_options(options)
            .expect("index options");
        field_type
            .set_store_term_vectors(vectors)
            .expect("store term vectors");
        field_type
            .set_store_term_vector_positions(positions)
            .expect("store term vector positions");
        field_type
            .set_store_term_vector_offsets(offsets)
            .expect("store term vector offsets");
        field_type
            .set_store_term_vector_payloads(payloads)
            .expect("store term vector payloads");
        field_type.freeze();
        field_type
    }

    fn tv_field(
        name: &str,
        options: IndexOptions,
        vectors: bool,
        positions: bool,
        offsets: bool,
        payloads: bool,
        tokens: Vec<Tok>,
    ) -> Box<dyn IndexableField> {
        let stream: Rc<RefCell<dyn TokenStream>> =
            Rc::new(RefCell::new(ScriptedTokenStream::new(tokens)));
        Box::new(
            Field::new_with_token_stream(
                name,
                stream,
                tv_field_type(options, vectors, positions, offsets, payloads),
            )
            .expect("token stream field"),
        )
    }

    /// One field of one document's term vector, decoded from the segment.
    #[derive(Debug, PartialEq, Eq)]
    struct VectorField {
        name: String,
        has_positions: bool,
        has_offsets: bool,
        has_payloads: bool,
        terms: Vec<VectorTerm>,
    }

    /// One term of one field of one document's term vector.
    #[derive(Debug, PartialEq, Eq)]
    struct VectorTerm {
        term: String,
        freq: i32,
        positions: Vec<i32>,
        offsets: Vec<(i32, i32)>,
        payloads: Vec<Option<Vec<u8>>>,
    }

    /// Decodes every term vector of `doc_id`, or `None` when the document has
    /// none.
    fn read_vectors(
        reader: &dyn crate::codecs::term_vectors::TermVectorsReader,
        doc_id: i32,
    ) -> Option<Vec<VectorField>> {
        let fields = reader.get(doc_id).expect("term vectors")?;
        let mut decoded = Vec::new();
        for name in fields.iterator() {
            let terms = fields
                .terms(&name)
                .expect("terms")
                .expect("the iterator only yields present fields");
            let has_positions = terms.has_positions();
            let has_offsets = terms.has_offsets();
            let has_payloads = terms.has_payloads();
            let mut iterator = terms.iterator().expect("terms enum");
            let mut decoded_terms = Vec::new();
            while let Some(term) = iterator.next().expect("next term") {
                let text = String::from_utf8(term.slice().to_vec()).expect("utf-8 term");
                let mut postings = iterator
                    .postings(None, crate::index::POSTINGS_ENUM_ALL)
                    .expect("postings");
                assert_eq!(postings.next_doc().expect("next doc"), 0);
                let freq = postings.freq().expect("freq");
                let mut positions = Vec::new();
                let mut offsets = Vec::new();
                let mut payloads = Vec::new();
                if has_positions || has_offsets {
                    for _ in 0..freq {
                        let position = postings.next_position().expect("next position");
                        if has_positions {
                            positions.push(position);
                        }
                        if has_offsets {
                            offsets.push((postings.start_offset(), postings.end_offset()));
                        }
                        if has_payloads {
                            payloads
                                .push(postings.get_payload().expect("payload").map(<[u8]>::to_vec));
                        }
                    }
                }
                decoded_terms.push(VectorTerm {
                    term: text,
                    freq,
                    positions,
                    offsets,
                    payloads,
                });
            }
            decoded.push(VectorField {
                name,
                has_positions,
                has_offsets,
                has_payloads,
                terms: decoded_terms,
            });
        }
        Some(decoded)
    }

    /// Indexes `documents`, flushes them, and returns the decoded term vectors
    /// of every document alongside the files the flush created.
    fn flush_and_read_vectors(
        documents: Vec<Document>,
    ) -> (Vec<String>, Vec<Option<Vec<VectorField>>>) {
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

        let mut files: Vec<String> = tracking.get_created_files().into_iter().collect();
        files.sort();

        let vectors = if files.iter().any(|name| name.ends_with(".tvd")) {
            let codec = ensure_codec();
            let reader = codec
                .term_vectors_format()
                .vectors_reader(
                    tracking.as_ref(),
                    &segment_info,
                    &finished,
                    &*crate::store::DEFAULT_IO_CONTEXT,
                )
                .expect("term vectors reader");
            (0..max_doc)
                .map(|doc_id| read_vectors(reader.as_ref(), doc_id))
                .collect()
        } else {
            (0..max_doc).map(|_| None).collect()
        };
        (files, vectors)
    }

    #[test]
    fn a_segment_without_vector_fields_writes_no_term_vector_file() {
        let mut document = Document::new();
        document.add(scripted_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        let (files, vectors) = flush_and_read_vectors(vec![document]);
        for extension in ["tvd", "tvx", "tvm"] {
            assert!(
                !files.iter().any(|name| name.ends_with(extension)),
                "no field asked for term vectors, yet {files:?} carries .{extension}"
            );
        }
        assert_eq!(vectors, vec![None]);
    }

    #[test]
    fn only_the_fields_that_asked_for_vectors_are_written() {
        let mut document = Document::new();
        document.add(tv_field(
            "with",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            false,
            false,
            false,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        document.add(scripted_field(
            "without",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            vec![Tok::new("beta", 1, 0, 4)],
        ));
        let (files, vectors) = flush_and_read_vectors(vec![document]);
        for extension in ["tvd", "tvx", "tvm"] {
            assert!(
                files.iter().any(|name| name.ends_with(extension)),
                "the flush must create a .{extension} file, got {files:?}"
            );
        }
        let doc = vectors[0].as_ref().expect("doc 0 has vectors");
        assert_eq!(
            doc.iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["with"]
        );
        assert_eq!(
            doc[0].terms,
            vec![VectorTerm {
                term: "alpha".to_string(),
                freq: 1,
                positions: Vec::new(),
                offsets: Vec::new(),
                payloads: Vec::new(),
            }]
        );
    }

    #[test]
    fn terms_come_back_sorted_with_their_frequency() {
        let mut document = Document::new();
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            false,
            false,
            false,
            vec![
                Tok::new("gamma", 1, 0, 5),
                Tok::new("alpha", 1, 6, 11),
                Tok::new("gamma", 1, 12, 17),
                Tok::new("beta", 1, 18, 22),
                Tok::new("gamma", 1, 23, 28),
            ],
        ));
        let (_, vectors) = flush_and_read_vectors(vec![document]);
        let doc = vectors[0].as_ref().expect("doc 0 has vectors");
        assert_eq!(
            doc[0]
                .terms
                .iter()
                .map(|term| (term.term.as_str(), term.freq))
                .collect::<Vec<_>>(),
            vec![("alpha", 1), ("beta", 1), ("gamma", 3)],
            "terms are sorted by their bytes and carry the in-document frequency"
        );
    }

    #[test]
    fn positions_offsets_and_payloads_round_trip_in_every_combination() {
        // Offsets may be stored without positions, positions without offsets,
        // payloads only alongside positions.
        for (positions, offsets, payloads) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, false),
            (true, false, true),
            (true, true, true),
        ] {
            let mut document = Document::new();
            document.add(tv_field(
                "body",
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                true,
                positions,
                offsets,
                payloads,
                vec![
                    Tok::new("alpha", 1, 0, 5).with_payload(b"one"),
                    Tok::new("beta", 2, 10, 14).with_payload(b"two"),
                    Tok::new("alpha", 1, 20, 25),
                ],
            ));
            let (_, vectors) = flush_and_read_vectors(vec![document]);
            let doc = vectors[0].as_ref().expect("doc 0 has vectors");
            let field = &doc[0];
            assert_eq!(field.has_positions, positions, "positions {positions:?}");
            assert_eq!(field.has_offsets, offsets, "offsets {offsets:?}");
            assert_eq!(
                field.has_payloads, payloads,
                "payloads are only reported once a token actually carried one"
            );

            let alpha = &field.terms[0];
            assert_eq!(alpha.term, "alpha");
            assert_eq!(alpha.freq, 2);
            if positions {
                assert_eq!(alpha.positions, vec![0, 3]);
            }
            if offsets {
                assert_eq!(alpha.offsets, vec![(0, 5), (20, 25)]);
            }
            if payloads {
                assert_eq!(
                    alpha.payloads,
                    vec![Some(b"one".to_vec()), None],
                    "only the first occurrence carried a payload"
                );
            }

            let beta = &field.terms[1];
            assert_eq!(beta.term, "beta");
            if positions {
                assert_eq!(beta.positions, vec![2]);
            }
            if offsets {
                assert_eq!(beta.offsets, vec![(10, 14)]);
            }
        }
    }

    #[test]
    fn a_payload_is_ignored_when_the_field_did_not_ask_for_one() {
        let mut document = Document::new();
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            false,
            false,
            vec![Tok::new("alpha", 1, 0, 5).with_payload(b"dropped")],
        ));
        let (_, vectors) = flush_and_read_vectors(vec![document]);
        let doc = vectors[0].as_ref().expect("doc 0 has vectors");
        assert!(
            !doc[0].has_payloads,
            "storeTermVectorPayloads was false, so the payload must not reach the stream"
        );
    }

    #[test]
    fn fields_are_written_in_name_order_whatever_the_document_order() {
        let mut document = Document::new();
        for name in ["zeta", "alpha", "mu"] {
            document.add(tv_field(
                name,
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                true,
                true,
                true,
                false,
                vec![Tok::new("token", 1, 0, 5)],
            ));
        }
        let (_, vectors) = flush_and_read_vectors(vec![document]);
        let doc = vectors[0].as_ref().expect("doc 0 has vectors");
        assert_eq!(
            doc.iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "mu", "zeta"]
        );
    }

    #[test]
    fn vector_fields_are_ordered_the_way_java_orders_strings() {
        // `String.compareTo` compares UTF-16 code units, so a supplementary
        // character sorts *before* `U+FFFF`; Rust's `str` ordering puts it
        // after. The order reaches the `.tvd` bytes, so the Java comparator is
        // the one that must be reproduced.
        let mut document = Document::new();
        for name in ["\u{FFFF}", "\u{10000}"] {
            document.add(tv_field(
                name,
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                true,
                true,
                false,
                false,
                vec![Tok::new("token", 1, 0, 5)],
            ));
        }
        let (_, vectors) = flush_and_read_vectors(vec![document]);
        let doc = vectors[0].as_ref().expect("doc 0 has vectors");
        assert_eq!(
            doc.iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["\u{10000}", "\u{FFFF}"]
        );
    }

    #[test]
    fn documents_without_vectors_keep_their_doc_id() {
        // Only the middle document carries vectors, and it is not the first, so
        // both the back-fill and the tail padding are exercised.
        let mut documents = Vec::new();
        for doc_id in 0..5 {
            let mut document = Document::new();
            if doc_id == 2 {
                document.add(tv_field(
                    "body",
                    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                    true,
                    true,
                    false,
                    false,
                    vec![Tok::new("alpha", 1, 0, 5)],
                ));
            } else {
                document.add(scripted_field(
                    "other",
                    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                    vec![Tok::new("beta", 1, 0, 4)],
                ));
            }
            documents.push(document);
        }
        let (_, vectors) = flush_and_read_vectors(documents);
        assert!(vectors[0].is_none());
        assert!(vectors[1].is_none());
        let doc = vectors[2].as_ref().expect("doc 2 has vectors");
        assert_eq!(doc[0].terms[0].term, "alpha");
        assert!(vectors[3].is_none());
        assert!(vectors[4].is_none());
    }

    #[test]
    fn an_empty_document_after_a_vector_document_still_occupies_a_frame() {
        let mut first = Document::new();
        first.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            true,
            false,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        let (_, vectors) = flush_and_read_vectors(vec![first, Document::new()]);
        assert!(vectors[0].is_some());
        assert!(
            vectors[1].is_none(),
            "the second document stored nothing, but its frame must exist"
        );
    }

    #[test]
    fn a_vector_field_with_no_token_writes_no_field_but_keeps_the_frame() {
        let mut first = Document::new();
        first.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            false,
            false,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        let mut second = Document::new();
        second.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            false,
            false,
            Vec::new(),
        ));
        let (_, vectors) = flush_and_read_vectors(vec![first, second]);
        assert!(vectors[0].is_some());
        assert!(
            vectors[1].is_none(),
            "a field that produced no term is not added to the document's frame"
        );
    }

    #[test]
    fn a_multi_valued_field_accumulates_one_vector() {
        let mut document = Document::new();
        for value in 0..2 {
            document.add(tv_field(
                "body",
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                true,
                true,
                true,
                false,
                vec![Tok::new("alpha", 1, value * 10, value * 10 + 5)],
            ));
        }
        let (_, vectors) = flush_and_read_vectors(vec![document]);
        let doc = vectors[0].as_ref().expect("doc 0 has vectors");
        assert_eq!(doc.len(), 1);
        assert_eq!(doc[0].terms.len(), 1);
        assert_eq!(doc[0].terms[0].freq, 2);
        assert_eq!(
            doc[0].terms[0].positions.len(),
            2,
            "both instances contribute to the same vector"
        );
    }

    #[test]
    fn many_documents_span_several_chunks() {
        // The compressing format flushes a chunk every 4 KiB of term bytes or
        // every 128 documents, so 400 documents of long terms cross both.
        let documents: Vec<Document> = (0..400)
            .map(|doc_id| {
                let mut document = Document::new();
                let tokens: Vec<Tok> = (0..8)
                    .map(|token| {
                        Tok::new(
                            &format!("term-{doc_id:04}-{token}-padding-padding"),
                            1,
                            token * 30,
                            token * 30 + 25,
                        )
                    })
                    .collect();
                document.add(tv_field(
                    "body",
                    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                    true,
                    true,
                    true,
                    false,
                    tokens,
                ));
                document
            })
            .collect();
        let (_, vectors) = flush_and_read_vectors(documents);
        for (doc_id, doc) in vectors.iter().enumerate() {
            let doc = doc.as_ref().unwrap_or_else(|| panic!("doc {doc_id}"));
            assert_eq!(doc[0].terms.len(), 8, "doc {doc_id}");
            assert_eq!(
                doc[0].terms[0].term,
                format!("term-{doc_id:04}-0-padding-padding")
            );
            assert_eq!(doc[0].terms[7].offsets, vec![(210, 235)]);
        }
    }

    #[test]
    fn a_position_delta_past_half_of_i32_round_trips() {
        // `writeProx` encodes the delta as `delta << 1`, which overflows an
        // `int` above `Integer.MAX_VALUE / 2`; Java wraps and `addProx` recovers
        // the value with `>>>`. `MAX_POSITION` still allows such a delta.
        let far = MAX_POSITION - 1;
        let mut document = Document::new();
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            false,
            false,
            vec![Tok::new("alpha", 1, 0, 5), Tok::new("alpha", far, 6, 11)],
        ));
        let (_, vectors) = flush_and_read_vectors(vec![document]);
        let doc = vectors[0].as_ref().expect("doc 0 has vectors");
        assert_eq!(doc[0].terms[0].positions, vec![0, far]);
    }

    #[test]
    fn vector_settings_must_not_change_between_instances_of_one_field() {
        let (mut bound, _tracking, _) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            false,
            false,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            false,
            false,
            false,
            vec![Tok::new("beta", 1, 6, 10)],
        ));
        let error = bound
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("the two instances disagree on storeTermVectorPositions");
        assert!(
            matches!(error, LuceneError::IllegalArgument(_)),
            "{error:?}"
        );
    }

    #[test]
    fn payloads_without_positions_are_rejected() {
        let (mut chain, _tracking, _) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            false,
            false,
            true,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("payloads need positions");
        assert!(
            matches!(&error, LuceneError::IllegalArgument(message)
                if message.contains("cannot index term vector payloads without term vector positions")),
            "{error:?}"
        );
    }

    #[test]
    fn vector_extras_without_vectors_are_rejected() {
        for (positions, offsets, payloads, expected) in [
            (true, false, false, "positions"),
            (false, true, false, "offsets"),
            (false, false, true, "payloads"),
        ] {
            let (mut chain, _tracking, _) = bound_chain(1);
            let mut field_infos = builder();
            let mut document = Document::new();
            document.add(tv_field(
                "body",
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
                false,
                positions,
                offsets,
                payloads,
                vec![Tok::new("alpha", 1, 0, 5)],
            ));
            let error = chain
                .process_document(0, &document, true, &mut field_infos)
                .expect_err("term vectors were not requested");
            assert!(
                matches!(&error, LuceneError::IllegalArgument(message)
                    if message.contains(&format!("cannot index term vector {expected} when term vectors are not indexed"))),
                "{error:?}"
            );
        }
    }

    #[test]
    fn term_vectors_on_a_field_that_is_not_indexed_are_rejected() {
        let (mut chain, _tracking, _) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        let mut field_type = FieldType::new();
        field_type.set_stored(true).expect("stored");
        field_type
            .set_store_term_vectors(true)
            .expect("store term vectors");
        field_type.freeze();
        document.add(Box::new(
            Field::new("title", "value".to_string(), field_type).expect("field"),
        ));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("a non-indexed field cannot store term vectors");
        assert!(
            matches!(&error, LuceneError::IllegalArgument(message)
                if message.contains("cannot store term vectors for a field that is not indexed")),
            "{error:?}"
        );
    }

    #[test]
    fn the_field_infos_record_that_a_field_stores_term_vectors() {
        let (mut chain, _tracking, _) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            false,
            false,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        document.add(scripted_field(
            "plain",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            vec![Tok::new("beta", 1, 0, 4)],
        ));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");
        let finished = field_infos.finish().expect("field infos");
        assert!(finished.has_term_vectors());
        assert!(finished
            .field_info("body")
            .expect("body")
            .has_term_vectors());
        assert!(!finished
            .field_info("plain")
            .expect("plain")
            .has_term_vectors());
    }

    #[test]
    fn the_term_vector_files_are_released_on_abort() {
        let (mut chain, tracking, _) = bound_chain(1);
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            true,
            false,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        chain
            .process_document(0, &document, true, &mut field_infos)
            .expect("process document");
        assert!(chain
            .term_vectors_consumer()
            .expect("bound chain")
            .has_writer());
        chain.abort();
        let consumer = chain.term_vectors_consumer().expect("bound chain");
        assert!(!consumer.has_writer());
        assert!(consumer.is_aborted());
        // The files were created and must be dropped by the DWPT, which is why
        // the tracking wrapper still lists them.
        assert!(tracking
            .get_created_files()
            .iter()
            .any(|name| name.ends_with(".tvd")));
    }

    #[test]
    fn an_unbound_chain_refuses_a_field_that_asks_for_vectors() {
        let mut chain = DefaultIndexingChain::new(config());
        let mut field_infos = builder();
        let mut document = Document::new();
        document.add(tv_field(
            "body",
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
            false,
            false,
            vec![Tok::new("alpha", 1, 0, 5)],
        ));
        let error = chain
            .process_document(0, &document, true, &mut field_infos)
            .expect_err("silently dropping term vectors would corrupt the segment");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn java_string_hash_codes_match_java() {
        // `String.hashCode` is specified rather than merely implemented, and
        // these values were read back from a JDK 21 `javac`-compiled program
        // rather than derived from the formula a second time.
        for (name, expected) in [
            ("", 0),
            ("a", 97),
            ("mnum", 3_356_665),
            ("mbin", 3_344_762),
            ("msort", 104_200_075),
            ("msnum", 104_199_200),
            ("mss", 108_429),
            ("sparse", -896_177_632),
            ("konst", 102_232_939),
        ] {
            assert_eq!(java_string_hashcode(name), expected, "hashCode({name:?})");
        }
    }

    #[test]
    fn field_hash_flush_order_follows_the_java_field_hash() {
        // Regression: the mask was read once, before the table had grown, so
        // every field inserted after the first rehash landed in the wrong
        // bucket. Nothing below three fields can show it — the table only
        // doubles twice by then — which is why a single mixed-type case is what
        // caught it. The expected orders here are the ones Lucene 10.5.0 itself
        // produced for the same names, read back out of the `.dvm` it wrote
        // (see `tests/portability/doc_values.rs`).
        assert_eq!(field_hash_order(&["sort"]), vec![0]);
        assert_eq!(field_hash_order(&["sparse", "all"]), vec![0, 1]);
        assert_eq!(field_hash_order(&["bin", "sbin"]), vec![1, 0]);
        assert_eq!(field_hash_order(&["num", "gcd", "konst"]), vec![1, 2, 0]);
        assert_eq!(
            field_hash_order(&["mnum", "mbin", "msort", "msnum", "mss"]),
            vec![3, 0, 1, 2, 4],
            "the stale-mask bug produced [3, 4, 0, 1, 2] here"
        );
    }
}

/// A [`LeafReader`] over the doc values a chain has buffered but not yet
/// written.
///
/// Equivalent to the anonymous `DocValuesLeafReader` that
/// `IndexingChain.getDocValuesLeafReader()` returns. It exists so the index
/// sorter can compare documents at flush time, before any file is written;
/// everything that is not a doc-values column is unsupported, exactly as in
/// Java.
///
/// **Divergence from Lucene 10.5.0.** Java's reader asks the per-field writer
/// for a fresh iterator on every call. A `Box<dyn …>` cannot be handed out
/// twice, so the views are built once and taken out on the first call for each
/// field; the sorter reads each field once, which is what Lucene's own
/// comparators do.
struct BufferedDocValuesReader {
    max_doc: i32,
    /// The buffered producer of each field, with the field info it answers for.
    producers: HashMap<
        String,
        (
            FieldInfo,
            Arc<dyn crate::codecs::doc_values::DocValuesProducer>,
        ),
    >,
}

impl std::fmt::Display for BufferedDocValuesReader {
    /// `DocValuesLeafReader` declares no `toString()`, and neither does the
    /// anonymous subclass `IndexingChain.getDocValuesLeafReader()` returns, so
    /// `java.lang.Object`'s applies.
    ///
    /// **Divergence from Lucene 10.5.0.** Java renders the anonymous class's
    /// own binary name, `org.apache.lucene.index.IndexingChain$N`, where `N` is
    /// the ordinal javac assigns. That ordinal is a compiler artifact, it is
    /// not reachable from outside the package to be measured, and no Lucene
    /// format or `toString()` depends on it; this port renders the named base
    /// class instead. The identity suffix diverges for the reason given on
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.DocValuesLeafReader",
            self,
        ))
    }
}

impl std::fmt::Debug for BufferedDocValuesReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferedDocValuesReader")
            .field("max_doc", &self.max_doc)
            .finish_non_exhaustive()
    }
}

impl crate::index::LeafReader for BufferedDocValuesReader {
    fn core(&self) -> &crate::index::index_reader::IndexReaderCore {
        // A buffered reader is never registered anywhere, so it needs no core
        // state; the one held here is created once and shared.
        static CORE: std::sync::LazyLock<crate::index::index_reader::IndexReaderCore> =
            std::sync::LazyLock::new(crate::index::index_reader::IndexReaderCore::new);
        &CORE
    }

    fn term_vectors(&self) -> Result<Box<dyn crate::index::TermVectors>> {
        Err(LuceneError::UnsupportedOperation(
            "the buffered doc-values reader has no term vectors".to_string(),
        ))
    }

    fn num_docs(&self) -> i32 {
        self.max_doc
    }

    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn crate::index::StoredFields>> {
        Err(LuceneError::UnsupportedOperation(
            "the buffered doc-values reader has no stored fields".to_string(),
        ))
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn crate::index::Terms>>> {
        Err(LuceneError::UnsupportedOperation(
            "the buffered doc-values reader has no terms".to_string(),
        ))
    }

    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::doc_values::NumericDocValues>>> {
        let Some((info, producer)) = self.producers.get(field) else {
            return Ok(None);
        };
        if info.doc_values_type != DocValuesType::NUMERIC {
            return Ok(None);
        }
        Ok(Some(producer.get_numeric(info)?))
    }

    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::doc_values::BinaryDocValues>>> {
        let Some((info, producer)) = self.producers.get(field) else {
            return Ok(None);
        };
        if info.doc_values_type != DocValuesType::BINARY {
            return Ok(None);
        }
        Ok(Some(producer.get_binary(info)?))
    }

    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::doc_values::SortedDocValues>>> {
        let Some((info, producer)) = self.producers.get(field) else {
            return Ok(None);
        };
        if info.doc_values_type != DocValuesType::SORTED {
            return Ok(None);
        }
        Ok(Some(producer.get_sorted(info)?))
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::doc_values::SortedNumericDocValues>>> {
        let Some((info, producer)) = self.producers.get(field) else {
            return Ok(None);
        };
        if info.doc_values_type != DocValuesType::SORTED_NUMERIC {
            return Ok(None);
        }
        Ok(Some(producer.get_sorted_numeric(info)?))
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::index::doc_values::SortedSetDocValues>>> {
        let Some((info, producer)) = self.producers.get(field) else {
            return Ok(None);
        };
        if info.doc_values_type != DocValuesType::SORTED_SET {
            return Ok(None);
        }
        Ok(Some(producer.get_sorted_set(info)?))
    }

    fn get_norm_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn crate::index::doc_values::NumericDocValues>>> {
        Ok(None)
    }

    fn get_doc_values_skipper(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn crate::index::doc_values::DocValuesSkipper>>> {
        Ok(None)
    }

    fn get_float_vector_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn crate::index::vector_values::FloatVectorValues>>> {
        Ok(None)
    }

    fn get_byte_vector_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn crate::index::vector_values::ByteVectorValues>>> {
        Ok(None)
    }

    fn get_field_infos(&self) -> crate::index::FieldInfos {
        crate::index::FieldInfos::default()
    }

    fn get_live_docs(&self) -> Option<Box<dyn crate::util::Bits>> {
        None
    }

    fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn crate::index::PointValues>>> {
        Ok(None)
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_meta_data(&self) -> crate::index::leaf_reader::LeafMetaData {
        crate::index::leaf_reader::LeafMetaData::new(
            crate::util::extra::Version::LATEST.major as i32,
            Some(crate::util::extra::Version::LATEST),
            None,
            false,
        )
        .expect("the latest version is always a valid leaf metadata version")
    }

    fn do_close(&self) -> Result<()> {
        Ok(())
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn crate::index::index_reader::CacheHelper>> {
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn crate::index::index_reader::CacheHelper>> {
        None
    }

    fn search_nearest_vectors(
        &self,
        _field: &str,
        _target: &[f32],
        _knn_collector: &mut dyn crate::search::KnnCollector,
        _accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn search_nearest_vectors_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _knn_collector: &mut dyn crate::search::KnnCollector,
        _accept_docs: &mut dyn crate::search::AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }
}

/// Writes one numeric value into `scratch` in the sortable byte encoding its
/// kind uses.
///
/// Equivalent to `IndexingChain.encodeSortablePointBytes(long, LongColumn.NumericKind, byte[])`:
/// the four- and eight-byte kinds encode differently, and a point field's bytes
/// must match the width its field type declares.
pub fn encode_sortable_point_bytes(
    raw: i64,
    kind: crate::document::column::NumericKind,
    scratch: &mut [u8],
) {
    use crate::document::column::NumericKind;
    match kind {
        NumericKind::Int | NumericKind::Float => {
            crate::util::NumericUtils::int_to_sortable_bytes(raw as i32, scratch, 0)
        }
        NumericKind::Long | NumericKind::Double => {
            crate::util::NumericUtils::long_to_sortable_bytes(raw, scratch, 0)
        }
    }
}
