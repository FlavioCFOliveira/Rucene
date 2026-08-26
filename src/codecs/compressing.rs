//! Generic compression engine for stored-fields and term-vectors formats.
//!
//! Ported from `org.apache.lucene.codecs.compressing`. This module provides
//! [`CompressionMode`], [`Compressor`], [`Decompressor`] and [`MatchingReaders`],
//! the shared engine used by `Lucene90CompressingStoredFieldsFormat` and
//! `Lucene90CompressingTermVectorsFormat`.
//!
//! [`CompressionMode`] also carries the mode Lucene declares outside that
//! package, `org.apache.lucene.codecs.lucene90.LZ4WithPresetDictCompressionMode`
//! ([`CompressionMode::LZ4_WITH_PRESET_DICT`]), because a Rust enum cannot be
//! extended from another module the way an abstract Java class can be
//! subclassed.

#![deny(unsafe_code)]

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

use crate::codecs::MergeState;
use crate::error::{LuceneError, Result};
use crate::store::{ByteBuffersDataOutput, DataInput, DataOutput};
use crate::util::compress::{
    FastCompressionHashTable, HighCompressionHashTable, Lz4, LZ4_MAX_DISTANCE,
};
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
    /// Raw deflate over a preset dictionary, splitting each chunk into sub
    /// blocks.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.DeflateWithPresetDictCompressionMode`,
    /// which is what `Lucene90StoredFieldsFormat` uses for
    /// [`Mode::BestCompression`](crate::codecs::lucene90::stored_fields::Mode).
    /// It is **not** interchangeable with [`Self::HIGH_COMPRESSION`].
    DEFLATE_WITH_PRESET_DICT,
    /// LZ4 over a preset dictionary, splitting each chunk into sub blocks.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.LZ4WithPresetDictCompressionMode`,
    /// which is what `Lucene90StoredFieldsFormat` uses for
    /// [`Mode::BestSpeed`](crate::codecs::lucene90::stored_fields::Mode). It is
    /// **not** interchangeable with [`Self::FAST`]: the bytes it writes have a
    /// different framing, so a segment written with one cannot be read with the
    /// other.
    LZ4_WITH_PRESET_DICT,
}

impl CompressionMode {
    /// Creates a new compressor for this mode.
    pub fn new_compressor(self) -> Box<dyn Compressor> {
        match self {
            Self::FAST => Box::new(Lz4FastCompressor::new()),
            Self::HIGH_COMPRESSION => Box::new(DeflateCompressor::new(6)),
            Self::FAST_DECOMPRESSION => Box::new(Lz4HighCompressor::new()),
            Self::LZ4_WITH_PRESET_DICT => Box::new(Lz4WithPresetDictCompressor::new()),
            Self::DEFLATE_WITH_PRESET_DICT => Box::new(DeflateWithPresetDictCompressor::new(
                DEFLATE_PRESET_DICT_LEVEL,
            )),
        }
    }

    /// Creates a new decompressor for this mode.
    pub fn new_decompressor(self) -> Box<dyn Decompressor> {
        match self {
            Self::FAST | Self::FAST_DECOMPRESSION => Box::new(Lz4Decompressor),
            Self::HIGH_COMPRESSION => Box::new(DeflateDecompressor::new()),
            Self::LZ4_WITH_PRESET_DICT => Box::new(Lz4WithPresetDictDecompressor::new()),
            Self::DEFLATE_WITH_PRESET_DICT => Box::new(DeflateWithPresetDictDecompressor::new()),
        }
    }
}

impl std::fmt::Display for CompressionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FAST => write!(f, "FAST"),
            Self::HIGH_COMPRESSION => write!(f, "HIGH_COMPRESSION"),
            Self::FAST_DECOMPRESSION => write!(f, "FAST_DECOMPRESSION"),
            // `LZ4WithPresetDictCompressionMode.toString()` returns BEST_SPEED.
            Self::LZ4_WITH_PRESET_DICT => write!(f, "BEST_SPEED"),
            // `DeflateWithPresetDictCompressionMode.toString()`.
            Self::DEFLATE_WITH_PRESET_DICT => write!(f, "BEST_COMPRESSION"),
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
// LZ4 with a preset dictionary
// -----------------------------------------------------------------------------

/// Number of sub blocks a chunk is split into.
///
/// Equivalent to `LZ4WithPresetDictCompressionMode.NUM_SUB_BLOCKS`.
const NUM_SUB_BLOCKS: usize = 10;

/// The dictionary is about this many times smaller than a sub block.
///
/// Equivalent to `LZ4WithPresetDictCompressionMode.DICT_SIZE_FACTOR`.
const DICT_SIZE_FACTOR: usize = 2;

/// Compresses a chunk as a dictionary plus [`NUM_SUB_BLOCKS`] sub blocks, each
/// LZ4-compressed against that dictionary.
///
/// Equivalent to `LZ4WithPresetDictCompressionMode.LZ4WithPresetDictCompressor`.
///
/// The layout it writes is: the dictionary length, the block length, one
/// `vInt` per compressed block (the dictionary first), and finally every
/// compressed block back to back. Writing all the lengths before all the data
/// is what lets the decompressor skip straight to the blocks it needs.
struct Lz4WithPresetDictCompressor {
    compressed: ByteBuffersDataOutput,
    hash_table: FastCompressionHashTable,
    buffer: Vec<u8>,
}

impl std::fmt::Debug for Lz4WithPresetDictCompressor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lz4WithPresetDictCompressor")
            .finish_non_exhaustive()
    }
}

impl Lz4WithPresetDictCompressor {
    fn new() -> Self {
        Self {
            compressed: ByteBuffersDataOutput::new_resettable_instance(),
            hash_table: FastCompressionHashTable::new(),
            buffer: Vec::new(),
        }
    }

    /// Compresses `buffer[dict_len..dict_len + len]` against
    /// `buffer[0..dict_len]` and writes the resulting length to `out`.
    fn do_compress(&mut self, dict_len: usize, len: usize, out: &mut dyn DataOutput) -> Result<()> {
        let before = self.compressed.size();
        Lz4::compress_with_dictionary(
            &self.buffer,
            0,
            dict_len,
            len,
            &mut self.compressed,
            &mut self.hash_table,
        )?;
        let written = self.compressed.size() - before;
        let written = i32::try_from(written).map_err(|_| {
            LuceneError::IllegalState(format!(
                "compressed block does not fit in an int: {written}"
            ))
        })?;
        out.write_v_int(written)
    }
}

impl Compressor for Lz4WithPresetDictCompressor {
    fn compress(&mut self, input: &ByteBuffersDataOutput, out: &mut dyn DataOutput) -> Result<()> {
        let bytes = input.to_array_copy();
        let len = bytes.len();
        let dict_length =
            std::cmp::min(LZ4_MAX_DISTANCE, len / (NUM_SUB_BLOCKS * DICT_SIZE_FACTOR));
        let block_length = (len - dict_length).div_ceil(NUM_SUB_BLOCKS);
        self.buffer.clear();
        self.buffer.resize(dict_length + block_length, 0);
        out.write_v_int(i32::try_from(dict_length).map_err(|_| {
            LuceneError::IllegalState(format!("dictionary too large: {dict_length}"))
        })?)?;
        out.write_v_int(
            i32::try_from(block_length).map_err(|_| {
                LuceneError::IllegalState(format!("block too large: {block_length}"))
            })?,
        )?;

        self.compressed.reset();
        // The dictionary first, compressed against nothing.
        self.buffer[..dict_length].copy_from_slice(&bytes[..dict_length]);
        self.do_compress(0, dict_length, out)?;

        // Then every sub block, compressed against the dictionary.
        let mut start = dict_length;
        while start < len {
            let l = std::cmp::min(block_length, len - start);
            self.buffer[dict_length..dict_length + l].copy_from_slice(&bytes[start..start + l]);
            self.do_compress(dict_length, l, out)?;
            start += block_length;
        }

        // Only the lengths have been written so far; the data follows.
        self.compressed.copy_to(out)
    }
}

/// Reads back what [`Lz4WithPresetDictCompressor`] wrote, decompressing only
/// the sub blocks that intersect the requested interval.
///
/// Equivalent to
/// `LZ4WithPresetDictCompressionMode.LZ4WithPresetDictDecompressor`.
#[derive(Debug, Default)]
struct Lz4WithPresetDictDecompressor {
    compressed_lengths: Vec<i32>,
    buffer: Vec<u8>,
}

impl Lz4WithPresetDictDecompressor {
    fn new() -> Self {
        Self::default()
    }

    /// Reads the per-block compressed lengths and returns how many blocks the
    /// chunk holds.
    fn read_compressed_lengths(
        &mut self,
        input: &mut dyn DataInput,
        original_length: usize,
        dict_length: usize,
        block_length: usize,
    ) -> Result<usize> {
        input.read_v_int()?; // compressed length of the dictionary, unused
        if block_length == 0 {
            return Ok(0);
        }
        let mut total_length = dict_length;
        let mut count = 0;
        let capacity = original_length / block_length + 1;
        if self.compressed_lengths.len() < capacity {
            self.compressed_lengths.resize(capacity, 0);
        }
        while total_length < original_length {
            if count >= self.compressed_lengths.len() {
                return Err(LuceneError::CorruptIndex(
                    "more compressed sub blocks than the chunk can hold".to_string(),
                ));
            }
            self.compressed_lengths[count] = input.read_v_int()?;
            count += 1;
            total_length += block_length;
        }
        Ok(count)
    }
}

impl Decompressor for Lz4WithPresetDictDecompressor {
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

        let dict_length = usize::try_from(input.read_v_int()?)
            .map_err(|_| LuceneError::CorruptIndex("negative dictionary length".to_string()))?;
        let block_length = usize::try_from(input.read_v_int()?)
            .map_err(|_| LuceneError::CorruptIndex("negative block length".to_string()))?;

        let num_blocks =
            self.read_compressed_lengths(input, original_length, dict_length, block_length)?;

        if self.buffer.len() < dict_length + block_length {
            self.buffer.resize(dict_length + block_length, 0);
        }
        bytes.length = 0;

        // The dictionary is needed by every block, so it is always read.
        if Lz4::decompress(input, dict_length, &mut self.buffer, 0)? != dict_length {
            return Err(LuceneError::CorruptIndex("Illegal dict length".to_string()));
        }

        let mut offset_in_block = dict_length;
        let mut offset_in_bytes_ref = offset;
        if offset >= dict_length {
            offset_in_bytes_ref -= dict_length;
            // Skip the blocks that end before the interval starts.
            let mut bytes_to_skip: i64 = 0;
            let mut index = 0;
            while index < num_blocks && offset_in_block + block_length < offset {
                bytes_to_skip += i64::from(self.compressed_lengths[index]);
                offset_in_block += block_length;
                offset_in_bytes_ref -= block_length;
                index += 1;
            }
            input.skip_bytes(bytes_to_skip)?;
        } else {
            // The dictionary itself holds bytes the caller asked for.
            if bytes.bytes.len() < dict_length {
                bytes.bytes.resize(dict_length, 0);
            }
            bytes.bytes[..dict_length].copy_from_slice(&self.buffer[..dict_length]);
            bytes.length = dict_length;
        }

        if offset_in_block < offset + length {
            let needed = bytes.length + offset + length - offset_in_block;
            if bytes.bytes.len() < needed {
                bytes.bytes.resize(needed, 0);
            }
        }
        while offset_in_block < offset + length {
            let to_decompress = std::cmp::min(block_length, offset + length - offset_in_block);
            Lz4::decompress(input, to_decompress, &mut self.buffer, dict_length)?;
            bytes.bytes[bytes.length..bytes.length + to_decompress]
                .copy_from_slice(&self.buffer[dict_length..dict_length + to_decompress]);
            bytes.length += to_decompress;
            offset_in_block += block_length;
        }

        bytes.offset = offset_in_bytes_ref;
        bytes.length = length;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Deflate
// -----------------------------------------------------------------------------

/// Compression level of the deflate preset-dictionary mode.
///
/// Lucene fixes level 6 — `DeflateWithPresetDictCompressionMode.java:55`, whose
/// comment reads "6 is the default, higher than that is just a waste of cpu" —
/// and that is exactly what this port uses when it is built against the C zlib
/// (`--features zlib-c`), where it is also what byte identity requires.
///
/// The default backend, `zlib-rs`, is a port of **zlib-ng**, whose level to
/// strategy table is not zlib's, and at level 6 it compresses redundant text
/// roughly twice as badly — which is precisely the input this mode exists for.
/// Measured on this machine, total bytes for one chunk framed exactly as this
/// mode frames it (`best of five`, release build):
///
/// | corpus | C zlib L6 | zlib-rs L6 | zlib-rs L7 |
/// | --- | --- | --- | --- |
/// | 620 016 B of repeated prose | 2 421 B @ 320 MB/s | 4 661 B @ 2 205 MB/s | 2 419 B @ 645 MB/s |
/// | 620 008 B of mixed text | 24 358 B @ 112 MB/s | 24 739 B @ 156 MB/s | 24 398 B @ 85 MB/s |
///
/// Level 7 is therefore the level at which `zlib-rs` reaches the ratio Lucene
/// gets from zlib at level 6 — within 0.2% on both corpora, and at twice the
/// throughput on the redundant one. Levels 8 and 9 buy nothing: on the mixed
/// corpus they are *worse* than 7 (24 414 B and 24 427 B) for two to six times
/// the CPU. The level is a property of the encoder, never of the format: a
/// reader never learns which level produced a stream, so the two backends
/// remain mutually readable.
#[cfg(not(feature = "zlib-c"))]
const DEFLATE_PRESET_DICT_LEVEL: u32 = 7;

/// Compression level of the deflate preset-dictionary mode: Lucene's own, used
/// when this crate is built against the C zlib. See the `zlib-rs` variant of
/// this constant for why the default backend differs.
#[cfg(feature = "zlib-c")]
const DEFLATE_PRESET_DICT_LEVEL: u32 = 6;

/// The dictionary of the deflate preset-dictionary mode is about this many
/// times smaller than a sub block.
///
/// Equivalent to `DeflateWithPresetDictCompressionMode.DICT_SIZE_FACTOR`. Note
/// that it differs from the LZ4 mode's factor of 2.
const DEFLATE_DICT_SIZE_FACTOR: usize = 6;

/// Deflates `input` in full and leaves the compressed bytes in `scratch`.
///
/// Equivalent to the `setInput` / `finish` / `deflate` loop Lucene runs in
/// `CompressionMode.DeflateCompressor.compress` and in
/// `DeflateWithPresetDictCompressionMode.DeflateWithPresetDictCompressor.doCompress`.
/// The caller must have reset `compressor` — and set its dictionary, when the
/// block needs one — beforehand.
fn deflate_to(compressor: &mut Compress, input: &[u8], scratch: &mut Vec<u8>) -> Result<()> {
    scratch.clear();
    scratch.reserve(64.max(input.len() / 2 + 16));
    loop {
        if scratch.len() == scratch.capacity() {
            // `compress_vec` only writes into spare capacity, so it must exist.
            scratch.reserve(scratch.capacity().max(64));
        }
        let consumed = compressor.total_in() as usize;
        let produced = compressor.total_out();
        let status = compressor
            .compress_vec(&input[consumed..], scratch, FlushCompress::Finish)
            .map_err(|error| LuceneError::Other(format!("deflate failed: {error}")))?;
        if status == Status::StreamEnd {
            return Ok(());
        }
        if compressor.total_in() as usize == consumed && compressor.total_out() == produced {
            return Err(LuceneError::Other(
                "deflate made no progress and did not finish".to_string(),
            ));
        }
    }
}

/// Writes one deflate-compressed block as Lucene frames it: a `vInt` length
/// followed by exactly that many bytes.
///
/// A zero-length block is written as a bare `vInt(0)` with no payload, matching
/// `doCompress`'s early return.
fn write_deflate_block(
    compressor: &mut Compress,
    input: &[u8],
    scratch: &mut Vec<u8>,
    out: &mut dyn DataOutput,
) -> Result<()> {
    if input.is_empty() {
        return out.write_v_int(0);
    }
    deflate_to(compressor, input, scratch)?;
    let length = i32::try_from(scratch.len()).map_err(|_| {
        LuceneError::IllegalState(format!("compressed block too large: {}", scratch.len()))
    })?;
    out.write_v_int(length)?;
    out.write_bytes(scratch, 0, scratch.len())
}

/// Reads one deflate-compressed block written by [`write_deflate_block`] and
/// appends the inflated bytes to `out`.
///
/// Equivalent to `DeflateWithPresetDictDecompressor.doDecompress`, including
/// the trailing dummy byte Lucene appends "for compliance" with the raw
/// inflater contract. `expected` is how many bytes the block is known to hold,
/// used only to size the destination.
fn read_deflate_block(
    input: &mut dyn DataInput,
    decompressor: &mut Decompress,
    compressed: &mut Vec<u8>,
    out: &mut Vec<u8>,
    expected: usize,
) -> Result<()> {
    let compressed_length = input.read_v_int()?;
    if compressed_length < 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "negative compressed block length: {compressed_length}"
        )));
    }
    if compressed_length == 0 {
        return Ok(());
    }
    let compressed_length = compressed_length as usize;
    compressed.clear();
    compressed.resize(compressed_length + 1, 0);
    input.read_bytes(compressed, 0, compressed_length)?;
    // The extra byte is already zero; raw inflate needs one byte past the end.
    out.reserve(expected.max(1));
    loop {
        if out.len() == out.capacity() {
            out.reserve(out.capacity().max(64));
        }
        let consumed = decompressor.total_in() as usize;
        let produced = decompressor.total_out();
        let status = decompressor
            .decompress_vec(&compressed[consumed..], out, FlushDecompress::Finish)
            .map_err(|error| LuceneError::CorruptIndex(format!("inflate failed: {error}")))?;
        if status == Status::StreamEnd {
            return Ok(());
        }
        if decompressor.total_in() as usize == consumed && decompressor.total_out() == produced {
            return Err(LuceneError::CorruptIndex(
                "invalid decoder state: inflate made no progress and did not finish".to_string(),
            ));
        }
    }
}

/// Deflates a whole chunk in one raw-deflate stream.
///
/// Equivalent to `CompressionMode.DeflateCompressor`. Lucene builds its
/// `Deflater` with `new Deflater(level, true)` — the `true` means **no zlib
/// wrapper** — so the bytes are a bare RFC 1951 stream, not RFC 1950.
///
/// This one keeps Lucene's level 6 on every backend: no format in Lucene 10.5.0
/// selects [`CompressionMode::HIGH_COMPRESSION`] — only `FAST`, from
/// `Lucene90TermVectorsFormat.java:170` — so it writes no index file this port
/// produces, and the faithful constant is the useful one.
#[derive(Debug)]
struct DeflateCompressor {
    compressor: Compress,
    scratch: Vec<u8>,
}

impl DeflateCompressor {
    fn new(level: u32) -> Self {
        Self {
            compressor: Compress::new(Compression::new(level), false),
            scratch: Vec::new(),
        }
    }
}

impl Compressor for DeflateCompressor {
    fn compress(&mut self, input: &ByteBuffersDataOutput, out: &mut dyn DataOutput) -> Result<()> {
        let bytes = input.to_array_copy();
        self.compressor.reset();
        write_deflate_block(&mut self.compressor, &bytes, &mut self.scratch, out)
    }
}

/// Inflates what [`DeflateCompressor`] wrote.
///
/// Equivalent to `CompressionMode.DeflateDecompressor`, which uses
/// `new Inflater(true)` — raw inflate.
#[derive(Debug)]
struct DeflateDecompressor {
    decompressor: Decompress,
    compressed: Vec<u8>,
}

impl DeflateDecompressor {
    fn new() -> Self {
        Self {
            decompressor: Decompress::new(false),
            compressed: Vec::new(),
        }
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
        let mut out: Vec<u8> = Vec::with_capacity(original_length);
        self.decompressor.reset(false);
        read_deflate_block(
            input,
            &mut self.decompressor,
            &mut self.compressed,
            &mut out,
            original_length,
        )?;
        if out.len() != original_length {
            return Err(LuceneError::CorruptIndex(format!(
                "Lengths mismatch: {} != {original_length}",
                out.len()
            )));
        }
        bytes.bytes = out;
        bytes.offset = offset;
        bytes.length = length;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Deflate with a preset dictionary
// -----------------------------------------------------------------------------

/// Compresses a chunk as a dictionary plus [`NUM_SUB_BLOCKS`] sub blocks, each
/// raw-deflated against that dictionary.
///
/// Equivalent to
/// `DeflateWithPresetDictCompressionMode.DeflateWithPresetDictCompressor`.
///
/// Unlike the LZ4 preset-dictionary mode, every block is written as
/// `vInt length` immediately followed by its own bytes; there is no separate
/// length section up front.
#[derive(Debug)]
struct DeflateWithPresetDictCompressor {
    compressor: Compress,
    scratch: Vec<u8>,
    buffer: Vec<u8>,
}

impl DeflateWithPresetDictCompressor {
    fn new(level: u32) -> Self {
        Self {
            compressor: Compress::new(Compression::new(level), false),
            scratch: Vec::new(),
            buffer: Vec::new(),
        }
    }
}

impl Compressor for DeflateWithPresetDictCompressor {
    fn compress(&mut self, input: &ByteBuffersDataOutput, out: &mut dyn DataOutput) -> Result<()> {
        let bytes = input.to_array_copy();
        let len = bytes.len();
        let dict_length = len / (NUM_SUB_BLOCKS * DEFLATE_DICT_SIZE_FACTOR);
        let block_length = (len - dict_length).div_ceil(NUM_SUB_BLOCKS);
        out.write_v_int(i32::try_from(dict_length).map_err(|_| {
            LuceneError::IllegalState(format!("dictionary too large: {dict_length}"))
        })?)?;
        out.write_v_int(
            i32::try_from(block_length).map_err(|_| {
                LuceneError::IllegalState(format!("block too large: {block_length}"))
            })?,
        )?;

        self.buffer.clear();
        self.buffer.resize(dict_length + block_length, 0);

        // The dictionary first, compressed against nothing.
        self.compressor.reset();
        self.buffer[..dict_length].copy_from_slice(&bytes[..dict_length]);
        write_deflate_block(
            &mut self.compressor,
            &self.buffer[..dict_length],
            &mut self.scratch,
            out,
        )?;

        // Then every sub block, each against the same dictionary.
        let mut start = dict_length;
        while start < len {
            self.compressor.reset();
            self.compressor
                .set_dictionary(&self.buffer[..dict_length])
                .map_err(|error| {
                    LuceneError::Other(format!("deflate dictionary rejected: {error}"))
                })?;
            let l = std::cmp::min(block_length, len - start);
            self.buffer[dict_length..dict_length + l].copy_from_slice(&bytes[start..start + l]);
            write_deflate_block(
                &mut self.compressor,
                &self.buffer[dict_length..dict_length + l],
                &mut self.scratch,
                out,
            )?;
            start += block_length;
        }
        Ok(())
    }
}

/// Reads back what [`DeflateWithPresetDictCompressor`] wrote, inflating only
/// the sub blocks that intersect the requested interval.
///
/// Equivalent to
/// `DeflateWithPresetDictCompressionMode.DeflateWithPresetDictDecompressor`.
#[derive(Debug)]
struct DeflateWithPresetDictDecompressor {
    decompressor: Decompress,
    compressed: Vec<u8>,
}

impl DeflateWithPresetDictDecompressor {
    fn new() -> Self {
        Self {
            decompressor: Decompress::new(false),
            compressed: Vec::new(),
        }
    }
}

impl Decompressor for DeflateWithPresetDictDecompressor {
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

        let dict_length = usize::try_from(input.read_v_int()?)
            .map_err(|_| LuceneError::CorruptIndex("negative dictionary length".to_string()))?;
        let block_length = usize::try_from(input.read_v_int()?)
            .map_err(|_| LuceneError::CorruptIndex("negative block length".to_string()))?;
        if block_length == 0 {
            // Only a zero-length chunk can have zero-length blocks, and that
            // case returned above; a zero here would loop forever.
            return Err(LuceneError::CorruptIndex(
                "a non-empty chunk cannot have zero-length sub blocks".to_string(),
            ));
        }

        let mut out: Vec<u8> = Vec::with_capacity(dict_length + block_length);
        self.decompressor.reset(false);
        read_deflate_block(
            input,
            &mut self.decompressor,
            &mut self.compressed,
            &mut out,
            dict_length,
        )?;
        if out.len() != dict_length {
            return Err(LuceneError::CorruptIndex(format!(
                "Unexpected dict length: {} != {dict_length}",
                out.len()
            )));
        }

        let mut offset_in_block = dict_length;
        let mut offset_in_bytes_ref = offset;
        // Skip the blocks that end before the interval starts.
        while offset_in_block + block_length < offset {
            let compressed_length = input.read_v_int()?;
            if compressed_length < 0 {
                return Err(LuceneError::CorruptIndex(format!(
                    "negative compressed block length: {compressed_length}"
                )));
            }
            input.skip_bytes(i64::from(compressed_length))?;
            offset_in_block += block_length;
            offset_in_bytes_ref -= block_length;
        }

        // Read the blocks that intersect the interval.
        while offset_in_block < offset + length {
            self.decompressor.reset(false);
            self.decompressor
                .set_dictionary(&out[..dict_length])
                .map_err(|error| {
                    LuceneError::CorruptIndex(format!("inflate dictionary rejected: {error}"))
                })?;
            read_deflate_block(
                input,
                &mut self.decompressor,
                &mut self.compressed,
                &mut out,
                block_length,
            )?;
            offset_in_block += block_length;
        }

        bytes.bytes = out;
        bytes.offset = offset_in_bytes_ref;
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
    fn lz4_with_preset_dict_round_trips_and_decompresses_slices() {
        // Long enough to be split into ten sub blocks plus a dictionary, so the
        // block-skipping path in the decompressor is exercised.
        let data: Vec<u8> = (0..40_000).map(|i| ((i * 31) % 251) as u8).collect();
        round_trip(CompressionMode::LZ4_WITH_PRESET_DICT, &data, 0, data.len());
        // A slice entirely inside the dictionary.
        round_trip(CompressionMode::LZ4_WITH_PRESET_DICT, &data, 0, 100);
        // A slice that starts in the dictionary and runs into the blocks.
        round_trip(CompressionMode::LZ4_WITH_PRESET_DICT, &data, 500, 5_000);
        // A slice deep inside the blocks, so leading blocks must be skipped.
        round_trip(CompressionMode::LZ4_WITH_PRESET_DICT, &data, 30_000, 5_000);
        // The tail.
        round_trip(
            CompressionMode::LZ4_WITH_PRESET_DICT,
            &data,
            data.len() - 10,
            10,
        );
    }

    #[test]
    fn lz4_with_preset_dict_handles_short_and_repetitive_payloads() {
        // Short enough that the dictionary is empty.
        round_trip(CompressionMode::LZ4_WITH_PRESET_DICT, b"abc", 0, 3);
        round_trip(CompressionMode::LZ4_WITH_PRESET_DICT, b"", 0, 0);
        let repetitive = vec![7u8; 5_000];
        round_trip(
            CompressionMode::LZ4_WITH_PRESET_DICT,
            &repetitive,
            0,
            repetitive.len(),
        );
        round_trip(
            CompressionMode::LZ4_WITH_PRESET_DICT,
            &repetitive,
            1_000,
            2_000,
        );
    }

    #[test]
    fn lz4_with_preset_dict_is_a_different_format_from_plain_lz4() {
        // The two modes must never be confused: `Lucene90StoredFieldsFormat`
        // uses the preset-dict one for BEST_SPEED, and a segment written with
        // one cannot be read with the other.
        let data: Vec<u8> = (0..4_000).map(|i| (i % 97) as u8).collect();
        let compress = |mode: CompressionMode| {
            let mut compressor = mode.new_compressor();
            let mut output = ByteArrayDataOutput::new();
            let mut buffered = ByteBuffersDataOutput::with_expected_size(data.len());
            buffered.write_bytes(&data, 0, data.len()).unwrap();
            compressor.compress(&buffered, &mut output).unwrap();
            output.into_inner()
        };
        assert_ne!(
            compress(CompressionMode::FAST),
            compress(CompressionMode::LZ4_WITH_PRESET_DICT)
        );
    }

    #[test]
    fn deflate_with_preset_dict_round_trips_and_decompresses_slices() {
        let data: Vec<u8> = (0..60_000).map(|i| ((i * 17) % 233) as u8).collect();
        round_trip(
            CompressionMode::DEFLATE_WITH_PRESET_DICT,
            &data,
            0,
            data.len(),
        );
        // Inside the dictionary only.
        round_trip(CompressionMode::DEFLATE_WITH_PRESET_DICT, &data, 0, 50);
        // Across the dictionary and into the blocks.
        round_trip(CompressionMode::DEFLATE_WITH_PRESET_DICT, &data, 100, 8_000);
        // Deep inside, so leading blocks must be skipped without inflating.
        round_trip(
            CompressionMode::DEFLATE_WITH_PRESET_DICT,
            &data,
            45_000,
            8_000,
        );
        // The tail.
        round_trip(
            CompressionMode::DEFLATE_WITH_PRESET_DICT,
            &data,
            data.len() - 10,
            10,
        );
    }

    #[test]
    fn deflate_with_preset_dict_handles_short_and_repetitive_payloads() {
        round_trip(CompressionMode::DEFLATE_WITH_PRESET_DICT, b"abc", 0, 3);
        round_trip(CompressionMode::DEFLATE_WITH_PRESET_DICT, b"", 0, 0);
        let repetitive = vec![3u8; 9_000];
        round_trip(
            CompressionMode::DEFLATE_WITH_PRESET_DICT,
            &repetitive,
            0,
            repetitive.len(),
        );
        round_trip(
            CompressionMode::DEFLATE_WITH_PRESET_DICT,
            &repetitive,
            2_000,
            3_000,
        );
    }

    #[test]
    fn deflate_writes_raw_streams_not_zlib_wrapped_ones() {
        // Lucene builds its `Deflater` with `new Deflater(level, true)`, which
        // means **no zlib wrapper**: the payload must not start with the RFC
        // 1950 header bytes `78 9c`.
        let data: Vec<u8> = (0..4_000).map(|i| (i % 251) as u8).collect();
        for mode in [
            CompressionMode::HIGH_COMPRESSION,
            CompressionMode::DEFLATE_WITH_PRESET_DICT,
        ] {
            let mut compressor = mode.new_compressor();
            let mut output = ByteArrayDataOutput::new();
            let mut buffered = ByteBuffersDataOutput::with_expected_size(data.len());
            buffered.write_bytes(&data, 0, data.len()).unwrap();
            compressor.compress(&buffered, &mut output).unwrap();
            let bytes = output.into_inner();
            assert!(
                !bytes.windows(2).any(|window| window == [0x78, 0x9c]),
                "{mode} must not emit a zlib header"
            );
        }
    }

    #[test]
    fn the_two_deflate_modes_are_different_formats() {
        let data: Vec<u8> = (0..9_000).map(|i| (i % 97) as u8).collect();
        let compress = |mode: CompressionMode| {
            let mut compressor = mode.new_compressor();
            let mut output = ByteArrayDataOutput::new();
            let mut buffered = ByteBuffersDataOutput::with_expected_size(data.len());
            buffered.write_bytes(&data, 0, data.len()).unwrap();
            compressor.compress(&buffered, &mut output).unwrap();
            output.into_inner()
        };
        assert_ne!(
            compress(CompressionMode::HIGH_COMPRESSION),
            compress(CompressionMode::DEFLATE_WITH_PRESET_DICT)
        );
    }

    #[test]
    fn compression_mode_names_match_the_java_to_string() {
        assert_eq!(CompressionMode::FAST.to_string(), "FAST");
        assert_eq!(
            CompressionMode::HIGH_COMPRESSION.to_string(),
            "HIGH_COMPRESSION"
        );
        assert_eq!(
            CompressionMode::FAST_DECOMPRESSION.to_string(),
            "FAST_DECOMPRESSION"
        );
        assert_eq!(
            CompressionMode::LZ4_WITH_PRESET_DICT.to_string(),
            "BEST_SPEED",
            "LZ4WithPresetDictCompressionMode.toString() returns BEST_SPEED"
        );
        assert_eq!(
            CompressionMode::DEFLATE_WITH_PRESET_DICT.to_string(),
            "BEST_COMPRESSION",
            "DeflateWithPresetDictCompressionMode.toString() returns BEST_COMPRESSION"
        );
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
