//! Stored-fields format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.StoredFieldsFormat`,
//! `StoredFieldsReader` and `StoredFieldsWriter`.

use std::fmt;

use crate::error::Result;
use crate::store::{Directory, IOContext};

use super::stub::{FieldInfo, FieldInfos, SegmentInfo, StoredFieldVisitor};

/// Controls the format of stored fields.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.StoredFieldsFormat`.
pub trait StoredFieldsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a [`StoredFieldsReader`] to load stored fields.
    fn fields_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>>;

    /// Returns a [`StoredFieldsWriter`] to write stored fields.
    fn fields_writer(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>>;
}

/// Codec API for reading stored fields.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.StoredFieldsReader`.
pub trait StoredFieldsReader: Send + Sync + fmt::Debug {
    /// Visits the stored fields of the given document.
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()>;

    /// Checks consistency of this reader.
    fn check_integrity(&self) -> Result<()>;

    /// Creates a clone that one caller at a time may use to read stored fields.
    fn clone_reader(&self) -> Box<dyn StoredFieldsReader>;

    /// Returns an instance optimized for merging.
    ///
    /// The default implementation returns a clone of `self`.
    fn get_merge_instance(&self) -> Box<dyn StoredFieldsReader> {
        self.clone_reader()
    }

    /// Optional hint that the given document will be read in the near future.
    fn prefetch(&self, _doc_id: i32) -> Result<()> {
        Ok(())
    }

    /// Closes this reader, releasing all resources.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Codec API for writing stored fields.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.StoredFieldsWriter`.
pub trait StoredFieldsWriter: fmt::Debug {
    /// Called before writing the stored fields of a document.
    fn start_document(&mut self) -> Result<()>;

    /// Called after a document and all its fields have been added.
    fn finish_document(&mut self) -> Result<()> {
        Ok(())
    }

    /// Writes a stored int value.
    fn write_field_i32(&mut self, info: &FieldInfo, value: i32) -> Result<()>;

    /// Writes a stored long value.
    fn write_field_i64(&mut self, info: &FieldInfo, value: i64) -> Result<()>;

    /// Writes a stored float value.
    fn write_field_f32(&mut self, info: &FieldInfo, value: f32) -> Result<()>;

    /// Writes a stored double value.
    fn write_field_f64(&mut self, info: &FieldInfo, value: f64) -> Result<()>;

    /// Writes a stored binary value.
    fn write_field_bytes(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()>;

    /// Writes a stored string value.
    fn write_field_string(&mut self, info: &FieldInfo, value: &str) -> Result<()>;

    /// Called before close, passing the number of documents written.
    fn finish(&mut self, num_docs: i32) -> Result<()>;

    /// Closes the writer and releases any resources.
    fn close(&mut self) -> Result<()>;
}

/// A minimal no-op stored-fields format.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyStoredFieldsFormat;

impl StoredFieldsFormat for EmptyStoredFieldsFormat {
    fn name(&self) -> &str {
        "EmptyStoredFields"
    }

    fn fields_reader(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _field_infos: &FieldInfos,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(EmptyStoredFieldsReader))
    }

    fn fields_writer(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        Ok(Box::new(EmptyStoredFieldsWriter))
    }
}

/// A minimal no-op stored-fields reader.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyStoredFieldsReader;

impl StoredFieldsReader for EmptyStoredFieldsReader {
    fn document(&self, _doc_id: i32, _visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        Ok(())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
        Box::new(*self)
    }
}

/// A minimal no-op stored-fields writer.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyStoredFieldsWriter;

impl StoredFieldsWriter for EmptyStoredFieldsWriter {
    fn start_document(&mut self) -> Result<()> {
        Ok(())
    }

    fn write_field_i32(&mut self, _info: &FieldInfo, _value: i32) -> Result<()> {
        Ok(())
    }

    fn write_field_i64(&mut self, _info: &FieldInfo, _value: i64) -> Result<()> {
        Ok(())
    }

    fn write_field_f32(&mut self, _info: &FieldInfo, _value: f32) -> Result<()> {
        Ok(())
    }

    fn write_field_f64(&mut self, _info: &FieldInfo, _value: f64) -> Result<()> {
        Ok(())
    }

    fn write_field_bytes(&mut self, _info: &FieldInfo, _value: &[u8]) -> Result<()> {
        Ok(())
    }

    fn write_field_string(&mut self, _info: &FieldInfo, _value: &str) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self, _num_docs: i32) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stored_fields_format_name() {
        let format = EmptyStoredFieldsFormat;
        assert_eq!(format.name(), "EmptyStoredFields");
    }

    #[test]
    fn empty_stored_fields_reader_methods() {
        let reader = EmptyStoredFieldsReader;
        reader.check_integrity().unwrap();
        let mut visitor = NullStoredFieldVisitor;
        reader.document(0, &mut visitor).unwrap();
        let _clone = reader.clone_reader();
    }

    #[test]
    fn empty_stored_fields_writer_methods() {
        let mut writer = EmptyStoredFieldsWriter;
        writer.start_document().unwrap();
        let info = FieldInfo::default();
        writer.write_field_i32(&info, 1).unwrap();
        writer.write_field_i64(&info, 2).unwrap();
        writer.write_field_f32(&info, 1.0).unwrap();
        writer.write_field_f64(&info, 2.0).unwrap();
        writer.write_field_bytes(&info, b"x").unwrap();
        writer.write_field_string(&info, "s").unwrap();
        writer.finish_document().unwrap();
        writer.finish(1).unwrap();
        writer.close().unwrap();
    }

    struct NullStoredFieldVisitor;

    impl StoredFieldVisitor for NullStoredFieldVisitor {
        fn needs_field(
            &mut self,
            _info: &FieldInfo,
        ) -> super::super::stub::StoredFieldVisitorStatus {
            super::super::stub::StoredFieldVisitorStatus::Yes
        }
    }
}
