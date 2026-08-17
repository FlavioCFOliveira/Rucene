//! Points format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.PointsFormat`, `PointsReader` and
//! `PointsWriter`.
//!
//! These traits provide the abstract read/write API for indexed point values.
//! Concrete codecs implement the format-specific encoding underneath this API.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::Result;

pub use crate::util::bkd::{IntersectVisitor, Relation};

use super::postings::MergeState;
use super::state::{SegmentReadState, SegmentWriteState};
use super::stub::FieldInfo;

// -----------------------------------------------------------------------------
// Doc-values visitor
// -----------------------------------------------------------------------------

/// Visitor that receives every indexed point together with its document id.
///
/// This is the codec-level counterpart to Java's
/// `PointValues.IntersectVisitor` used while writing a field: the writer
/// needs to consume all `(doc_id, packed_value)` pairs, not only the ones
/// matching a query.
pub trait DocValuesVisitor {
    /// Called once for every indexed point value.
    fn visit(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()>;
}

impl<F> DocValuesVisitor for F
where
    F: FnMut(i32, &[u8]) -> Result<()> + Send + Sync,
{
    fn visit(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        (self)(doc_id, packed_value)
    }
}

// -----------------------------------------------------------------------------
// Point values
// -----------------------------------------------------------------------------

/// Access to indexed point values for a single field.
///
/// Equivalent to `org.apache.lucene.index.PointValues`.
pub trait PointValues: Send + Sync {
    /// Returns the number of bytes in each dimension's values.
    fn bytes_per_dimension(&self) -> i32;

    /// Returns the number of dimensions.
    fn num_dimensions(&self) -> i32;

    /// Returns the number of dimensions used for the index key.
    fn num_index_dimensions(&self) -> i32;

    /// Returns the total number of indexed points.
    fn size(&self) -> i64;

    /// Returns the number of documents that have at least one point.
    fn doc_count(&self) -> i32;

    /// Returns the minimum packed value.
    fn min_packed_value(&self) -> Result<Vec<u8>>;

    /// Returns the maximum packed value.
    fn max_packed_value(&self) -> Result<Vec<u8>>;

    /// Iterates every indexed point value for this field.
    ///
    /// The default implementation is a no-op so that existing implementors keep
    /// compiling. Concrete sources that can enumerate their values (for
    /// example a BKD-backed reader) must override this method.
    fn visit_doc_values(&self, _visitor: &mut dyn DocValuesVisitor) -> Result<()> {
        Ok(())
    }

    /// Finds all matching points for the provided intersection visitor.
    ///
    /// The default implementation enumerates every stored point and invokes the
    /// visitor directly. BKD-backed implementations override this to use the
    /// tree index for efficient range/intersection queries.
    fn intersect(&self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        struct IntersectDocValuesVisitor<'a> {
            visitor: &'a mut dyn IntersectVisitor,
        }

        impl<'a> DocValuesVisitor for IntersectDocValuesVisitor<'a> {
            fn visit(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
                self.visitor.visit_point(doc_id, packed_value);
                Ok(())
            }
        }

        self.visit_doc_values(&mut IntersectDocValuesVisitor { visitor })
    }
}

// -----------------------------------------------------------------------------
// Reader
// -----------------------------------------------------------------------------

/// Reads point values from an index.
///
/// Equivalent to `org.apache.lucene.codecs.PointsReader`.
pub trait PointsReader: Send + Sync + fmt::Debug {
    /// Checks consistency of this reader.
    fn check_integrity(&self) -> Result<()>;

    /// Returns the point values for the given field.
    fn get_values(&self, field: &str) -> Result<Box<dyn PointValues>>;

    /// Returns an instance optimized for merging.
    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>>;

    /// Closes this reader, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Writer
// -----------------------------------------------------------------------------

/// Writes point values to an index.
///
/// Equivalent to `org.apache.lucene.codecs.PointsWriter`.
pub trait PointsWriter: Send + Sync + fmt::Debug {
    /// Writes all values contained in the provided reader for one field.
    fn write_field(&mut self, field_info: &FieldInfo, values: &dyn PointsReader) -> Result<()>;

    /// Merges the values for a single field.
    ///
    /// The default implementation is a no-op; concrete formats override it with
    /// format-specific merge logic.
    fn merge_one_field(
        &mut self,
        _merge_state: &MergeState,
        _field_info: &FieldInfo,
    ) -> Result<()> {
        Ok(())
    }

    /// Merges the segment point values from the readers in `merge_state`.
    ///
    /// The default implementation is a no-op; concrete formats override it with
    /// format-specific merge logic.
    fn merge(&mut self, _merge_state: &MergeState) -> Result<()> {
        Ok(())
    }

    /// Called once at the end before close.
    fn finish(&mut self) -> Result<()>;

    /// Closes this writer, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Format
// -----------------------------------------------------------------------------

/// Encodes and decodes point values.
///
/// Equivalent to `org.apache.lucene.codecs.PointsFormat`.
pub trait PointsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a writer to write points to the index.
    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>>;

    /// Returns a reader to read points from the index.
    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>>;
}

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

/// A no-op point-values instance that reports zero dimensions and no values.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyPointValues;

impl PointValues for EmptyPointValues {
    fn bytes_per_dimension(&self) -> i32 {
        0
    }

    fn num_dimensions(&self) -> i32 {
        0
    }

    fn num_index_dimensions(&self) -> i32 {
        0
    }

    fn size(&self) -> i64 {
        0
    }

    fn doc_count(&self) -> i32 {
        0
    }

    fn min_packed_value(&self) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    fn max_packed_value(&self) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

/// A no-op points reader.
#[derive(Debug, Default, Clone)]
pub struct EmptyPointsReader;

impl PointsReader for EmptyPointsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_values(&self, _field: &str) -> Result<Box<dyn PointValues>> {
        Ok(Box::new(EmptyPointValues))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A no-op points writer.
#[derive(Debug, Default, Clone)]
pub struct EmptyPointsWriter;

impl PointsWriter for EmptyPointsWriter {
    fn write_field(&mut self, _field_info: &FieldInfo, _values: &dyn PointsReader) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A no-op points format.
#[derive(Debug, Default, Clone)]
pub struct EmptyPointsFormat {
    name: String,
}

impl EmptyPointsFormat {
    /// Creates a new no-op points format with the given SPI name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl PointsFormat for EmptyPointsFormat {
    fn name(&self) -> &str {
        &self.name
    }

    fn fields_writer(&self, _state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(EmptyPointsWriter))
    }

    fn fields_reader(&self, _state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(EmptyPointsReader))
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
    fn empty_point_values_reports_zero() {
        let values = EmptyPointValues;
        assert_eq!(values.bytes_per_dimension(), 0);
        assert_eq!(values.num_dimensions(), 0);
        assert_eq!(values.num_index_dimensions(), 0);
        assert_eq!(values.size(), 0);
        assert_eq!(values.doc_count(), 0);
        assert!(values.min_packed_value().unwrap().is_empty());
        assert!(values.max_packed_value().unwrap().is_empty());
    }

    #[test]
    fn empty_points_reader_returns_empty_values() {
        let mut reader = EmptyPointsReader;
        assert_eq!(reader.get_values("field").unwrap().size(), 0);
        reader.check_integrity().unwrap();
        let _merge = reader.get_merge_instance().unwrap();
        reader.close().unwrap();
    }

    #[test]
    fn empty_points_writer_accepts_field() {
        let mut writer = EmptyPointsWriter;
        let reader = EmptyPointsReader;
        let field = FieldInfo::default();
        writer.write_field(&field, &reader).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn empty_points_format_name_and_factories() {
        let format = EmptyPointsFormat::new("EmptyPoints");
        assert_eq!(format.name(), "EmptyPoints");

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
        let _writer = format.fields_writer(&write_state).unwrap();

        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);
        let _reader = format.fields_reader(&read_state).unwrap();
    }
}
