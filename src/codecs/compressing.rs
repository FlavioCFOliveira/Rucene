//! Generic compression engine for stored-fields and term-vectors formats.
//!
//! Ported from `org.apache.lucene.codecs.compressing`. This module provides
//! [`CompressionMode`], [`Compressor`], [`Decompressor`] and [`MatchingReaders`],
//! the shared engine used by `Lucene90CompressingStoredFieldsFormat` and
//! `Lucene90CompressingTermVectorsFormat`.
//!
//! **Divergence from Lucene 10.5.0.** Lucene declares two more modes outside
//! this package: `org.apache.lucene.codecs.lucene90.LZ4WithPresetDictCompressionMode`
//! and `DeflateWithPresetDictCompressionMode`, each a file of its own, and the
//! anonymous `SortingStoredFieldsConsumer.NO_COMPRESSION`
//! (`SortingStoredFieldsConsumer.java:42-79`). Their compressors share this
//! module's LZ4 and deflate framing helpers, so their implementations live
//! here; the public types are re-exported from
//! [`codecs::lucene90`](crate::codecs::lucene90), and the singletons Java
//! declares elsewhere are declared where Java declares them
//! ([`BEST_SPEED_MODE`](crate::codecs::lucene90::stored_fields::BEST_SPEED_MODE),
//! [`BEST_COMPRESSION_MODE`](crate::codecs::lucene90::stored_fields::BEST_COMPRESSION_MODE),
//! and `sorting_consumers::NO_COMPRESSION`). Only the location of the code
//! differs; the types, constants and bytes are Lucene's.

#![deny(unsafe_code)]

use std::sync::{Arc, LazyLock};

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
/// Equivalent to the abstract class
/// `org.apache.lucene.codecs.compressing.CompressionMode`
/// (`CompressionMode.java:37,118,121`): a mode is anything that hands out a
/// [`Compressor`] and a matching [`Decompressor`], and a user may implement it
/// for a mode of their own. Lucene's constants are [`FAST`],
/// [`HIGH_COMPRESSION`] and [`FAST_DECOMPRESSION`].
///
/// Holders keep a mode as an `Arc<dyn CompressionMode>`, Java's reference;
/// [`same_mode`] is Java's `==` between two of them.
///
/// [`to_string`](Self::to_string) is Java's `toString()`: a mode that does
/// not override it inherits `Object.toString()`, and Lucene's own modes
/// override it exactly where Java's do.
pub trait CompressionMode: Send + Sync + std::fmt::Debug {
    /// Creates a new compressor for this mode.
    ///
    /// Equivalent to `CompressionMode.newCompressor()`.
    fn new_compressor(&self) -> Box<dyn Compressor>;

    /// Creates a new decompressor for this mode.
    ///
    /// Equivalent to `CompressionMode.newDecompressor()`.
    fn new_decompressor(&self) -> Box<dyn Decompressor>;

    /// Returns this mode's string form.
    ///
    /// Equivalent to `toString()`. The default is the `Object.toString()` a
    /// Java mode inherits when it declares none:
    /// `getClass().getName() + "@" + hex identity`, rendered by
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// with the implementing type's name standing in for the Java class name,
    /// since a Rust type has no Java binary name.
    fn to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string(std::any::type_name::<Self>(), self)
    }
}

/// Whether `a` and `b` are the same mode object, Java's `a == b`.
///
/// The writers compare the reader's mode with their own by reference
/// (`Lucene90CompressingStoredFieldsWriter.java:531,692`,
/// `Lucene90CompressingTermVectorsWriter.java:819,964`); the singletons below make
/// that hold for the same constant, as Java's `static final` fields do.
pub fn same_mode(a: &Arc<dyn CompressionMode>, b: &Arc<dyn CompressionMode>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// The anonymous mode behind [`FAST`] (`CompressionMode.java:44-62`).
#[derive(Debug)]
struct FastMode;

impl CompressionMode for FastMode {
    fn new_compressor(&self) -> Box<dyn Compressor> {
        Box::new(Lz4FastCompressor::new())
    }

    fn new_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(Lz4Decompressor)
    }

    /// Equivalent to `toString()`.
    fn to_string(&self) -> String {
        "FAST".to_string()
    }
}

/// The anonymous mode behind [`HIGH_COMPRESSION`] (`CompressionMode.java:68-89`).
#[derive(Debug)]
struct HighCompressionMode;

impl CompressionMode for HighCompressionMode {
    fn new_compressor(&self) -> Box<dyn Compressor> {
        // 3 is the highest level that doesn't have lazy match evaluation;
        // 6 is the default, higher than that is just a waste of cpu.
        Box::new(DeflateCompressor::new(6))
    }

    fn new_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(DeflateDecompressor::new())
    }

    /// Equivalent to `toString()`.
    fn to_string(&self) -> String {
        "HIGH_COMPRESSION".to_string()
    }
}

/// The anonymous mode behind [`FAST_DECOMPRESSION`] (`CompressionMode.java:95-113`).
#[derive(Debug)]
struct FastDecompressionMode;

impl CompressionMode for FastDecompressionMode {
    fn new_compressor(&self) -> Box<dyn Compressor> {
        Box::new(Lz4HighCompressor::new())
    }

    fn new_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(Lz4Decompressor)
    }

    /// Equivalent to `toString()`.
    fn to_string(&self) -> String {
        "FAST_DECOMPRESSION".to_string()
    }
}

/// A compression mode that compresses and decompresses fast, at a lower ratio.
///
/// Equivalent to `CompressionMode.FAST` (`CompressionMode.java:44`): LZ4 fast
/// compressor with LZ4 decompressor.
pub static FAST: LazyLock<Arc<dyn CompressionMode>> = LazyLock::new(|| Arc::new(FastMode));

/// A compression mode that trades speed for compression ratio.
///
/// Equivalent to `CompressionMode.HIGH_COMPRESSION` (`CompressionMode.java:68`):
/// deflate compressor with deflate decompressor.
pub static HIGH_COMPRESSION: LazyLock<Arc<dyn CompressionMode>> =
    LazyLock::new(|| Arc::new(HighCompressionMode));

/// A compression mode that compresses slowly but decompresses fast.
///
/// Equivalent to `CompressionMode.FAST_DECOMPRESSION` (`CompressionMode.java:95`):
/// LZ4 high-compression compressor with LZ4 decompressor.
pub static FAST_DECOMPRESSION: LazyLock<Arc<dyn CompressionMode>> =
    LazyLock::new(|| Arc::new(FastDecompressionMode));

/// LZ4 over a preset dictionary, splitting each chunk into sub blocks.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.LZ4WithPresetDictCompressionMode`,
/// which is what `Lucene90StoredFieldsFormat` uses for
/// [`Mode::BestSpeed`](crate::codecs::lucene90::stored_fields::Mode). It is
/// **not** interchangeable with [`FAST`]: the bytes it writes have a different
/// framing, so a segment written with one cannot be read with the other.
#[derive(Debug, Default)]
#[allow(clippy::upper_case_acronyms)]
pub struct LZ4WithPresetDictCompressionMode;

impl LZ4WithPresetDictCompressionMode {
    /// Equivalent to `LZ4WithPresetDictCompressionMode()`.
    pub fn new() -> Self {
        Self
    }
}

impl CompressionMode for LZ4WithPresetDictCompressionMode {
    fn new_compressor(&self) -> Box<dyn Compressor> {
        Box::new(Lz4WithPresetDictCompressor::new())
    }

    fn new_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(Lz4WithPresetDictDecompressor::new())
    }

    /// Equivalent to `LZ4WithPresetDictCompressionMode.toString()`.
    fn to_string(&self) -> String {
        "BEST_SPEED".to_string()
    }
}

/// Raw deflate over a preset dictionary, splitting each chunk into sub blocks.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.DeflateWithPresetDictCompressionMode`,
/// which is what `Lucene90StoredFieldsFormat` uses for
/// [`Mode::BestCompression`](crate::codecs::lucene90::stored_fields::Mode).
/// It is **not** interchangeable with [`HIGH_COMPRESSION`].
#[derive(Debug, Default)]
pub struct DeflateWithPresetDictCompressionMode;

impl DeflateWithPresetDictCompressionMode {
    /// Equivalent to `DeflateWithPresetDictCompressionMode()`.
    pub fn new() -> Self {
        Self
    }
}

impl CompressionMode for DeflateWithPresetDictCompressionMode {
    fn new_compressor(&self) -> Box<dyn Compressor> {
        Box::new(DeflateWithPresetDictCompressor::new(
            DEFLATE_PRESET_DICT_LEVEL,
        ))
    }

    fn new_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(DeflateWithPresetDictDecompressor::new())
    }

    /// Equivalent to `DeflateWithPresetDictCompressionMode.toString()`.
    fn to_string(&self) -> String {
        "BEST_COMPRESSION".to_string()
    }
}

/// No compression at all: the compressor copies its input, the decompressor
/// reads the requested slice.
///
/// Equivalent to the anonymous `CompressionMode` behind
/// `SortingStoredFieldsConsumer.NO_COMPRESSION`
/// (`SortingStoredFieldsConsumer.java:42-79`), which the temporary
/// stored-fields and term-vectors formats of an index-sorted flush use, since
/// their files are read back once and deleted. The singleton is
/// `sorting_consumers::NO_COMPRESSION`.
#[derive(Debug)]
pub(crate) struct NoCompressionMode;

impl CompressionMode for NoCompressionMode {
    fn new_compressor(&self) -> Box<dyn Compressor> {
        Box::new(NoCompressionCompressor)
    }

    fn new_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(NoCompressionDecompressor)
    }

    /// The anonymous class declares no `toString()`, so `Object`'s applies;
    /// see `java_object_to_string` for the identity suffix.
    fn to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SortingStoredFieldsConsumer$1",
            self,
        )
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

    /// Returns an independent decompressor over the same algorithm, so that a
    /// cloned reader gets its own decompression state.
    ///
    /// Equivalent to the abstract `Decompressor.clone()`. Rust's [`Clone`] is
    /// not object safe, so the contract is expressed as a boxed factory method,
    /// exactly as [`IndexInput::clone_input`](crate::store::IndexInput::clone_input)
    /// expresses `IndexInput.clone()`.
    fn clone_decompressor(&self) -> Box<dyn Decompressor>;
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

/// The compressor of [`NoCompressionMode`]: `out.copyBytes(buffersInput,
/// buffersInput.length())`.
#[derive(Debug, Default, Clone, Copy)]
struct NoCompressionCompressor;

impl Compressor for NoCompressionCompressor {
    fn compress(&mut self, input: &ByteBuffersDataOutput, out: &mut dyn DataOutput) -> Result<()> {
        input.copy_to(out)
    }
}

/// The decompressor of [`NoCompressionMode`]: skip `offset`
/// bytes and read `length`; `originalLength` is not needed.
#[derive(Debug, Default, Clone, Copy)]
struct NoCompressionDecompressor;

impl Decompressor for NoCompressionDecompressor {
    /// Java's `clone()` returns `this`; this returns an equal copy of the unit
    /// struct.
    fn clone_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(*self)
    }

    fn decompress(
        &mut self,
        input: &mut dyn DataInput,
        _original_length: usize,
        offset: usize,
        length: usize,
        bytes: &mut BytesRef,
    ) -> Result<()> {
        if bytes.bytes.len() < length {
            bytes.bytes = vec![0; length];
        }
        input.skip_bytes(offset as i64)?;
        input.read_bytes(&mut bytes.bytes, 0, length)?;
        bytes.offset = 0;
        bytes.length = length;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Lz4Decompressor;

impl Decompressor for Lz4Decompressor {
    /// Java's LZ4 decompressor is stateless, and its `clone()` returns `this`
    /// (`CompressionMode.java:145`); this returns an equal copy of the unit
    /// struct, which is the same thing without shared ownership.
    fn clone_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(*self)
    }

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
            LuceneError::illegal_state(format!(
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
            LuceneError::illegal_state(format!("dictionary too large: {dict_length}"))
        })?)?;
        out.write_v_int(i32::try_from(block_length).map_err(|_| {
            LuceneError::illegal_state(format!("block too large: {block_length}"))
        })?)?;

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
    /// Equivalent to `LZ4WithPresetDictDecompressor.clone()`, which is
    /// `new LZ4WithPresetDictDecompressor()` — a fresh instance with empty
    /// scratch buffers, not a copy of this one's.
    fn clone_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(Self::new())
    }

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
        LuceneError::illegal_state(format!("compressed block too large: {}", scratch.len()))
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
/// selects [`HIGH_COMPRESSION`] — only `FAST`, from
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
    /// Equivalent to `DeflateDecompressor.clone()`, which is
    /// `new DeflateDecompressor()`.
    fn clone_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(Self::new())
    }

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
            LuceneError::illegal_state(format!("dictionary too large: {dict_length}"))
        })?)?;
        out.write_v_int(i32::try_from(block_length).map_err(|_| {
            LuceneError::illegal_state(format!("block too large: {block_length}"))
        })?)?;

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
    /// Equivalent to `DeflateWithPresetDictDecompressor.clone()`, which is
    /// `new DeflateWithPresetDictDecompressor()`.
    fn clone_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(Self::new())
    }

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

        // `MatchingReaders.java:67-72`.
        if merge_state.info_stream.is_enabled("SM") {
            merge_state.info_stream.message(
                "SM",
                &format!("merge store matchedCount={matched_count} vs {num_readers}"),
            );
            if matched_count as usize != num_readers {
                merge_state.info_stream.message(
                    "SM",
                    &format!("{} non-bulk merges", num_readers - matched_count as usize),
                );
            }
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
    use crate::store::VecDataOutput;

    fn round_trip(mode: &dyn CompressionMode, data: &[u8], offset: usize, length: usize) {
        let mut compressor = mode.new_compressor();
        let mut output = VecDataOutput::new();
        let mut bbo = ByteBuffersDataOutput::with_expected_size(data.len());
        bbo.write_bytes(data, 0, data.len()).unwrap();
        compressor.compress(&bbo, &mut output).unwrap();
        compressor.close().unwrap();

        let compressed = output.into_inner();
        let mut input = ByteArrayDataInput::new(&compressed);
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
        round_trip(&**FAST, &data, 0, data.len());
        round_trip(&**FAST, &data, 100, 200);
    }

    #[test]
    fn lz4_fast_decompresses_slice() {
        let data = b"the quick brown fox jumps over the lazy dog";
        round_trip(&**FAST, data, 4, 15);
    }

    #[test]
    fn fast_decompression_round_trips() {
        let data = vec![0u8; 512];
        round_trip(&**FAST_DECOMPRESSION, &data, 0, data.len());

        let mut varied = vec![0u8; 256];
        for (i, b) in varied.iter_mut().enumerate() {
            *b = (i * 7) as u8;
        }
        round_trip(&**FAST_DECOMPRESSION, &varied, 0, varied.len());
    }

    #[test]
    fn deflate_high_compression_round_trips() {
        let data: Vec<u8> = (0..2048).map(|i| (i % 127) as u8).collect();
        round_trip(&**HIGH_COMPRESSION, &data, 0, data.len());
        round_trip(&**HIGH_COMPRESSION, &data, 500, 1000);
    }

    #[test]
    fn deflate_empty_payload() {
        let data: Vec<u8> = vec![];
        round_trip(&**HIGH_COMPRESSION, &data, 0, 0);
    }

    #[test]
    fn lz4_with_preset_dict_round_trips_and_decompresses_slices() {
        // Long enough to be split into ten sub blocks plus a dictionary, so the
        // block-skipping path in the decompressor is exercised.
        let data: Vec<u8> = (0..40_000).map(|i| ((i * 31) % 251) as u8).collect();
        round_trip(&LZ4WithPresetDictCompressionMode, &data, 0, data.len());
        // A slice entirely inside the dictionary.
        round_trip(&LZ4WithPresetDictCompressionMode, &data, 0, 100);
        // A slice that starts in the dictionary and runs into the blocks.
        round_trip(&LZ4WithPresetDictCompressionMode, &data, 500, 5_000);
        // A slice deep inside the blocks, so leading blocks must be skipped.
        round_trip(&LZ4WithPresetDictCompressionMode, &data, 30_000, 5_000);
        // The tail.
        round_trip(
            &LZ4WithPresetDictCompressionMode,
            &data,
            data.len() - 10,
            10,
        );
    }

    #[test]
    fn lz4_with_preset_dict_handles_short_and_repetitive_payloads() {
        // Short enough that the dictionary is empty.
        round_trip(&LZ4WithPresetDictCompressionMode, b"abc", 0, 3);
        round_trip(&LZ4WithPresetDictCompressionMode, b"", 0, 0);
        let repetitive = vec![7u8; 5_000];
        round_trip(
            &LZ4WithPresetDictCompressionMode,
            &repetitive,
            0,
            repetitive.len(),
        );
        round_trip(&LZ4WithPresetDictCompressionMode, &repetitive, 1_000, 2_000);
    }

    #[test]
    fn lz4_with_preset_dict_is_a_different_format_from_plain_lz4() {
        // The two modes must never be confused: `Lucene90StoredFieldsFormat`
        // uses the preset-dict one for BEST_SPEED, and a segment written with
        // one cannot be read with the other.
        let data: Vec<u8> = (0..4_000).map(|i| (i % 97) as u8).collect();
        let compress = |mode: &dyn CompressionMode| {
            let mut compressor = mode.new_compressor();
            let mut output = VecDataOutput::new();
            let mut buffered = ByteBuffersDataOutput::with_expected_size(data.len());
            buffered.write_bytes(&data, 0, data.len()).unwrap();
            compressor.compress(&buffered, &mut output).unwrap();
            output.into_inner()
        };
        assert_ne!(
            compress(&**FAST),
            compress(&LZ4WithPresetDictCompressionMode)
        );
    }

    #[test]
    fn deflate_with_preset_dict_round_trips_and_decompresses_slices() {
        let data: Vec<u8> = (0..60_000).map(|i| ((i * 17) % 233) as u8).collect();
        round_trip(&DeflateWithPresetDictCompressionMode, &data, 0, data.len());
        // Inside the dictionary only.
        round_trip(&DeflateWithPresetDictCompressionMode, &data, 0, 50);
        // Across the dictionary and into the blocks.
        round_trip(&DeflateWithPresetDictCompressionMode, &data, 100, 8_000);
        // Deep inside, so leading blocks must be skipped without inflating.
        round_trip(&DeflateWithPresetDictCompressionMode, &data, 45_000, 8_000);
        // The tail.
        round_trip(
            &DeflateWithPresetDictCompressionMode,
            &data,
            data.len() - 10,
            10,
        );
    }

    #[test]
    fn deflate_with_preset_dict_handles_short_and_repetitive_payloads() {
        round_trip(&DeflateWithPresetDictCompressionMode, b"abc", 0, 3);
        round_trip(&DeflateWithPresetDictCompressionMode, b"", 0, 0);
        let repetitive = vec![3u8; 9_000];
        round_trip(
            &DeflateWithPresetDictCompressionMode,
            &repetitive,
            0,
            repetitive.len(),
        );
        round_trip(
            &DeflateWithPresetDictCompressionMode,
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
        for mode in [&**HIGH_COMPRESSION, &DeflateWithPresetDictCompressionMode] {
            let mut compressor = mode.new_compressor();
            let mut output = VecDataOutput::new();
            let mut buffered = ByteBuffersDataOutput::with_expected_size(data.len());
            buffered.write_bytes(&data, 0, data.len()).unwrap();
            compressor.compress(&buffered, &mut output).unwrap();
            let bytes = output.into_inner();
            assert!(
                !bytes.windows(2).any(|window| window == [0x78, 0x9c]),
                "{} must not emit a zlib header",
                mode.to_string()
            );
        }
    }

    #[test]
    fn the_two_deflate_modes_are_different_formats() {
        let data: Vec<u8> = (0..9_000).map(|i| (i % 97) as u8).collect();
        let compress = |mode: &dyn CompressionMode| {
            let mut compressor = mode.new_compressor();
            let mut output = VecDataOutput::new();
            let mut buffered = ByteBuffersDataOutput::with_expected_size(data.len());
            buffered.write_bytes(&data, 0, data.len()).unwrap();
            compressor.compress(&buffered, &mut output).unwrap();
            output.into_inner()
        };
        assert_ne!(
            compress(&**HIGH_COMPRESSION),
            compress(&DeflateWithPresetDictCompressionMode)
        );
    }

    /// `CompressionMode` is open, as Java's abstract class is
    /// (`CompressionMode.java:37,118,121`): a user mode supplies its own
    /// compressor and decompressor, and a format built with it uses them.
    #[test]
    fn a_user_compression_mode_round_trips() {
        /// Stores every byte inverted, framed as `NO_COMPRESSION` frames it.
        #[derive(Debug)]
        struct Inverting;

        #[derive(Debug, Clone, Copy)]
        struct InvertingCompressor;

        impl Compressor for InvertingCompressor {
            fn compress(
                &mut self,
                input: &ByteBuffersDataOutput,
                out: &mut dyn DataOutput,
            ) -> Result<()> {
                let bytes: Vec<u8> = input.to_array_copy().iter().map(|b| !b).collect();
                out.write_bytes(&bytes, 0, bytes.len())
            }
        }

        #[derive(Debug, Clone, Copy)]
        struct InvertingDecompressor;

        impl Decompressor for InvertingDecompressor {
            fn decompress(
                &mut self,
                input: &mut dyn DataInput,
                _original_length: usize,
                offset: usize,
                length: usize,
                bytes: &mut BytesRef,
            ) -> Result<()> {
                input.skip_bytes(offset as i64)?;
                let mut raw = vec![0u8; length];
                input.read_bytes(&mut raw, 0, length)?;
                *bytes = BytesRef::new(raw.iter().map(|b| !b).collect());
                Ok(())
            }

            fn clone_decompressor(&self) -> Box<dyn Decompressor> {
                Box::new(*self)
            }
        }

        impl CompressionMode for Inverting {
            fn new_compressor(&self) -> Box<dyn Compressor> {
                Box::new(InvertingCompressor)
            }

            fn new_decompressor(&self) -> Box<dyn Decompressor> {
                Box::new(InvertingDecompressor)
            }
        }

        let data: Vec<u8> = (0..1_000).map(|i| (i % 251) as u8).collect();
        round_trip(&Inverting, &data, 0, data.len());
        round_trip(&Inverting, &data, 100, 300);

        let mode: Arc<dyn CompressionMode> = Arc::new(Inverting);
        assert!(same_mode(&mode, &Arc::clone(&mode)));
        assert!(!same_mode(&mode, &FAST));
        assert!(same_mode(&FAST, &FAST));

        // A mode that declares no `toString()` inherits `Object.toString()`:
        // its class name, `@`, and its identity in hexadecimal.
        let rendered = mode.to_string();
        let identity = format!("{:x}", Arc::as_ptr(&mode).cast::<()>() as usize);
        assert_eq!(
            rendered,
            format!("{}@{identity}", std::any::type_name::<Inverting>())
        );
    }

    #[test]
    fn compression_mode_names_match_the_java_to_string() {
        assert_eq!(FAST.to_string(), "FAST");
        assert_eq!(HIGH_COMPRESSION.to_string(), "HIGH_COMPRESSION");
        assert_eq!(FAST_DECOMPRESSION.to_string(), "FAST_DECOMPRESSION");
        assert_eq!(
            &LZ4WithPresetDictCompressionMode.to_string(),
            "BEST_SPEED",
            "LZ4WithPresetDictCompressionMode.toString() returns BEST_SPEED"
        );
        assert_eq!(
            &DeflateWithPresetDictCompressionMode.to_string(),
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

        let mut merge_state = MergeState::from_fields_producers(vec![], vec![0, 0]);
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

        let mut merge_state = MergeState::from_fields_producers(vec![], vec![0]);
        merge_state.field_infos = vec![FieldInfos::new(vec![fi_b0]).unwrap()];
        merge_state.merge_field_infos = merged;

        let matching = MatchingReaders::new(&merge_state);
        assert_eq!(matching.count, 0);
        assert!(!matching.matching_readers[0]);
    }

    /// An info stream that keeps every message of the components it enables.
    struct RecordingInfoStream {
        component: &'static str,
        messages: std::sync::Mutex<Vec<String>>,
    }

    impl crate::util::InfoStream for RecordingInfoStream {
        fn java_class_name(&self) -> &'static str {
            "org.apache.lucene.tests.util.RecordingInfoStream"
        }

        fn message(&self, component: &str, message: &str) {
            assert_eq!(component, self.component, "only enabled components log");
            self.messages.lock().unwrap().push(message.to_string());
        }

        fn is_enabled(&self, component: &str) -> bool {
            component == self.component
        }

        fn close(&self) {}
    }

    /// `MatchingReaders` reports how many readers can be bulk-merged on the
    /// `SM` component, and how many cannot when that is not all of them, with
    /// Java's exact text (`MatchingReaders.java:67-72`).
    #[test]
    fn matching_readers_reports_the_matched_count_on_the_info_stream() {
        let fields = || FieldInfos::new(vec![FieldInfo::new("a", 0), FieldInfo::new("b", 1)]);
        let renumbered = FieldInfos::new(vec![FieldInfo::new("b", 0)]).unwrap();

        let run = |field_infos: Vec<FieldInfos>| {
            let stream = std::sync::Arc::new(RecordingInfoStream {
                component: "SM",
                messages: std::sync::Mutex::new(Vec::new()),
            });
            let max_docs = vec![0; field_infos.len()];
            let mut merge_state = MergeState::from_fields_producers(vec![], max_docs);
            merge_state.field_infos = field_infos;
            merge_state.merge_field_infos = fields().unwrap();
            merge_state.info_stream = stream.clone();
            MatchingReaders::new(&merge_state);
            let messages = stream.messages.lock().unwrap().clone();
            messages
        };

        assert_eq!(
            run(vec![fields().unwrap(), fields().unwrap()]),
            vec!["merge store matchedCount=2 vs 2".to_string()],
            "every reader matches, so there is nothing more to say"
        );
        assert_eq!(
            run(vec![fields().unwrap(), renumbered.clone(), renumbered]),
            vec![
                "merge store matchedCount=1 vs 3".to_string(),
                "2 non-bulk merges".to_string()
            ]
        );
    }
}
