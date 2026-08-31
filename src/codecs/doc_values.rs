//! Doc-values format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.DocValuesFormat`,
//! `DocValuesConsumer` and `DocValuesProducer`.
//!
//! These traits provide the abstract read/write API for per-document columnar
//! values (numeric, binary, sorted, sorted-set and sorted-numeric). Concrete
//! codecs implement the format-specific encoding underneath this API.
//!
//! The per-document value iterators are re-exported from
//! [`crate::index::doc_values`] so that the codec layer and the index layer
//! share the same iterator contract (position via `next_doc`/`advance`, value
//! retrieval only after the iterator is positioned on a doc that has a value).

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, RwLock};

use crate::error::{LuceneError, Result};

pub use crate::index::doc_values::{
    BinaryDocValues, DocValues, DocValuesIterator, DocValuesSkipper, EmptyBinaryDocValues,
    EmptyDocValuesProducer, EmptyDocValuesSkipper, EmptyNumericDocValues, EmptySortedDocValues,
    EmptySortedNumericDocValues, EmptySortedSetDocValues, NumericDocValues,
    SingletonSortedNumericDocValues, SingletonSortedSetDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};

use super::postings::MergeState;
use super::state::{SegmentReadState, SegmentWriteState};
use super::stub::FieldInfo;

// -----------------------------------------------------------------------------
// Producer
// -----------------------------------------------------------------------------

/// Reads doc-values fields from a segment.
///
/// Equivalent to `org.apache.lucene.codecs.DocValuesProducer`.
/// The five value getters and the skipper return **single-threaded** views, as
/// Lucene's do: `DocValuesProducer.getNumeric` and its siblings document that
/// "the returned instance need not be thread-safe: it will only be used by a
/// single thread", and `LeafReader.getNumericDocValues` repeats it. The
/// producer itself is shared by the reader and so stays `Send + Sync`; the
/// views it hands out are not, which is what lets them hold a cursor over the
/// data file instead of a copy of its contents.
pub trait DocValuesProducer: Send + Sync + fmt::Debug {
    /// Returns the numeric values for the given field.
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>>;

    /// Returns the binary values for the given field.
    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>>;

    /// Returns the sorted values for the given field.
    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues>>;

    /// Returns the sorted-numeric values for the given field.
    fn get_sorted_numeric(&self, field: &FieldInfo) -> Result<Box<dyn SortedNumericDocValues>>;

    /// Returns the sorted-set values for the given field.
    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Box<dyn SortedSetDocValues>>;

    /// Returns the skip index for the given field.
    fn get_skipper(&self, field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper>>;

    /// Checks consistency of this producer.
    fn check_integrity(&self) -> Result<()>;

    /// Returns an instance optimized for merging.
    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>>;

    /// Closes this producer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Consumer
// -----------------------------------------------------------------------------

/// Writes doc-values fields for a segment.
///
/// Equivalent to `org.apache.lucene.codecs.DocValuesConsumer`.
pub trait DocValuesConsumer: Send + Sync + fmt::Debug {
    /// Writes a numeric doc-values field.
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()>;

    /// Writes a binary doc-values field.
    fn add_binary_field(&mut self, field: &FieldInfo, values: &dyn DocValuesProducer)
        -> Result<()>;

    /// Writes a sorted doc-values field.
    fn add_sorted_field(&mut self, field: &FieldInfo, values: &dyn DocValuesProducer)
        -> Result<()>;

    /// Writes a sorted-numeric doc-values field.
    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()>;

    /// Writes a sorted-set doc-values field.
    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()>;

    /// Merges the doc-values fields from the readers in `merge_state`.
    ///
    /// Equivalent to `DocValuesConsumer.merge(MergeState)`. Every producer is
    /// checked for integrity first, then each field of the merged `FieldInfos`
    /// is dispatched by its doc-values type to
    /// [`merge_numeric_field`](Self::merge_numeric_field) and its four
    /// siblings.
    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        for producer in merge_state.doc_values_producers.iter().flatten() {
            producer.check_integrity()?;
        }
        for merge_field_info in merge_state.merge_field_infos.iter() {
            match merge_field_info.get_doc_values_type() {
                DocValuesType::NONE => {}
                DocValuesType::NUMERIC => {
                    self.merge_numeric_field(merge_field_info, merge_state)?
                }
                DocValuesType::BINARY => self.merge_binary_field(merge_field_info, merge_state)?,
                DocValuesType::SORTED => self.merge_sorted_field(merge_field_info, merge_state)?,
                DocValuesType::SORTED_SET => {
                    self.merge_sorted_set_field(merge_field_info, merge_state)?
                }
                DocValuesType::SORTED_NUMERIC => {
                    self.merge_sorted_numeric_field(merge_field_info, merge_state)?
                }
            }
        }
        Ok(())
    }

    /// Merges one numeric field.
    ///
    /// Equivalent to `DocValuesConsumer.mergeNumericField(FieldInfo, MergeState)`.
    fn merge_numeric_field(&mut self, field: &FieldInfo, merge_state: &MergeState) -> Result<()> {
        let producer = MergedDocValuesProducer::new(field, merge_state, None);
        self.add_numeric_field(field, &producer)
    }

    /// Merges one binary field.
    ///
    /// Equivalent to `DocValuesConsumer.mergeBinaryField(FieldInfo, MergeState)`.
    fn merge_binary_field(&mut self, field: &FieldInfo, merge_state: &MergeState) -> Result<()> {
        let producer = MergedDocValuesProducer::new(field, merge_state, None);
        self.add_binary_field(field, &producer)
    }

    /// Merges one sorted-numeric field.
    ///
    /// Equivalent to `DocValuesConsumer.mergeSortedNumericField(FieldInfo, MergeState)`.
    fn merge_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<()> {
        let producer = MergedDocValuesProducer::new(field, merge_state, None);
        self.add_sorted_numeric_field(field, &producer)
    }

    /// Merges one sorted field.
    ///
    /// Equivalent to `DocValuesConsumer.mergeSortedField(FieldInfo, MergeState)`,
    /// which runs in three steps: mark the terms still in use, build the
    /// ordinal map that conceptually performs the merge, then add the field.
    fn merge_sorted_field(&mut self, field: &FieldInfo, merge_state: &MergeState) -> Result<()> {
        let map = create_ordinal_map_for_sorted_dv(field, merge_state)?;
        let producer = MergedDocValuesProducer::new(field, merge_state, Some(Arc::new(map)));
        self.add_sorted_field(field, &producer)
    }

    /// Merges one sorted-set field.
    ///
    /// Equivalent to `DocValuesConsumer.mergeSortedSetField(FieldInfo, MergeState)`.
    fn merge_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<()> {
        let to_merge = select_leaves_to_merge(field, merge_state)?;
        let map = create_ordinal_map_for_sorted_set_dv(to_merge, merge_state)?;
        let producer = MergedDocValuesProducer::new(field, merge_state, Some(Arc::new(map)));
        self.add_sorted_set_field(field, &producer)
    }

    /// Closes this consumer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Format
// -----------------------------------------------------------------------------

/// Encodes and decodes doc values (columnar per-document values).
///
/// Equivalent to `org.apache.lucene.codecs.DocValuesFormat`.
pub trait DocValuesFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a consumer to write docvalues to the index.
    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>>;

    /// Returns a producer to read docvalues from the index.
    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>>;
}

// -----------------------------------------------------------------------------
// Doc-values format registry
// -----------------------------------------------------------------------------

/// A registry mapping doc-values-format short names to [`DocValuesFormat`]
/// implementations.
///
/// The registry intentionally does not use reflection or SPI loading. Formats
/// are registered explicitly with [`DocValuesFormatRegistry::register`] and
/// looked up by name with [`DocValuesFormatRegistry::for_name`].
#[derive(Debug, Default, Clone)]
pub struct DocValuesFormatRegistry {
    formats: Arc<RwLock<HashMap<String, Arc<dyn DocValuesFormat>>>>,
}

impl DocValuesFormatRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a doc-values format under the given short name.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `name` is empty, contains
    /// characters other than ASCII alphanumerics, or is longer than 127 bytes.
    /// Returns [`LuceneError::IllegalState`] if the name is already registered.
    pub fn register<F>(&self, name: impl Into<String>, format: F) -> Result<()>
    where
        F: DocValuesFormat + 'static,
    {
        let name = name.into();
        super::validate_service_name(&name)?;

        let mut formats = self.formats.write().map_err(|_| {
            LuceneError::IllegalState("doc-values format registry lock was poisoned".to_string())
        })?;

        if formats.contains_key(&name) {
            return Err(LuceneError::IllegalState(format!(
                "doc-values format already registered: {name}"
            )));
        }

        formats.insert(name, Arc::new(format));
        Ok(())
    }

    /// Looks up a doc-values format by name.
    ///
    /// Returns `None` if no format has been registered under the given name.
    /// Registers an already-shared format under its own name.
    ///
    /// The counterpart of [`register`](Self::register) for a format the caller
    /// already holds behind an `Arc`. As in Lucene's `reload`, a name that is
    /// already taken is left alone rather than replaced.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the format's name is not
    /// a valid service name.
    pub fn register_arc(&self, format: Arc<dyn DocValuesFormat>) -> Result<()> {
        let name = format.name().to_string();
        crate::codecs::validate_service_name(&name)?;
        let mut formats = self.formats.write().map_err(|_| {
            LuceneError::IllegalState("doc-values format registry lock was poisoned".to_string())
        })?;
        // Only new formats are added; an existing one is never replaced.
        formats.entry(name).or_insert(format);
        Ok(())
    }

    pub fn for_name(&self, name: &str) -> Option<Arc<dyn DocValuesFormat>> {
        self.formats
            .read()
            .map_err(|_| {
                LuceneError::IllegalState(
                    "doc-values format registry lock was poisoned".to_string(),
                )
            })
            .ok()?
            .get(name)
            .cloned()
    }

    /// Returns the names of all registered doc-values formats, sorted
    /// alphabetically.
    pub fn available_doc_values_formats(&self) -> Vec<String> {
        let Ok(formats) = self.formats.read() else {
            return Vec::new();
        };
        let mut names: Vec<String> = formats.keys().cloned().collect();
        names.sort();
        names
    }
}

/// Adds `format` to the global registry under its own name.
///
/// Equivalent to `DocValuesFormat.reloadDocValuesFormats(ClassLoader)`, which rescans a classloader
/// for formats it has not seen. Rust has no run-time service discovery, so the
/// caller supplies the format.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the format's name is not a
/// valid service name.
pub fn reload_doc_values_formats(format: Arc<dyn DocValuesFormat>) -> Result<()> {
    GLOBAL_DOC_VALUES_REGISTRY.register_arc(format)
}

static GLOBAL_DOC_VALUES_REGISTRY: LazyLock<DocValuesFormatRegistry> =
    LazyLock::new(DocValuesFormatRegistry::new);

/// Looks up a doc-values format by name from the global registry.
///
/// Returns `None` if no format has been registered under the given name.
pub fn doc_values_for_name(name: &str) -> Option<Arc<dyn DocValuesFormat>> {
    GLOBAL_DOC_VALUES_REGISTRY.for_name(name)
}

/// Returns the names of all doc-values formats registered in the global
/// registry.
pub fn available_doc_values_formats() -> Vec<String> {
    GLOBAL_DOC_VALUES_REGISTRY.available_doc_values_formats()
}

/// Registers a doc-values format in the global registry.
///
/// This mirrors the service loading behind `DocValuesFormat.forName` in Apache
/// Lucene: the per-field format resolves the format name a segment's
/// field-info attributes record when it opens the segment for reading, so a
/// format that writes fields must be resolvable by name afterwards — including
/// when the same process merges or reopens segments it just wrote.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] if the name is invalid, or
/// [`LuceneError::IllegalState`] if the name is already registered.
pub fn register_doc_values_format(
    name: impl Into<String>,
    format: impl DocValuesFormat + 'static,
) -> Result<()> {
    GLOBAL_DOC_VALUES_REGISTRY.register(name, format)
}

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

/// A no-op doc-values consumer.
#[derive(Debug, Default, Clone)]
pub struct EmptyDocValuesConsumer;

impl DocValuesConsumer for EmptyDocValuesConsumer {
    fn add_numeric_field(
        &mut self,
        _field: &FieldInfo,
        _values: &dyn DocValuesProducer,
    ) -> Result<()> {
        Ok(())
    }

    fn add_binary_field(
        &mut self,
        _field: &FieldInfo,
        _values: &dyn DocValuesProducer,
    ) -> Result<()> {
        Ok(())
    }

    fn add_sorted_field(
        &mut self,
        _field: &FieldInfo,
        _values: &dyn DocValuesProducer,
    ) -> Result<()> {
        Ok(())
    }

    fn add_sorted_numeric_field(
        &mut self,
        _field: &FieldInfo,
        _values: &dyn DocValuesProducer,
    ) -> Result<()> {
        Ok(())
    }

    fn add_sorted_set_field(
        &mut self,
        _field: &FieldInfo,
        _values: &dyn DocValuesProducer,
    ) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A no-op doc-values format.
#[derive(Debug, Default, Clone)]
pub struct EmptyDocValuesFormat {
    name: String,
}

impl EmptyDocValuesFormat {
    /// Creates a new no-op doc-values format with the given SPI name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl DocValuesFormat for EmptyDocValuesFormat {
    fn name(&self) -> &str {
        &self.name
    }

    fn fields_consumer<'a>(
        &self,
        _state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        Ok(Box::new(EmptyDocValuesConsumer))
    }

    fn fields_producer<'a>(
        &self,
        _state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(EmptyDocValuesProducer))
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{DocIdSetIterator, NO_MORE_DOCS};

    #[test]
    fn empty_numeric_doc_values_returns_no_docs() {
        let mut values = EmptyNumericDocValues::new();
        assert_eq!(values.doc_id(), -1);
        assert_eq!(values.next_doc().unwrap(), NO_MORE_DOCS);
        assert!(!values.advance_exact(0).unwrap());
        assert!(values.long_value().is_err());
    }

    #[test]
    fn empty_binary_doc_values_returns_no_docs() {
        let mut values = EmptyBinaryDocValues::new();
        assert_eq!(values.next_doc().unwrap(), NO_MORE_DOCS);
        assert!(values.binary_value().is_err());
    }

    #[test]
    fn empty_sorted_doc_values_has_no_values() {
        let values = EmptySortedDocValues::new();
        assert_eq!(values.get_value_count().unwrap(), 0);
        assert_eq!(values.lookup_ord(0).unwrap().length, 0);
        assert!(values.ord_value().is_err());
    }

    #[test]
    fn empty_sorted_numeric_doc_values_has_zero_values() {
        let mut values = EmptySortedNumericDocValues::new();
        assert!(!values.advance_exact(0).unwrap());
        assert_eq!(values.doc_value_count().unwrap(), 0);
    }

    #[test]
    fn empty_sorted_set_doc_values_has_empty_dictionary() {
        let mut values = EmptySortedSetDocValues::new();
        assert!(!values.advance_exact(0).unwrap());
        assert_eq!(values.get_value_count().unwrap(), 0);
        assert_eq!(values.lookup_ord(0).unwrap().length, 0);
    }

    #[test]
    fn empty_doc_values_skipper_reports_no_interval() {
        let mut skipper = EmptyDocValuesSkipper;
        skipper.advance(42).unwrap();
        assert_eq!(skipper.num_levels(), 1);
        assert_eq!(skipper.min_doc_id(0), -1);
        assert_eq!(skipper.max_doc_id(0), -1);
        assert_eq!(skipper.global_doc_count(), 0);
    }

    #[test]
    fn empty_doc_values_producer_returns_empty_iterators() {
        let producer = EmptyDocValuesProducer;
        let field = FieldInfo::default();
        let mut numeric = producer.get_numeric(&field).unwrap();
        assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
        let mut binary = producer.get_binary(&field).unwrap();
        assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted = producer.get_sorted(&field).unwrap();
        assert_eq!(sorted.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted_numeric = producer.get_sorted_numeric(&field).unwrap();
        assert_eq!(sorted_numeric.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted_set = producer.get_sorted_set(&field).unwrap();
        assert_eq!(sorted_set.next_doc().unwrap(), NO_MORE_DOCS);
        let skipper = producer.get_skipper(&field).unwrap();
        assert_eq!(skipper.num_levels(), 1);
        producer.check_integrity().unwrap();
    }

    #[test]
    fn empty_doc_values_consumer_accepts_all_fields() {
        let mut consumer = EmptyDocValuesConsumer;
        let producer = EmptyDocValuesProducer;
        let field = FieldInfo::default();
        consumer.add_numeric_field(&field, &producer).unwrap();
        consumer.add_binary_field(&field, &producer).unwrap();
        consumer.add_sorted_field(&field, &producer).unwrap();
        consumer
            .add_sorted_numeric_field(&field, &producer)
            .unwrap();
        consumer.add_sorted_set_field(&field, &producer).unwrap();
        consumer.close().unwrap();
    }

    #[test]
    fn empty_doc_values_format_name_and_factories() {
        use crate::codecs::stub::BufferedUpdates;
        use crate::index::FieldInfos;

        let format = EmptyDocValuesFormat::new("EmptyDV");
        assert_eq!(format.name(), "EmptyDV");

        let dir = crate::store::RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let field_infos = FieldInfos::default();
        let segment_info = crate::codecs::tests::test_segment_info("test", 10);
        let state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &BufferedUpdates,
            context,
        );
        let _consumer = format.fields_consumer(&state).unwrap();

        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);
        let _producer = format.fields_producer(&read_state).unwrap();
    }
}

// -----------------------------------------------------------------------------
// Merging
// -----------------------------------------------------------------------------
//
// Ported from the merge machinery of `org.apache.lucene.codecs.DocValuesConsumer`.
//
// Java builds each merged view as an anonymous `DocValuesProducer` handed to
// `addNumericField` and friends, which pull the merged iterator back out when
// they are ready to write. This port keeps that shape: the merged view is a
// borrowing adapter over the `MergeState`, so the consumer sees exactly the
// producer-shaped API Lucene gives it and the iterator is still built lazily,
// on the consumer's own schedule.

use crate::index::doc_values::{OrdinalMap, SortedDocValuesTermsEnum, SortedSetDocValuesTermsEnum};
use crate::index::merge::{DocIDMerger, DocIDMergerSub, DocMap};
use crate::index::terms::{SeekStatus, TermState, TermsEnum};
use crate::index::DocValuesType;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::{AttributeSource, BytesRef, LongBitSet};

/// The acceptable overhead ratio Lucene passes when it builds a merge-time
/// ordinal map.
///
/// Equivalent to `PackedInts.COMPACT`, which `DocValuesConsumer` uses for both
/// `OrdinalMap.build` calls: a merge writes the map once and reads it once, so
/// the smallest encoding wins over the fastest.
const COMPACT: f32 = 0.0;

/// One sub-reader's numeric values, as the merger walks them.
///
/// Equivalent to `DocValuesConsumer.NumericDocValuesSub`.
pub(crate) struct NumericDocValuesSub {
    pub(crate) doc_map: DocMap,
    pub(crate) values: Box<dyn NumericDocValues>,
    pub(crate) mapped_doc_id: i32,
}

impl DocIDMergerSub for NumericDocValuesSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.values.next_doc()
    }
    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }
    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }
    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// One sub-reader's binary values.
///
/// Equivalent to `DocValuesConsumer.BinaryDocValuesSub`.
struct BinaryDocValuesSub {
    doc_map: DocMap,
    values: Box<dyn BinaryDocValues>,
    mapped_doc_id: i32,
}

impl DocIDMergerSub for BinaryDocValuesSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.values.next_doc()
    }
    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }
    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }
    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// One sub-reader's sorted values, with the map from its ordinals to global
/// ones.
///
/// Equivalent to `DocValuesConsumer.SortedDocValuesSub`.
struct SortedDocValuesSub {
    doc_map: DocMap,
    values: Box<dyn SortedDocValues>,
    map: Arc<OrdinalMap>,
    segment: usize,
    mapped_doc_id: i32,
}

impl DocIDMergerSub for SortedDocValuesSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.values.next_doc()
    }
    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }
    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }
    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// One sub-reader's sorted-numeric values.
///
/// Equivalent to `DocValuesConsumer.SortedNumericDocValuesSub`.
struct SortedNumericDocValuesSub {
    doc_map: DocMap,
    values: Box<dyn SortedNumericDocValues>,
    mapped_doc_id: i32,
}

impl DocIDMergerSub for SortedNumericDocValuesSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.values.next_doc()
    }
    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }
    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }
    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// One sub-reader's sorted-set values, with the map from its ordinals to global
/// ones.
///
/// Equivalent to `DocValuesConsumer.SortedSetDocValuesSub`.
struct SortedSetDocValuesSub {
    doc_map: DocMap,
    values: Box<dyn SortedSetDocValues>,
    map: Arc<OrdinalMap>,
    segment: usize,
    mapped_doc_id: i32,
}

impl DocIDMergerSub for SortedSetDocValuesSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.values.next_doc()
    }
    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }
    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }
    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// Returns each sub-reader's view of `field`, or `None` where the sub does not
/// carry it with the requested doc-values type.
///
/// This is the loop `getMergedNumericDocValues` and its four siblings all open
/// with: look the field up in the sub's own `FieldInfos`, check the type
/// matches, and only then ask the producer for it.
fn sub_field_info<'a>(
    merge_state: &'a MergeState,
    index: usize,
    field: &FieldInfo,
    expected: DocValuesType,
) -> Option<&'a FieldInfo> {
    let reader_field_info = merge_state
        .field_infos
        .get(index)?
        .field_info(&field.name)?;
    if reader_field_info.get_doc_values_type() == expected {
        Some(reader_field_info)
    } else {
        None
    }
}

/// Fails the way Java's merged iterators do when asked to seek.
///
/// The merged views produced here are strictly forward-only: every one of
/// Lucene's anonymous merge iterators throws `UnsupportedOperationException`
/// from `advance` and `advanceExact`, because a merge only ever walks the
/// merged doc-id space once, in order.
fn merge_iterator_is_forward_only() -> LuceneError {
    LuceneError::UnsupportedOperation(
        "a merged doc-values iterator can only be advanced with next_doc".to_string(),
    )
}

macro_rules! merged_iterator_common {
    () => {
        fn doc_id(&self) -> i32 {
            self.doc_id
        }

        fn advance(&mut self, _target: i32) -> Result<i32> {
            Err(merge_iterator_is_forward_only())
        }

        fn cost(&self) -> i64 {
            self.cost
        }
    };
}

macro_rules! merged_advance_exact {
    () => {
        fn advance_exact(&mut self, _target: i32) -> Result<bool> {
            Err(merge_iterator_is_forward_only())
        }
    };
}

// -- Numeric ------------------------------------------------------------------

/// The merged numeric view over a set of sub-readers.
struct MergedNumericDocValues {
    merger: DocIDMerger<NumericDocValuesSub>,
    doc_id: i32,
    cost: i64,
    exhausted: bool,
}

impl DocIdSetIterator for MergedNumericDocValues {
    merged_iterator_common!();

    fn next_doc(&mut self) -> Result<i32> {
        match self.merger.next_sub()? {
            Some(sub) => self.doc_id = sub.mapped_doc_id(),
            None => {
                self.doc_id = NO_MORE_DOCS;
                self.exhausted = true;
            }
        }
        Ok(self.doc_id)
    }
}

impl DocValuesIterator for MergedNumericDocValues {
    merged_advance_exact!();
}

impl NumericDocValues for MergedNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        match self.merger.current_sub() {
            Some(sub) if !self.exhausted => sub.values.long_value(),
            _ => Err(LuceneError::IllegalState(
                "no current document in the merged numeric doc values".to_string(),
            )),
        }
    }
}

/// Merges the numeric values of `subs` into one forward-only iterator.
///
/// Equivalent to `DocValuesConsumer.mergeNumericValues(List, boolean)`.
pub(crate) fn merge_numeric_values(
    subs: Vec<NumericDocValuesSub>,
    index_is_sorted: bool,
) -> Result<Box<dyn NumericDocValues>> {
    let cost = subs.iter().map(|sub| sub.values.cost()).sum();
    Ok(Box::new(MergedNumericDocValues {
        merger: DocIDMerger::new(subs, index_is_sorted)?,
        doc_id: -1,
        cost,
        exhausted: false,
    }))
}

/// Builds the merged numeric view of `field` over every sub-reader.
///
/// Equivalent to `DocValuesConsumer.getMergedNumericDocValues(MergeState, FieldInfo)`.
pub fn get_merged_numeric_doc_values(
    merge_state: &MergeState,
    field: &FieldInfo,
) -> Result<Box<dyn NumericDocValues>> {
    let mut subs = Vec::new();
    debug_assert_eq!(
        merge_state.doc_maps.len(),
        merge_state.doc_values_producers.len()
    );
    for i in 0..merge_state.doc_values_producers.len() {
        let Some(producer) = merge_state.doc_values_producers[i].as_ref() else {
            continue;
        };
        let Some(reader_field_info) = sub_field_info(merge_state, i, field, DocValuesType::NUMERIC)
        else {
            continue;
        };
        subs.push(NumericDocValuesSub {
            doc_map: Arc::clone(&merge_state.doc_maps[i]),
            values: producer.get_numeric(reader_field_info)?,
            mapped_doc_id: -1,
        });
    }
    merge_numeric_values(subs, merge_state.needs_index_sort)
}

// -- Binary -------------------------------------------------------------------

/// The merged binary view over a set of sub-readers.
struct MergedBinaryDocValues {
    merger: DocIDMerger<BinaryDocValuesSub>,
    doc_id: i32,
    cost: i64,
    exhausted: bool,
}

impl DocIdSetIterator for MergedBinaryDocValues {
    merged_iterator_common!();

    fn next_doc(&mut self) -> Result<i32> {
        match self.merger.next_sub()? {
            Some(sub) => self.doc_id = sub.mapped_doc_id(),
            None => {
                self.doc_id = NO_MORE_DOCS;
                self.exhausted = true;
            }
        }
        Ok(self.doc_id)
    }
}

impl DocValuesIterator for MergedBinaryDocValues {
    merged_advance_exact!();
}

impl BinaryDocValues for MergedBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        match self.merger.current_sub() {
            Some(sub) if !self.exhausted => sub.values.binary_value(),
            _ => Err(LuceneError::IllegalState(
                "no current document in the merged binary doc values".to_string(),
            )),
        }
    }
}

/// Builds the merged binary view of `field` over every sub-reader.
///
/// Equivalent to `DocValuesConsumer.getMergedBinaryDocValues(FieldInfo, MergeState)`.
pub fn get_merged_binary_doc_values(
    field: &FieldInfo,
    merge_state: &MergeState,
) -> Result<Box<dyn BinaryDocValues>> {
    let mut subs = Vec::new();
    let mut cost = 0i64;
    for i in 0..merge_state.doc_values_producers.len() {
        let Some(producer) = merge_state.doc_values_producers[i].as_ref() else {
            continue;
        };
        let Some(reader_field_info) = sub_field_info(merge_state, i, field, DocValuesType::BINARY)
        else {
            continue;
        };
        let values = producer.get_binary(reader_field_info)?;
        cost += values.cost();
        subs.push(BinaryDocValuesSub {
            doc_map: Arc::clone(&merge_state.doc_maps[i]),
            values,
            mapped_doc_id: -1,
        });
    }
    Ok(Box::new(MergedBinaryDocValues {
        merger: DocIDMerger::new(subs, merge_state.needs_index_sort)?,
        doc_id: -1,
        cost,
        exhausted: false,
    }))
}

// -- Sorted numeric -----------------------------------------------------------

/// The merged sorted-numeric view over a set of sub-readers.
struct MergedSortedNumericDocValues {
    merger: DocIDMerger<SortedNumericDocValuesSub>,
    doc_id: i32,
    cost: i64,
    exhausted: bool,
}

impl DocIdSetIterator for MergedSortedNumericDocValues {
    merged_iterator_common!();

    fn next_doc(&mut self) -> Result<i32> {
        match self.merger.next_sub()? {
            Some(sub) => self.doc_id = sub.mapped_doc_id(),
            None => {
                self.doc_id = NO_MORE_DOCS;
                self.exhausted = true;
            }
        }
        Ok(self.doc_id)
    }
}

impl DocValuesIterator for MergedSortedNumericDocValues {
    merged_advance_exact!();
}

impl SortedNumericDocValues for MergedSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        if self.exhausted {
            return Err(LuceneError::IllegalState(
                "no current document in the merged sorted numeric doc values".to_string(),
            ));
        }
        match self.merger.current_sub_mut() {
            Some(sub) => sub.values.next_value(),
            None => Err(LuceneError::IllegalState(
                "no current document in the merged sorted numeric doc values".to_string(),
            )),
        }
    }

    fn doc_value_count(&self) -> Result<i32> {
        match self.merger.current_sub() {
            Some(sub) if !self.exhausted => sub.values.doc_value_count(),
            _ => Err(LuceneError::IllegalState(
                "no current document in the merged sorted numeric doc values".to_string(),
            )),
        }
    }
}

/// Builds the merged sorted-numeric view of `field` over every sub-reader.
///
/// Equivalent to `DocValuesConsumer.getMergedSortedNumericDocValues(FieldInfo, MergeState)`.
/// When every sub is single-valued the result is a singleton view over the
/// merged numeric values, which is the shape codecs can encode most tightly —
/// the specialisation Lucene documents here.
pub fn get_merged_sorted_numeric_doc_values(
    field: &FieldInfo,
    merge_state: &MergeState,
) -> Result<Box<dyn SortedNumericDocValues>> {
    // A fresh iterator, and a fresh DocIDMerger, are needed for each view.
    let mut subs = Vec::new();
    let mut cost = 0i64;
    let mut all_singletons = true;
    for i in 0..merge_state.doc_values_producers.len() {
        let values: Box<dyn SortedNumericDocValues> = match merge_state.doc_values_producers[i]
            .as_ref()
            .zip(sub_field_info(
                merge_state,
                i,
                field,
                DocValuesType::SORTED_NUMERIC,
            )) {
            Some((producer, reader_field_info)) => {
                producer.get_sorted_numeric(reader_field_info)?
            }
            None => Box::new(DocValues::empty_sorted_numeric()),
        };
        cost += values.cost();
        if all_singletons && !values.is_singleton() {
            all_singletons = false;
        }
        subs.push(SortedNumericDocValuesSub {
            doc_map: Arc::clone(&merge_state.doc_maps[i]),
            values,
            mapped_doc_id: -1,
        });
    }

    if all_singletons {
        // Every sub is single-valued. Specialising here is what lets a codec
        // encode the merged field as a plain numeric one.
        let mut single_valued_subs = Vec::with_capacity(subs.len());
        for sub in subs {
            let single = sub.values.into_singleton().ok_or_else(|| {
                LuceneError::IllegalState(
                    "sorted numeric doc values reported singleton but did not unwrap".to_string(),
                )
            })?;
            single_valued_subs.push(NumericDocValuesSub {
                doc_map: sub.doc_map,
                values: single,
                mapped_doc_id: -1,
            });
        }
        let merged = merge_numeric_values(single_valued_subs, merge_state.needs_index_sort)?;
        return Ok(Box::new(DocValues::singleton_numeric(merged)));
    }

    Ok(Box::new(MergedSortedNumericDocValues {
        merger: DocIDMerger::new(subs, merge_state.needs_index_sort)?,
        doc_id: -1,
        cost,
        exhausted: false,
    }))
}

// -- Sorted -------------------------------------------------------------------

/// The merged sorted view over a set of sub-readers.
struct MergedSortedDocValues {
    merger: DocIDMerger<SortedDocValuesSub>,
    map: Arc<OrdinalMap>,
    doc_id: i32,
    cost: i64,
    exhausted: bool,
}

impl DocIdSetIterator for MergedSortedDocValues {
    merged_iterator_common!();

    fn next_doc(&mut self) -> Result<i32> {
        match self.merger.next_sub()? {
            Some(sub) => self.doc_id = sub.mapped_doc_id(),
            None => {
                self.doc_id = NO_MORE_DOCS;
                self.exhausted = true;
            }
        }
        Ok(self.doc_id)
    }
}

impl DocValuesIterator for MergedSortedDocValues {
    merged_advance_exact!();
}

impl SortedDocValues for MergedSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        match self.merger.current_sub() {
            Some(sub) if !self.exhausted => {
                let sub_ord = sub.values.ord_value()?;
                debug_assert_ne!(sub_ord, -1);
                Ok(sub.map.get_global_ords(sub.segment).get(i64::from(sub_ord)) as i32)
            }
            _ => Err(LuceneError::IllegalState(
                "no current document in the merged sorted doc values".to_string(),
            )),
        }
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.map.get_value_count() as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        let segment_number = self.map.get_first_segment_number(i64::from(ord));
        let segment_ord = self.map.get_first_segment_ord(i64::from(ord)) as i32;
        self.merger.subs()[segment_number]
            .values
            .lookup_ord(segment_ord)
    }
}

/// Builds the ordinal map that merges the sorted field's dictionaries.
///
/// Equivalent to `DocValuesConsumer.createOrdinalMapForSortedDV(FieldInfo, MergeState)`.
/// A sub with deletions contributes only the ordinals its live documents still
/// use, so terms that no surviving document references are dropped from the
/// merged dictionary.
pub fn create_ordinal_map_for_sorted_dv(
    field: &FieldInfo,
    merge_state: &MergeState,
) -> Result<OrdinalMap> {
    let to_merge = select_sorted_leaves_to_merge(field, merge_state)?;
    let mut live_terms: Vec<Box<dyn TermsEnum>> = Vec::with_capacity(to_merge.len());
    let mut weights: Vec<i64> = Vec::with_capacity(to_merge.len());
    for (sub, mut dv) in to_merge.into_iter().enumerate() {
        match merge_state.live_docs.get(sub).and_then(Option::as_ref) {
            None => {
                weights.push(i64::from(dv.get_value_count()?));
                live_terms.push(SortedDocValuesTermsEnum::new(dv));
            }
            Some(live_docs) => {
                let mut bitset = LongBitSet::new(i64::from(dv.get_value_count()?))?;
                loop {
                    let doc_id = dv.next_doc()?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    if live_docs.get(doc_id as usize) {
                        let ord = dv.ord_value()?;
                        if ord >= 0 {
                            bitset.set(i64::from(ord));
                        }
                    }
                }
                weights.push(bitset.cardinality());
                live_terms.push(Box::new(BitsFilteredTermsEnum::new(
                    SortedDocValuesTermsEnum::new(dv),
                    bitset,
                )));
            }
        }
    }
    OrdinalMap::build(live_terms, &weights, COMPACT)
}

/// Returns each sub-reader's sorted view of `field`, empty where it has none.
fn select_sorted_leaves_to_merge(
    field: &FieldInfo,
    merge_state: &MergeState,
) -> Result<Vec<Box<dyn SortedDocValues>>> {
    let mut to_merge: Vec<Box<dyn SortedDocValues>> =
        Vec::with_capacity(merge_state.doc_values_producers.len());
    for i in 0..merge_state.doc_values_producers.len() {
        match merge_state.doc_values_producers[i]
            .as_ref()
            .zip(sub_field_info(merge_state, i, field, DocValuesType::SORTED))
        {
            Some((producer, reader_field_info)) => {
                to_merge.push(producer.get_sorted(reader_field_info)?)
            }
            None => to_merge.push(Box::new(DocValues::empty_sorted())),
        }
    }
    Ok(to_merge)
}

/// Builds the merged sorted view of `field`, given the ordinal map that maps
/// each sub's ordinals into the merged dictionary.
///
/// Equivalent to `DocValuesConsumer.getMergedSortedSetDocValues(FieldInfo, MergeState, OrdinalMap)`
/// — the three-argument overload, which returns a `SortedDocValues`.
pub fn get_merged_sorted_doc_values(
    field: &FieldInfo,
    merge_state: &MergeState,
    map: Arc<OrdinalMap>,
) -> Result<Box<dyn SortedDocValues>> {
    let mut subs = Vec::new();
    for (i, values) in select_sorted_leaves_to_merge(field, merge_state)?
        .into_iter()
        .enumerate()
    {
        subs.push(SortedDocValuesSub {
            doc_map: Arc::clone(&merge_state.doc_maps[i]),
            values,
            map: Arc::clone(&map),
            segment: i,
            mapped_doc_id: -1,
        });
    }
    merge_sorted_values(subs, merge_state.needs_index_sort, map)
}

/// Merges the sorted values of `subs` through `map`.
///
/// Equivalent to `DocValuesConsumer.mergeSortedValues(List, MergeState, OrdinalMap)`.
fn merge_sorted_values(
    subs: Vec<SortedDocValuesSub>,
    index_is_sorted: bool,
    map: Arc<OrdinalMap>,
) -> Result<Box<dyn SortedDocValues>> {
    let cost = subs.iter().map(|sub| sub.values.cost()).sum();
    Ok(Box::new(MergedSortedDocValues {
        merger: DocIDMerger::new(subs, index_is_sorted)?,
        map,
        doc_id: -1,
        cost,
        exhausted: false,
    }))
}

// -- Sorted set ---------------------------------------------------------------

/// The merged sorted-set view over a set of sub-readers.
struct MergedSortedSetDocValues {
    merger: DocIDMerger<SortedSetDocValuesSub>,
    map: Arc<OrdinalMap>,
    doc_id: i32,
    cost: i64,
    exhausted: bool,
}

impl DocIdSetIterator for MergedSortedSetDocValues {
    merged_iterator_common!();

    fn next_doc(&mut self) -> Result<i32> {
        match self.merger.next_sub()? {
            Some(sub) => self.doc_id = sub.mapped_doc_id(),
            None => {
                self.doc_id = NO_MORE_DOCS;
                self.exhausted = true;
            }
        }
        Ok(self.doc_id)
    }
}

impl DocValuesIterator for MergedSortedSetDocValues {
    merged_advance_exact!();
}

impl SortedSetDocValues for MergedSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        if self.exhausted {
            return Err(LuceneError::IllegalState(
                "no current document in the merged sorted set doc values".to_string(),
            ));
        }
        let Some(sub) = self.merger.current_sub_mut() else {
            return Err(LuceneError::IllegalState(
                "no current document in the merged sorted set doc values".to_string(),
            ));
        };
        let sub_ord = sub.values.next_ord()?;
        debug_assert_ne!(sub_ord, -1);
        Ok(sub.map.get_global_ords(sub.segment).get(sub_ord))
    }

    fn doc_value_count(&self) -> Result<i32> {
        match self.merger.current_sub() {
            Some(sub) if !self.exhausted => sub.values.doc_value_count(),
            _ => Err(LuceneError::IllegalState(
                "no current document in the merged sorted set doc values".to_string(),
            )),
        }
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        let segment_number = self.map.get_first_segment_number(ord);
        let segment_ord = self.map.get_first_segment_ord(ord);
        self.merger.subs()[segment_number]
            .values
            .lookup_ord(segment_ord)
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.map.get_value_count())
    }
}

/// Returns each sub-reader's sorted-set view of `field`, empty where it has
/// none.
///
/// Equivalent to `DocValuesConsumer.selectLeavesToMerge(FieldInfo, MergeState)`.
pub fn select_leaves_to_merge(
    field: &FieldInfo,
    merge_state: &MergeState,
) -> Result<Vec<Box<dyn SortedSetDocValues>>> {
    let mut to_merge: Vec<Box<dyn SortedSetDocValues>> =
        Vec::with_capacity(merge_state.doc_values_producers.len());
    for i in 0..merge_state.doc_values_producers.len() {
        match merge_state.doc_values_producers[i]
            .as_ref()
            .zip(sub_field_info(
                merge_state,
                i,
                field,
                DocValuesType::SORTED_SET,
            )) {
            Some((producer, reader_field_info)) => {
                to_merge.push(producer.get_sorted_set(reader_field_info)?)
            }
            None => to_merge.push(Box::new(DocValues::empty_sorted_set())),
        }
    }
    Ok(to_merge)
}

/// Builds the ordinal map that merges a sorted-set field's dictionaries.
///
/// Equivalent to `DocValuesConsumer.createOrdinalMapForSortedSetDV(List, MergeState)`.
pub fn create_ordinal_map_for_sorted_set_dv(
    to_merge: Vec<Box<dyn SortedSetDocValues>>,
    merge_state: &MergeState,
) -> Result<OrdinalMap> {
    let mut live_terms: Vec<Box<dyn TermsEnum>> = Vec::with_capacity(to_merge.len());
    let mut weights: Vec<i64> = Vec::with_capacity(to_merge.len());
    for (sub, mut dv) in to_merge.into_iter().enumerate() {
        match merge_state.live_docs.get(sub).and_then(Option::as_ref) {
            None => {
                weights.push(dv.get_value_count()?);
                live_terms.push(SortedSetDocValuesTermsEnum::new(dv));
            }
            Some(live_docs) => {
                let mut bitset = LongBitSet::new(dv.get_value_count()?)?;
                loop {
                    let doc_id = dv.next_doc()?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    if live_docs.get(doc_id as usize) {
                        let count = dv.doc_value_count()?;
                        for _ in 0..count {
                            let ord = dv.next_ord()?;
                            bitset.set(ord);
                        }
                    }
                }
                weights.push(bitset.cardinality());
                live_terms.push(Box::new(BitsFilteredTermsEnum::new(
                    SortedSetDocValuesTermsEnum::new(dv),
                    bitset,
                )));
            }
        }
    }
    OrdinalMap::build(live_terms, &weights, COMPACT)
}

/// Builds the merged sorted-set view of `field`.
///
/// Equivalent to `DocValuesConsumer.getMergedSortedSetDocValues(FieldInfo, MergeState, OrdinalMap, List)`.
/// As with sorted-numeric, a field whose every sub is single-valued is
/// specialised to a singleton view so codecs can encode it as a plain sorted
/// field.
pub fn get_merged_sorted_set_doc_values(
    field: &FieldInfo,
    merge_state: &MergeState,
    map: Arc<OrdinalMap>,
) -> Result<Box<dyn SortedSetDocValues>> {
    let mut subs = Vec::new();
    let mut cost = 0i64;
    let mut all_singletons = true;
    for (i, values) in select_leaves_to_merge(field, merge_state)?
        .into_iter()
        .enumerate()
    {
        cost += values.cost();
        if all_singletons && !values.is_singleton() {
            all_singletons = false;
        }
        subs.push(SortedSetDocValuesSub {
            doc_map: Arc::clone(&merge_state.doc_maps[i]),
            values,
            map: Arc::clone(&map),
            segment: i,
            mapped_doc_id: -1,
        });
    }

    if all_singletons {
        let mut single_valued_subs = Vec::with_capacity(subs.len());
        for sub in subs {
            let single = sub.values.into_singleton().ok_or_else(|| {
                LuceneError::IllegalState(
                    "sorted set doc values reported singleton but did not unwrap".to_string(),
                )
            })?;
            single_valued_subs.push(SortedDocValuesSub {
                doc_map: sub.doc_map,
                values: single,
                map: sub.map,
                segment: sub.segment,
                mapped_doc_id: -1,
            });
        }
        let merged = merge_sorted_values(single_valued_subs, merge_state.needs_index_sort, map)?;
        return Ok(Box::new(DocValues::singleton_sorted(merged)));
    }

    Ok(Box::new(MergedSortedSetDocValues {
        merger: DocIDMerger::new(subs, merge_state.needs_index_sort)?,
        map,
        doc_id: -1,
        cost,
        exhausted: false,
    }))
}

/// Returns whether every document holds at most one value.
///
/// Equivalent to `DocValuesConsumer.isSingleValued(Iterable<Number>)`.
pub fn is_single_valued<I: IntoIterator<Item = i64>>(doc_to_value_count: I) -> bool {
    doc_to_value_count.into_iter().all(|count| count <= 1)
}

/// Views a multi-valued sequence that happens to be single-valued as a plain
/// one, substituting `missing_value` where a document has no value.
///
/// Equivalent to `DocValuesConsumer.singletonView(Iterable<Number>, Iterable<Number>, Number)`.
pub fn singleton_view(
    doc_to_value_count: Vec<i64>,
    values: Vec<i64>,
    missing_value: Option<i64>,
) -> Vec<Option<i64>> {
    debug_assert!(is_single_valued(doc_to_value_count.iter().copied()));
    let mut out = Vec::with_capacity(doc_to_value_count.len());
    let mut iter = values.into_iter();
    for count in doc_to_value_count {
        if count == 0 {
            out.push(missing_value);
        } else {
            out.push(iter.next());
        }
    }
    out
}

/// A terms enum that hides the terms no live document still uses.
///
/// Equivalent to `DocValuesConsumer.BitsFilteredTermsEnum`, which Lucene builds
/// as a `FilteredTermsEnum` whose `accept` consults a `LongBitSet` of live
/// ordinals — deliberately constructed with `startWithSeek == false`, so it
/// walks the dictionary linearly.
///
/// **Divergence from Lucene 10.5.0.** This port wraps the delegate directly
/// rather than going through [`FilteredTermsEnum`](crate::index::terms::FilteredTermsEnum),
/// because that type's filter is handed only the term bytes and this decision
/// is made on the delegate's *ordinal*. Wrapping keeps the delegate reachable;
/// the accepted sequence of terms and their ordinals are identical either way.
struct BitsFilteredTermsEnum {
    inner: Box<dyn TermsEnum>,
    live_terms: LongBitSet,
}

impl BitsFilteredTermsEnum {
    fn new(inner: Box<dyn TermsEnum>, live_terms: LongBitSet) -> Self {
        Self { inner, live_terms }
    }
}

impl TermsEnum for BitsFilteredTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
    }

    fn term(&self) -> Result<BytesRef> {
        self.inner.term()
    }

    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }

    fn doc_freq(&self) -> Result<i32> {
        self.inner.doc_freq()
    }

    fn total_term_freq(&self) -> Result<i64> {
        self.inner.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn crate::index::postings_enum::PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn crate::index::postings_enum::PostingsEnum>> {
        self.inner.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn crate::index::postings_enum::ImpactsEnum>> {
        self.inner.impacts(flags)
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        loop {
            match self.inner.next()? {
                None => return Ok(None),
                Some(term) => {
                    if self.live_terms.get(self.inner.ord()?) {
                        return Ok(Some(term));
                    }
                }
            }
        }
    }
}

/// The merged view of one field, in the shape the consumer expects.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` subclasses
/// `DocValuesConsumer.merge*Field` hands to `add*Field`: it exists only so the
/// consumer can pull the merged iterator when it is ready to write, and it
/// rejects any field but the one it was built for, exactly as Java's
/// "wrong fieldInfo" guard does.
#[derive(Debug)]
struct MergedDocValuesProducer<'a> {
    field: &'a FieldInfo,
    merge_state: &'a MergeState,
    /// The ordinal map, for the two dictionary-backed kinds.
    map: Option<Arc<OrdinalMap>>,
}

impl<'a> MergedDocValuesProducer<'a> {
    fn new(
        field: &'a FieldInfo,
        merge_state: &'a MergeState,
        map: Option<Arc<OrdinalMap>>,
    ) -> Self {
        Self {
            field,
            merge_state,
            map,
        }
    }

    /// Fails when asked for a field other than the one being merged.
    fn check_field(&self, field: &FieldInfo) -> Result<()> {
        if field.name != self.field.name {
            return Err(LuceneError::IllegalArgument("wrong fieldInfo".to_string()));
        }
        Ok(())
    }

    /// Returns the ordinal map, which the two dictionary kinds always carry.
    fn map(&self) -> Result<Arc<OrdinalMap>> {
        self.map.clone().ok_or_else(|| {
            LuceneError::IllegalState(
                "a sorted or sorted-set merge needs an ordinal map".to_string(),
            )
        })
    }

    fn unsupported(&self, kind: &str) -> LuceneError {
        LuceneError::UnsupportedOperation(format!(
            "the merged view of field '{}' does not provide {kind} doc values",
            self.field.name
        ))
    }
}

impl DocValuesProducer for MergedDocValuesProducer<'_> {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        self.check_field(field)?;
        get_merged_numeric_doc_values(self.merge_state, self.field)
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>> {
        self.check_field(field)?;
        get_merged_binary_doc_values(self.field, self.merge_state)
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues>> {
        self.check_field(field)?;
        get_merged_sorted_doc_values(self.field, self.merge_state, self.map()?)
    }

    fn get_sorted_numeric(&self, field: &FieldInfo) -> Result<Box<dyn SortedNumericDocValues>> {
        self.check_field(field)?;
        get_merged_sorted_numeric_doc_values(self.field, self.merge_state)
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Box<dyn SortedSetDocValues>> {
        self.check_field(field)?;
        get_merged_sorted_set_doc_values(self.field, self.merge_state, self.map()?)
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper>> {
        Err(self.unsupported("skip-index"))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Err(self.unsupported("merge-instance"))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
