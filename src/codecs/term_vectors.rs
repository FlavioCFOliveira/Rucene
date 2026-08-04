//! Term-vectors format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.TermVectorsFormat`,
//! `TermVectorsReader` and `TermVectorsWriter`.

use std::fmt;

use crate::error::Result;
use crate::store::{Directory, IOContext};
use crate::util::BytesRef;

use super::stub::{FieldInfo, FieldInfos, Fields, SegmentInfo};

/// Controls the format of term vectors.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.TermVectorsFormat`.
pub trait TermVectorsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a [`TermVectorsReader`] to read term vectors.
    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>>;

    /// Returns a [`TermVectorsWriter`] to write term vectors.
    fn vectors_writer(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>>;
}

/// Codec API for reading term vectors.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.TermVectorsReader`.
pub trait TermVectorsReader: Send + Sync + fmt::Debug {
    /// Checks consistency of this reader.
    fn check_integrity(&self) -> Result<()>;

    /// Creates a clone that one caller at a time may use to read term vectors.
    fn clone_reader(&self) -> Box<dyn TermVectorsReader>;

    /// Returns an instance optimized for merging.
    ///
    /// The default implementation returns a clone of `self`.
    fn get_merge_instance(&self) -> Box<dyn TermVectorsReader> {
        self.clone_reader()
    }

    /// Returns term vectors for this document, or `None` if none exist.
    fn get(&self, doc: i32) -> Result<Option<Fields>>;

    /// Optional hint that the given document will be read in the near future.
    fn prefetch(&self, _doc_id: i32) -> Result<()> {
        Ok(())
    }
}

/// Codec API for writing term vectors.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.TermVectorsWriter`.
pub trait TermVectorsWriter: Send + Sync + fmt::Debug {
    /// Called before writing the term vectors of a document.
    fn start_document(&mut self, num_vector_fields: i32) -> Result<()>;

    /// Called after a document and all its fields have been added.
    fn finish_document(&mut self) -> Result<()> {
        Ok(())
    }

    /// Called before writing the terms of a field.
    fn start_field(
        &mut self,
        info: &FieldInfo,
        num_terms: i32,
        positions: bool,
        offsets: bool,
        payloads: bool,
    ) -> Result<()>;

    /// Called after a field and all its terms have been added.
    fn finish_field(&mut self) -> Result<()> {
        Ok(())
    }

    /// Adds a term and its term frequency.
    fn start_term(&mut self, term: &BytesRef, freq: i32) -> Result<()>;

    /// Called after a term and all its positions have been added.
    fn finish_term(&mut self) -> Result<()> {
        Ok(())
    }

    /// Adds a term position and offsets.
    fn add_position(
        &mut self,
        position: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<&BytesRef>,
    ) -> Result<()>;

    /// Called before close, passing the number of documents written.
    fn finish(&mut self, num_docs: i32) -> Result<()>;

    /// Closes the writer and releases any resources.
    fn close(&mut self) -> Result<()>;
}

/// A minimal no-op term-vectors format.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyTermVectorsFormat;

impl TermVectorsFormat for EmptyTermVectorsFormat {
    fn name(&self) -> &str {
        "EmptyTermVectors"
    }

    fn vectors_reader(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _field_infos: &FieldInfos,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(EmptyTermVectorsReader))
    }

    fn vectors_writer(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        Ok(Box::new(EmptyTermVectorsWriter))
    }
}

/// A minimal no-op term-vectors reader.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyTermVectorsReader;

impl TermVectorsReader for EmptyTermVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
        Box::new(*self)
    }

    fn get(&self, _doc: i32) -> Result<Option<Fields>> {
        Ok(None)
    }
}

/// A minimal no-op term-vectors writer.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyTermVectorsWriter;

impl TermVectorsWriter for EmptyTermVectorsWriter {
    fn start_document(&mut self, _num_vector_fields: i32) -> Result<()> {
        Ok(())
    }

    fn start_field(
        &mut self,
        _info: &FieldInfo,
        _num_terms: i32,
        _positions: bool,
        _offsets: bool,
        _payloads: bool,
    ) -> Result<()> {
        Ok(())
    }

    fn start_term(&mut self, _term: &BytesRef, _freq: i32) -> Result<()> {
        Ok(())
    }

    fn add_position(
        &mut self,
        _position: i32,
        _start_offset: i32,
        _end_offset: i32,
        _payload: Option<&BytesRef>,
    ) -> Result<()> {
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
    fn empty_term_vectors_format_name() {
        let format = EmptyTermVectorsFormat;
        assert_eq!(format.name(), "EmptyTermVectors");
    }

    #[test]
    fn empty_term_vectors_reader_methods() {
        let reader = EmptyTermVectorsReader;
        reader.check_integrity().unwrap();
        assert!(reader.get(0).unwrap().is_none());
        let _clone = reader.clone_reader();
    }

    #[test]
    fn empty_term_vectors_writer_methods() {
        let mut writer = EmptyTermVectorsWriter;
        writer.start_document(1).unwrap();
        let info = FieldInfo;
        writer.start_field(&info, 1, true, true, true).unwrap();
        let term = BytesRef::new(b"term".to_vec());
        writer.start_term(&term, 1).unwrap();
        writer.add_position(0, 0, 4, Some(&term)).unwrap();
        writer.finish_term().unwrap();
        writer.finish_field().unwrap();
        writer.finish_document().unwrap();
        writer.finish(1).unwrap();
        writer.close().unwrap();
    }
}
