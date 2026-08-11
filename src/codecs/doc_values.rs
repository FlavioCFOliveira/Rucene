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
    EmptyDocValuesSkipper, EmptyNumericDocValues, EmptySortedDocValues,
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
pub trait DocValuesProducer: Send + Sync + fmt::Debug {
    /// Returns the numeric values for the given field.
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues + Send + Sync>>;

    /// Returns the binary values for the given field.
    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues + Send + Sync>>;

    /// Returns the sorted values for the given field.
    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues + Send + Sync>>;

    /// Returns the sorted-numeric values for the given field.
    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn SortedNumericDocValues + Send + Sync>>;

    /// Returns the sorted-set values for the given field.
    fn get_sorted_set(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn SortedSetDocValues + Send + Sync>>;

    /// Returns the skip index for the given field.
    fn get_skipper(&self, field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper + Send + Sync>>;

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
    /// The default implementation is a no-op; concrete formats override it with
    /// format-specific merge logic.
    fn merge(&mut self, _merge_state: &MergeState) -> Result<()> {
        Ok(())
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
    ) -> Result<Box<dyn DocValuesProducer + 'a>>;
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

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

/// A no-op doc-values producer.
#[derive(Debug, Default, Clone)]
pub struct EmptyDocValuesProducer;

impl DocValuesProducer for EmptyDocValuesProducer {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Box<dyn NumericDocValues + Send + Sync>> {
        Ok(Box::new(EmptyNumericDocValues::new()))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Box<dyn BinaryDocValues + Send + Sync>> {
        Ok(Box::new(EmptyBinaryDocValues::new()))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Box<dyn SortedDocValues + Send + Sync>> {
        Ok(Box::new(EmptySortedDocValues::new()))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Box<dyn SortedNumericDocValues + Send + Sync>> {
        Ok(Box::new(EmptySortedNumericDocValues::new()))
    }

    fn get_sorted_set(
        &self,
        _field: &FieldInfo,
    ) -> Result<Box<dyn SortedSetDocValues + Send + Sync>> {
        Ok(Box::new(EmptySortedSetDocValues::new()))
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper + Send + Sync>> {
        Ok(Box::new(EmptyDocValuesSkipper))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

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
    ) -> Result<Box<dyn DocValuesProducer + 'a>> {
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
