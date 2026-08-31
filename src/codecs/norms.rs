//! Norms format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.NormsFormat`,
//! `NormsConsumer` and `NormsProducer`.
//!
//! These traits provide the abstract read/write API for per-document score
//! normalization values. Concrete codecs implement the format-specific encoding
//! underneath this API.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::Result;

use super::postings::MergeState;
use super::state::{SegmentReadState, SegmentWriteState};
use super::stub::FieldInfo;
use crate::index::doc_values::{DocValues, EmptyNumericDocValues, NumericDocValues};

// -----------------------------------------------------------------------------
// Producer
// -----------------------------------------------------------------------------

/// Reads normalization values from a segment.
///
/// Equivalent to `org.apache.lucene.codecs.NormsProducer`.
pub trait NormsProducer: Send + Sync + fmt::Debug {
    /// Returns the numeric norm values for the given field.
    fn get_norms(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>>;

    /// Checks consistency of this producer.
    fn check_integrity(&self) -> Result<()>;

    /// Returns an instance optimized for merging.
    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>>;

    /// Closes this producer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Consumer
// -----------------------------------------------------------------------------

/// Writes normalization values for a segment.
///
/// Equivalent to `org.apache.lucene.codecs.NormsConsumer`.
pub trait NormsConsumer: fmt::Debug {
    /// Writes normalization values for a field.
    fn add_norms_field(&mut self, field: &FieldInfo, values: &dyn NormsProducer) -> Result<()>;

    /// Merges the norm fields from the readers in `merge_state`.
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

/// Encodes and decodes document normalization values.
///
/// Equivalent to `org.apache.lucene.codecs.NormsFormat`.
pub trait NormsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a consumer to write norms to the index.
    fn norms_consumer(&self, state: &SegmentWriteState) -> Result<Box<dyn NormsConsumer>>;

    /// Returns a producer to read norms from the index.
    fn norms_producer(&self, state: &SegmentReadState) -> Result<Box<dyn NormsProducer>>;
}

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

/// A no-op numeric doc-values iterator that returns no documents.
///
/// This is an alias for [`EmptyNumericDocValues`] from the `index` module so
/// that the norms API uses the same iterator-based [`NumericDocValues`] trait
/// as the rest of the doc-values stack.
pub type EmptyNormsDocValues = EmptyNumericDocValues;

/// A no-op norms producer.
#[derive(Debug, Default, Clone)]
pub struct EmptyNormsProducer;

impl NormsProducer for EmptyNormsProducer {
    fn get_norms(&self, _field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        Ok(Box::new(DocValues::empty_numeric()))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A no-op norms consumer.
#[derive(Debug, Default, Clone)]
pub struct EmptyNormsConsumer;

impl NormsConsumer for EmptyNormsConsumer {
    fn add_norms_field(&mut self, _field: &FieldInfo, _values: &dyn NormsProducer) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A no-op norms format.
#[derive(Debug, Default, Clone)]
pub struct EmptyNormsFormat {
    name: String,
}

impl EmptyNormsFormat {
    /// Creates a new no-op norms format with the given SPI name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl NormsFormat for EmptyNormsFormat {
    fn name(&self) -> &str {
        &self.name
    }

    fn norms_consumer(&self, _state: &SegmentWriteState) -> Result<Box<dyn NormsConsumer>> {
        Ok(Box::new(EmptyNormsConsumer))
    }

    fn norms_producer(&self, _state: &SegmentReadState) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(EmptyNormsProducer))
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::BufferedUpdates;
    use crate::index::FieldInfos;

    #[test]
    fn empty_norms_doc_values_is_exhausted() {
        use crate::index::doc_values::DocValuesIterator;
        use crate::search::DocIdSetIterator;

        let mut norms = EmptyNormsDocValues::default();
        assert_eq!(norms.doc_id(), -1);
        assert_eq!(norms.next_doc().unwrap(), crate::search::NO_MORE_DOCS);
        assert!(!norms.advance_exact(0).unwrap());
        assert_eq!(norms.cost(), 0);
    }

    #[test]
    fn empty_norms_producer_returns_empty_values() {
        use crate::search::DocIdSetIterator;

        let mut producer = EmptyNormsProducer;
        let field = FieldInfo::default();
        let mut values = producer.get_norms(&field).unwrap();
        assert_eq!(values.next_doc().unwrap(), crate::search::NO_MORE_DOCS);
        producer.check_integrity().unwrap();
        let _merge = producer.get_merge_instance().unwrap();
        producer.close().unwrap();
    }

    #[test]
    fn empty_norms_consumer_accepts_field() {
        let mut consumer = EmptyNormsConsumer;
        let producer = EmptyNormsProducer;
        let field = FieldInfo::default();
        consumer.add_norms_field(&field, &producer).unwrap();
        consumer.close().unwrap();
    }

    #[test]
    fn empty_norms_format_name_and_factories() {
        let format = EmptyNormsFormat::new("EmptyNorms");
        assert_eq!(format.name(), "EmptyNorms");

        let dir = crate::store::RamDirectory::default();
        let dir_ref: &dyn crate::store::Directory = &dir;
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let field_infos = FieldInfos::default();
        let segment_info = crate::codecs::tests::test_segment_info("test", 10);
        let write_state = SegmentWriteState::new(
            crate::util::default_info_stream(),
            dir_ref,
            &segment_info,
            &field_infos,
            &BufferedUpdates,
            context,
        );
        let _consumer = format.norms_consumer(&write_state).unwrap();

        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);
        let _producer = format.norms_producer(&read_state).unwrap();
    }
}
