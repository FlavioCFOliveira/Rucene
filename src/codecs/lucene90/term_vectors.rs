//! Lucene 9.0 term-vectors format implementation.
//!
//! Ports `Lucene90TermVectorsFormat` and the underlying
//! `Lucene90CompressingTermVectorsFormat` / `Lucene90CompressingTermVectorsReader` /
//! `Lucene90CompressingTermVectorsWriter` classes from Apache Lucene Core 10.5.0.
//!
//! The format writes three files per segment:
//!
//! * `.tvd` – compressed term-vector chunks.
//! * `.tvx` – index mapping documents to chunks.
//! * `.tvm` – metadata (offsets into `.tvx`, number of chunks, etc.).
//!
//! This module currently provides the file envelope implementation
//! (headers/footers and metadata layout). The full compressed chunk encoding is
//! implemented in the `compressing` module and will be wired in a follow-up task.
//!
//! Lucene Core equivalents:
//! * `org.apache.lucene.codecs.lucene90.Lucene90TermVectorsFormat`
//! * `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsFormat`
//! * `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsReader`
//! * `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter`

#![deny(unsafe_code)]

use std::fmt;

use crate::codecs::codec_util::{
    check_footer, check_index_header, write_footer, write_index_header,
};
use crate::codecs::compressing::CompressionMode;
use crate::codecs::stub::{FieldInfo, FieldInfos, SegmentInfo};
use crate::codecs::term_vectors::{TermVectorsFormat, TermVectorsReader, TermVectorsWriter};
use crate::error::{LuceneError, Result};
use crate::index::segment_file_name;
use crate::store::{Directory, IOContext, IndexInput, IndexOutput};

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

/// Extension of the term-vectors data file (`.tvd`).
pub const VECTORS_EXTENSION: &str = "tvd";
/// Extension of the term-vectors index file (`.tvx`).
pub const INDEX_EXTENSION: &str = "tvx";
/// Extension of the term-vectors metadata file (`.tvm`).
pub const META_EXTENSION: &str = "tvm";

/// Codec name written into the `.tvd` header.
pub const VECTORS_CODEC_NAME: &str = "Lucene90TermVectorsData";
/// Codec name written into the `.tvx` header.
pub const INDEX_CODEC_NAME: &str = "Lucene90TermVectorsIndex";
/// Codec name written into the `.tvm` header.
pub const META_CODEC_NAME: &str = "Lucene90TermVectorsMeta";

/// Initial term-vectors format version.
pub const VERSION_START: i32 = 0;
/// Current term-vectors format version.
pub const VERSION_CURRENT: i32 = VERSION_START;

// -----------------------------------------------------------------------------
// Term vectors format
// -----------------------------------------------------------------------------

/// Lucene 9.0 term-vectors format.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene90.Lucene90TermVectorsFormat`.
#[derive(Debug, Default, Clone)]
pub struct Lucene90TermVectorsFormat;

impl Lucene90TermVectorsFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

impl TermVectorsFormat for Lucene90TermVectorsFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        Lucene90CompressingTermVectorsFormat::new(CompressionMode::FAST, 1 << 12).vectors_reader(
            directory,
            segment_info,
            field_infos,
            context,
        )
    }

    fn vectors_writer(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        Lucene90CompressingTermVectorsFormat::new(CompressionMode::FAST, 1 << 12).vectors_writer(
            directory,
            segment_info,
            context,
        )
    }
}

// -----------------------------------------------------------------------------
// Compressing term vectors format
// -----------------------------------------------------------------------------

/// Compressing term-vectors format used by `Lucene90TermVectorsFormat`.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsFormat`.
#[derive(Debug, Clone)]
pub struct Lucene90CompressingTermVectorsFormat {
    compression_mode: CompressionMode,
    chunk_size: i32,
    version: i32,
}

impl Lucene90CompressingTermVectorsFormat {
    /// Creates the format with the given compression mode and chunk size.
    pub fn new(compression_mode: CompressionMode, chunk_size: i32) -> Self {
        Self {
            compression_mode,
            chunk_size,
            version: VERSION_CURRENT,
        }
    }
}

impl TermVectorsFormat for Lucene90CompressingTermVectorsFormat {
    fn name(&self) -> &str {
        "Lucene90TermVectors"
    }

    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        _field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(Lucene90CompressingTermVectorsReader::new(
            directory,
            segment_info,
            context,
            self.version,
        )?))
    }

    fn vectors_writer(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        Ok(Box::new(Lucene90CompressingTermVectorsWriter::new(
            directory,
            segment_info,
            context,
            self.compression_mode,
            self.chunk_size,
            self.version,
        )?))
    }
}

// -----------------------------------------------------------------------------
// Term vectors writer
// -----------------------------------------------------------------------------

/// Writer for the compressing term-vectors format.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsWriter`.
pub struct Lucene90CompressingTermVectorsWriter {
    segment_suffix: String,
    #[allow(dead_code)]
    compression_mode: CompressionMode,
    #[allow(dead_code)]
    chunk_size: i32,
    version: i32,
    vectors_out: Option<Box<dyn IndexOutput>>,
    index_out: Option<Box<dyn IndexOutput>>,
    meta_out: Option<Box<dyn IndexOutput>>,
    finished: bool,
}

impl Lucene90CompressingTermVectorsWriter {
    /// Creates a new writer.
    pub fn new(
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
        compression_mode: CompressionMode,
        chunk_size: i32,
        version: i32,
    ) -> Result<Self> {
        let vectors_name = segment_file_name(&segment_info.name, "", VECTORS_EXTENSION);
        let mut vectors_out = directory.create_output(&vectors_name, context)?;
        write_index_header(
            vectors_out.as_mut(),
            VECTORS_CODEC_NAME,
            version,
            &segment_info.id(),
            "",
        )?;

        let index_name = segment_file_name(&segment_info.name, "", INDEX_EXTENSION);
        let mut index_out = directory.create_output(&index_name, context)?;
        write_index_header(
            index_out.as_mut(),
            INDEX_CODEC_NAME,
            version,
            &segment_info.id(),
            "",
        )?;

        let meta_name = segment_file_name(&segment_info.name, "", META_EXTENSION);
        let mut meta_out = directory.create_output(&meta_name, context)?;
        write_index_header(
            meta_out.as_mut(),
            META_CODEC_NAME,
            version,
            &segment_info.id(),
            "",
        )?;

        Ok(Self {
            segment_suffix: segment_info.name.clone(),
            compression_mode,
            chunk_size,
            version,
            vectors_out: Some(vectors_out),
            index_out: Some(index_out),
            meta_out: Some(meta_out),
            finished: false,
        })
    }

    fn write_footer_and_close(&mut self) -> Result<()> {
        let mut vectors_out = self.vectors_out.take().unwrap();
        let mut index_out = self.index_out.take().unwrap();
        let mut meta_out = self.meta_out.take().unwrap();

        write_footer(vectors_out.as_mut())?;
        write_footer(index_out.as_mut())?;
        write_footer(meta_out.as_mut())?;

        vectors_out.close()?;
        index_out.close()?;
        meta_out.close()?;
        Ok(())
    }
}

impl fmt::Debug for Lucene90CompressingTermVectorsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90CompressingTermVectorsWriter")
            .field("segment_suffix", &self.segment_suffix)
            .field("version", &self.version)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl TermVectorsWriter for Lucene90CompressingTermVectorsWriter {
    fn start_document(&mut self, _num_vector_fields: i32) -> Result<()> {
        Ok(())
    }

    fn finish_document(&mut self) -> Result<()> {
        Ok(())
    }

    fn start_field(
        &mut self,
        _field_info: &FieldInfo,
        _num_terms: i32,
        _positions: bool,
        _offsets: bool,
        _payloads: bool,
    ) -> Result<()> {
        Ok(())
    }

    fn finish_field(&mut self) -> Result<()> {
        Ok(())
    }

    fn start_term(&mut self, _term: &crate::util::BytesRef, _freq: i32) -> Result<()> {
        Ok(())
    }

    fn add_position(
        &mut self,
        _position: i32,
        _start_offset: i32,
        _end_offset: i32,
        _payload: Option<&crate::util::BytesRef>,
    ) -> Result<()> {
        Ok(())
    }

    fn finish_term(&mut self) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self, _num_docs: i32) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "Lucene90CompressingTermVectorsWriter already finished".to_string(),
            ));
        }
        self.finished = true;
        self.write_footer_and_close()
    }

    fn close(&mut self) -> Result<()> {
        if let Some(mut out) = self.vectors_out.take() {
            out.close()?;
        }
        if let Some(mut out) = self.index_out.take() {
            out.close()?;
        }
        if let Some(mut out) = self.meta_out.take() {
            out.close()?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Term vectors reader
// -----------------------------------------------------------------------------

/// Reader for the compressing term-vectors format.
///
/// Lucene Core equivalent:
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsReader`.
#[derive(Debug, Default, Clone)]
pub struct Lucene90CompressingTermVectorsReader;

impl Lucene90CompressingTermVectorsReader {
    /// Creates a new reader.
    pub fn new(
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        _context: &dyn IOContext,
        version: i32,
    ) -> Result<Self> {
        let vectors_name = segment_file_name(&segment_info.name, "", VECTORS_EXTENSION);
        let mut vectors_in = directory.open_checksum_input(&vectors_name)?;
        check_index_header(
            &mut *vectors_in,
            VECTORS_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &segment_info.id(),
            "",
        )?;
        check_footer(&mut *vectors_in)?;
        vectors_in.close()?;

        let index_name = segment_file_name(&segment_info.name, "", INDEX_EXTENSION);
        let mut index_in = directory.open_checksum_input(&index_name)?;
        check_index_header(
            &mut *index_in,
            INDEX_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &segment_info.id(),
            "",
        )?;
        check_footer(&mut *index_in)?;
        index_in.close()?;

        let meta_name = segment_file_name(&segment_info.name, "", META_EXTENSION);
        let mut meta_in = directory.open_checksum_input(&meta_name)?;
        check_index_header(
            &mut *meta_in,
            META_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &segment_info.id(),
            "",
        )?;
        check_footer(&mut *meta_in)?;
        meta_in.close()?;

        let _ = version;
        Ok(Self)
    }
}

impl TermVectorsReader for Lucene90CompressingTermVectorsReader {
    fn get(&self, _doc: i32) -> Result<Option<crate::codecs::stub::Fields>> {
        Err(LuceneError::UnsupportedOperation(
            "Lucene90CompressingTermVectorsReader skeleton cannot read vectors".to_string(),
        ))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn TermVectorsReader> {
        Box::new(self.clone())
    }

    fn get_merge_instance(&self) -> Box<dyn TermVectorsReader> {
        Box::new(self.clone())
    }
}
