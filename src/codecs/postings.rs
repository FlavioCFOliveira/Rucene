//! Postings SPI base ported from `org.apache.lucene.codecs`.
//!
//! This module contains the abstract factories and pull/push base classes that
//! concrete postings formats plug into the codec framework. It mirrors the
//! lifecycle of the Java originals (init, write/flush, merge, close) and the
//! field/term/state abstractions used by the default `Lucene104` codec.
//!
//! The key traits are:
//!
//! * [`PostingsFormat`] — named factory that creates per-segment readers and
//!   writers.
//! * [`FieldsConsumer`] / [`FieldsProducer`] — segment-level pull API for
//!   writing and reading inverted fields.
//! * [`PostingsReaderBase`] / [`PostingsWriterBase`] — low-level pull API used
//!   by term dictionaries to decode/encode postings metadata.
//! * [`PushPostingsWriterBase`] — push API that concrete postings writers may
//!   implement; a blanket [`PostingsWriterBase`] implementation turns the push
//!   callbacks into the pull lifecycle.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, RwLock};

use crate::codecs::doc_values::DocValuesProducer;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::codecs::points::PointsReader;
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stored_fields::StoredFieldsReader;
use crate::codecs::stub::{FieldInfo, FieldInfos};
use crate::codecs::term_vectors::TermVectorsReader;
use crate::error::{LuceneError, Result};
use crate::index::IndexOptions;
use crate::search::NO_MORE_DOCS;
use crate::store::{DataInput, DataOutput, IndexInput, IndexOutput};
use crate::util::{BytesRef, FixedBitSet};

// -----------------------------------------------------------------------------
// Postings flags (re-exported from the index module)
// -----------------------------------------------------------------------------

pub use crate::index::{
    feature_requested as postings_feature_requested, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_NONE, POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};

// -----------------------------------------------------------------------------
// State abstractions
// -----------------------------------------------------------------------------

pub use crate::codecs::term_state::{
    BlockTermState, CompetitiveImpactAccumulator, Impact, TermStats,
};

/// State passed to [`FieldsConsumer::merge`] describing the source segments.
///
pub use crate::index::merge::MergeState;

// -----------------------------------------------------------------------------
// Doc-values / norms abstractions used by postings
// -----------------------------------------------------------------------------

/// Access to a single numeric doc-values field.
///
/// Equivalent to `org.apache.lucene.index.NumericDocValues`.
pub trait NumericDocValues {
    /// Returns the value for the given document.
    fn get(&self, doc_id: i32) -> Result<i64>;
}

/// Source of normalization values for a segment.
///
/// Equivalent to `org.apache.lucene.codecs.NormsProducer`.
pub trait NormsProducer: Send + Sync {
    /// Returns the numeric doc-values for the requested field.
    fn get_norms(&self, field_info: &FieldInfo) -> Result<Box<dyn NumericDocValues>>;
}

// -----------------------------------------------------------------------------
// Field / term / postings iteration (re-exported from the index module)
// -----------------------------------------------------------------------------

pub use crate::index::{
    EmptyFields, EmptyTerms, EmptyTermsEnum, Fields, ImpactsEnum, PostingsEnum, SeekStatus,
    TermState, Terms, TermsEnum,
};

/// Empty postings iterator.
pub use crate::index::EmptyPostingsEnum;

/// Returns an empty terms iterator.
pub fn empty_terms_enum() -> Box<dyn TermsEnum> {
    Box::new(EmptyTermsEnum::new())
}

/// Terms iterator that always reports a single fixed term and delegates
/// postings to an underlying iterator.
///
/// Equivalent to `org.apache.lucene.index.SingleTermsEnum`.
pub struct SingleTermsEnum {
    inner: Box<dyn TermsEnum>,
    term: BytesRef,
    atts: crate::util::attribute::AttributeSource,
}

impl SingleTermsEnum {
    /// Creates a single-term enum wrapping `inner` and reporting `term`.
    pub fn new(inner: Box<dyn TermsEnum>, term: BytesRef) -> Self {
        Self {
            inner,
            term,
            atts: crate::util::attribute::AttributeSource::new(),
        }
    }
}

impl std::fmt::Debug for SingleTermsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SingleTermsEnum")
            .field("term", &self.term)
            .finish_non_exhaustive()
    }
}

impl TermsEnum for SingleTermsEnum {
    fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
        &mut self.atts
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.term.clone())
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.inner.postings(reuse, flags)
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Ok(false)
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Ok(SeekStatus::END)
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Ok(())
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Ok(())
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "ord not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn doc_freq(&self) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "doc_freq not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn total_term_freq(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "total_term_freq not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn crate::index::ImpactsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "impacts not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Err(LuceneError::UnsupportedOperation(
            "term_state not supported by SingleTermsEnum".to_string(),
        ))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        Ok(None)
    }
}

// -----------------------------------------------------------------------------
// Segment-level consumer / producer
// -----------------------------------------------------------------------------

/// Writes all fields, terms and postings for a segment.
///
/// Equivalent to `org.apache.lucene.codecs.FieldsConsumer`.
pub trait FieldsConsumer: Send + Sync {
    /// Writes every field/term/posting in `fields`.
    fn write(&mut self, fields: &dyn Fields, norms: &dyn NormsProducer) -> Result<()>;

    /// Merges the source segments described by `merge_state`.
    fn merge(&mut self, merge_state: &MergeState, norms: &dyn NormsProducer) -> Result<()>;

    /// Closes this consumer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

/// Reads all fields, terms and postings for a segment.
///
/// Equivalent to `org.apache.lucene.codecs.FieldsProducer`.
pub trait FieldsProducer: Fields + Send + Sync {
    /// Verifies the integrity of the underlying data files.
    fn check_integrity(&self) -> Result<()>;

    /// Returns an instance optimized for merging.
    ///
    /// The default implementation returns a clone of `self`. Implementations
    /// may override this to return a lightweight merge-only view.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>>;

    /// Closes this producer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Postings format factory
// -----------------------------------------------------------------------------

/// Encodes and decodes postings (inverted-index term-document lists).
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.PostingsFormat`.
pub trait PostingsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Creates a writer for a new segment.
    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>>;

    /// Creates a reader for an existing segment.
    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>>;
}

/// A registry mapping postings-format short names to [`PostingsFormat`]
/// implementations.
///
/// The registry intentionally does not use reflection or SPI loading. Formats
/// are registered explicitly with [`PostingsFormatRegistry::register`] and
/// looked up by name with [`PostingsFormatRegistry::for_name`].
#[derive(Debug, Default, Clone)]
pub struct PostingsFormatRegistry {
    formats: Arc<RwLock<HashMap<String, Arc<dyn PostingsFormat>>>>,
}

impl PostingsFormatRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a postings format under the given short name.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `name` is empty, contains
    /// characters other than ASCII alphanumerics, or is longer than 127 bytes.
    /// Returns [`LuceneError::IllegalState`] if the name is already registered.
    pub fn register<F>(&self, name: impl Into<String>, format: F) -> Result<()>
    where
        F: PostingsFormat + 'static,
    {
        let name = name.into();
        super::validate_service_name(&name)?;

        let mut formats = self.formats.write().map_err(|_| {
            LuceneError::IllegalState("postings format registry lock was poisoned".to_string())
        })?;

        if formats.contains_key(&name) {
            return Err(LuceneError::IllegalState(format!(
                "postings format already registered: {name}"
            )));
        }

        formats.insert(name, Arc::new(format));
        Ok(())
    }

    /// Looks up a postings format by name.
    ///
    /// Returns `None` if no format has been registered under the given name.
    pub fn for_name(&self, name: &str) -> Option<Arc<dyn PostingsFormat>> {
        self.formats
            .read()
            .map_err(|_| {
                LuceneError::IllegalState("postings format registry lock was poisoned".to_string())
            })
            .ok()?
            .get(name)
            .cloned()
    }

    /// Returns the names of all registered postings formats, sorted
    /// alphabetically.
    pub fn available_postings_formats(&self) -> Vec<String> {
        let Ok(formats) = self.formats.read() else {
            return Vec::new();
        };
        let mut names: Vec<String> = formats.keys().cloned().collect();
        names.sort();
        names
    }
}

static GLOBAL_POSTINGS_REGISTRY: LazyLock<PostingsFormatRegistry> =
    LazyLock::new(PostingsFormatRegistry::new);

/// Looks up a postings format by name from the global registry.
///
/// Returns `None` if no format has been registered under the given name.
pub fn postings_for_name(name: &str) -> Option<Arc<dyn PostingsFormat>> {
    GLOBAL_POSTINGS_REGISTRY.for_name(name)
}

/// Returns the names of all postings formats registered in the global
/// registry.
pub fn available_postings_formats() -> Vec<String> {
    GLOBAL_POSTINGS_REGISTRY.available_postings_formats()
}

// -----------------------------------------------------------------------------
// Pull postings reader / writer base
// -----------------------------------------------------------------------------

/// Low-level postings reader used by term dictionaries.
///
/// Equivalent to `org.apache.lucene.codecs.PostingsReaderBase`.
pub trait PostingsReaderBase: Send + Sync {
    /// Performs initialization, such as reading and verifying the header from
    /// the terms dictionary input.
    fn init(&mut self, terms_in: &mut dyn IndexInput, state: &SegmentReadState) -> Result<()>;

    /// Returns a newly created empty term state.
    fn new_term_state(&self) -> Result<BlockTermState>;

    /// Decodes metadata for the next term.
    fn decode_term(
        &mut self,
        input: &mut dyn DataInput,
        field_info: &FieldInfo,
        state: &mut BlockTermState,
        absolute: bool,
    ) -> Result<()>;

    /// Returns a postings iterator for the given term state.
    fn postings(
        &mut self,
        field_info: &FieldInfo,
        state: &BlockTermState,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>>;

    /// Returns an impacts iterator for the given term state.
    fn impacts(
        &mut self,
        field_info: &FieldInfo,
        state: &BlockTermState,
        flags: i32,
    ) -> Result<Box<dyn ImpactsEnum>>;

    /// Verifies the integrity of the underlying postings data.
    fn check_integrity(&self) -> Result<()>;

    /// Closes this reader.
    fn close(&mut self) -> Result<()>;
}

/// Low-level postings writer used by term dictionaries.
///
/// Equivalent to `org.apache.lucene.codecs.PostingsWriterBase`.
pub trait PostingsWriterBase: Send + Sync {
    /// Performs one-time initialization, typically writing a header.
    fn init(&mut self, terms_out: &mut dyn IndexOutput, state: &SegmentWriteState) -> Result<()>;

    /// Writes all postings for the current term.
    ///
    /// The provided `terms_enum` is already positioned on the term to write.
    /// This method must set a bit in `docs_seen` for every document written.
    /// If no documents contain the term, it returns `Ok(None)` and the terms
    /// dictionary will skip the term.
    fn write_term(
        &mut self,
        term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        docs_seen: &mut FixedBitSet,
        norms: &dyn NormsProducer,
    ) -> Result<Option<BlockTermState>>;

    /// Encodes term metadata to the provided output.
    fn encode_term(
        &mut self,
        out: &mut dyn DataOutput,
        field_info: &FieldInfo,
        state: &BlockTermState,
        absolute: bool,
    ) -> Result<()>;

    /// Sets the current field for writing.
    fn set_field(&mut self, field_info: &FieldInfo) -> Result<()>;

    /// Closes this writer.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Push postings writer base
// -----------------------------------------------------------------------------

/// Mutable state shared by the default push writer implementation.
///
/// Concrete push writers embed this struct and expose it through
/// [`PushPostingsWriterBase::push_state`] /
/// [`PushPostingsWriterBase::push_state_mut`].
#[derive(Debug, Default, Clone)]
pub struct PushPostingsWriterState {
    /// The field currently being written.
    pub field_info: Option<FieldInfo>,
    /// Index options of the current field.
    pub index_options: IndexOptions,
    /// Whether frequencies are written for the current field.
    pub write_freqs: bool,
    /// Whether positions are written for the current field.
    pub write_positions: bool,
    /// Whether payloads are written for the current field.
    pub write_payloads: bool,
    /// Whether offsets are written for the current field.
    pub write_offsets: bool,
    /// Flags used to request the correct postings enum for the current field.
    pub enum_flags: i32,
}

impl PushPostingsWriterState {
    /// Creates an empty push writer state.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Push-style postings writer.
///
/// Equivalent to `org.apache.lucene.codecs.PushPostingsWriterBase`. Types that
/// implement this trait automatically implement [`PostingsWriterBase`] via the
/// blanket implementation below; they only need to supply the lifecycle,
/// metadata encoding and push callbacks.
pub trait PushPostingsWriterBase: Send + Sync {
    /// Returns the shared mutable state.
    fn push_state(&self) -> &PushPostingsWriterState;

    /// Returns the shared mutable state.
    fn push_state_mut(&mut self) -> &mut PushPostingsWriterState;

    /// Performs one-time initialization, typically writing a header.
    fn init(&mut self, terms_out: &mut dyn IndexOutput, state: &SegmentWriteState) -> Result<()>;

    /// Encodes term metadata to the provided output.
    fn encode_term(
        &mut self,
        out: &mut dyn DataOutput,
        field_info: &FieldInfo,
        state: &BlockTermState,
        absolute: bool,
    ) -> Result<()>;

    /// Closes this writer.
    fn close(&mut self) -> Result<()>;

    /// Returns a newly created empty term state.
    fn new_term_state(&self) -> Result<BlockTermState>;

    /// Starts a new term.
    fn start_term(&mut self, norms: Option<&dyn NumericDocValues>) -> Result<()>;

    /// Finishes the current term.
    fn finish_term(&mut self, state: &mut BlockTermState) -> Result<()>;

    /// Starts a new document within the current term.
    fn start_doc(&mut self, doc_id: i32, freq: i32) -> Result<()>;

    /// Adds a position, payload and offsets for the current document.
    fn add_position(
        &mut self,
        position: i32,
        payload: Option<&[u8]>,
        start_offset: i32,
        end_offset: i32,
    ) -> Result<()>;

    /// Finishes the current document.
    fn finish_doc(&mut self) -> Result<()>;

    /// Sets the current field and computes per-field flags.
    ///
    /// The default implementation stores the field in [`PushPostingsWriterState`]
    /// and derives `write_freqs`, `write_positions`, `write_payloads`,
    /// `write_offsets` and `enum_flags`.
    fn set_field(&mut self, field_info: &FieldInfo) -> Result<()> {
        let state = self.push_state_mut();
        state.field_info = Some(field_info.clone());
        state.index_options = field_info.index_options;
        state.write_freqs = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS);
        state.write_positions = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        state.write_offsets = field_info
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        state.write_payloads = field_info.has_payloads();

        if !state.write_freqs {
            state.enum_flags = POSTINGS_ENUM_NONE;
        } else if !state.write_positions {
            state.enum_flags = POSTINGS_ENUM_FREQS;
        } else if !state.write_offsets {
            if state.write_payloads {
                state.enum_flags = POSTINGS_ENUM_PAYLOADS;
            } else {
                state.enum_flags = POSTINGS_ENUM_POSITIONS;
            }
        } else if state.write_payloads {
            state.enum_flags = POSTINGS_ENUM_PAYLOADS | POSTINGS_ENUM_OFFSETS;
        } else {
            state.enum_flags = POSTINGS_ENUM_OFFSETS;
        }
        Ok(())
    }

    /// Pulls postings from `terms_enum` and invokes the push callbacks.
    ///
    /// The default implementation mirrors `PushPostingsWriterBase.writeTerm` in
    /// Java: it calls `start_term`, iterates over the postings, calls
    /// `start_doc` / `add_position` / `finish_doc`, and finally `finish_term`.
    fn write_term(
        &mut self,
        _term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        docs_seen: &mut FixedBitSet,
        norms: &dyn NormsProducer,
    ) -> Result<Option<BlockTermState>> {
        let field_info = self
            .push_state()
            .field_info
            .as_ref()
            .ok_or_else(|| LuceneError::IllegalState("set_field not called".to_string()))?
            .clone();

        let mut norms_values: Option<Box<dyn NumericDocValues>> = None;
        if field_info.has_norms() {
            norms_values = Some(norms.get_norms(&field_info)?);
        }
        let norms_ref = norms_values.as_deref();
        self.start_term(norms_ref)?;

        let enum_flags = self.push_state().enum_flags;
        let write_freqs = self.push_state().write_freqs;
        let write_positions = self.push_state().write_positions;
        let write_payloads = self.push_state().write_payloads;
        let write_offsets = self.push_state().write_offsets;

        let mut postings_enum = terms_enum.postings(None, enum_flags)?;
        let mut doc_freq = 0i32;
        let mut total_term_freq = 0i64;
        loop {
            let doc_id = postings_enum.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            doc_freq += 1;
            if (doc_id as usize) < docs_seen.length() {
                docs_seen.set(doc_id as usize);
            }
            let freq = if write_freqs {
                postings_enum.freq()?
            } else {
                -1
            };
            if freq >= 0 {
                total_term_freq += freq as i64;
            }
            self.start_doc(doc_id, freq)?;

            if write_positions {
                for _ in 0..freq {
                    let position = postings_enum.next_position()?;
                    let payload = if write_payloads {
                        postings_enum.get_payload()?
                    } else {
                        None
                    };
                    let (start_offset, end_offset) = if write_offsets {
                        (postings_enum.start_offset(), postings_enum.end_offset())
                    } else {
                        (-1, -1)
                    };
                    self.add_position(position, payload, start_offset, end_offset)?;
                }
            }
            self.finish_doc()?;
        }

        if doc_freq == 0 {
            Ok(None)
        } else {
            let mut state = self.new_term_state()?;
            state.doc_freq = doc_freq;
            state.total_term_freq = if write_freqs { total_term_freq } else { -1 };
            self.finish_term(&mut state)?;
            Ok(Some(state))
        }
    }
}

impl<T: PushPostingsWriterBase + ?Sized> PostingsWriterBase for T {
    fn init(&mut self, terms_out: &mut dyn IndexOutput, state: &SegmentWriteState) -> Result<()> {
        PushPostingsWriterBase::init(self, terms_out, state)
    }

    fn encode_term(
        &mut self,
        out: &mut dyn DataOutput,
        field_info: &FieldInfo,
        state: &BlockTermState,
        absolute: bool,
    ) -> Result<()> {
        PushPostingsWriterBase::encode_term(self, out, field_info, state, absolute)
    }

    fn set_field(&mut self, field_info: &FieldInfo) -> Result<()> {
        PushPostingsWriterBase::set_field(self, field_info)
    }

    fn write_term(
        &mut self,
        term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        docs_seen: &mut FixedBitSet,
        norms: &dyn NormsProducer,
    ) -> Result<Option<BlockTermState>> {
        PushPostingsWriterBase::write_term(self, term, terms_enum, docs_seen, norms)
    }

    fn close(&mut self) -> Result<()> {
        PushPostingsWriterBase::close(self)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocIdSetIterator;

    // Stub Fields -----------------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubFields;

    impl Fields for StubFields {
        fn size(&self) -> i32 {
            0
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty::<String>())
        }
    }

    // Stub TermsEnum --------------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubTermsEnum {
        atts: crate::util::attribute::AttributeSource,
    }

    impl TermsEnum for StubTermsEnum {
        fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
            &mut self.atts
        }

        fn term(&self) -> Result<BytesRef> {
            Ok(BytesRef::default())
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(StubPostingsEnum::default()))
        }

        fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
            Ok(false)
        }

        fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
            Ok(SeekStatus::END)
        }

        fn seek_ord(&mut self, _ord: i64) -> Result<()> {
            Ok(())
        }

        fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
            Ok(())
        }

        fn ord(&self) -> Result<i64> {
            Ok(-1)
        }

        fn doc_freq(&self) -> Result<i32> {
            Ok(0)
        }

        fn total_term_freq(&self) -> Result<i64> {
            Ok(0)
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn crate::index::ImpactsEnum>> {
            Ok(Box::new(StubImpactsEnum::default()))
        }

        fn term_state(&mut self) -> Result<Box<dyn TermState>> {
            Ok(Box::new(
                crate::codecs::term_state::BlockTermState::default(),
            ))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            Ok(None)
        }
    }

    // Stub PostingsEnum -----------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPostingsEnum {
        doc: i32,
    }

    impl DocIdSetIterator for StubPostingsEnum {
        fn doc_id(&self) -> i32 {
            self.doc
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.doc = NO_MORE_DOCS;
            Ok(NO_MORE_DOCS)
        }

        fn advance(&mut self, _target: i32) -> Result<i32> {
            self.doc = NO_MORE_DOCS;
            Ok(NO_MORE_DOCS)
        }

        fn cost(&self) -> i64 {
            0
        }
    }

    impl PostingsEnum for StubPostingsEnum {
        fn freq(&self) -> Result<i32> {
            Ok(0)
        }

        fn next_position(&mut self) -> Result<i32> {
            Ok(-1)
        }

        fn start_offset(&self) -> i32 {
            -1
        }

        fn end_offset(&self) -> i32 {
            -1
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    // Stub ImpactsEnum ------------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubImpactsEnum;

    impl DocIdSetIterator for StubImpactsEnum {
        fn doc_id(&self) -> i32 {
            NO_MORE_DOCS
        }

        fn next_doc(&mut self) -> Result<i32> {
            Ok(NO_MORE_DOCS)
        }

        fn advance(&mut self, _target: i32) -> Result<i32> {
            Ok(NO_MORE_DOCS)
        }

        fn cost(&self) -> i64 {
            0
        }
    }

    impl PostingsEnum for StubImpactsEnum {
        fn freq(&self) -> Result<i32> {
            Ok(0)
        }

        fn next_position(&mut self) -> Result<i32> {
            Ok(-1)
        }

        fn start_offset(&self) -> i32 {
            -1
        }

        fn end_offset(&self) -> i32 {
            -1
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    impl crate::index::ImpactsSource for StubImpactsEnum {
        fn advance_shallow(&mut self, _target: i32) -> Result<()> {
            Ok(())
        }

        fn get_impacts(&mut self) -> Result<Box<dyn crate::index::Impacts>> {
            Ok(Box::new(StubImpacts::default()))
        }
    }

    impl ImpactsEnum for StubImpactsEnum {}

    #[derive(Debug, Default, Clone)]
    struct StubImpacts;

    impl crate::index::Impacts for StubImpacts {
        fn num_levels(&self) -> i32 {
            1
        }

        fn doc_id_up_to(&self, _level: i32) -> i32 {
            i32::MAX
        }

        fn get_impacts(&self, _level: i32) -> crate::index::FreqAndNormBuffer {
            let mut buf = crate::index::FreqAndNormBuffer::new();
            buf.add(1, 1);
            buf
        }
    }

    // Stub NumericDocValues / NormsProducer ---------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubNumericDocValues;

    impl NumericDocValues for StubNumericDocValues {
        fn get(&self, _doc_id: i32) -> Result<i64> {
            Ok(0)
        }
    }

    #[derive(Debug, Default, Clone)]
    struct StubNormsProducer;

    impl NormsProducer for StubNormsProducer {
        fn get_norms(&self, _field_info: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
            Ok(Box::new(StubNumericDocValues))
        }
    }

    // Stub FieldsConsumer ---------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubFieldsConsumer;

    impl FieldsConsumer for StubFieldsConsumer {
        fn write(&mut self, _fields: &dyn Fields, _norms: &dyn NormsProducer) -> Result<()> {
            Ok(())
        }

        fn merge(&mut self, _merge_state: &MergeState, _norms: &dyn NormsProducer) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // Stub FieldsProducer ---------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubFieldsProducer;

    impl Fields for StubFieldsProducer {
        fn size(&self) -> i32 {
            0
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::empty::<String>())
        }
    }

    impl FieldsProducer for StubFieldsProducer {
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // Stub PostingsFormat ---------------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPostingsFormat;

    impl PostingsFormat for StubPostingsFormat {
        fn name(&self) -> &str {
            "StubPostingsFormat"
        }

        fn fields_consumer<'a>(
            &self,
            _state: &SegmentWriteState<'a>,
        ) -> Result<Box<dyn FieldsConsumer + 'a>> {
            Ok(Box::new(StubFieldsConsumer))
        }

        fn fields_producer<'a>(
            &self,
            _state: &SegmentReadState<'a>,
        ) -> Result<Box<dyn FieldsProducer>> {
            Ok(Box::new(StubFieldsProducer))
        }
    }

    // Stub PostingsReaderBase ----------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPostingsReader;

    impl PostingsReaderBase for StubPostingsReader {
        fn init(
            &mut self,
            _terms_in: &mut dyn IndexInput,
            _state: &SegmentReadState,
        ) -> Result<()> {
            Ok(())
        }

        fn new_term_state(&self) -> Result<BlockTermState> {
            Ok(BlockTermState::default())
        }

        fn decode_term(
            &mut self,
            _input: &mut dyn DataInput,
            _field_info: &FieldInfo,
            _state: &mut BlockTermState,
            _absolute: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn postings(
            &mut self,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(StubPostingsEnum::default()))
        }

        fn impacts(
            &mut self,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _flags: i32,
        ) -> Result<Box<dyn ImpactsEnum>> {
            Ok(Box::new(StubImpactsEnum))
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // Stub PushPostingsWriterBase -------------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPushWriter {
        state: PushPostingsWriterState,
    }

    impl PushPostingsWriterBase for StubPushWriter {
        fn push_state(&self) -> &PushPostingsWriterState {
            &self.state
        }

        fn push_state_mut(&mut self) -> &mut PushPostingsWriterState {
            &mut self.state
        }

        fn init(
            &mut self,
            _terms_out: &mut dyn IndexOutput,
            _state: &SegmentWriteState,
        ) -> Result<()> {
            Ok(())
        }

        fn encode_term(
            &mut self,
            _out: &mut dyn DataOutput,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _absolute: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn new_term_state(&self) -> Result<BlockTermState> {
            Ok(BlockTermState::default())
        }

        fn start_term(&mut self, _norms: Option<&dyn NumericDocValues>) -> Result<()> {
            Ok(())
        }

        fn finish_term(&mut self, _state: &mut BlockTermState) -> Result<()> {
            Ok(())
        }

        fn start_doc(&mut self, _doc_id: i32, _freq: i32) -> Result<()> {
            Ok(())
        }

        fn add_position(
            &mut self,
            _position: i32,
            _payload: Option<&[u8]>,
            _start_offset: i32,
            _end_offset: i32,
        ) -> Result<()> {
            Ok(())
        }

        fn finish_doc(&mut self) -> Result<()> {
            Ok(())
        }
    }

    // Stub standalone PostingsWriterBase -------------------------------------
    #[derive(Debug, Default, Clone)]
    struct StubPostingsWriter;

    impl PostingsWriterBase for StubPostingsWriter {
        fn init(
            &mut self,
            _terms_out: &mut dyn IndexOutput,
            _state: &SegmentWriteState,
        ) -> Result<()> {
            Ok(())
        }

        fn write_term(
            &mut self,
            _term: &BytesRef,
            _terms_enum: &mut dyn TermsEnum,
            _docs_seen: &mut FixedBitSet,
            _norms: &dyn NormsProducer,
        ) -> Result<Option<BlockTermState>> {
            Ok(None)
        }

        fn encode_term(
            &mut self,
            _out: &mut dyn DataOutput,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _absolute: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn set_field(&mut self, _field_info: &FieldInfo) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stub_postings_format_compiles_and_runs() {
        let format = StubPostingsFormat;
        assert_eq!(format.name(), "StubPostingsFormat");

        let dir = crate::store::RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let segment_info = crate::index::SegmentInfo::new(
            std::sync::Arc::new(crate::store::RamDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            "_0".to_string(),
            0,
            false,
            false,
            std::sync::Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            crate::util::StringHelper::random_id(),
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        let field_infos = crate::codecs::stub::FieldInfos::default();
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let seg_updates = crate::codecs::stub::BufferedUpdates;
        let info_stream = crate::util::default_info_stream();
        let write_state = SegmentWriteState::new(
            info_stream,
            dir_ref,
            &segment_info,
            &field_infos,
            &seg_updates,
            context,
        );
        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);

        let mut consumer = format.fields_consumer(&write_state).unwrap();
        consumer.write(&StubFields, &StubNormsProducer).unwrap();
        consumer
            .merge(
                &MergeState::from_fields_producers(Vec::new(), Vec::new()),
                &StubNormsProducer,
            )
            .unwrap();
        consumer.close().unwrap();

        let mut producer = format.fields_producer(&read_state).unwrap();
        assert_eq!(producer.size(), 0);
        producer.check_integrity().unwrap();
        let _merge_instance = producer.get_merge_instance().unwrap();
        producer.close().unwrap();
    }

    #[test]
    fn stub_postings_reader_base_compiles_and_runs() {
        let dir = crate::store::RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let segment_info = crate::index::SegmentInfo::new(
            std::sync::Arc::new(crate::store::RamDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            "_0".to_string(),
            0,
            false,
            false,
            std::sync::Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            crate::util::StringHelper::random_id(),
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        let field_infos = crate::codecs::stub::FieldInfos::default();
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);

        let mut reader = StubPostingsReader;
        reader
            .init(
                &mut crate::store::MockIndexInput::new(vec![], "test"),
                &read_state,
            )
            .unwrap();
        let state = reader.new_term_state().unwrap();
        assert_eq!(state.doc_freq, 0);
        reader
            .decode_term(
                &mut crate::store::ByteArrayDataInput::new(vec![]),
                &FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false),
                &mut BlockTermState::default(),
                true,
            )
            .unwrap();
        let _enum_ = reader
            .postings(
                &FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false),
                &BlockTermState::default(),
                None,
                POSTINGS_ENUM_NONE,
            )
            .unwrap();
        let _impacts = reader
            .impacts(
                &FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false),
                &BlockTermState::default(),
                POSTINGS_ENUM_NONE,
            )
            .unwrap();
        reader.check_integrity().unwrap();
        reader.close().unwrap();
    }

    #[test]
    fn stub_push_postings_writer_blanket_impl_runs() {
        let dir = crate::store::RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let segment_info = crate::index::SegmentInfo::new(
            std::sync::Arc::new(crate::store::RamDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            "_0".to_string(),
            0,
            false,
            false,
            std::sync::Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            crate::util::StringHelper::random_id(),
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        let field_infos = crate::codecs::stub::FieldInfos::default();
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let seg_updates = crate::codecs::stub::BufferedUpdates;
        let info_stream = crate::util::default_info_stream();
        let write_state = SegmentWriteState::new(
            info_stream,
            dir_ref,
            &segment_info,
            &field_infos,
            &seg_updates,
            context,
        );

        let mut writer = StubPushWriter::default();
        // `StubPushWriter` implements `PushPostingsWriterBase`; the blanket
        // impl provides `PostingsWriterBase`.
        PostingsWriterBase::init(
            &mut writer,
            &mut crate::store::MockIndexOutput::new("test", "test"),
            &write_state,
        )
        .unwrap();

        let field_info =
            FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false);
        crate::codecs::postings::PostingsWriterBase::set_field(&mut writer, &field_info).unwrap();

        let mut docs_seen = FixedBitSet::new(8);
        let result = PostingsWriterBase::write_term(
            &mut writer,
            &BytesRef::default(),
            &mut StubTermsEnum::default(),
            &mut docs_seen,
            &StubNormsProducer,
        )
        .unwrap();
        assert!(result.is_none());

        PostingsWriterBase::encode_term(
            &mut writer,
            &mut crate::store::ByteArrayDataOutput::new(),
            &field_info,
            &BlockTermState::default(),
            true,
        )
        .unwrap();
        PostingsWriterBase::close(&mut writer).unwrap();
    }

    #[test]
    fn stub_postings_writer_base_compiles_and_runs() {
        let dir = crate::store::RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let segment_info = crate::index::SegmentInfo::new(
            std::sync::Arc::new(crate::store::RamDirectory::default()),
            crate::util::Version::LUCENE_10_5_0,
            Some(crate::util::Version::LUCENE_10_5_0),
            "_0".to_string(),
            0,
            false,
            false,
            std::sync::Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            std::collections::HashMap::new(),
            crate::util::StringHelper::random_id(),
            std::collections::HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        let field_infos = crate::codecs::stub::FieldInfos::default();
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let seg_updates = crate::codecs::stub::BufferedUpdates;
        let info_stream = crate::util::default_info_stream();
        let write_state = SegmentWriteState::new(
            info_stream,
            dir_ref,
            &segment_info,
            &field_infos,
            &seg_updates,
            context,
        );

        let mut writer = StubPostingsWriter;
        writer
            .init(
                &mut crate::store::MockIndexOutput::new("test", "test"),
                &write_state,
            )
            .unwrap();
        let field_info =
            FieldInfo::new("field", 0).with_postings_options(IndexOptions::DOCS, false, false);
        writer.set_field(&field_info).unwrap();
        let mut docs_seen = FixedBitSet::new(8);
        let result = writer
            .write_term(
                &BytesRef::default(),
                &mut StubTermsEnum::default(),
                &mut docs_seen,
                &StubNormsProducer,
            )
            .unwrap();
        assert!(result.is_none());
        writer
            .encode_term(
                &mut crate::store::ByteArrayDataOutput::new(),
                &field_info,
                &BlockTermState::default(),
                true,
            )
            .unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn postings_format_registry_registers_and_looks_up() {
        let registry = PostingsFormatRegistry::new();
        registry.register("Stub", StubPostingsFormat).unwrap();
        let looked_up = registry.for_name("Stub").expect("format should be present");
        assert_eq!(looked_up.name(), "StubPostingsFormat");
        assert_eq!(
            registry.available_postings_formats(),
            vec!["Stub".to_string()]
        );
    }

    #[test]
    fn postings_format_registry_rejects_duplicates_and_invalid_names() {
        let registry = PostingsFormatRegistry::new();
        registry.register("Stub", StubPostingsFormat).unwrap();
        let err = registry
            .register("Stub", StubPostingsFormat)
            .expect_err("duplicate should fail");
        assert!(matches!(err, LuceneError::IllegalState(_)));

        let err = registry
            .register("", StubPostingsFormat)
            .expect_err("empty name should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));

        let err = registry
            .register("Bad Name", StubPostingsFormat)
            .expect_err("space should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn push_writer_set_field_computes_flags() {
        let mut writer = StubPushWriter::default();
        let field_info = FieldInfo::new("text", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            false,
            true,
        );
        crate::codecs::postings::PushPostingsWriterBase::set_field(&mut writer, &field_info)
            .unwrap();
        assert_eq!(writer.push_state().enum_flags, POSTINGS_ENUM_PAYLOADS);
        assert!(writer.push_state().write_positions);
        assert!(writer.push_state().write_payloads);
    }
}
