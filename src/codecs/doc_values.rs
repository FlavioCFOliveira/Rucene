//! Doc-values format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.DocValuesFormat`,
//! `DocValuesConsumer` and `DocValuesProducer`.
//!
//! These traits provide the abstract read/write API for per-document columnar
//! values (numeric, binary, sorted, sorted-set and sorted-numeric). Concrete
//! codecs implement the format-specific encoding underneath this API.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::Result;
use crate::util::BytesRef;

use super::postings::MergeState;
use super::state::{SegmentReadState, SegmentWriteState};
use super::stub::FieldInfo;

// -----------------------------------------------------------------------------
// Per-field value abstractions
// -----------------------------------------------------------------------------

/// Iterator over the numeric doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.NumericDocValues`.
pub trait NumericDocValues: Send + Sync {
    /// Returns the numeric value for the given document.
    fn get(&self, doc_id: i32) -> Result<i64>;
}

/// Iterator over the binary doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.BinaryDocValues`.
pub trait BinaryDocValues: Send + Sync {
    /// Returns the binary value for the given document.
    fn get(&self, doc_id: i32) -> Result<BytesRef>;
}

/// Iterator over the sorted binary doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.SortedDocValues`.
pub trait SortedDocValues: Send + Sync {
    /// Returns the ordinal for the current document.
    fn ord_value(&self) -> Result<i32>;

    /// Returns the number of unique values.
    fn get_value_count(&self) -> Result<i32>;

    /// Looks up the binary value for the given ordinal.
    fn lookup_ord(&self, ord: i32) -> Result<BytesRef>;
}

/// Iterator over the sorted numeric doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.SortedNumericDocValues`.
pub trait SortedNumericDocValues: Send + Sync {
    /// Positions the iterator on or after the given document and returns the
    /// number of values for that document.
    fn set_document(&self, doc_id: i32) -> Result<i32>;

    /// Returns the next numeric value for the current document.
    fn next_value(&self) -> Result<i64>;
}

/// Iterator over the sorted set doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.SortedSetDocValues`.
pub trait SortedSetDocValues: Send + Sync {
    /// Positions the iterator on or after the given document and returns the
    /// number of values for that document.
    fn set_document(&self, doc_id: i32) -> Result<i32>;

    /// Returns the next ordinal for the current document.
    fn next_ord(&self) -> Result<i64>;

    /// Returns the number of unique values.
    fn get_value_count(&self) -> Result<i64>;

    /// Looks up the binary value for the given ordinal.
    fn lookup_ord(&self, ord: i64) -> Result<BytesRef>;
}

/// Skip index for fast-forwarding inside a doc-values field.
///
/// Equivalent to `org.apache.lucene.index.DocValuesSkipper`.
pub trait DocValuesSkipper: Send + Sync {
    /// Positions the skipper at or after the target document.
    fn advance(&self, target: i32) -> Result<i32>;

    /// Returns the first doc ID covered by the current block.
    fn min_doc_id(&self) -> i32;

    /// Returns the last doc ID (inclusive) covered by the current block.
    fn max_doc_id(&self) -> i32;

    /// Returns the minimum value in the current block.
    fn min_value(&self) -> i64;

    /// Returns the maximum value in the current block.
    fn max_value(&self) -> i64;
}

// -----------------------------------------------------------------------------
// Producer
// -----------------------------------------------------------------------------

/// Reads doc-values fields from a segment.
///
/// Equivalent to `org.apache.lucene.codecs.DocValuesProducer`.
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
    fn fields_consumer(&self, state: &SegmentWriteState) -> Result<Box<dyn DocValuesConsumer>>;

    /// Returns a producer to read docvalues from the index.
    fn fields_producer(&self, state: &SegmentReadState) -> Result<Box<dyn DocValuesProducer>>;
}

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

/// A no-op numeric doc-values iterator that always returns zero.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyNumericDocValues;

impl NumericDocValues for EmptyNumericDocValues {
    fn get(&self, _doc_id: i32) -> Result<i64> {
        Ok(0)
    }
}

/// A no-op binary doc-values iterator that always returns an empty value.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyBinaryDocValues;

impl BinaryDocValues for EmptyBinaryDocValues {
    fn get(&self, _doc_id: i32) -> Result<BytesRef> {
        Ok(BytesRef::new(Vec::new()))
    }
}

/// A no-op sorted doc-values iterator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptySortedDocValues;

impl SortedDocValues for EmptySortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(-1)
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(0)
    }

    fn lookup_ord(&self, _ord: i32) -> Result<BytesRef> {
        Ok(BytesRef::new(Vec::new()))
    }
}

/// A no-op sorted-numeric doc-values iterator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptySortedNumericDocValues;

impl SortedNumericDocValues for EmptySortedNumericDocValues {
    fn set_document(&self, _doc_id: i32) -> Result<i32> {
        Ok(0)
    }

    fn next_value(&self) -> Result<i64> {
        Ok(0)
    }
}

/// A no-op sorted-set doc-values iterator.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptySortedSetDocValues;

impl SortedSetDocValues for EmptySortedSetDocValues {
    fn set_document(&self, _doc_id: i32) -> Result<i32> {
        Ok(0)
    }

    fn next_ord(&self) -> Result<i64> {
        Ok(-1)
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(0)
    }

    fn lookup_ord(&self, _ord: i64) -> Result<BytesRef> {
        Ok(BytesRef::new(Vec::new()))
    }
}

/// A no-op doc-values skipper.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyDocValuesSkipper;

impl DocValuesSkipper for EmptyDocValuesSkipper {
    fn advance(&self, target: i32) -> Result<i32> {
        Ok(target)
    }

    fn min_doc_id(&self) -> i32 {
        0
    }

    fn max_doc_id(&self) -> i32 {
        0
    }

    fn min_value(&self) -> i64 {
        0
    }

    fn max_value(&self) -> i64 {
        0
    }
}

/// A no-op doc-values producer.
#[derive(Debug, Default, Clone)]
pub struct EmptyDocValuesProducer;

impl DocValuesProducer for EmptyDocValuesProducer {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        Ok(Box::new(EmptyNumericDocValues))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>> {
        Ok(Box::new(EmptyBinaryDocValues))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Box<dyn SortedDocValues>> {
        Ok(Box::new(EmptySortedDocValues))
    }

    fn get_sorted_numeric(&self, _field: &FieldInfo) -> Result<Box<dyn SortedNumericDocValues>> {
        Ok(Box::new(EmptySortedNumericDocValues))
    }

    fn get_sorted_set(&self, _field: &FieldInfo) -> Result<Box<dyn SortedSetDocValues>> {
        Ok(Box::new(EmptySortedSetDocValues))
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper>> {
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

    fn fields_consumer(&self, _state: &SegmentWriteState) -> Result<Box<dyn DocValuesConsumer>> {
        Ok(Box::new(EmptyDocValuesConsumer))
    }

    fn fields_producer(&self, _state: &SegmentReadState) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(EmptyDocValuesProducer))
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_numeric_doc_values_returns_zero() {
        let values = EmptyNumericDocValues;
        assert_eq!(values.get(0).unwrap(), 0);
        assert_eq!(values.get(100).unwrap(), 0);
    }

    #[test]
    fn empty_binary_doc_values_returns_empty() {
        let values = EmptyBinaryDocValues;
        assert_eq!(values.get(0).unwrap().length, 0);
    }

    #[test]
    fn empty_sorted_doc_values_has_no_values() {
        let values = EmptySortedDocValues;
        assert_eq!(values.ord_value().unwrap(), -1);
        assert_eq!(values.get_value_count().unwrap(), 0);
    }

    #[test]
    fn empty_sorted_numeric_doc_values_has_no_values() {
        let values = EmptySortedNumericDocValues;
        assert_eq!(values.set_document(0).unwrap(), 0);
    }

    #[test]
    fn empty_sorted_set_doc_values_has_no_values() {
        let values = EmptySortedSetDocValues;
        assert_eq!(values.set_document(0).unwrap(), 0);
        assert_eq!(values.next_ord().unwrap(), -1);
    }

    #[test]
    fn empty_doc_values_skipper_advance_is_identity() {
        let skipper = EmptyDocValuesSkipper;
        assert_eq!(skipper.advance(42).unwrap(), 42);
    }

    #[test]
    fn empty_doc_values_producer_returns_empty_iterators() {
        let producer = EmptyDocValuesProducer;
        let field = FieldInfo;
        assert_eq!(producer.get_numeric(&field).unwrap().get(0).unwrap(), 0);
        assert_eq!(
            producer.get_binary(&field).unwrap().get(0).unwrap().length,
            0
        );
        assert_eq!(
            producer.get_sorted(&field).unwrap().ord_value().unwrap(),
            -1
        );
        assert_eq!(
            producer
                .get_sorted_numeric(&field)
                .unwrap()
                .set_document(0)
                .unwrap(),
            0
        );
        assert_eq!(
            producer
                .get_sorted_set(&field)
                .unwrap()
                .set_document(0)
                .unwrap(),
            0
        );
        assert_eq!(producer.get_skipper(&field).unwrap().min_doc_id(), 0);
        producer.check_integrity().unwrap();
    }

    #[test]
    fn empty_doc_values_consumer_accepts_all_fields() {
        let mut consumer = EmptyDocValuesConsumer;
        let producer = EmptyDocValuesProducer;
        let field = FieldInfo;
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
        use crate::codecs::stub::{BufferedUpdates, FieldInfos, SegmentInfo};

        let format = EmptyDocValuesFormat::new("EmptyDV");
        assert_eq!(format.name(), "EmptyDV");

        let dir = crate::store::RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &SegmentInfo,
            &FieldInfos,
            &BufferedUpdates,
            context,
        );
        let _consumer = format.fields_consumer(&state).unwrap();

        let read_state = SegmentReadState::new(dir_ref, &SegmentInfo, &FieldInfos, context);
        let _producer = format.fields_producer(&read_state).unwrap();
    }
}
