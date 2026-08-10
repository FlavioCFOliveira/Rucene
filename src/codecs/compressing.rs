//! Generic compression engine for stored-fields and term-vectors formats.
//!
//! Ported from `org.apache.lucene.codecs.compressing`. This module provides
//! [`CompressionMode`], [`Compressor`], [`Decompressor`] and [`MatchingReaders`],
//! the shared engine used by `Lucene90CompressingStoredFieldsFormat` and
//! `Lucene90CompressingTermVectorsFormat`.

#![deny(unsafe_code)]

use std::io::{Read, Write};

use crate::codecs::MergeState;
use crate::error::{LuceneError, Result};
use crate::store::{ByteBuffersDataOutput, DataInput, DataOutput};
use crate::util::compress::{FastCompressionHashTable, HighCompressionHashTable, Lz4};
use crate::util::BytesRef;

/// A compression mode describing the speed/ratio trade-off.
///
/// Equivalent to `org.apache.lucene.codecs.compressing.CompressionMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum CompressionMode {
    /// LZ4 fast compressor with LZ4 decompressor.
    FAST,
    /// Deflate compressor with Deflate decompressor.
    HIGH_COMPRESSION,
    /// LZ4 high-compression compressor with LZ4 decompressor.
    FAST_DECOMPRESSION,
}

impl CompressionMode {
    /// Creates a new compressor for this mode.
    pub fn new_compressor(self) -> Box<dyn Compressor> {
        match self {
            Self::FAST => Box::new(Lz4FastCompressor::new()),
            Self::HIGH_COMPRESSION => Box::new(DeflateCompressor::new(6)),
            Self::FAST_DECOMPRESSION => Box::new(Lz4HighCompressor::new()),
        }
    }

    /// Creates a new decompressor for this mode.
    pub fn new_decompressor(self) -> Box<dyn Decompressor> {
        match self {
            Self::FAST | Self::FAST_DECOMPRESSION => Box::new(Lz4Decompressor),
            Self::HIGH_COMPRESSION => Box::new(DeflateDecompressor::new()),
        }
    }
}

impl std::fmt::Display for CompressionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FAST => write!(f, "FAST"),
            Self::HIGH_COMPRESSION => write!(f, "HIGH_COMPRESSION"),
            Self::FAST_DECOMPRESSION => write!(f, "FAST_DECOMPRESSION"),
        }
    }
}

/// Compresses a stream of bytes.
///
/// Equivalent to `org.apache.lucene.codecs.compressing.Compressor`.
pub trait Compressor: Send + Sync + std::fmt::Debug {
    /// Compresses the content of `input` into `out`.
    fn compress(&mut self, input: &ByteBuffersDataOutput, out: &mut dyn DataOutput) -> Result<()>;

    /// Closes this compressor, releasing any native resources.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Decompresses a stream of bytes.
///
/// Equivalent to `org.apache.lucene.codecs.compressing.Decompressor`.
pub trait Decompressor: Send + Sync + std::fmt::Debug {
    /// Decompresses bytes from `input` into `bytes`.
    ///
    /// Only the slice `[offset, offset + length)` of the original (uncompressed)
    /// stream is exposed. The implementation must ensure that `bytes.length`
    /// equals `length` after the call.
    fn decompress(
        &mut self,
        input: &mut dyn DataInput,
        original_length: usize,
        offset: usize,
        length: usize,
        bytes: &mut BytesRef,
    ) -> Result<()>;
}

// -----------------------------------------------------------------------------
// LZ4
// -----------------------------------------------------------------------------

#[derive(Default)]
struct Lz4FastCompressor {
    ht: FastCompressionHashTable,
}

impl std::fmt::Debug for Lz4FastCompressor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lz4FastCompressor").finish_non_exhaustive()
    }
}

impl Lz4FastCompressor {
    fn new() -> Self {
        Self::default()
    }
}

impl Compressor for Lz4FastCompressor {
    fn compress(&mut self, input: &ByteBuffersDataOutput, out: &mut dyn DataOutput) -> Result<()> {
        let len = input.size();
        let bytes = input.to_array_copy();
        Lz4::compress(&bytes, 0, len, out, &mut self.ht)
    }
}

#[derive(Default)]
struct Lz4HighCompressor {
    ht: HighCompressionHashTable,
}

impl std::fmt::Debug for Lz4HighCompressor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lz4HighCompressor").finish_non_exhaustive()
    }
}

impl Lz4HighCompressor {
    fn new() -> Self {
        Self::default()
    }
}

impl Compressor for Lz4HighCompressor {
    fn compress(&mut self, input: &ByteBuffersDataOutput, out: &mut dyn DataOutput) -> Result<()> {
        let len = input.size();
        let bytes = input.to_array_copy();
        Lz4::compress(&bytes, 0, len, out, &mut self.ht)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Lz4Decompressor;

impl Decompressor for Lz4Decompressor {
    fn decompress(
        &mut self,
        input: &mut dyn DataInput,
        original_length: usize,
        offset: usize,
        length: usize,
        bytes: &mut BytesRef,
    ) -> Result<()> {
        if offset + length > original_length {
            return Err(LuceneError::IllegalArgument(format!(
                "offset + length ({offset} + {length}) > originalLength ({original_length})"
            )));
        }
        // Add 7 padding bytes; not required but may help decompression speed.
        let buffer_len = original_length + 7;
        if bytes.bytes.len() < buffer_len {
            bytes.bytes = vec![0; buffer_len];
        }
        let decompressed_length = Lz4::decompress(input, offset + length, &mut bytes.bytes, 0)?;
        if decompressed_length > original_length {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("corrupted: lengths mismatch: {decompressed_length} > {original_length}"),
            )));
        }
        bytes.offset = offset;
        bytes.length = length;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Deflate
// -----------------------------------------------------------------------------

#[derive(Debug)]
struct DeflateCompressor {
    level: i32,
}

impl DeflateCompressor {
    fn new(level: i32) -> Self {
        Self { level }
    }
}

impl Compressor for DeflateCompressor {
    fn compress(&mut self, input: &ByteBuffersDataOutput, out: &mut dyn DataOutput) -> Result<()> {
        let bytes = input.to_array_copy();

        let mut encoder = flate2::write::ZlibEncoder::new(
            Vec::new(),
            flate2::Compression::new(self.level as u32),
        );
        encoder.write_all(&bytes)?;
        let compressed = encoder.finish().map_err(LuceneError::Io)?;

        out.write_v_int(compressed.len() as i32)?;
        out.write_bytes(&compressed, 0, compressed.len())?;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct DeflateDecompressor {
    compressed: Vec<u8>,
}

impl DeflateDecompressor {
    fn new() -> Self {
        Self::default()
    }
}

impl Decompressor for DeflateDecompressor {
    fn decompress(
        &mut self,
        input: &mut dyn DataInput,
        original_length: usize,
        offset: usize,
        length: usize,
        bytes: &mut BytesRef,
    ) -> Result<()> {
        if offset + length > original_length {
            return Err(LuceneError::IllegalArgument(format!(
                "offset + length ({offset} + {length}) > originalLength ({original_length})"
            )));
        }
        if length == 0 {
            bytes.offset = 0;
            bytes.length = 0;
            return Ok(());
        }
        let compressed_length = input.read_v_int()? as usize;
        self.compressed.resize(compressed_length, 0);
        input.read_bytes(&mut self.compressed, 0, compressed_length)?;

        let mut decoder = flate2::read::ZlibDecoder::new(&self.compressed[..]);
        let mut decompressed = vec![0; original_length];
        decoder
            .read_exact(&mut decompressed)
            .map_err(LuceneError::Io)?;

        if decompressed.len() != original_length {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "lengths mismatch: {} != {original_length}",
                    decompressed.len()
                ),
            )));
        }

        if bytes.bytes.len() < original_length {
            bytes.bytes = vec![0; original_length];
        }
        bytes.bytes[..original_length].copy_from_slice(&decompressed);
        bytes.offset = offset;
        bytes.length = length;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// MatchingReaders
// -----------------------------------------------------------------------------

/// Computes which source segments have identical field name-to-number mappings,
/// allowing stored fields and term vectors to be bulk-merged.
///
/// Equivalent to `org.apache.lucene.codecs.compressing.MatchingReaders`.
#[derive(Debug, Clone)]
pub struct MatchingReaders {
    /// Per-reader flag: `true` if the reader's field mapping matches the merged
    /// field infos.
    pub matching_readers: Vec<bool>,
    /// Number of matching readers.
    pub count: i32,
}

impl MatchingReaders {
    /// Computes matching readers from a merge state.
    pub fn new(merge_state: &MergeState) -> Self {
        let num_readers = merge_state.max_docs.len();
        let mut matching_readers = vec![false; num_readers];
        let mut matched_count: i32 = 0;

        'next_reader: for (i, matched) in matching_readers.iter_mut().enumerate() {
            let field_infos = &merge_state.field_infos[i];
            for fi in field_infos.iter() {
                let other = merge_state
                    .merge_field_infos
                    .field_info_by_number(fi.number);
                if other.map(|o| o.name != fi.name).unwrap_or(true) {
                    continue 'next_reader;
                }
            }
            *matched = true;
            matched_count += 1;
        }

        Self {
            matching_readers,
            count: matched_count,
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::{FieldInfo, FieldInfos};
    use crate::codecs::MergeState;
    use crate::store::ByteArrayDataInput;
    use crate::store::ByteArrayDataOutput;

    fn round_trip(mode: CompressionMode, data: &[u8], offset: usize, length: usize) {
        let mut compressor = mode.new_compressor();
        let mut output = ByteArrayDataOutput::new();
        let mut bbo = ByteBuffersDataOutput::with_expected_size(data.len());
        bbo.write_bytes(data, 0, data.len()).unwrap();
        compressor.compress(&bbo, &mut output).unwrap();
        compressor.close().unwrap();

        let compressed = output.into_inner();
        let mut input = ByteArrayDataInput::new(compressed);
        let mut decompressor = mode.new_decompressor();
        let mut bytes = BytesRef::default();
        decompressor
            .decompress(&mut input, data.len(), offset, length, &mut bytes)
            .unwrap();

        assert_eq!(bytes.length, length);
        assert_eq!(bytes.slice(), &data[offset..offset + length]);
    }

    #[test]
    fn lz4_fast_round_trips_random_data() {
        let data: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
        round_trip(CompressionMode::FAST, &data, 0, data.len());
        round_trip(CompressionMode::FAST, &data, 100, 200);
    }

    #[test]
    fn lz4_fast_decompresses_slice() {
        let data = b"the quick brown fox jumps over the lazy dog";
        round_trip(CompressionMode::FAST, data, 4, 15);
    }

    #[test]
    fn fast_decompression_round_trips() {
        let data = vec![0u8; 512];
        round_trip(CompressionMode::FAST_DECOMPRESSION, &data, 0, data.len());

        let mut varied = vec![0u8; 256];
        for (i, b) in varied.iter_mut().enumerate() {
            *b = (i * 7) as u8;
        }
        round_trip(
            CompressionMode::FAST_DECOMPRESSION,
            &varied,
            0,
            varied.len(),
        );
    }

    #[test]
    fn deflate_high_compression_round_trips() {
        let data: Vec<u8> = (0..2048).map(|i| (i % 127) as u8).collect();
        round_trip(CompressionMode::HIGH_COMPRESSION, &data, 0, data.len());
        round_trip(CompressionMode::HIGH_COMPRESSION, &data, 500, 1000);
    }

    #[test]
    fn deflate_empty_payload() {
        let data: Vec<u8> = vec![];
        round_trip(CompressionMode::HIGH_COMPRESSION, &data, 0, 0);
    }

    #[test]
    fn matching_readers_detects_identical_mappings() {
        let fi1 = FieldInfo::new("a", 0);
        let fi2 = FieldInfo::new("b", 1);
        let field_infos_a = FieldInfos::new(vec![fi1.clone(), fi2.clone()]).unwrap();
        let field_infos_b = FieldInfos::new(vec![fi1.clone(), fi2.clone()]).unwrap();
        let merged = FieldInfos::new(vec![fi1, fi2]).unwrap();

        let mut merge_state = MergeState::new(vec![], vec![0, 0]);
        merge_state.field_infos = vec![field_infos_a, field_infos_b];
        merge_state.merge_field_infos = merged;

        let matching = MatchingReaders::new(&merge_state);
        assert_eq!(matching.count, 2);
        assert!(matching.matching_readers.iter().all(|b| *b));
    }

    #[test]
    fn matching_readers_detects_divergent_name() {
        let fi_a0 = FieldInfo::new("a", 0);
        let fi_b0 = FieldInfo::new("x", 0);
        let merged = FieldInfos::new(vec![FieldInfo::new("a", 0)]).unwrap();

        let mut merge_state = MergeState::new(vec![], vec![0]);
        merge_state.field_infos = vec![FieldInfos::new(vec![fi_b0]).unwrap()];
        merge_state.merge_field_infos = merged;

        let matching = MatchingReaders::new(&merge_state);
        assert_eq!(matching.count, 0);
        assert!(!matching.matching_readers[0]);
    }
}
