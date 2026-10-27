//! Lucene 9.0 stored-fields format implementation.
//!
//! Ports `Lucene90StoredFieldsFormat` and the underlying
//! `Lucene90CompressingStoredFieldsReader` / `Lucene90CompressingStoredFieldsWriter`
//! classes from Apache Lucene Core 10.5.0.
//!
//! The format writes three files per segment:
//!
//! * `.fdt` – compressed chunks of serialized documents.
//! * `.fdx` – two `DirectMonotonic` arrays mapping chunk start doc IDs and file
//!   pointers.
//! * `.fdm` – metadata for the monotonic arrays plus chunk/dirty counters.
//!
//! Two compression modes are supported, matching Lucene's `Mode`:
//!
//! * `Mode::BestSpeed` – LZ4 over a preset dictionary
//!   ([`CompressionMode::LZ4_WITH_PRESET_DICT`]), 80 KiB target chunks, 1024
//!   documents per chunk. This is what `Lucene104Codec` selects, and its bytes
//!   are verified against Lucene 10.5.0 in
//!   `tests/portability/stored_fields.rs`.
//! * `Mode::BestCompression` – raw deflate over a preset dictionary
//!   ([`CompressionMode::DEFLATE_WITH_PRESET_DICT`]), 480 KiB target chunks,
//!   4096 documents per chunk. Reachable through `Lucene104Codec::with_mode`.
//!
//! # What "compatible" means per mode
//!
//! `BEST_SPEED` compresses with `org.apache.lucene.util.compress.LZ4`, an
//! encoder Lucene specifies itself and this crate ports directly, so the files
//! are **byte-identical** to Lucene's. `tests/portability/stored_fields.rs`
//! asserts exactly that.
//!
//! `BEST_COMPRESSION` delegates to `java.util.zip.Deflater`, that is, to
//! whichever zlib the JVM happens to be linked against. Deflate output is
//! implementation-defined within RFC 1951 — two zlib builds, or zlib and
//! zlib-ng, legitimately emit different bytes for the same input — so byte
//! equality is not a property Lucene itself guarantees across JVMs.
//!
//! On the default `zlib-rs` backend this port therefore does not claim it. What
//! *is* guaranteed, and what the portability tests prove in both directions, is
//! that Lucene 10.5.0 reads every byte this port writes and this port reads
//! every byte Lucene writes; a ratio guard keeps the compression itself within
//! 10% of Lucene's. Building with `--features zlib-c` links the same C zlib
//! family the JVM uses and *does* give byte identity, which the same tests then
//! assert.
//!
//! Where the two differ is narrow and measured. Comparing a `zlib-rs`
//! `BEST_COMPRESSION` `.fdt` with Lucene's for the same content and segment id,
//! everything structural is identical: the chunk header, the `numStoredFields`
//! and `lengths` arrays, `dictLength`, `blockLength` and the count of one
//! dictionary plus ten sub blocks. What differs is the deflate payloads **and
//! the per-block `vInt` length prefixes that describe their sizes**, since
//! those prefixes record how long each payload turned out to be.

use std::cmp;
use std::fmt;
use std::sync::atomic::AtomicI32;
use std::sync::{Arc, Mutex};

use crate::codecs::codec_util::{
    check_footer, check_footer_with_prior, check_header, check_index_header, checksum_entire_file,
    retrieve_checksum, write_footer, write_header, write_index_header,
};
use crate::codecs::compressing::{CompressionMode, Compressor, Decompressor, MatchingReaders};
use crate::codecs::stored_fields::{
    merge_visitor_remapper, MergeVisitor, StoredFieldsFormat, StoredFieldsReader,
    StoredFieldsWriter,
};
use crate::codecs::stub::{
    FieldInfo, FieldInfos, SegmentInfo, StoredFieldVisitor, StoredFieldVisitorStatus,
};
use crate::error::{LuceneError, Result};
use crate::index::{
    segment_file_name, DocIDMerger, DocIDMergerSub, DocMap, MergeState, StoredFieldDataInput,
};
use crate::store::{
    ByteArrayDataInput, ByteBuffersDataOutput, DataInput, DataOutput, Directory, IOContext,
    IndexInput, IndexOutput,
};
use crate::util::packed::{DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter};
use crate::util::Accountable;
use crate::util::BitUtil;
use crate::util::BytesRef;

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

const FIELDS_EXTENSION: &str = "fdt";
const INDEX_EXTENSION: &str = "fdx";
const META_EXTENSION: &str = "fdm";

const INDEX_CODEC_NAME: &str = "Lucene90FieldsIndex";

const TYPE_STRING: i32 = 0x00;
const TYPE_BYTE_ARR: i32 = 0x01;
const TYPE_NUMERIC_INT: i32 = 0x02;
const TYPE_NUMERIC_FLOAT: i32 = 0x03;
const TYPE_NUMERIC_LONG: i32 = 0x04;
const TYPE_NUMERIC_DOUBLE: i32 = 0x05;

// bits required to store the largest type constant
const TYPE_BITS: i32 = 3;
const TYPE_MASK: i32 = (1 << TYPE_BITS) - 1;

// version written into the data file
const DATA_VERSION_START: i32 = 1;
const DATA_VERSION_CURRENT: i32 = DATA_VERSION_START;

// version written into the meta and index files
const INDEX_VERSION_START: i32 = 0;
const INDEX_VERSION_CURRENT: i32 = INDEX_VERSION_START;
/// Oldest version the `.fdm` metadata header may carry.
///
/// Equivalent to `Lucene90CompressingStoredFieldsWriter.META_VERSION_START`.
/// The metadata header is *written* with [`DATA_VERSION_CURRENT`], not with
/// [`INDEX_VERSION_CURRENT`]: it belongs to the stored-fields data, and only
/// the `.fdx` file carries the fields-index version.
const META_VERSION_START: i32 = 0;

// timestamp compression multipliers
const SECOND_MS: i64 = 1000;
const HOUR_MS: i64 = 60 * 60 * SECOND_MS;
const DAY_MS: i64 = 24 * HOUR_MS;
const SECOND_ENCODING: i32 = 0x40;
const HOUR_ENCODING: i32 = 0x80;
const DAY_ENCODING: i32 = 0xC0;

const NEGATIVE_ZERO_FLOAT_BITS: u32 = 0x80000000;
const NEGATIVE_ZERO_DOUBLE_BITS: u64 = 0x8000000000000000;

const STORED_FIELDS_INTS_BLOCK_SIZE: usize = 128;

// -----------------------------------------------------------------------------
// StoredFieldsInts
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.codecs.lucene90.compressing.StoredFieldsInts`.
///
/// Packed integer encoding for the per-chunk `numStoredFields` and
/// `lengths` arrays.
struct StoredFieldsInts;

impl StoredFieldsInts {
    /// Writes `count` non-negative integers from `values[start..]`.
    fn write_ints(
        values: &[i32],
        start: usize,
        count: usize,
        out: &mut dyn DataOutput,
    ) -> Result<()> {
        if count == 0 {
            return Ok(());
        }

        let mut all_equal = true;
        for i in 1..count {
            if values[start + i] != values[start] {
                all_equal = false;
                break;
            }
        }

        if all_equal {
            out.write_byte(0)?;
            out.write_v_int(values[start])?;
            return Ok(());
        }

        let mut max: u32 = 0;
        for i in 0..count {
            max |= values[start + i] as u32;
        }

        if max <= 0xff {
            out.write_byte(8)?;
            Self::write_ints8(out, count, values, start)?;
        } else if max <= 0xffff {
            out.write_byte(16)?;
            Self::write_ints16(out, count, values, start)?;
        } else {
            out.write_byte(32)?;
            Self::write_ints32(out, count, values, start)?;
        }
        Ok(())
    }

    fn write_ints8(
        out: &mut dyn DataOutput,
        count: usize,
        values: &[i32],
        offset: usize,
    ) -> Result<()> {
        let mut k = 0usize;
        while k + STORED_FIELDS_INTS_BLOCK_SIZE - 1 < count {
            let step = offset + k;
            for i in 0..16 {
                let l: u64 = ((values[step + i] as u64) << 56)
                    | ((values[step + 16 + i] as u64) << 48)
                    | ((values[step + 32 + i] as u64) << 40)
                    | ((values[step + 48 + i] as u64) << 32)
                    | ((values[step + 64 + i] as u64) << 24)
                    | ((values[step + 80 + i] as u64) << 16)
                    | ((values[step + 96 + i] as u64) << 8)
                    | (values[step + 112 + i] as u64);
                Self::write_le_long(out, l)?;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            out.write_byte(values[offset + k] as u8)?;
        }
        Ok(())
    }

    fn write_ints16(
        out: &mut dyn DataOutput,
        count: usize,
        values: &[i32],
        offset: usize,
    ) -> Result<()> {
        let mut k = 0usize;
        while k + STORED_FIELDS_INTS_BLOCK_SIZE - 1 < count {
            let step = offset + k;
            for i in 0..32 {
                let l: u64 = ((values[step + i] as u64) << 48)
                    | ((values[step + 32 + i] as u64) << 32)
                    | ((values[step + 64 + i] as u64) << 16)
                    | (values[step + 96 + i] as u64);
                Self::write_le_long(out, l)?;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            Self::write_le_short(out, values[offset + k] as u16)?;
        }
        Ok(())
    }

    fn write_ints32(
        out: &mut dyn DataOutput,
        count: usize,
        values: &[i32],
        offset: usize,
    ) -> Result<()> {
        let mut k = 0usize;
        while k + STORED_FIELDS_INTS_BLOCK_SIZE - 1 < count {
            let step = offset + k;
            for i in 0..64 {
                let l: u64 = ((values[step + i] as u64) << 32) | (values[step + 64 + i] as u64);
                Self::write_le_long(out, l)?;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            Self::write_le_int(out, values[offset + k] as u32)?;
        }
        Ok(())
    }

    // Java's `DataOutput.writeShort/writeInt/writeLong` and the matching
    // `DataInput` readers are **little-endian**, and every fixed-width value of
    // this format goes through them. The helpers below exist only to spell that
    // out at each call site; they are the plain little-endian primitives.

    fn write_le_short(out: &mut dyn DataOutput, s: u16) -> Result<()> {
        out.write_short(s as i16)
    }

    fn write_le_int(out: &mut dyn DataOutput, i: u32) -> Result<()> {
        out.write_int(i as i32)
    }

    fn write_le_long(out: &mut dyn DataOutput, l: u64) -> Result<()> {
        out.write_long(l as i64)
    }

    /// Reads `count` integers into `values[offset..]`. Each value is
    /// guaranteed to fit in an unsigned 32-bit range.
    fn read_ints(
        input: &mut dyn DataInput,
        count: usize,
        values: &mut [i64],
        offset: usize,
    ) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let bpv = input.read_byte()?;
        match bpv {
            0 => {
                let v = input.read_v_int()? as i64;
                values[offset..offset + count].fill(v);
            }
            8 => Self::read_ints8(input, count, values, offset)?,
            16 => Self::read_ints16(input, count, values, offset)?,
            32 => Self::read_ints32(input, count, values, offset)?,
            _ => {
                return Err(LuceneError::CorruptIndex(format!(
                    "Unsupported number of bits per value: {bpv}"
                )))
            }
        }
        Ok(())
    }

    fn read_ints8(
        input: &mut dyn DataInput,
        count: usize,
        values: &mut [i64],
        offset: usize,
    ) -> Result<()> {
        let mut k = 0usize;
        while k + STORED_FIELDS_INTS_BLOCK_SIZE - 1 < count {
            let step = offset + k;
            for i in 0..16 {
                let l = Self::read_le_long(input)?;
                values[step + i] = ((l >> 56) & 0xFF) as i64;
                values[step + 16 + i] = ((l >> 48) & 0xFF) as i64;
                values[step + 32 + i] = ((l >> 40) & 0xFF) as i64;
                values[step + 48 + i] = ((l >> 32) & 0xFF) as i64;
                values[step + 64 + i] = ((l >> 24) & 0xFF) as i64;
                values[step + 80 + i] = ((l >> 16) & 0xFF) as i64;
                values[step + 96 + i] = ((l >> 8) & 0xFF) as i64;
                values[step + 112 + i] = (l & 0xFF) as i64;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            values[offset + k] = input.read_byte()? as i64;
        }
        Ok(())
    }

    fn read_ints16(
        input: &mut dyn DataInput,
        count: usize,
        values: &mut [i64],
        offset: usize,
    ) -> Result<()> {
        let mut k = 0usize;
        while k + STORED_FIELDS_INTS_BLOCK_SIZE - 1 < count {
            let step = offset + k;
            for i in 0..32 {
                let l = Self::read_le_long(input)?;
                values[step + i] = ((l >> 48) & 0xFFFF) as i64;
                values[step + 32 + i] = ((l >> 32) & 0xFFFF) as i64;
                values[step + 64 + i] = ((l >> 16) & 0xFFFF) as i64;
                values[step + 96 + i] = (l & 0xFFFF) as i64;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            values[offset + k] = Self::read_le_short(input)? as i64;
        }
        Ok(())
    }

    fn read_ints32(
        input: &mut dyn DataInput,
        count: usize,
        values: &mut [i64],
        offset: usize,
    ) -> Result<()> {
        let mut k = 0usize;
        while k + STORED_FIELDS_INTS_BLOCK_SIZE - 1 < count {
            let step = offset + k;
            for i in 0..64 {
                let l = Self::read_le_long(input)?;
                values[step + i] = ((l >> 32) & 0xFFFFFFFF) as i64;
                values[step + 64 + i] = (l & 0xFFFFFFFF) as i64;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            values[offset + k] = Self::read_le_int(input)? as i64;
        }
        Ok(())
    }

    fn read_le_short(input: &mut dyn DataInput) -> Result<u16> {
        Ok(input.read_short()? as u16)
    }

    fn read_le_int(input: &mut dyn DataInput) -> Result<u32> {
        Ok(input.read_int()? as u32)
    }

    fn read_le_long(input: &mut dyn DataInput) -> Result<u64> {
        Ok(input.read_long()? as u64)
    }
}

// -----------------------------------------------------------------------------
// FieldsIndex
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.codecs.lucene90.compressing.FieldsIndex`.
///
/// Doc-id to block-offset mapping used by the stored-fields reader.
pub(crate) trait FieldsIndex: Send + Sync {
    /// Returns the block that contains `doc_id`.
    ///
    /// Equivalent to `FieldsIndex.getBlockID(int)`.
    fn get_block_id(&self, doc_id: i32) -> Result<i64>;

    /// Returns the file pointer where the given block starts.
    ///
    /// Equivalent to `FieldsIndex.getBlockStartPointer(long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the index holds no such
    /// block, where Java's monotonic reader throws an unchecked
    /// `ArrayIndexOutOfBoundsException`.
    fn get_block_start_pointer(&self, block_id: i64) -> Result<i64>;

    /// Returns the byte length of the given block.
    ///
    /// Equivalent to `FieldsIndex.getBlockLength(long)`.
    ///
    /// # Errors
    ///
    /// As [`get_block_start_pointer`](Self::get_block_start_pointer).
    fn get_block_length(&self, block_id: i64) -> Result<i64>;

    /// Returns the file pointer where the block containing `doc_id` starts.
    ///
    /// Equivalent to `FieldsIndex.getStartPointer(int)` (`FieldsIndex.java:34-36`).
    fn get_start_pointer(&self, doc_id: i32) -> Result<i64> {
        let block_id = self.get_block_id(doc_id)?;
        self.get_block_start_pointer(block_id)
    }

    /// Validates the integrity of the index.
    fn check_integrity(&self) -> Result<()>;

    /// Creates an independent copy.
    fn clone_index(&self) -> Result<Box<dyn FieldsIndex>>;

    /// Closes any open resources.
    fn close(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// FieldsIndexReader
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.codecs.lucene90.compressing.FieldsIndexReader`.
///
/// Reader side of the block index.
pub(crate) struct FieldsIndexReader {
    max_doc: i32,
    block_shift: i32,
    num_blocks: usize,
    docs: Vec<i64>,
    start_pointers: Vec<i64>,
    docs_start_pointer: i64,
    start_pointers_start_pointer: i64,
    start_pointers_end_pointer: i64,
    max_pointer: i64,
    /// The `.fdx` file, kept open so that every use clones it rather than
    /// re-opening the file, and so that the reader never needs to remember
    /// which directory it came from. Lucene keeps the same handle.
    index_input: Box<dyn IndexInput>,
}

impl FieldsIndexReader {
    pub(crate) fn new(
        directory: &dyn Directory,
        segment: &str,
        suffix: &str,
        extension: &str,
        codec_name: &str,
        id: [u8; 16],
        meta_in: &mut dyn DataInput,
        io_context: &dyn IOContext,
    ) -> Result<Self> {
        let max_doc = meta_in.read_int()?;
        let block_shift = meta_in.read_int()?;
        let num_values = meta_in.read_int()?;
        // Both come off disk. `FieldsIndexWriter.finish` writes `totalChunks + 1`
        // here (`FieldsIndexWriter.java:120`) and `writeIndex` is called once
        // per chunk with at least one document, so Lucene's own invariant is
        // `numValues <= maxDoc + 1`. Checking it turns a corrupt count into an
        // error instead of a loop that fills two vectors with billions of
        // entries.
        if max_doc < 0 || num_values < 0 || i64::from(num_values) > i64::from(max_doc) + 1 {
            return Err(LuceneError::CorruptIndex(format!(
                "the fields index of {segment}{suffix}.{extension} claims {num_values} chunk \
                 boundaries for {max_doc} documents"
            )));
        }
        let num_values = num_values as usize;
        let docs_start_pointer = meta_in.read_long()?;
        let docs_meta = DirectMonotonicMeta::load(meta_in, num_values as i64, block_shift)?;
        let start_pointers_start_pointer = meta_in.read_long()?;
        let start_pointers_meta =
            DirectMonotonicMeta::load(meta_in, num_values as i64, block_shift)?;
        let start_pointers_end_pointer = meta_in.read_long()?;
        let max_pointer = meta_in.read_long()?;

        let index_file = segment_file_name(segment, suffix, extension);
        let mut index_input = directory.open_input(&index_file, io_context)?;

        let header_name = format!("{codec_name}Idx");
        check_index_header(
            index_input.as_mut(),
            &header_name,
            INDEX_VERSION_START,
            INDEX_VERSION_CURRENT,
            &id,
            suffix,
        )?;
        retrieve_checksum(index_input.as_mut())?;

        let docs_data = Self::read_slice(
            index_input.as_mut(),
            docs_start_pointer,
            start_pointers_start_pointer,
        )?;
        let start_pointers_data = Self::read_slice(
            index_input.as_mut(),
            start_pointers_start_pointer,
            start_pointers_end_pointer,
        )?;

        let docs_reader = DirectMonotonicReader::new(docs_meta, docs_data)?;
        let start_pointers_reader =
            DirectMonotonicReader::new(start_pointers_meta, start_pointers_data)?;

        // Nothing is reserved up front and every read is checked: `num_values`
        // is a file value, and the two monotonic readers are built over slices
        // whose length is a file value too, so an index they cannot serve must
        // be an error rather than the panic `LongValues::get` would raise.
        let mut docs = Vec::new();
        let mut start_pointers = Vec::new();
        for i in 0..num_values {
            docs.push(docs_reader.get_checked(i as i64)?);
            start_pointers.push(start_pointers_reader.get_checked(i as i64)?);
        }

        Ok(Self {
            max_doc,
            block_shift,
            num_blocks: num_values,
            docs,
            start_pointers,
            docs_start_pointer,
            start_pointers_start_pointer,
            start_pointers_end_pointer,
            max_pointer,
            index_input,
        })
    }

    fn read_slice(input: &mut dyn IndexInput, start: i64, end: i64) -> Result<Vec<u8>> {
        // The bounds come out of the `.fdm` metadata, so they are untrusted.
        // Java reaches this through `IndexInput.slice`, which validates the
        // range against the file length and throws `IllegalArgumentException`
        // without allocating anything; allocating first would let a corrupt
        // pointer pair request petabytes and abort the process on a failed
        // allocation, which no exception can catch.
        let length = input.length();
        if start < 0 || end < start || end > length {
            return Err(LuceneError::CorruptIndex(format!(
                "slice [{start}, {end}) is outside the {length}-byte fields index"
            )));
        }
        let len = (end - start) as usize;
        let mut buf = vec![0u8; len];
        input.seek(start)?;
        input.read_bytes(&mut buf, 0, len)?;
        Ok(buf)
    }
}

impl fmt::Debug for FieldsIndexReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldsIndexReader")
            .field("max_doc", &self.max_doc)
            .field("block_shift", &self.block_shift)
            .field("num_blocks", &self.num_blocks)
            .field("max_pointer", &self.max_pointer)
            .finish_non_exhaustive()
    }
}

impl FieldsIndexReader {
    pub(crate) fn max_pointer(&self) -> i64 {
        self.max_pointer
    }
}

impl FieldsIndexReader {
    /// Returns the value `values` holds for `block_id`, or a corrupt-index
    /// error for a block this index does not hold.
    ///
    /// Java reads it through `DirectMonotonicReader.get(long)`, which throws an
    /// unchecked `ArrayIndexOutOfBoundsException` for such a block — a hostile
    /// `.fdm` that declares no chunks for a non-empty segment reaches it with
    /// block `-1` — so Rust's indexing would panic where Java throws.
    fn block_value(&self, values: &[i64], block_id: i64) -> Result<i64> {
        usize::try_from(block_id)
            .ok()
            .and_then(|block| values.get(block))
            .copied()
            .ok_or_else(|| {
                LuceneError::CorruptIndex(format!(
                    "block {block_id} is outside the {} blocks of the fields index",
                    self.num_blocks
                ))
            })
    }
}

impl FieldsIndex for FieldsIndexReader {
    /// Equivalent to `FieldsIndexReader.getBlockID(int)`
    /// (`FieldsIndexReader.java:119-127`): `docs.binarySearch(0, numChunks,
    /// docID)`, an exact match or `-2 - (-1 - insertionPoint)`, the block
    /// before the insertion point. An index without chunks answers `-1`, as
    /// Java's does.
    fn get_block_id(&self, doc_id: i32) -> Result<i64> {
        if doc_id < 0 || doc_id >= self.max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "doc_id {doc_id} out of bounds [0, {})",
                self.max_doc
            )));
        }

        // `DirectMonotonicReader.binarySearch(long, long, long)` over the
        // decoded values; `num_blocks` is at most `maxDoc + 1`, so neither the
        // bounds nor their sum overflow.
        let key = i64::from(doc_id);
        let mut lo: i64 = 0;
        let mut hi: i64 = self.num_blocks as i64 - 1;
        let mut block_index: i64 = -1;
        while lo <= hi {
            let mid = (lo + hi) >> 1;
            match self.docs[mid as usize].cmp(&key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid - 1,
                std::cmp::Ordering::Equal => {
                    block_index = mid;
                    break;
                }
            }
        }
        if lo > hi {
            block_index = -1 - lo;
        }
        if block_index < 0 {
            block_index = -2 - block_index;
        }
        Ok(block_index)
    }

    /// Equivalent to `FieldsIndexReader.getBlockStartPointer(long)`
    /// (`FieldsIndexReader.java:129-132`).
    fn get_block_start_pointer(&self, block_id: i64) -> Result<i64> {
        self.block_value(&self.start_pointers, block_id)
    }

    /// Equivalent to `FieldsIndexReader.getBlockLength(long)`
    /// (`FieldsIndexReader.java:134-143`).
    fn get_block_length(&self, block_id: i64) -> Result<i64> {
        let end_pointer = if block_id == self.num_blocks as i64 - 1 {
            self.max_pointer
        } else {
            self.block_value(&self.start_pointers, block_id + 1)?
        };
        Ok(end_pointer - self.get_block_start_pointer(block_id)?)
    }

    fn check_integrity(&self) -> Result<()> {
        let mut input = self.index_input.clone_input()?;
        input.seek(0)?;
        checksum_entire_file(input.as_mut())?;
        Ok(())
    }

    fn clone_index(&self) -> Result<Box<dyn FieldsIndex>> {
        Ok(Box::new(Self {
            max_doc: self.max_doc,
            block_shift: self.block_shift,
            num_blocks: self.num_blocks,
            docs: self.docs.clone(),
            start_pointers: self.start_pointers.clone(),
            docs_start_pointer: self.docs_start_pointer,
            start_pointers_start_pointer: self.start_pointers_start_pointer,
            start_pointers_end_pointer: self.start_pointers_end_pointer,
            max_pointer: self.max_pointer,
            index_input: self.index_input.clone_input()?,
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Rewrites the metadata file `meta_file` so that its fields index declares no
/// chunks, and recomputes the footer checksum: the hostile `.fdm`/`.tvm` that
/// reaches `FieldsIndexReader.getBlockID(int)` with `numChunks == 0` for a
/// non-empty segment, which Lucene opens.
///
/// `vints_before` is how many variable-length ints the format writes between
/// the index header and the fields-index metadata: one (`chunkSize`) for stored
/// fields, two (`packedIntsVersion`, `chunkSize`) for term vectors. Each
/// monotonic array must fit one 21-byte block of `DirectMonotonicWriter`
/// metadata (`min`, `avgInc`, `offset`, `bitsRequired`), which is removed.
#[cfg(test)]
pub(crate) fn declare_no_fields_index_chunks(
    dir: &dyn Directory,
    meta_file: &str,
    vints_before: usize,
) {
    const MONOTONIC_BLOCK_META: usize = 8 + 4 + 8 + 1;
    let ctx = &*crate::store::DEFAULT_IO_CONTEXT;
    let len = dir.file_length(meta_file).unwrap() as usize;
    let mut bytes = vec![0u8; len];
    dir.open_input(meta_file, ctx)
        .unwrap()
        .read_bytes(&mut bytes, 0, len)
        .unwrap();
    // Index header: magic, codec name, version, segment id, suffix.
    let mut at = 4;
    at += 1 + bytes[at] as usize;
    at += 4 + 16;
    at += 1 + bytes[at] as usize;
    for _ in 0..vints_before {
        while bytes[at] & 0x80 != 0 {
            at += 1;
        }
        at += 1;
    }
    let int_at = |at: usize| i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let (max_doc, block_shift, num_values) = (int_at(at), int_at(at + 4), int_at(at + 8));
    assert!(max_doc > 0 && num_values > 0 && num_values <= 1 << block_shift);
    let num_values_at = at + 8;
    let docs_meta_at = num_values_at + 4 + 8;
    bytes.drain(docs_meta_at..docs_meta_at + MONOTONIC_BLOCK_META);
    let start_pointers_meta_at = docs_meta_at + 8;
    bytes.drain(start_pointers_meta_at..start_pointers_meta_at + MONOTONIC_BLOCK_META);
    bytes[num_values_at..num_values_at + 4].copy_from_slice(&0i32.to_le_bytes());
    let body = bytes.len() - 8;
    let checksum = u64::from(crc32fast::hash(&bytes[..body]));
    bytes[body..].copy_from_slice(&checksum.to_be_bytes());
    dir.delete_file(meta_file).unwrap();
    let mut out = dir.create_output(meta_file, ctx).unwrap();
    out.write_bytes(&bytes, 0, bytes.len()).unwrap();
    out.close().unwrap();
}

// -----------------------------------------------------------------------------
// FieldsIndexWriter
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.codecs.lucene90.compressing.FieldsIndexWriter`.
///
/// Writer side of the block index. As in Lucene, the per-chunk document
/// counts and start-pointer deltas are streamed to two temporary outputs
/// (`FieldsIndexWriter.java:83-87`) and only turned into the index file — two
/// monotonic arrays — by [`finish`](Self::finish), which then deletes the
/// temporary files. A writer that is closed without finishing, such as the one
/// of an aborted flush, deletes them and leaves no index file behind.
pub(crate) struct FieldsIndexWriter {
    dir: Arc<dyn Directory>,
    name: String,
    suffix: String,
    extension: String,
    codec_name: String,
    id: [u8; 16],
    block_shift: i32,
    io_context: Box<dyn IOContext>,
    /// `docsOut`: the output while open, and its name until the file is
    /// deleted. Java keeps one nullable field; Rust closes an output by value,
    /// so the two halves of its life are kept apart.
    docs_out: Option<Box<dyn IndexOutput>>,
    docs_out_name: Option<String>,
    /// `filePointersOut`, kept as [`Self::docs_out`] is.
    file_pointers_out: Option<Box<dyn IndexOutput>>,
    file_pointers_out_name: Option<String>,
    total_docs: i32,
    total_chunks: i32,
    previous_fp: i64,
}

impl fmt::Debug for FieldsIndexWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldsIndexWriter")
            .field("name", &self.name)
            .field("block_shift", &self.block_shift)
            .field("total_chunks", &self.total_chunks)
            .field("total_docs", &self.total_docs)
            .finish_non_exhaustive()
    }
}

impl FieldsIndexWriter {
    /// Opens the two temporary outputs and writes their headers.
    ///
    /// Equivalent to the `FieldsIndexWriter` constructor
    /// (`FieldsIndexWriter.java:64-93`): when a header or the second output
    /// fails, the outputs opened so far are closed and deleted.
    ///
    /// # Errors
    ///
    /// Propagates what creating or writing a temporary output raised.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        dir: &Arc<dyn Directory>,
        name: &str,
        suffix: &str,
        extension: &str,
        codec_name: &str,
        id: [u8; 16],
        block_shift: i32,
        io_context: &dyn IOContext,
    ) -> Result<Self> {
        let docs_out =
            dir.create_temp_output(name, &format!("{codec_name}-doc_ids"), io_context)?;
        let docs_out_name = docs_out.name().to_string();
        let mut writer = Self {
            dir: Arc::clone(dir),
            name: name.to_string(),
            suffix: suffix.to_string(),
            extension: extension.to_string(),
            codec_name: codec_name.to_string(),
            id,
            block_shift,
            io_context: io_context.with_hints(io_context.hints()),
            docs_out: Some(docs_out),
            docs_out_name: Some(docs_out_name),
            file_pointers_out: None,
            file_pointers_out_name: None,
            total_docs: 0,
            total_chunks: 0,
            previous_fp: 0,
        };
        let opened = (|| -> Result<()> {
            let docs_out = writer
                .docs_out
                .as_mut()
                .expect("INVARIANT: docs_out was just opened");
            write_header(
                docs_out.as_mut(),
                &format!("{codec_name}Docs"),
                INDEX_VERSION_CURRENT,
            )?;
            let mut file_pointers_out = writer.dir.create_temp_output(
                name,
                &format!("{codec_name}file_pointers"),
                io_context,
            )?;
            writer.file_pointers_out_name = Some(file_pointers_out.name().to_string());
            let written = write_header(
                file_pointers_out.as_mut(),
                &format!("{codec_name}FilePointers"),
                INDEX_VERSION_CURRENT,
            );
            writer.file_pointers_out = Some(file_pointers_out);
            written
        })();
        if let Err(error) = opened {
            let _ = writer.close();
            return Err(error);
        }
        Ok(writer)
    }

    /// Records one chunk.
    ///
    /// Equivalent to `FieldsIndexWriter.writeIndex(int, long)`.
    ///
    /// # Errors
    ///
    /// Propagates what writing to a temporary output raised.
    pub(crate) fn write_index(&mut self, num_docs: i32, start_pointer: i64) -> Result<()> {
        debug_assert!(start_pointer >= self.previous_fp);
        Self::open_output(&mut self.docs_out)?.write_v_int(num_docs)?;
        Self::open_output(&mut self.file_pointers_out)?
            .write_v_long(start_pointer - self.previous_fp)?;
        self.previous_fp = start_pointer;
        self.total_docs += num_docs;
        self.total_chunks += 1;
        Ok(())
    }

    fn open_output(
        output: &mut Option<Box<dyn IndexOutput>>,
    ) -> Result<&mut (dyn IndexOutput + 'static)> {
        output
            .as_deref_mut()
            .ok_or_else(|| LuceneError::AlreadyClosed("this FieldsIndexWriter is closed".into()))
    }

    /// Writes the index file from the two temporary outputs, then deletes
    /// them.
    ///
    /// Equivalent to `FieldsIndexWriter.finish(int, long, IndexOutput)`
    /// (`FieldsIndexWriter.java:105-181`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when `num_docs` is not the number
    /// of documents indexed, [`LuceneError::CorruptIndex`] when a temporary
    /// file does not read back as written, and propagates I/O failures.
    pub(crate) fn finish(
        &mut self,
        num_docs: i32,
        max_pointer: i64,
        meta_out: &mut dyn IndexOutput,
    ) -> Result<()> {
        if num_docs != self.total_docs {
            return Err(LuceneError::IllegalState(format!(
                "Expected {num_docs} docs, but got {}",
                self.total_docs
            )));
        }
        // `CodecUtil.writeFooter(docsOut); CodecUtil.writeFooter(filePointersOut);
        // IOUtils.close(docsOut, filePointersOut);`
        write_footer(Self::open_output(&mut self.docs_out)?)?;
        write_footer(Self::open_output(&mut self.file_pointers_out)?)?;
        let closed = [
            self.docs_out.take().map_or(Ok(()), |mut out| out.close()),
            self.file_pointers_out
                .take()
                .map_or(Ok(()), |mut out| out.close()),
        ];
        closed.into_iter().find(Result::is_err).unwrap_or(Ok(()))?;

        let data_file = segment_file_name(&self.name, &self.suffix, &self.extension);
        let mut data_out = self
            .dir
            .create_output(&data_file, self.io_context.as_ref())?;
        // `try (IndexOutput dataOut = ...)`: closed whatever the body does.
        let written = self.write_data(data_out.as_mut(), num_docs, max_pointer, meta_out);
        let closed = data_out.close();
        written.and(closed)
    }

    /// The body of `finish`'s `try (IndexOutput dataOut = ...)`.
    fn write_data(
        &mut self,
        data_out: &mut dyn IndexOutput,
        num_docs: i32,
        max_pointer: i64,
        meta_out: &mut dyn IndexOutput,
    ) -> Result<()> {
        let codec_name = self.codec_name.clone();
        write_index_header(
            data_out,
            &format!("{codec_name}Idx"),
            INDEX_VERSION_CURRENT,
            &self.id,
            &self.suffix,
        )?;

        let num_values = i64::from(self.total_chunks) + 1;
        meta_out.write_int(num_docs)?;
        meta_out.write_int(self.block_shift)?;
        meta_out.write_int(self.total_chunks + 1)?;
        meta_out.write_long(data_out.file_pointer())?;

        let docs_name = self
            .docs_out_name
            .clone()
            .expect("INVARIANT: the doc ids file exists until finish deletes it");
        {
            let mut docs_in = self.dir.open_checksum_input(&docs_name)?;
            check_header(
                docs_in.as_mut(),
                &format!("{codec_name}Docs"),
                INDEX_VERSION_CURRENT,
                INDEX_VERSION_CURRENT,
            )?;
            let read = (|| -> Result<()> {
                let mut docs =
                    DirectMonotonicWriter::new(meta_out, data_out, num_values, self.block_shift)?;
                let mut doc = 0i64;
                docs.add(doc)?;
                for _ in 0..self.total_chunks {
                    doc += i64::from(docs_in.read_v_int()?);
                    docs.add(doc)?;
                }
                docs.finish()?;
                if doc != i64::from(self.total_docs) {
                    return Err(LuceneError::CorruptIndex(format!(
                        "Docs don't add up (resource={docs_name})"
                    )));
                }
                Ok(())
            })();
            // `CodecUtil.checkFooter(docsIn, priorE)`.
            match read {
                Ok(()) => {
                    check_footer(docs_in.as_mut())?;
                }
                Err(prior) => return Err(check_footer_with_prior(docs_in.as_mut(), prior)),
            }
        }
        self.dir.delete_file(&docs_name)?;
        self.docs_out_name = None;

        meta_out.write_long(data_out.file_pointer())?;
        let file_pointers_name = self
            .file_pointers_out_name
            .clone()
            .expect("INVARIANT: the file pointers file exists until finish deletes it");
        {
            let mut file_pointers_in = self.dir.open_checksum_input(&file_pointers_name)?;
            check_header(
                file_pointers_in.as_mut(),
                &format!("{codec_name}FilePointers"),
                INDEX_VERSION_CURRENT,
                INDEX_VERSION_CURRENT,
            )?;
            let read = (|| -> Result<()> {
                let mut file_pointers =
                    DirectMonotonicWriter::new(meta_out, data_out, num_values, self.block_shift)?;
                let mut fp = 0i64;
                for _ in 0..self.total_chunks {
                    fp += file_pointers_in.read_v_long()?;
                    file_pointers.add(fp)?;
                }
                if max_pointer < fp {
                    return Err(LuceneError::CorruptIndex(format!(
                        "File pointers don't add up (resource={file_pointers_name})"
                    )));
                }
                file_pointers.add(max_pointer)?;
                file_pointers.finish()?;
                Ok(())
            })();
            match read {
                Ok(()) => {
                    check_footer(file_pointers_in.as_mut())?;
                }
                Err(prior) => {
                    return Err(check_footer_with_prior(file_pointers_in.as_mut(), prior));
                }
            }
        }
        self.dir.delete_file(&file_pointers_name)?;
        self.file_pointers_out_name = None;

        meta_out.write_long(data_out.file_pointer())?;
        meta_out.write_long(max_pointer)?;

        write_footer(data_out)
    }

    /// Closes the temporary outputs still open and deletes the temporary
    /// files still there.
    ///
    /// Equivalent to `FieldsIndexWriter.close()` (`FieldsIndexWriter.java:183-201`):
    /// after [`finish`](Self::finish) there is nothing left, so closing twice
    /// is a no-op. Both outputs are closed and both files deleted even when
    /// one step fails; as in a Java `finally`, a failure to delete replaces a
    /// failure to close.
    pub(crate) fn close(&mut self) -> Result<()> {
        let closed = [
            self.docs_out.take().map_or(Ok(()), |mut out| out.close()),
            self.file_pointers_out
                .take()
                .map_or(Ok(()), |mut out| out.close()),
        ]
        .into_iter()
        .find(Result::is_err)
        .unwrap_or(Ok(()));
        // `IOUtils.deleteFiles(dir, fileNames)` tries every file and throws
        // the first failure.
        let deleted = [
            self.docs_out_name.take(),
            self.file_pointers_out_name.take(),
        ]
        .into_iter()
        .flatten()
        .map(|name| self.dir.delete_file(&name))
        .collect::<Vec<_>>()
        .into_iter()
        .find(Result::is_err)
        .unwrap_or(Ok(()));
        deleted.and(closed)
    }
}

// -----------------------------------------------------------------------------
// Lucene90CompressingStoredFieldsWriter
// -----------------------------------------------------------------------------

// bulk merge is scary: it has caused corruption bugs in the past.
// we try to be extra safe with this impl, but keep an escape hatch to work
// around undiscovered bugs.
//
// Divergence: Lucene reads the switch from the system property named by
// `BULK_MERGE_ENABLED_SYSPROP`
// (`Lucene90CompressingStoredFieldsWriter.java:494-509`). Rust has no system
// properties, and reading an environment variable at first use would give the
// constant a different lifetime than Java's static initialiser, so the escape
// hatch is a compile-time constant with Java's default value.
const BULK_MERGE_ENABLED: bool = true;

/// How one source segment's stored fields are copied into the merged one.
///
/// Equivalent to the private enum
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsWriter.MergeStrategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeStrategy {
    /// Copy chunk by chunk in a compressed format.
    Bulk,

    /// Copy document by document in a decompressed format.
    Doc,

    /// Copy field by field of decompressed documents.
    Visitor,
}

/// One source segment of a stored-fields merge, together with the strategy
/// chosen for it.
///
/// Equivalent to the private class
/// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsWriter.CompressingStoredFieldsMergeSub`.
struct CompressingStoredFieldsMergeSub {
    doc_map: DocMap,
    reader_index: usize,
    max_doc: i32,
    merge_strategy: MergeStrategy,
    doc_id: i32,
    mapped_doc_id: i32,
}

impl DocIDMergerSub for CompressingStoredFieldsMergeSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id += 1;
        if self.doc_id == self.max_doc {
            Ok(crate::search::NO_MORE_DOCS)
        } else {
            Ok(self.doc_id)
        }
    }

    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }

    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }

    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// What one step of `DocIDMerger` tells the merge loop.
///
/// Java holds the `CompressingStoredFieldsMergeSub` that `DocIDMerger.next()`
/// returns and compares consecutive steps with `==`, that is by object
/// identity, to find the run of documents that one segment contributes in a
/// row (`Lucene90CompressingStoredFieldsWriter.java:625-634`). A `&mut`
/// reference cannot be held across the next call here, so the loop keeps the
/// sub's position in the merger — which is exactly its identity — together
/// with the fields it reads.
#[derive(Debug, Clone, Copy)]
struct StoredFieldsMergeStep {
    /// Position of the sub inside the merger; stands in for Java's identity
    /// comparison of the sub itself.
    sub_index: usize,
    reader_index: usize,
    doc_id: i32,
    merge_strategy: MergeStrategy,
    mapped_doc_id: i32,
}

/// Advances the merger one document and reports what it landed on, or `None`
/// once every source segment is exhausted.
///
/// Equivalent to `DocIDMerger.next()` as `Lucene90CompressingStoredFieldsWriter#merge`
/// uses it (`Lucene90CompressingStoredFieldsWriter.java:621`, `629`, `639`,
/// `646`).
fn next_stored_fields_step(
    merger: &mut DocIDMerger<CompressingStoredFieldsMergeSub>,
) -> Result<Option<StoredFieldsMergeStep>> {
    if merger.next_sub()?.is_none() {
        return Ok(None);
    }
    let sub_index = merger.current_index().ok_or_else(|| {
        LuceneError::IllegalState("DocIDMerger returned a sub it cannot locate".to_string())
    })?;
    let sub = merger.sub(sub_index);
    Ok(Some(StoredFieldsMergeStep {
        sub_index,
        reader_index: sub.reader_index,
        doc_id: sub.doc_id,
        merge_strategy: sub.merge_strategy,
        mapped_doc_id: sub.mapped_doc_id,
    }))
}

/// Compresses blocks of stored documents.
pub struct Lucene90CompressingStoredFieldsWriter {
    index_writer: FieldsIndexWriter,
    meta_stream: Box<dyn IndexOutput>,
    fields_stream: Box<dyn IndexOutput>,
    compressor: Box<dyn Compressor>,
    compression_mode: CompressionMode,
    chunk_size: usize,
    max_docs_per_chunk: usize,
    buffered_docs: ByteBuffersDataOutput,
    num_stored_fields: Vec<i32>,
    end_offsets: Vec<i32>,
    doc_base: i32,
    num_buffered_docs: usize,
    num_stored_fields_in_doc: i32,
    num_chunks: i64,
    num_dirty_chunks: i64,
    num_dirty_docs: i64,
}

impl Lucene90CompressingStoredFieldsWriter {
    fn new(
        directory: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        context: &dyn IOContext,
        format_name: &str,
        compression_mode: CompressionMode,
        chunk_size: usize,
        max_docs_per_chunk: usize,
        block_shift: i32,
    ) -> Result<Self> {
        let segment = segment_info.name.clone();
        let id = segment_info.id();

        // `boolean success = false; try { ... } finally { if (!success)
        // IOUtils.closeWhileHandlingException(metaStream, fieldsStream,
        // indexWriter); }` (`Lucene90CompressingStoredFieldsWriter.java:127-162`):
        // whatever fails, everything opened so far is closed, and the index
        // writer's close deletes its temporary files.
        let mut meta_stream: Option<Box<dyn IndexOutput>> = None;
        let mut fields_stream: Option<Box<dyn IndexOutput>> = None;
        let mut index_writer: Option<FieldsIndexWriter> = None;
        let opened = (|| -> Result<()> {
            let meta_file = segment_file_name(&segment, segment_suffix, META_EXTENSION);
            let meta = meta_stream.insert(directory.create_output(&meta_file, context)?);
            write_index_header(
                meta.as_mut(),
                &format!("{INDEX_CODEC_NAME}Meta"),
                DATA_VERSION_CURRENT,
                &id,
                segment_suffix,
            )?;

            let fields_file = segment_file_name(&segment, segment_suffix, FIELDS_EXTENSION);
            let fields = fields_stream.insert(directory.create_output(&fields_file, context)?);
            write_index_header(
                fields.as_mut(),
                format_name,
                DATA_VERSION_CURRENT,
                &id,
                segment_suffix,
            )?;

            index_writer = Some(FieldsIndexWriter::new(
                directory,
                &segment,
                segment_suffix,
                INDEX_EXTENSION,
                INDEX_CODEC_NAME,
                id,
                block_shift,
                context,
            )?);

            meta.write_v_int(chunk_size as i32)?;
            Ok(())
        })();
        if let Err(error) = opened {
            if let Some(mut out) = meta_stream {
                let _ = out.close();
            }
            if let Some(mut out) = fields_stream {
                let _ = out.close();
            }
            if let Some(mut writer) = index_writer {
                let _ = writer.close();
            }
            return Err(error);
        }
        let (Some(meta_stream), Some(fields_stream), Some(index_writer)) =
            (meta_stream, fields_stream, index_writer)
        else {
            unreachable!("INVARIANT: a successful open set all three resources");
        };

        Ok(Self {
            index_writer,
            meta_stream,
            fields_stream,
            compressor: compression_mode.new_compressor(),
            compression_mode,
            chunk_size,
            max_docs_per_chunk,
            buffered_docs: ByteBuffersDataOutput::new_resettable_instance(),
            num_stored_fields: vec![0; 16],
            end_offsets: vec![0; 16],
            doc_base: 0,
            num_buffered_docs: 0,
            num_stored_fields_in_doc: 0,
            num_chunks: 0,
            num_dirty_chunks: 0,
            num_dirty_docs: 0,
        })
    }

    fn write_field_header(&mut self, info: &FieldInfo, type_bits: i32) -> Result<()> {
        let info_and_bits = ((info.number as u64) << (TYPE_BITS as u32)) | (type_bits as u64);
        self.buffered_docs.write_v_long(info_and_bits as i64)
    }

    fn save_ints(values: &[i32], length: usize, out: &mut dyn DataOutput) -> Result<()> {
        if length == 1 {
            out.write_v_int(values[0])
        } else {
            StoredFieldsInts::write_ints(values, 0, length, out)
        }
    }

    fn trigger_flush(&self) -> bool {
        self.buffered_docs.size() >= self.chunk_size
            || self.num_buffered_docs >= self.max_docs_per_chunk
    }

    fn write_header(
        &mut self,
        doc_base: i32,
        num_buffered_docs: usize,
        sliced: bool,
        dirty_chunk: bool,
    ) -> Result<()> {
        let sliced_bit = if sliced { 1 } else { 0 };
        let dirty_bit = if dirty_chunk { 2 } else { 0 };
        self.fields_stream.write_v_int(doc_base)?;
        self.fields_stream
            .write_v_int(((num_buffered_docs as i32) << 2) | dirty_bit | sliced_bit)
    }

    /// Compresses and writes the buffered documents as one chunk.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsWriter.flush(boolean)`.
    /// `force` is the caller's statement that the chunk is *not* full — the
    /// end of a segment, or a bulk merge about to copy whole chunks — so it
    /// must be exactly the opposite of [`trigger_flush`](Self::trigger_flush),
    /// which is what Java asserts (`Lucene90CompressingStoredFieldsWriter.java:235`).
    fn flush(&mut self, force: bool) -> Result<()> {
        debug_assert!(self.trigger_flush() != force);
        self.num_chunks += 1;
        if force {
            self.num_dirty_chunks += 1;
            self.num_dirty_docs += self.num_buffered_docs as i64;
        }
        let start_pointer = self.fields_stream.file_pointer();
        self.index_writer
            .write_index(self.num_buffered_docs as i32, start_pointer)?;

        // Transform end offsets into lengths in place.
        for i in (1..self.num_buffered_docs).rev() {
            self.end_offsets[i] -= self.end_offsets[i - 1];
            debug_assert!(self.end_offsets[i] >= 0);
        }

        let sliced = self.buffered_docs.size() >= 2 * self.chunk_size;
        self.write_header(self.doc_base, self.num_buffered_docs, sliced, force)?;
        Self::save_ints(
            &self.num_stored_fields,
            self.num_buffered_docs,
            self.fields_stream.as_mut(),
        )?;
        Self::save_ints(
            &self.end_offsets,
            self.num_buffered_docs,
            self.fields_stream.as_mut(),
        )?;

        if sliced {
            let bytes = self.buffered_docs.to_array_copy();
            let capacity = bytes.len();
            let chunk_size = self.chunk_size;
            for compressed in (0..capacity).step_by(chunk_size) {
                let l = cmp::min(chunk_size, capacity - compressed);
                let mut chunk = ByteBuffersDataOutput::with_expected_size(l);
                chunk.write_bytes(&bytes, compressed, l)?;
                self.compressor
                    .compress(&chunk, self.fields_stream.as_mut())?;
            }
        } else {
            self.compressor
                .compress(&self.buffered_docs, self.fields_stream.as_mut())?;
        }

        self.doc_base += self.num_buffered_docs as i32;
        self.num_buffered_docs = 0;
        self.buffered_docs.reset();
        Ok(())
    }

    /// Returns whether `candidate` should be recompressed rather than
    /// bulk-merged.
    ///
    /// The last chunk written for a segment is typically incomplete, so without
    /// recompressing, in some worst-case situations (frequent reopens with tiny
    /// flushes) the compression ratio degrades over time. A segment counts as
    /// dirty only when it has enough dirty documents to fill a whole block
    /// *and* more than one per cent of its blocks are dirty.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsWriter#tooDirty`.
    fn too_dirty(&self, candidate: &Lucene90CompressingStoredFieldsReader) -> bool {
        candidate.num_dirty_docs() > self.max_docs_per_chunk as i64
            && candidate.num_dirty_chunks() * 100 > candidate.num_chunks()
    }

    /// Decides how one source segment's stored fields should be merged.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsWriter#getMergeStrategy`.
    fn get_merge_strategy(
        &self,
        merge_state: &MergeState,
        matching_readers: &MatchingReaders,
        reader_index: usize,
    ) -> MergeStrategy {
        let candidate = merge_state.stored_fields_readers[reader_index]
            .as_ref()
            .and_then(|reader| reader.as_any())
            .and_then(|any| any.downcast_ref::<Lucene90CompressingStoredFieldsReader>());
        let Some(reader) = candidate else {
            return MergeStrategy::Visitor;
        };
        if !matching_readers.matching_readers[reader_index]
            || reader.version() != DATA_VERSION_CURRENT
        {
            return MergeStrategy::Visitor;
        }
        if BULK_MERGE_ENABLED
            && reader.compression_mode() == self.compression_mode
            && reader.chunk_size() == self.chunk_size
            // it's not worth fine-graining this if there are deletions.
            && merge_state.live_docs[reader_index].is_none()
            && !self.too_dirty(reader)
        {
            MergeStrategy::Bulk
        } else {
            MergeStrategy::Doc
        }
    }

    /// Copies one document from `reader` in its decompressed form, re-encoding
    /// it into this writer's own chunk buffer.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the source chunk is
    /// inconsistent, and propagates any I/O error.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsWriter#copyOneDoc`.
    fn copy_one_doc(
        &mut self,
        reader: &Lucene90CompressingStoredFieldsReader,
        doc_id: i32,
    ) -> Result<()> {
        debug_assert_eq!(reader.version(), DATA_VERSION_CURRENT);
        let doc = reader.serialized_document(doc_id)?;
        self.start_document()?;
        let mut input = ByteArrayDataInput::new(&doc.bytes);
        self.buffered_docs
            .copy_bytes(&mut input, doc.length as i64)?;
        self.num_stored_fields_in_doc = doc.num_stored_fields;
        self.finish_document()?;
        Ok(())
    }

    /// Copies documents `[from_doc_id, to_doc_id)` of one source segment,
    /// moving whole compressed chunks byte for byte wherever a chunk boundary
    /// allows it and falling back to
    /// [`copy_one_doc`](Self::copy_one_doc) for the documents at either end
    /// that do not line up with one.
    ///
    /// Only the chunk header is rewritten, to rebase the document numbers onto
    /// the merged segment; the compressed payload is copied verbatim, which is
    /// what makes a bulk merge cheap.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when a chunk header disagrees with
    /// the stored-fields index, and propagates any I/O error.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsWriter#copyChunks`.
    fn copy_chunks(
        &mut self,
        merge_state: &MergeState,
        sub: &CompressingStoredFieldsMergeSub,
        from_doc_id: i32,
        to_doc_id: i32,
    ) -> Result<()> {
        let reader = merge_state.stored_fields_readers[sub.reader_index]
            .as_ref()
            .and_then(|reader| reader.as_any())
            .and_then(|any| any.downcast_ref::<Lucene90CompressingStoredFieldsReader>())
            .ok_or_else(|| {
                LuceneError::IllegalState(
                    "bulk merge needs a Lucene90CompressingStoredFieldsReader".to_string(),
                )
            })?;
        debug_assert_eq!(reader.version(), DATA_VERSION_CURRENT);
        debug_assert_eq!(reader.chunk_size(), self.chunk_size);
        debug_assert_eq!(reader.compression_mode(), self.compression_mode);
        debug_assert!(!self.too_dirty(reader));
        debug_assert!(merge_state.live_docs[sub.reader_index].is_none());

        let mut doc_id = from_doc_id;

        // copy docs that belong to the previous chunk
        while doc_id < to_doc_id && reader.is_loaded(doc_id)? {
            self.copy_one_doc(reader, doc_id)?;
            doc_id += 1;
        }
        if doc_id >= to_doc_id {
            return Ok(());
        }
        // copy chunks
        let mut from_pointer = reader.index_reader().get_start_pointer(doc_id)?;
        let to_pointer = if to_doc_id == sub.max_doc {
            reader.max_pointer()
        } else {
            reader.index_reader().get_start_pointer(to_doc_id)?
        };
        if from_pointer < to_pointer {
            if self.num_buffered_docs > 0 {
                self.flush(true)?;
            }
            let mut raw_docs = reader.fields_stream()?;
            raw_docs.seek(from_pointer)?;
            loop {
                let base = raw_docs.read_v_int()?;
                let code = raw_docs.read_v_int()?;
                let buffered_docs = ((code as u32) >> 2) as i32;
                if base != doc_id {
                    return Err(LuceneError::CorruptIndex(format!(
                        "invalid state: base={base}, docID={doc_id}"
                    )));
                }
                // write a new index entry and new header for this chunk.
                self.index_writer
                    .write_index(buffered_docs, self.fields_stream.file_pointer())?;
                self.fields_stream.write_v_int(self.doc_base)?; // rebase
                self.fields_stream.write_v_int(code)?;
                doc_id += buffered_docs;
                self.doc_base += buffered_docs;
                if doc_id > to_doc_id {
                    return Err(LuceneError::CorruptIndex(format!(
                        "invalid state: base={base}, count={buffered_docs}, toDocID={to_doc_id}"
                    )));
                }
                // copy bytes until the next chunk boundary (or end of chunk data).
                // using the stored fields index for this isn't the most efficient, but fast
                // enough and is a source of redundancy for detecting bad things.
                let end_chunk_pointer = if doc_id == sub.max_doc {
                    reader.max_pointer()
                } else {
                    reader.index_reader().get_start_pointer(doc_id)?
                };
                let to_copy = end_chunk_pointer - raw_docs.file_pointer();
                self.fields_stream.copy_bytes(raw_docs.as_mut(), to_copy)?;
                self.num_chunks += 1;
                let dirty_chunk = (code & 2) != 0;
                if dirty_chunk {
                    debug_assert!((buffered_docs as usize) < self.max_docs_per_chunk);
                    self.num_dirty_chunks += 1;
                    self.num_dirty_docs += i64::from(buffered_docs);
                }
                from_pointer = end_chunk_pointer;
                if from_pointer >= to_pointer {
                    break;
                }
            }
        }

        // copy leftover docs that don't form a complete chunk
        debug_assert!(!reader.is_loaded(doc_id)?);
        while doc_id < to_doc_id {
            self.copy_one_doc(reader, doc_id)?;
            doc_id += 1;
        }
        Ok(())
    }

    fn write_z_float(out: &mut dyn DataOutput, value: f32) -> Result<()> {
        let int_val = value as i32;
        let float_bits = value.to_bits();

        if value == int_val as f32
            && int_val >= -1
            && int_val <= 0x7D
            && float_bits != NEGATIVE_ZERO_FLOAT_BITS
        {
            out.write_byte((0x80 | (1 + int_val)) as u8)
        } else if (float_bits >> 31) == 0 {
            out.write_byte((float_bits >> 24) as u8)?;
            StoredFieldsInts::write_le_short(out, (float_bits >> 8) as u16)?;
            out.write_byte(float_bits as u8)
        } else {
            out.write_byte(0xFF)?;
            StoredFieldsInts::write_le_int(out, float_bits)
        }
    }

    fn write_z_double(out: &mut dyn DataOutput, value: f64) -> Result<()> {
        let int_val = value as i32;
        let double_bits = value.to_bits();

        if value == int_val as f64
            && int_val >= -1
            && int_val <= 0x7C
            && double_bits != NEGATIVE_ZERO_DOUBLE_BITS
        {
            out.write_byte((0x80 | (int_val + 1)) as u8)
        } else if value == (value as f32) as f64 {
            out.write_byte(0xFE)?;
            StoredFieldsInts::write_le_int(out, (value as f32).to_bits())
        } else if (double_bits >> 63) == 0 {
            out.write_byte((double_bits >> 56) as u8)?;
            StoredFieldsInts::write_le_int(out, (double_bits >> 24) as u32)?;
            StoredFieldsInts::write_le_short(out, (double_bits >> 8) as u16)?;
            out.write_byte(double_bits as u8)
        } else {
            out.write_byte(0xFF)?;
            StoredFieldsInts::write_le_long(out, double_bits)
        }
    }

    fn write_t_long(out: &mut dyn DataOutput, mut value: i64) -> Result<()> {
        let header: u8;
        if value % SECOND_MS != 0 {
            header = 0;
        } else if value % DAY_MS == 0 {
            header = DAY_ENCODING as u8;
            value /= DAY_MS;
        } else if value % HOUR_MS == 0 {
            header = HOUR_ENCODING as u8;
            value /= HOUR_MS;
        } else {
            header = SECOND_ENCODING as u8;
            value /= SECOND_MS;
        }

        let zigzag = BitUtil::zig_zag_encode_long(value);
        let mut header = header | (zigzag & 0x1F) as u8;
        // Java shifts with `>>>`: the zig-zag encoding of a value of magnitude
        // 2^62 or more has its top bit set, and an arithmetic shift would keep
        // that bit, yielding a negative `upperBits` that no `vLong` can carry.
        let upper = ((zigzag as u64) >> 5) as i64;
        if upper != 0 {
            header |= 0x20;
        }
        out.write_byte(header)?;
        if upper != 0 {
            out.write_v_long(upper)
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for Lucene90CompressingStoredFieldsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90CompressingStoredFieldsWriter")
            .field("compression_mode", &format!("{}", self.compression_mode))
            .field("chunk_size", &self.chunk_size)
            .field("max_docs_per_chunk", &self.max_docs_per_chunk)
            .field("doc_base", &self.doc_base)
            .field("num_buffered_docs", &self.num_buffered_docs)
            .finish_non_exhaustive()
    }
}

impl Accountable for Lucene90CompressingStoredFieldsWriter {
    /// Bytes buffered for the documents of the chunk being built.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsWriter.ramBytesUsed()`.
    /// Java adds `bufferedDocs.ramBytesUsed()`, the *allocated* size of the
    /// byte blocks; `ByteBuffersDataOutput` here reports the bytes actually
    /// written, so the estimate is slightly lower than Lucene's for a
    /// partially filled block. Both are estimates feeding the same
    /// flush-by-RAM decision, and neither reaches the index files.
    fn ram_bytes_used(&self) -> i64 {
        self.buffered_docs.size() as i64
            + (self.num_stored_fields.capacity() as i64) * 4
            + (self.end_offsets.capacity() as i64) * 4
    }
}

impl StoredFieldsWriter for Lucene90CompressingStoredFieldsWriter {
    fn start_document(&mut self) -> Result<()> {
        Ok(())
    }

    fn finish_document(&mut self) -> Result<()> {
        if self.num_buffered_docs == self.num_stored_fields.len() {
            let new_len = self.num_stored_fields.len() * 2;
            self.num_stored_fields.resize(new_len, 0);
            self.end_offsets.resize(new_len, 0);
        }
        self.num_stored_fields[self.num_buffered_docs] = self.num_stored_fields_in_doc;
        self.num_stored_fields_in_doc = 0;
        self.end_offsets[self.num_buffered_docs] = self.buffered_docs.size() as i32;
        self.num_buffered_docs += 1;
        if self.trigger_flush() {
            self.flush(false)
        } else {
            Ok(())
        }
    }

    fn write_field_i32(&mut self, info: &FieldInfo, value: i32) -> Result<()> {
        self.num_stored_fields_in_doc += 1;
        self.write_field_header(info, TYPE_NUMERIC_INT)?;
        self.buffered_docs.write_z_int(value)
    }

    fn write_field_i64(&mut self, info: &FieldInfo, value: i64) -> Result<()> {
        self.num_stored_fields_in_doc += 1;
        self.write_field_header(info, TYPE_NUMERIC_LONG)?;
        Self::write_t_long(&mut self.buffered_docs, value)
    }

    fn write_field_f32(&mut self, info: &FieldInfo, value: f32) -> Result<()> {
        self.num_stored_fields_in_doc += 1;
        self.write_field_header(info, TYPE_NUMERIC_FLOAT)?;
        Self::write_z_float(&mut self.buffered_docs, value)
    }

    fn write_field_f64(&mut self, info: &FieldInfo, value: f64) -> Result<()> {
        self.num_stored_fields_in_doc += 1;
        self.write_field_header(info, TYPE_NUMERIC_DOUBLE)?;
        Self::write_z_double(&mut self.buffered_docs, value)
    }

    fn write_field_bytes(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()> {
        self.num_stored_fields_in_doc += 1;
        self.write_field_header(info, TYPE_BYTE_ARR)?;
        self.buffered_docs.write_v_int(value.len() as i32)?;
        self.buffered_docs.write_bytes(value, 0, value.len())
    }

    fn write_field_data_input(
        &mut self,
        info: &FieldInfo,
        value: &mut StoredFieldDataInput<'_>,
    ) -> Result<()> {
        let length = value.length();
        if length < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "stored binary field \"{}\" has a negative length: {length}",
                info.name
            )));
        }
        self.num_stored_fields_in_doc += 1;
        self.write_field_header(info, TYPE_BYTE_ARR)?;
        self.buffered_docs.write_v_int(length)?;
        // `copyBytes` in Lucene: the bytes go from the input straight into the
        // chunk buffer, never through an intermediate array.
        self.buffered_docs
            .copy_bytes(value.data_input(), i64::from(length))
    }

    fn write_field_string(&mut self, info: &FieldInfo, value: &str) -> Result<()> {
        self.num_stored_fields_in_doc += 1;
        self.write_field_header(info, TYPE_STRING)?;
        self.buffered_docs.write_string(value)
    }

    fn finish(&mut self, num_docs: i32) -> Result<()> {
        if self.num_buffered_docs > 0 {
            self.flush(true)?;
        }
        if self.doc_base != num_docs {
            return Err(LuceneError::IllegalState(format!(
                "Wrote {} docs, finish called with numDocs={num_docs}",
                self.doc_base
            )));
        }
        let max_pointer = self.fields_stream.file_pointer();
        self.index_writer
            .finish(num_docs, max_pointer, self.meta_stream.as_mut())?;
        self.meta_stream.write_v_long(self.num_chunks)?;
        self.meta_stream.write_v_long(self.num_dirty_chunks)?;
        self.meta_stream.write_v_long(self.num_dirty_docs)?;
        write_footer(self.meta_stream.as_mut())?;
        write_footer(self.fields_stream.as_mut())?;
        Ok(())
    }

    /// Copies every document's stored fields from the segments being merged,
    /// picking for each source segment the cheapest route that is safe for it.
    ///
    /// [`get_merge_strategy`](Self::get_merge_strategy) makes that choice once
    /// per segment:
    ///
    /// * [`MergeStrategy::Bulk`] copies whole compressed chunks byte for byte
    ///   through [`copy_chunks`](Self::copy_chunks), so the documents are never
    ///   decompressed. A run of consecutive documents from the same segment is
    ///   gathered first, because a chunk can only be copied as a whole.
    /// * [`MergeStrategy::Doc`] copies one document at a time in its serialized
    ///   form through [`copy_one_doc`](Self::copy_one_doc), decompressing but
    ///   not re-encoding it.
    /// * [`MergeStrategy::Visitor`] decodes every field and writes it again,
    ///   which is what the trait's own [`StoredFieldsWriter::merge`] does for
    ///   every segment.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when a segment of the merge has no
    /// stored-fields reader, or when a strategy that needs this format's own
    /// reader is handed another one, and propagates every I/O and corruption
    /// error raised while reading the source segments.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsWriter#merge`
    /// (`Lucene90CompressingStoredFieldsWriter.java:602-653`).
    fn merge(&mut self, merge_state: &MergeState) -> Result<i32> {
        let num_readers = merge_state.stored_fields_readers.len();
        let matching_readers = MatchingReaders::new(merge_state);
        // Java builds a `MergeVisitor` per segment that needs one and keeps it
        // in a parallel array; the visitor's whole state is the remapping its
        // constructor computes, so that is what is kept here. The visitor
        // itself borrows the writer and cannot outlive one document.
        let mut remappers: Vec<Option<FieldInfos>> = Vec::with_capacity(num_readers);
        let mut subs: Vec<CompressingStoredFieldsMergeSub> = Vec::with_capacity(num_readers);
        for i in 0..num_readers {
            merge_state.check_aborted()?;
            let Some(reader) = merge_state.stored_fields_readers[i].as_ref() else {
                // Java dereferences the reader unconditionally, so a missing
                // one is a broken merge state either way; this reports it
                // instead of dropping the segment's documents silently.
                return Err(LuceneError::IllegalState(format!(
                    "segment {i} of the merge has no stored-fields reader"
                )));
            };
            reader.check_integrity()?;
            let merge_strategy = self.get_merge_strategy(merge_state, &matching_readers, i);
            remappers.push(if merge_strategy == MergeStrategy::Visitor {
                merge_visitor_remapper(merge_state, i)
            } else {
                None
            });
            subs.push(CompressingStoredFieldsMergeSub {
                doc_map: Arc::clone(&merge_state.doc_maps[i]),
                reader_index: i,
                max_doc: merge_state.max_docs[i],
                merge_strategy,
                doc_id: -1,
                mapped_doc_id: -1,
            });
        }

        let mut doc_count = 0i32;
        let mut merger = DocIDMerger::new(subs, merge_state.needs_index_sort)?;
        let mut step = next_stored_fields_step(&mut merger)?;
        while let Some(current) = step {
            debug_assert_eq!(current.mapped_doc_id, doc_count);
            match current.merge_strategy {
                MergeStrategy::Bulk => {
                    // Gather the run of documents this segment contributes in a
                    // row: chunks can only be copied whole.
                    let from_doc_id = current.doc_id;
                    let mut to_doc_id = from_doc_id;
                    loop {
                        step = next_stored_fields_step(&mut merger)?;
                        match step {
                            Some(next) if next.sub_index == current.sub_index => {
                                to_doc_id += 1;
                                debug_assert_eq!(next.doc_id, to_doc_id);
                            }
                            _ => break,
                        }
                    }
                    to_doc_id += 1; // exclusive bound
                    self.copy_chunks(
                        merge_state,
                        merger.sub(current.sub_index),
                        from_doc_id,
                        to_doc_id,
                    )?;
                    doc_count += to_doc_id - from_doc_id;
                }
                MergeStrategy::Doc => {
                    let reader = merge_state.stored_fields_readers[current.reader_index]
                        .as_ref()
                        .and_then(|reader| reader.as_any())
                        .and_then(|any| any.downcast_ref::<Lucene90CompressingStoredFieldsReader>())
                        .ok_or_else(|| {
                            LuceneError::IllegalState(
                                "document-by-document merge needs a \
                                 Lucene90CompressingStoredFieldsReader"
                                    .to_string(),
                            )
                        })?;
                    self.copy_one_doc(reader, current.doc_id)?;
                    doc_count += 1;
                    step = next_stored_fields_step(&mut merger)?;
                }
                MergeStrategy::Visitor => {
                    let Some(reader) =
                        merge_state.stored_fields_readers[current.reader_index].as_ref()
                    else {
                        return Err(LuceneError::IllegalState(format!(
                            "segment {} of the merge has no stored-fields reader",
                            current.reader_index
                        )));
                    };
                    self.start_document()?;
                    let mut visitor = MergeVisitor {
                        writer: self,
                        remapper: remappers[current.reader_index].as_ref(),
                    };
                    reader.document(current.doc_id, &mut visitor)?;
                    self.finish_document()?;
                    doc_count += 1;
                    step = next_stored_fields_step(&mut merger)?;
                }
            }
        }
        self.finish(doc_count)?;
        Ok(doc_count)
    }

    /// Releases every resource of the writer.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsWriter.close()`, which is
    /// `IOUtils.close(metaStream, fieldsStream, indexWriter, compressor)`:
    /// **all four** are closed even when one of them fails, and the first
    /// failure is what propagates. Short-circuiting on the first error would
    /// leak the remaining handles, which on Windows blocks the very files an
    /// aborted flush is about to delete. Rust has no suppressed-exception
    /// chain, so the later failures are dropped rather than attached.
    fn close(&mut self) -> Result<()> {
        let outcomes = [
            self.meta_stream.close(),
            self.fields_stream.close(),
            self.index_writer.close(),
            self.compressor.close(),
        ];
        outcomes.into_iter().find(Result::is_err).unwrap_or(Ok(()))
    }
}

// -----------------------------------------------------------------------------
// Lucene90CompressingStoredFieldsReader
// -----------------------------------------------------------------------------

/// Decompresses and visits stored fields for a segment.
pub struct Lucene90CompressingStoredFieldsReader {
    version: i32,
    field_infos: FieldInfos,
    index_reader: Box<dyn FieldsIndex>,
    max_pointer: i64,
    chunk_size: usize,
    compression_mode: CompressionMode,
    decompressor: Mutex<Box<dyn Decompressor>>,
    num_docs: i32,
    num_chunks: i64,
    num_dirty_chunks: i64,
    num_dirty_docs: i64,
    /// The `.fdt` file, kept open exactly as Lucene keeps `fieldsStream`. Every
    /// read clones it, so the reader works on whatever directory it was opened
    /// from — including a compound-file directory, whose files no `SegmentInfo`
    /// can point at.
    fields_stream: Box<dyn IndexInput>,
    merging: bool,
    state: Mutex<BlockState>,
    /// The last [`PREFETCH_CACHE_SIZE`] blocks
    /// [`prefetch`](StoredFieldsReader::prefetch) asked the `.fdt` input for,
    /// overwritten round-robin; `-1` marks a free slot.
    ///
    /// Equivalent to Lucene's `long[] prefetchedBlockIDCache`
    /// (`Lucene90CompressingStoredFieldsReader.java:98`). `prefetch` takes
    /// `&self`, so the array sits behind a lock.
    prefetched_block_id_cache: Mutex<[i64; PREFETCH_CACHE_SIZE]>,
    /// How many blocks have been prefetched, which picks the next slot.
    ///
    /// Equivalent to Lucene's `int prefetchedBlockIDCacheIndex`
    /// (`Lucene90CompressingStoredFieldsReader.java:99`); it wraps as Java's
    /// `int` does.
    prefetched_block_id_cache_index: AtomicI32,
    /// Whether [`close`](StoredFieldsReader::close) has already run.
    ///
    /// Equivalent to Lucene's `private boolean closed`.
    closed: bool,
}

/// Number of block ids the reader remembers having prefetched.
///
/// Equivalent to `Lucene90CompressingStoredFieldsReader.PREFETCH_CACHE_SIZE`.
const PREFETCH_CACHE_SIZE: usize = 1 << 4;

/// Mask that turns the running prefetch counter into a cache slot.
///
/// Equivalent to `Lucene90CompressingStoredFieldsReader.PREFETCH_CACHE_MASK`.
const PREFETCH_CACHE_MASK: i32 = PREFETCH_CACHE_SIZE as i32 - 1;

#[derive(Debug)]
struct BlockState {
    doc_base: i32,
    chunk_docs: i32,
    sliced: bool,
    offsets: Vec<i64>,
    num_stored_fields: Vec<i64>,
    start_pointer: i64,
    bytes: Vec<u8>,
}

impl BlockState {
    fn new() -> Self {
        Self {
            doc_base: 0,
            chunk_docs: 0,
            sliced: false,
            offsets: Vec::new(),
            num_stored_fields: Vec::new(),
            start_pointer: 0,
            bytes: Vec::new(),
        }
    }

    fn contains(&self, doc_id: i32) -> bool {
        // `contains` runs on a header that has been read but not yet
        // validated, so a corrupt `docBase`/`chunkDocs` pair reaches it. Java's
        // `int` arithmetic wraps here (`Reader.java:442-444`) and the
        // validation that follows rejects the block. No corrupt header has been
        // found that actually overflows the sum — the short circuit means
        // `docBase <= docID`, which keeps it small — so this is defensive
        // rather than a fix for a reproduced abort; it costs nothing and makes
        // the arithmetic match Java's exactly instead of relying on that
        // argument staying true.
        doc_id >= self.doc_base && doc_id < self.doc_base.wrapping_add(self.chunk_docs)
    }

    /// Loads the block that contains `doc_id`.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.BlockState.reset`,
    /// whose `finally` sets `chunkDocs = 0` whenever `doReset` failed: a block
    /// whose header was read but whose body turned out to be corrupt must not
    /// stay cached, or the next read of a document in that block would be
    /// served from half-decoded state instead of raising the same corruption
    /// error again.
    fn reset(
        &mut self,
        doc_id: i32,
        fields_stream: &mut dyn IndexInput,
        num_docs: i32,
        chunk_size: usize,
        decompressor: &mut dyn Decompressor,
        merging: bool,
    ) -> Result<()> {
        let outcome = self.do_reset(
            doc_id,
            fields_stream,
            num_docs,
            chunk_size,
            decompressor,
            merging,
        );
        if outcome.is_err() {
            // Condemn the block: `contains` now answers `false` for every doc,
            // so the header is decoded again on the next attempt.
            self.chunk_docs = 0;
        }
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    fn do_reset(
        &mut self,
        doc_id: i32,
        fields_stream: &mut dyn IndexInput,
        num_docs: i32,
        chunk_size: usize,
        decompressor: &mut dyn Decompressor,
        merging: bool,
    ) -> Result<()> {
        self.doc_base = fields_stream.read_v_int()?;
        let token = fields_stream.read_v_int()?;
        self.chunk_docs = token >> 2;
        self.sliced = (token & 1) != 0;

        if !self.contains(doc_id) || self.doc_base + self.chunk_docs > num_docs {
            return Err(LuceneError::CorruptIndex(format!(
                "Corrupted: doc_id={doc_id}, doc_base={}, chunk_docs={}, num_docs={num_docs}",
                self.doc_base, self.chunk_docs
            )));
        }

        self.offsets.resize(self.chunk_docs as usize + 1, 0);
        self.num_stored_fields.resize(self.chunk_docs as usize, 0);

        if self.chunk_docs == 1 {
            self.num_stored_fields[0] = fields_stream.read_v_int()? as i64;
            self.offsets[1] = fields_stream.read_v_int()? as i64;
        } else {
            StoredFieldsInts::read_ints(
                fields_stream,
                self.chunk_docs as usize,
                &mut self.num_stored_fields,
                0,
            )?;
            StoredFieldsInts::read_ints(
                fields_stream,
                self.chunk_docs as usize,
                &mut self.offsets,
                1,
            )?;
            for i in 0..self.chunk_docs as usize {
                self.offsets[i + 1] += self.offsets[i];
            }
            for i in 0..self.chunk_docs as usize {
                let len = self.offsets[i + 1] - self.offsets[i];
                let stored = self.num_stored_fields[i];
                if (len == 0) != (stored == 0) {
                    return Err(LuceneError::CorruptIndex(format!(
                        "length={len}, num_stored_fields={stored}"
                    )));
                }
            }
        }

        self.start_pointer = fields_stream.file_pointer();

        if merging {
            let total_length = self.offsets[self.chunk_docs as usize] as usize;
            self.bytes.clear();
            self.bytes.reserve(total_length);
            if self.sliced {
                let mut decompressed = 0;
                while decompressed < total_length {
                    let to_decompress = cmp::min(total_length - decompressed, chunk_size);
                    let mut spare = BytesRef::default();
                    // Every slice was compressed independently and carries its
                    // own framing, so its *own* uncompressed length is the
                    // `originalLength` the decompressor must be given — not the
                    // length of the whole chunk. Passing the chunk length makes
                    // a framed codec such as LZ4-with-preset-dictionary read
                    // compressed payload as framing and decode garbage.
                    decompressor.decompress(
                        fields_stream,
                        to_decompress,
                        0,
                        to_decompress,
                        &mut spare,
                    )?;
                    self.bytes.extend_from_slice(spare.slice());
                    decompressed += to_decompress;
                }
            } else {
                let mut bytes = BytesRef::default();
                decompressor.decompress(
                    fields_stream,
                    total_length,
                    0,
                    total_length,
                    &mut bytes,
                )?;
                self.bytes.extend_from_slice(bytes.slice());
            }
            if self.bytes.len() != total_length {
                return Err(LuceneError::CorruptIndex(format!(
                    "expected chunk size = {total_length}, got {}",
                    self.bytes.len()
                )));
            }
        }

        Ok(())
    }

    fn document(
        &self,
        doc_id: i32,
        fields_stream: Option<&mut dyn IndexInput>,
        chunk_size: usize,
        decompressor: &mut dyn Decompressor,
        merging: bool,
    ) -> Result<SerializedDocument> {
        if !self.contains(doc_id) {
            return Err(LuceneError::IllegalArgument(format!(
                "doc_id {doc_id} is not in the current block"
            )));
        }
        let index = (doc_id - self.doc_base) as usize;
        // The offsets are a running sum decoded from the chunk header, so a
        // corrupt header can make them non-monotonic or overflow an `i32`.
        // Java's `Math.toIntExact` raises `ArithmeticException` for the latter
        // and the negative length that the former produces trips its array
        // bounds; both surface as an exception, so both are reported here
        // rather than turning into a wild slice index.
        let start = self.offsets[index];
        let end = self.offsets[index + 1];
        if end < start || start < 0 || end > i64::from(i32::MAX) {
            return Err(LuceneError::CorruptIndex(format!(
                "document {doc_id} spans [{start}, {end}) inside its chunk"
            )));
        }
        let offset = start as usize;
        let length = (end - start) as usize;
        let num_stored_fields = self.num_stored_fields[index] as i32;

        if length == 0 {
            return Ok(SerializedDocument {
                bytes: Vec::new(),
                length: 0,
                num_stored_fields,
            });
        }

        let data: Vec<u8> = if merging {
            self.bytes[offset..offset + length].to_vec()
        } else {
            let input = fields_stream
                .ok_or_else(|| LuceneError::IllegalState("missing fields input".to_string()))?;
            let total_length = self.offsets[self.chunk_docs as usize] as usize;
            input.seek(self.start_pointer)?;
            // A known deviation, tracked as its own task: Java returns a lazy
            // `DataInput` that starts inflating at the document's own offset
            // (`Lucene90CompressingStoredFieldsReader.java:563-566`) and stops
            // as soon as the visitor is satisfied, whereas this port inflates
            // the whole chunk on every document read. The bytes the visitor
            // sees are identical; the cost is not.
            //
            // *Time*: measured with 300 reads each on a Lucene-written sliced
            // chunk — reading the 6-byte document that merely neighbours a
            // 243 001-byte one costs 0.460 ms here against 0.006 ms for the
            // same read in an ordinary chunk, a factor of 77; the large
            // document itself costs 0.625 ms.
            //
            // *Memory*: Java's buffer is bounded by `dictLength + blockLength`,
            // about 90 KiB at `BEST_SPEED`, no matter how large the document
            // is. This port allocates the whole chunk *and* a second full copy
            // of the document, so for a document near Lucene's documented 2 GiB
            // limit that is roughly 4 GiB against a fixed 90 KiB. The
            // difference is unbounded versus bounded, not merely "higher".
            if self.sliced {
                let mut full = Vec::with_capacity(total_length);
                let mut decompressed = 0;
                while decompressed < total_length {
                    let to_decompress = cmp::min(total_length - decompressed, chunk_size);
                    let mut spare = BytesRef::default();
                    // See the merging branch of `reset`: each slice frames its
                    // own uncompressed length.
                    decompressor.decompress(input, to_decompress, 0, to_decompress, &mut spare)?;
                    full.extend_from_slice(spare.slice());
                    decompressed += to_decompress;
                }
                if full.len() != total_length {
                    return Err(LuceneError::CorruptIndex(format!(
                        "expected chunk size = {total_length}, got {}",
                        full.len()
                    )));
                }
                full[offset..offset + length].to_vec()
            } else {
                let mut bytes = BytesRef::default();
                decompressor.decompress(input, total_length, offset, length, &mut bytes)?;
                bytes.slice().to_vec()
            }
        };

        Ok(SerializedDocument {
            bytes: data,
            length,
            num_stored_fields,
        })
    }
}

/// A document as it appears inside a decompressed chunk.
///
/// Java holds a `DataInput` here
/// (`Lucene90CompressingStoredFieldsReader.java:396-406`); this port holds the
/// bytes the input would read and each consumer
/// wraps them in a [`ByteArrayDataInput`] of its own, because a struct holding
/// both a buffer and a view of that buffer is a self-reference Rust forbids.
struct SerializedDocument {
    bytes: Vec<u8>,
    length: usize,
    num_stored_fields: i32,
}

impl Lucene90CompressingStoredFieldsReader {
    fn new(
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        field_infos: &FieldInfos,
        io_context: &dyn IOContext,
        format_name: &str,
        compression_mode: CompressionMode,
    ) -> Result<Self> {
        let segment = segment_info.name.clone();
        let id = segment_info.id();
        let num_docs = segment_info.max_doc()?;

        let fields_file = segment_file_name(&segment, segment_suffix, FIELDS_EXTENSION);
        let mut fields_stream = directory.open_input(&fields_file, io_context)?;
        let version = check_index_header(
            fields_stream.as_mut(),
            format_name,
            DATA_VERSION_START,
            DATA_VERSION_CURRENT,
            &id,
            segment_suffix,
        )?;
        retrieve_checksum(fields_stream.as_mut())?;

        let meta_file = segment_file_name(&segment, segment_suffix, META_EXTENSION);
        let mut meta_in = directory.open_checksum_input(&meta_file)?;
        check_index_header(
            meta_in.as_mut(),
            &format!("{INDEX_CODEC_NAME}Meta"),
            META_VERSION_START,
            version,
            &id,
            segment_suffix,
        )?;

        let chunk_size = meta_in.read_v_int()? as usize;

        let index_reader = FieldsIndexReader::new(
            directory,
            &segment,
            segment_suffix,
            INDEX_EXTENSION,
            INDEX_CODEC_NAME,
            id,
            meta_in.as_mut(),
            io_context,
        )?;
        let max_pointer = index_reader.max_pointer;

        let num_chunks = meta_in.read_v_long()?;
        let num_dirty_chunks = meta_in.read_v_long()?;
        let num_dirty_docs = meta_in.read_v_long()?;

        if num_chunks < num_dirty_chunks {
            return Err(LuceneError::CorruptIndex(format!(
                "Cannot have more dirty chunks than chunks: numChunks={num_chunks}, numDirtyChunks={num_dirty_chunks}"
            )));
        }
        if (num_dirty_chunks == 0) != (num_dirty_docs == 0) {
            return Err(LuceneError::CorruptIndex(
                "Cannot have dirty chunks without dirty docs or vice-versa".to_string(),
            ));
        }
        if num_dirty_docs < num_dirty_chunks {
            return Err(LuceneError::CorruptIndex(format!(
                "Cannot have more dirty chunks than documents within dirty chunks: numDirtyChunks={num_dirty_chunks}, numDirtyDocs={num_dirty_docs}"
            )));
        }

        check_footer(meta_in.as_mut())?;
        meta_in.close()?;

        let cloned_field_infos =
            FieldInfos::new(field_infos.iter().map(|fi| fi.clone()).collect())?;

        Ok(Self {
            version,
            field_infos: cloned_field_infos,
            index_reader: Box::new(index_reader),
            max_pointer,
            chunk_size,
            compression_mode,
            decompressor: Mutex::new(compression_mode.new_decompressor()),
            num_docs,
            num_chunks,
            num_dirty_chunks,
            num_dirty_docs,
            fields_stream,
            merging: false,
            state: Mutex::new(BlockState::new()),
            prefetched_block_id_cache: Mutex::new([-1; PREFETCH_CACHE_SIZE]),
            prefetched_block_id_cache_index: AtomicI32::new(0),
            closed: false,
        })
    }
}

impl fmt::Debug for Lucene90CompressingStoredFieldsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90CompressingStoredFieldsReader")
            .field("version", &self.version)
            .field("compression_mode", &format!("{}", self.compression_mode))
            .field("chunk_size", &self.chunk_size)
            .field("num_docs", &self.num_docs)
            .field("num_chunks", &self.num_chunks)
            .field("merging", &self.merging)
            .finish_non_exhaustive()
    }
}

impl StoredFieldsReader for Lucene90CompressingStoredFieldsReader {
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        let doc = self.serialized_document(doc_id)?;

        let mut input = ByteArrayDataInput::new(&doc.bytes);
        for field_idx in 0..doc.num_stored_fields {
            let info_and_bits = input.read_v_long()? as u64;
            let field_number = (info_and_bits >> (TYPE_BITS as u32)) as i32;
            let bits = (info_and_bits & TYPE_MASK as u64) as i32;

            let field_info = self
                .field_infos
                .field_info_by_number(field_number)
                .ok_or_else(|| {
                    LuceneError::CorruptIndex(format!("field number {field_number} not found"))
                })?;

            match visitor.needs_field(field_info)? {
                StoredFieldVisitorStatus::Yes => {
                    Self::read_field(&mut input, visitor, field_info, bits)?;
                }
                StoredFieldVisitorStatus::No => {
                    if field_idx == doc.num_stored_fields - 1 {
                        return Ok(());
                    }
                    Self::skip_field(&mut input, bits)?;
                }
                StoredFieldVisitorStatus::Stop => return Ok(()),
            }
        }
        Ok(())
    }

    fn check_integrity(&self) -> Result<()> {
        self.index_reader.check_integrity()?;
        let mut input = self.fields_stream.clone_input()?;
        input.seek(0)?;
        checksum_entire_file(input.as_mut())?;
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
        // Java guards this with `ensureOpen()`, which throws the *unchecked*
        // `AlreadyClosedException`. The trait's return type cannot carry an
        // error, and the `clone_input` below already panics once the stream is
        // closed, so the same condition is reported here with Java's message
        // instead of as an opaque clone failure.
        self.ensure_open().expect("this FieldsReader is closed");
        Box::new(Self {
            version: self.version,
            field_infos: FieldInfos::new(self.field_infos.iter().map(|fi| fi.clone()).collect())
                .expect("clone field infos"),
            index_reader: self.index_reader.clone_index().expect("clone failed"),
            max_pointer: self.max_pointer,
            chunk_size: self.chunk_size,
            compression_mode: self.compression_mode,
            decompressor: Mutex::new(self.compression_mode.new_decompressor()),
            num_docs: self.num_docs,
            num_chunks: self.num_chunks,
            num_dirty_chunks: self.num_dirty_chunks,
            num_dirty_docs: self.num_dirty_docs,
            fields_stream: self.fields_stream.clone_input().expect("clone failed"),
            merging: false,
            state: Mutex::new(BlockState::new()),
            prefetched_block_id_cache: Mutex::new([-1; PREFETCH_CACHE_SIZE]),
            prefetched_block_id_cache_index: AtomicI32::new(0),
            closed: false,
        })
    }

    fn get_merge_instance(&self) -> Box<dyn StoredFieldsReader> {
        // See the note on `clone_reader`: Java guards this with the same
        // unchecked `ensureOpen()`.
        self.ensure_open().expect("this FieldsReader is closed");
        Box::new(Self {
            version: self.version,
            field_infos: FieldInfos::new(self.field_infos.iter().map(|fi| fi.clone()).collect())
                .expect("clone field infos"),
            index_reader: self.index_reader.clone_index().expect("clone failed"),
            max_pointer: self.max_pointer,
            chunk_size: self.chunk_size,
            compression_mode: self.compression_mode,
            decompressor: Mutex::new(self.compression_mode.new_decompressor()),
            num_docs: self.num_docs,
            num_chunks: self.num_chunks,
            num_dirty_chunks: self.num_dirty_chunks,
            num_dirty_docs: self.num_dirty_docs,
            fields_stream: self.fields_stream.clone_input().expect("clone failed"),
            merging: true,
            state: Mutex::new(BlockState::new()),
            prefetched_block_id_cache: Mutex::new([-1; PREFETCH_CACHE_SIZE]),
            prefetched_block_id_cache_index: AtomicI32::new(0),
            closed: false,
        })
    }

    /// Closes the fields index and the `.fdt` stream, once.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.close()`: both are
    /// closed even when the first fails, the first failure is the one reported,
    /// and the reader is only marked closed once the close succeeded — so a
    /// failed close can be retried, exactly as Java's `if (!closed)` allows.
    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let index = self.index_reader.close();
        let stream = self.fields_stream.close();
        index.and(stream)?;
        self.closed = true;
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Asks the `.fdt` input to load the block that holds `doc_id`, unless it
    /// was one of the last sixteen blocks prefetched.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.prefetch(int)`
    /// (`Lucene90CompressingStoredFieldsReader.java:629-643`).
    fn prefetch(&self, doc_id: i32) -> Result<()> {
        let block_id = self.index_reader.get_block_id(doc_id)?;

        let mut prefetched_block_id_cache = self.prefetched_block_id_cache.lock().unwrap();
        for &prefetched_block_id in prefetched_block_id_cache.iter() {
            if prefetched_block_id == block_id {
                return Ok(());
            }
        }

        let block_start_pointer = self.index_reader.get_block_start_pointer(block_id)?;
        let block_length = self.index_reader.get_block_length(block_id)?;
        self.fields_stream
            .prefetch(block_start_pointer, block_length)?;

        let index = self
            .prefetched_block_id_cache_index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        prefetched_block_id_cache[(index & PREFETCH_CACHE_MASK) as usize] = block_id;
        Ok(())
    }
}

impl Lucene90CompressingStoredFieldsReader {
    /// Returns the number of documents this reader covers, which is the
    /// segment's `maxDoc`.
    ///
    /// Equivalent to the package-private `Lucene90CompressingStoredFieldsReader#getNumDocs`
    /// (`Lucene90CompressingStoredFieldsReader.java:759-761`).
    pub fn get_num_docs(&self) -> i32 {
        self.num_docs
    }

    /// Fails when this reader has already been closed.
    ///
    /// Equivalent to the private
    /// `Lucene90CompressingStoredFieldsReader.ensureOpen()`, including its
    /// message.
    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(LuceneError::AlreadyClosed(
                "this FieldsReader is closed".to_string(),
            ));
        }
        Ok(())
    }

    /// Returns the format version this segment was written with.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.getVersion`.
    fn version(&self) -> i32 {
        self.version
    }

    /// Returns the uncompressed size of one chunk, in bytes.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.getChunkSize`.
    fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    /// Returns the compression mode the chunks were written with.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.getCompressionMode`.
    fn compression_mode(&self) -> CompressionMode {
        self.compression_mode
    }

    /// Returns the number of chunks in this segment.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.getNumChunks`.
    fn num_chunks(&self) -> i64 {
        self.num_chunks
    }

    /// Returns the number of chunks that were flushed before they were full.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.getNumDirtyChunks`.
    fn num_dirty_chunks(&self) -> i64 {
        self.num_dirty_chunks
    }

    /// Returns the number of documents held by those incomplete chunks.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.getNumDirtyDocs`.
    fn num_dirty_docs(&self) -> i64 {
        self.num_dirty_docs
    }

    /// Returns the offset just past the last chunk of the `.fdt` file.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.getMaxPointer`.
    fn max_pointer(&self) -> i64 {
        self.max_pointer
    }

    /// Returns the stored-fields index, which maps a document to the start of
    /// the chunk that holds it.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.getIndexReader`.
    fn index_reader(&self) -> &dyn FieldsIndex {
        self.index_reader.as_ref()
    }

    /// Opens a cursor over the `.fdt` file, positioned at its start.
    ///
    /// Divergence: Lucene hands out its own `fieldsStream` and lets the caller
    /// seek it (`Lucene90CompressingStoredFieldsReader.java:696-698`), which is
    /// safe there because only a merge instance is ever driven this way. A
    /// `&self` method cannot hand out a seekable cursor here, so an independent
    /// clone of the same file is returned; a clone starts at offset zero and
    /// the caller seeks it exactly as Java does.
    ///
    /// # Errors
    ///
    /// Propagates the I/O error raised while cloning the input.
    fn fields_stream(&self) -> Result<Box<dyn IndexInput>> {
        self.fields_stream.clone_input()
    }

    /// Returns whether the chunk that holds `doc_id` is the one already
    /// decoded, so that reading the document costs no decompression.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.isLoaded`
    /// (`Lucene90CompressingStoredFieldsReader.java:655-664`). Only the bulk
    /// merge asks, and only of the merge instance of a reader on the current
    /// format version: the answer is about the block a *merge* instance keeps
    /// decoded, which a search instance does not keep in the same way.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] with Java's message when this is
    /// not a merge instance, or when the reader is not on the current version.
    fn is_loaded(&self, doc_id: i32) -> Result<bool> {
        if !self.merging {
            return Err(LuceneError::IllegalState(
                "isLoaded should only ever get called on a merge instance".to_string(),
            ));
        }
        if self.version != DATA_VERSION_CURRENT {
            return Err(LuceneError::IllegalState(
                "isLoaded should only ever get called when the reader is on the current version"
                    .to_string(),
            ));
        }
        Ok(self.state.lock().unwrap().contains(doc_id))
    }

    /// Returns one document as it sits inside its decompressed chunk: the raw
    /// bytes of its stored fields, their total length, and how many there are.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the chunk header or the
    /// document offsets are inconsistent, and propagates any I/O error.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsReader#serializedDocument`.
    fn serialized_document(&self, doc_id: i32) -> Result<SerializedDocument> {
        let mut state = self.state.lock().unwrap();
        if !state.contains(doc_id) {
            let mut input = self.fields_stream.clone_input()?;
            let start_pointer = self.index_reader.get_start_pointer(doc_id)?;
            input.seek(start_pointer)?;
            state.reset(
                doc_id,
                input.as_mut(),
                self.num_docs,
                self.chunk_size,
                &mut **self.decompressor.lock().unwrap(),
                self.merging,
            )?;
        }
        debug_assert!(state.contains(doc_id));

        let mut decompressor = self.decompressor.lock().unwrap();
        if self.merging {
            state.document(doc_id, None, self.chunk_size, &mut **decompressor, true)
        } else {
            let mut inp = self.fields_stream.clone_input()?;
            let start_pointer = self.index_reader.get_start_pointer(doc_id)?;
            inp.seek(start_pointer)?;
            state.document(
                doc_id,
                Some(inp.as_mut()),
                self.chunk_size,
                &mut **decompressor,
                false,
            )
        }
    }

    fn read_field(
        input: &mut dyn DataInput,
        visitor: &mut dyn StoredFieldVisitor,
        info: &FieldInfo,
        bits: i32,
    ) -> Result<()> {
        match bits & TYPE_MASK {
            TYPE_BYTE_ARR => {
                // Java hands the visitor a `StoredFieldDataInput` over the live
                // document cursor so that a visitor which only forwards the
                // bytes (`SortingStoredFieldsConsumer.CopyVisitor`) never
                // materialises them. The default visitor callback still copies
                // into a fresh buffer, so a plain visitor sees the same bytes.
                let length = input.read_v_int()?;
                let mut value = StoredFieldDataInput::new(input, length);
                visitor.binary_field_data_input(info, &mut value)
            }
            TYPE_STRING => {
                let s = input.read_string()?;
                visitor.string_field(info, &s)
            }
            TYPE_NUMERIC_INT => visitor.int_field(info, input.read_z_int()?),
            TYPE_NUMERIC_FLOAT => visitor.float_field(info, Self::read_z_float(input)?),
            TYPE_NUMERIC_LONG => visitor.long_field(info, Self::read_t_long(input)?),
            TYPE_NUMERIC_DOUBLE => visitor.double_field(info, Self::read_z_double(input)?),
            _ => Err(LuceneError::CorruptIndex(format!(
                "Unknown type flag: 0x{bits:x}"
            ))),
        }
    }

    fn skip_field(input: &mut dyn DataInput, bits: i32) -> Result<()> {
        match bits & TYPE_MASK {
            TYPE_BYTE_ARR | TYPE_STRING => {
                let length = input.read_v_int()? as i64;
                input.skip_bytes(length)
            }
            TYPE_NUMERIC_INT => {
                input.read_z_int()?;
                Ok(())
            }
            TYPE_NUMERIC_FLOAT => {
                Self::read_z_float(input)?;
                Ok(())
            }
            TYPE_NUMERIC_LONG => {
                Self::read_t_long(input)?;
                Ok(())
            }
            TYPE_NUMERIC_DOUBLE => {
                Self::read_z_double(input)?;
                Ok(())
            }
            _ => Err(LuceneError::CorruptIndex(format!(
                "Unknown type flag: 0x{bits:x}"
            ))),
        }
    }

    fn read_z_float(input: &mut dyn DataInput) -> Result<f32> {
        let b = input.read_byte()? as u32;
        if b == 0xFF {
            Ok(f32::from_bits(StoredFieldsInts::read_le_int(input)?))
        } else if (b & 0x80) != 0 {
            // Small integer in [-1..125]. Java computes `(b & 0x7f) - 1` in a
            // signed `int`; doing it in `u32` underflows for the encoding of
            // -1, whose header byte is exactly 0x80.
            Ok(((b & 0x7F) as i32 - 1) as f32)
        } else {
            let bits = (b << 24)
                | ((StoredFieldsInts::read_le_short(input)? as u32) << 8)
                | (input.read_byte()? as u32);
            Ok(f32::from_bits(bits))
        }
    }

    fn read_z_double(input: &mut dyn DataInput) -> Result<f64> {
        let b = input.read_byte()? as u64;
        if b == 0xFF {
            Ok(f64::from_bits(StoredFieldsInts::read_le_long(input)?))
        } else if b == 0xFE {
            let float_bits = StoredFieldsInts::read_le_int(input)?;
            Ok(f64::from(f32::from_bits(float_bits)))
        } else if (b & 0x80) != 0 {
            // Small integer in [-1..124]; see `read_z_float` for why this must
            // be signed arithmetic.
            Ok(((b & 0x7F) as i32 - 1) as f64)
        } else {
            let bits = (b << 56)
                | ((StoredFieldsInts::read_le_int(input)? as u64) << 24)
                | ((StoredFieldsInts::read_le_short(input)? as u64) << 8)
                | (input.read_byte()? as u64);
            Ok(f64::from_bits(bits))
        }
    }

    fn read_t_long(input: &mut dyn DataInput) -> Result<i64> {
        let header = input.read_byte()? as i64;
        let mut bits = header & 0x1F;
        if (header & 0x20) != 0 {
            bits |= input.read_v_long()? << 5;
        }
        let mut l = BitUtil::zig_zag_decode_long(bits);
        // A corrupt stream can encode a magnitude that overflows the scaling
        // step. Java multiplies in a `long` and wraps silently
        // (`Reader.java:373-382`), returning a nonsensical timestamp rather
        // than failing, so this port wraps too: a decoder must not abort the
        // process on bytes it did not write, and inventing an error here would
        // reject values Lucene accepts.
        match header as i32 & DAY_ENCODING {
            SECOND_ENCODING => l = l.wrapping_mul(SECOND_MS),
            HOUR_ENCODING => l = l.wrapping_mul(HOUR_MS),
            DAY_ENCODING => l = l.wrapping_mul(DAY_MS),
            0 => {}
            _ => {
                return Err(LuceneError::CorruptIndex(
                    "Invalid TLong encoding".to_string(),
                ))
            }
        }
        Ok(l)
    }
}

// -----------------------------------------------------------------------------
// Lucene90CompressingStoredFieldsFormat
// -----------------------------------------------------------------------------

/// Concrete stored-fields format that writes compressed chunks.
#[derive(Debug, Clone, Copy)]
pub struct Lucene90CompressingStoredFieldsFormat {
    format_name: &'static str,
    compression_mode: CompressionMode,
    chunk_size: usize,
    max_docs_per_chunk: usize,
    block_shift: i32,
}

impl Lucene90CompressingStoredFieldsFormat {
    /// Creates a new format instance.
    pub fn new(
        format_name: &'static str,
        compression_mode: CompressionMode,
        chunk_size: usize,
        max_docs_per_chunk: usize,
        block_shift: i32,
    ) -> Result<Self> {
        if chunk_size < 1 {
            return Err(LuceneError::IllegalArgument(
                "chunkSize must be >= 1".to_string(),
            ));
        }
        if max_docs_per_chunk < 1 {
            return Err(LuceneError::IllegalArgument(
                "maxDocsPerChunk must be >= 1".to_string(),
            ));
        }
        if !(DirectMonotonicWriter::MIN_BLOCK_SHIFT..=DirectMonotonicWriter::MAX_BLOCK_SHIFT)
            .contains(&block_shift)
        {
            return Err(LuceneError::IllegalArgument(format!(
                "blockShift must be in [{}, {}], got {block_shift}",
                DirectMonotonicWriter::MIN_BLOCK_SHIFT,
                DirectMonotonicWriter::MAX_BLOCK_SHIFT
            )));
        }
        Ok(Self {
            format_name,
            compression_mode,
            chunk_size,
            max_docs_per_chunk,
            block_shift,
        })
    }
}

impl StoredFieldsFormat for Lucene90CompressingStoredFieldsFormat {
    fn name(&self) -> &str {
        self.format_name
    }

    fn fields_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(Lucene90CompressingStoredFieldsReader::new(
            directory,
            segment_info,
            "",
            field_infos,
            context,
            self.format_name,
            self.compression_mode,
        )?))
    }

    fn fields_writer(
        &self,
        directory: &std::sync::Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        Ok(Box::new(Lucene90CompressingStoredFieldsWriter::new(
            directory,
            segment_info,
            "",
            context,
            self.format_name,
            self.compression_mode,
            self.chunk_size,
            self.max_docs_per_chunk,
            self.block_shift,
        )?))
    }
}

// -----------------------------------------------------------------------------
// Lucene90StoredFieldsFormat
// -----------------------------------------------------------------------------

/// Stored-fields mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// LZ4-based compression tuned for speed.
    BestSpeed,
    /// Deflate-based compression tuned for ratio.
    BestCompression,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::BestSpeed
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::BestSpeed => write!(f, "BEST_SPEED"),
            Mode::BestCompression => write!(f, "BEST_COMPRESSION"),
        }
    }
}

/// Lucene 9.0 stored-fields format.
#[derive(Debug, Clone, Copy)]
pub struct Lucene90StoredFieldsFormat {
    mode: Mode,
}

impl Default for Lucene90StoredFieldsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl Lucene90StoredFieldsFormat {
    /// Codec attribute key used to persist the chosen mode.
    pub const MODE_KEY: &str = "Lucene90StoredFieldsFormat.mode";

    /// Creates the format with the default mode (`BestSpeed`).
    pub fn new() -> Self {
        Self {
            mode: Mode::BestSpeed,
        }
    }

    /// Creates the format with the given mode.
    pub fn with_mode(mode: Mode) -> Self {
        Self { mode }
    }

    /// Returns the compressing format that implements `mode`.
    ///
    /// Equivalent to the package-private `Lucene90StoredFieldsFormat#impl`
    /// (`Lucene90StoredFieldsFormat.java:157-172`); `impl` is a Rust keyword,
    /// hence the name. Java's `default` branch throws `AssertionError`; the
    /// `match` over [`Mode`] is exhaustive, so it has no counterpart. The
    /// result does not depend on `self`, exactly as in Java, where the method
    /// reads no field.
    pub fn impl_for_mode(&self, mode: Mode) -> Lucene90CompressingStoredFieldsFormat {
        match mode {
            Mode::BestSpeed => Lucene90CompressingStoredFieldsFormat {
                format_name: "Lucene90StoredFieldsFastData",
                // `Lucene90StoredFieldsFormat.BEST_SPEED_MODE` is
                // `LZ4WithPresetDictCompressionMode`, not plain LZ4: the chunk
                // is split into ten sub blocks sharing one dictionary.
                compression_mode: CompressionMode::LZ4_WITH_PRESET_DICT,
                chunk_size: 10 * 8 * 1024,
                max_docs_per_chunk: 1024,
                block_shift: 10,
            },
            Mode::BestCompression => Lucene90CompressingStoredFieldsFormat {
                format_name: "Lucene90StoredFieldsHighData",
                // `Lucene90StoredFieldsFormat.BEST_COMPRESSION_MODE` is
                // `DeflateWithPresetDictCompressionMode`, not plain deflate.
                compression_mode: CompressionMode::DEFLATE_WITH_PRESET_DICT,
                chunk_size: 10 * 48 * 1024,
                max_docs_per_chunk: 4096,
                block_shift: 10,
            },
        }
    }
}

impl StoredFieldsFormat for Lucene90StoredFieldsFormat {
    fn name(&self) -> &str {
        "Lucene90StoredFieldsFormat"
    }

    fn fields_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        let mode = match segment_info.get_attribute(Self::MODE_KEY) {
            Some(v) => match v.as_str() {
                "BEST_SPEED" => Mode::BestSpeed,
                "BEST_COMPRESSION" => Mode::BestCompression,
                _ => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "unknown mode {v} for {}",
                        Self::MODE_KEY
                    )))
                }
            },
            None => {
                return Err(LuceneError::IllegalState(format!(
                    "missing value for {} for segment: {}",
                    Self::MODE_KEY,
                    segment_info.name
                )))
            }
        };
        self.impl_for_mode(mode)
            .fields_reader(directory, segment_info, field_infos, context)
    }

    fn fields_writer(
        &self,
        directory: &std::sync::Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        let mode_name = format!("{}", self.mode);
        let previous = segment_info.put_attribute(Self::MODE_KEY.to_string(), mode_name);
        if let Some(prev) = previous {
            if prev != format!("{}", self.mode) {
                return Err(LuceneError::IllegalState(format!(
                    "found existing value for {} for segment: {} old={prev}, new={}",
                    Self::MODE_KEY,
                    segment_info.name,
                    self.mode
                )));
            }
        }
        self.impl_for_mode(self.mode)
            .fields_writer(directory, segment_info, context)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::FieldInfo as RealFieldInfo;
    use crate::search::Sort;
    use crate::store::RamDirectory;
    use crate::store::DEFAULT_IO_CONTEXT;
    use crate::util::string_helper::ID_LENGTH;
    use crate::util::Bits;
    use crate::util::Version;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct CollectingVisitor {
        fields: Vec<(String, StoredValue)>,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum StoredValue {
        Int(i32),
        Long(i64),
        Float(f32),
        Double(f64),
        Bytes(Vec<u8>),
        String(String),
    }

    impl StoredFieldVisitor for CollectingVisitor {
        fn binary_field(&mut self, info: &FieldInfo, value: &[u8]) -> Result<()> {
            self.fields
                .push((info.name.clone(), StoredValue::Bytes(value.to_vec())));
            Ok(())
        }

        fn string_field(&mut self, info: &FieldInfo, value: &str) -> Result<()> {
            self.fields
                .push((info.name.clone(), StoredValue::String(value.to_string())));
            Ok(())
        }

        fn int_field(&mut self, info: &FieldInfo, value: i32) -> Result<()> {
            self.fields
                .push((info.name.clone(), StoredValue::Int(value)));
            Ok(())
        }

        fn long_field(&mut self, info: &FieldInfo, value: i64) -> Result<()> {
            self.fields
                .push((info.name.clone(), StoredValue::Long(value)));
            Ok(())
        }

        fn float_field(&mut self, info: &FieldInfo, value: f32) -> Result<()> {
            self.fields
                .push((info.name.clone(), StoredValue::Float(value)));
            Ok(())
        }

        fn double_field(&mut self, info: &FieldInfo, value: f64) -> Result<()> {
            self.fields
                .push((info.name.clone(), StoredValue::Double(value)));
            Ok(())
        }

        fn needs_field(&mut self, _info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
            Ok(StoredFieldVisitorStatus::Yes)
        }
    }

    fn make_field_infos() -> FieldInfos {
        let mut id_field = RealFieldInfo::new("id", 0);
        id_field.index_options = crate::index::IndexOptions::DOCS;
        let mut text_field = RealFieldInfo::new("text", 1);
        text_field.index_options = crate::index::IndexOptions::DOCS_AND_FREQS;
        let mut value_field = RealFieldInfo::new("value", 2);
        value_field.index_options = crate::index::IndexOptions::DOCS;
        FieldInfos::new(vec![id_field, text_field, value_field]).unwrap()
    }

    fn make_segment_info(dir: Arc<dyn Directory>, name: &str, max_doc: i32) -> SegmentInfo {
        SegmentInfo::new(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            Arc::new(crate::codecs::tests::DummyCodec::new("Dummy")),
            HashMap::new(),
            [0u8; ID_LENGTH],
            HashMap::new(),
            Sort::default(),
        )
        .unwrap()
    }

    fn write_read_round_trip(mode: Mode) {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = make_field_infos();
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", 4);

        let format = Lucene90StoredFieldsFormat::with_mode(mode);
        {
            let mut writer = format
                .fields_writer(&dir, &segment_info, &*DEFAULT_IO_CONTEXT)
                .unwrap();

            // Document 0: one string
            writer.start_document().unwrap();
            writer
                .write_field_string(field_infos.field_info("id").unwrap(), "doc0")
                .unwrap();
            writer.finish_document().unwrap();

            // Document 1: int, long, float, double, binary
            writer.start_document().unwrap();
            writer
                .write_field_i32(field_infos.field_info("id").unwrap(), 42)
                .unwrap();
            writer
                .write_field_i64(field_infos.field_info("value").unwrap(), 1234567890123i64)
                .unwrap();
            writer
                .write_field_f32(
                    field_infos.field_info("value").unwrap(),
                    std::f32::consts::PI,
                )
                .unwrap();
            writer
                .write_field_f64(
                    field_infos.field_info("value").unwrap(),
                    std::f64::consts::E,
                )
                .unwrap();
            writer
                .write_field_bytes(field_infos.field_info("text").unwrap(), b"binary payload")
                .unwrap();
            writer.finish_document().unwrap();

            // Document 2: empty
            writer.start_document().unwrap();
            writer.finish_document().unwrap();

            // Document 3: negative values and timestamps
            writer.start_document().unwrap();
            writer
                .write_field_i32(field_infos.field_info("value").unwrap(), -7)
                .unwrap();
            writer
                .write_field_f64(field_infos.field_info("value").unwrap(), -0.5)
                .unwrap();
            writer
                .write_field_i64(field_infos.field_info("value").unwrap(), 86400000i64)
                .unwrap();
            writer
                .write_field_string(field_infos.field_info("text").unwrap(), "hello world")
                .unwrap();
            writer.finish_document().unwrap();

            writer.finish(4).unwrap();
            writer.close().unwrap();
        }

        let reader = format
            .fields_reader(
                dir.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap();

        let mut v0 = CollectingVisitor::default();
        reader.document(0, &mut v0).unwrap();
        assert_eq!(
            v0.fields,
            vec![("id".to_string(), StoredValue::String("doc0".to_string()))]
        );

        let mut v1 = CollectingVisitor::default();
        reader.document(1, &mut v1).unwrap();
        assert_eq!(
            v1.fields,
            vec![
                ("id".to_string(), StoredValue::Int(42)),
                ("value".to_string(), StoredValue::Long(1234567890123i64)),
                (
                    "value".to_string(),
                    StoredValue::Float(std::f32::consts::PI)
                ),
                (
                    "value".to_string(),
                    StoredValue::Double(std::f64::consts::E)
                ),
                (
                    "text".to_string(),
                    StoredValue::Bytes(b"binary payload".to_vec())
                ),
            ]
        );

        let mut v2 = CollectingVisitor::default();
        reader.document(2, &mut v2).unwrap();
        assert!(v2.fields.is_empty());

        let mut v3 = CollectingVisitor::default();
        reader.document(3, &mut v3).unwrap();
        assert_eq!(
            v3.fields,
            vec![
                ("value".to_string(), StoredValue::Int(-7)),
                ("value".to_string(), StoredValue::Double(-0.5)),
                ("value".to_string(), StoredValue::Long(86400000i64)),
                (
                    "text".to_string(),
                    StoredValue::String("hello world".to_string())
                ),
            ]
        );

        reader.check_integrity().unwrap();
        let _clone = reader.clone_reader();
        let _merge = reader.get_merge_instance();
    }

    #[test]
    fn round_trip_best_speed() {
        write_read_round_trip(Mode::BestSpeed);
    }

    /// `BEST_COMPRESSION` round-trips **within Rucene only**.
    ///
    /// Lucene's `BEST_COMPRESSION` uses
    /// `DeflateWithPresetDictCompressionMode`, which this port cannot reproduce
    /// while its deflate backend has no preset-dictionary support (see the
    /// module documentation). This test therefore pins self-consistency, not
    /// byte compatibility; `BEST_SPEED`, the mode every codec selects by
    /// default, is verified against Lucene in
    /// `tests/portability/stored_fields.rs`.
    #[test]
    fn round_trip_best_compression_within_rucene() {
        write_read_round_trip(Mode::BestCompression);
    }

    #[test]
    fn stored_fields_format_name() {
        let format = Lucene90StoredFieldsFormat::new();
        assert_eq!(format.name(), "Lucene90StoredFieldsFormat");

        let format = Lucene90StoredFieldsFormat::with_mode(Mode::BestCompression);
        assert_eq!(format.name(), "Lucene90StoredFieldsFormat");
    }

    #[test]
    fn mode_default_is_best_speed() {
        assert_eq!(Mode::default(), Mode::BestSpeed);
    }

    /// Regression test for a signed-shift bug in `write_t_long`.
    ///
    /// `Lucene90CompressingStoredFieldsWriter.writeTLong` splits the zig-zag
    /// encoding of the value into a 5-bit header remainder and an `upperBits`
    /// `vLong`, using Java's **unsigned** shift `>>>`. This port used Rust's
    /// `>>` on an `i64`, which is arithmetic: for any value whose zig-zag
    /// encoding has the sign bit set — magnitude 2^62 and above — `upperBits`
    /// came out negative and `write_v_long` rejected it outright, so those
    /// longs could not be stored at all.
    #[test]
    fn stored_longs_beyond_two_to_the_sixty_two_round_trip() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = make_field_infos();
        let value_field = field_infos.field_info("value").unwrap();

        // Every one of these has |zigZag(v)| >= 2^63, which is exactly the
        // range the arithmetic shift corrupted. 86_400_000_000_000_000 is the
        // same magnitude expressed as a whole number of days, so it also
        // exercises the DAY_ENCODING branch, where the scaling happens before
        // the shift.
        let values = [
            i64::MAX,
            i64::MIN,
            i64::MAX - 1,
            i64::MIN + 1,
            1i64 << 62,
            -(1i64 << 62),
            (1i64 << 62) + 12345,
            86_400_000 * 100_000_000_000i64,
        ];
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", values.len() as i32);
        let format = Lucene90StoredFieldsFormat::new();
        {
            let mut writer = format
                .fields_writer(&dir, &segment_info, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            for value in values {
                writer.start_document().unwrap();
                writer
                    .write_field_i64(value_field, value)
                    .unwrap_or_else(|error| panic!("writing {value} must succeed: {error}"));
                writer.finish_document().unwrap();
            }
            writer.finish(values.len() as i32).unwrap();
            writer.close().unwrap();
        }

        let reader = format
            .fields_reader(
                dir.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap();
        for (doc_id, expected) in values.iter().enumerate() {
            let mut visitor = CollectingVisitor::default();
            reader.document(doc_id as i32, &mut visitor).unwrap();
            assert_eq!(
                visitor.fields,
                vec![("value".to_string(), StoredValue::Long(*expected))],
                "doc {doc_id}"
            );
        }
    }

    /// The codec overrides `write_field_data_input` to stream the bytes, so the
    /// result must be indistinguishable from `write_field_bytes`, which is what
    /// Lucene's `writeField(FieldInfo, StoredFieldDataInput)` guarantees: both
    /// write a `BYTE_ARR` field.
    #[test]
    fn a_streamed_binary_field_is_byte_identical_to_a_copied_one() {
        use crate::index::StoredFieldDataInput;
        use crate::store::ByteArrayDataInput;

        let payload: Vec<u8> = (0..=255u8).collect();

        let write = |streamed: bool| -> Vec<u8> {
            let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let field_infos = make_field_infos();
            let text_field = field_infos.field_info("text").unwrap();
            let segment_info = make_segment_info(Arc::clone(&dir), "_0", 1);
            let format = Lucene90StoredFieldsFormat::new();
            {
                let mut writer = format
                    .fields_writer(&dir, &segment_info, &*DEFAULT_IO_CONTEXT)
                    .unwrap();
                writer.start_document().unwrap();
                if streamed {
                    let mut input = ByteArrayDataInput::new(&payload);
                    let mut value = StoredFieldDataInput::new(&mut input, payload.len() as i32);
                    writer
                        .write_field_data_input(text_field, &mut value)
                        .unwrap();
                } else {
                    writer.write_field_bytes(text_field, &payload).unwrap();
                }
                writer.finish_document().unwrap();
                writer.finish(1).unwrap();
                writer.close().unwrap();
            }
            let mut input = dir
                .open_input("_0_Lucene90FieldsIndex-doc_ids.fdt", &*DEFAULT_IO_CONTEXT)
                .or_else(|_| dir.open_input("_0.fdt", &*DEFAULT_IO_CONTEXT))
                .expect("the stored-fields data file");
            let length = input.length() as usize;
            let mut bytes = vec![0u8; length];
            input.read_bytes(&mut bytes, 0, length).expect("read");
            bytes
        };

        assert_eq!(
            write(true),
            write(false),
            "the streaming path must produce the same .fdt bytes as the copying path"
        );
    }

    /// A visitor that overrides the data-input callback must receive the value
    /// without the reader materialising it first, which is what
    /// `SortingStoredFieldsConsumer.CopyVisitor` relies on.
    #[test]
    fn the_reader_offers_binary_values_through_the_data_input_callback() {
        use crate::index::StoredFieldDataInput;

        #[derive(Default)]
        struct StreamingVisitor {
            streamed: Vec<Vec<u8>>,
            materialised: usize,
        }

        impl StoredFieldVisitor for StreamingVisitor {
            fn binary_field_data_input(
                &mut self,
                _info: &FieldInfo,
                value: &mut StoredFieldDataInput<'_>,
            ) -> Result<()> {
                let length = value.length() as usize;
                let mut bytes = vec![0u8; length];
                value.data_input().read_bytes(&mut bytes, 0, length)?;
                self.streamed.push(bytes);
                Ok(())
            }

            fn binary_field(&mut self, _info: &FieldInfo, _value: &[u8]) -> Result<()> {
                self.materialised += 1;
                Ok(())
            }

            fn needs_field(&mut self, _info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
                Ok(StoredFieldVisitorStatus::Yes)
            }
        }

        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = make_field_infos();
        let text_field = field_infos.field_info("text").unwrap();
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", 1);
        let format = Lucene90StoredFieldsFormat::new();
        {
            let mut writer = format
                .fields_writer(&dir, &segment_info, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            writer.start_document().unwrap();
            writer.write_field_bytes(text_field, b"first").unwrap();
            writer.write_field_bytes(text_field, b"second").unwrap();
            writer.finish_document().unwrap();
            writer.finish(1).unwrap();
            writer.close().unwrap();
        }
        let reader = format
            .fields_reader(
                dir.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap();
        let mut visitor = StreamingVisitor::default();
        reader.document(0, &mut visitor).unwrap();
        assert_eq!(
            visitor.streamed,
            vec![b"first".to_vec(), b"second".to_vec()]
        );
        assert_eq!(
            visitor.materialised, 0,
            "the reader must not copy the bytes when the visitor streams them"
        );
    }

    /// Regression test for a byte-order bug in the fixed-width primitives.
    ///
    /// Lucene's `DataOutput.writeShort/writeInt/writeLong` are **little-endian**
    /// and `writeZFloat`/`writeZDouble` go through them. This port wrote them
    /// big-endian, so every stored float and double, and every
    /// `StoredFieldsInts` block of 128 or more values, had the wrong byte order
    /// on disk: readable by Rucene, unreadable by Lucene.
    ///
    /// The expected bytes below are derived from the Java algorithm, not from
    /// this port's output.
    #[test]
    fn z_float_and_z_double_use_lucenes_little_endian_byte_order() {
        fn write_float(value: f32) -> Vec<u8> {
            let mut out = crate::store::VecDataOutput::new();
            Lucene90CompressingStoredFieldsWriter::write_z_float(&mut out, value).unwrap();
            out.into_inner()
        }
        fn write_double(value: f64) -> Vec<u8> {
            let mut out = crate::store::VecDataOutput::new();
            Lucene90CompressingStoredFieldsWriter::write_z_double(&mut out, value).unwrap();
            out.into_inner()
        }

        // Float.MAX_VALUE: writeByte(bits>>24), writeShort(bits>>>8) LE,
        // writeByte(bits).
        assert_eq!(write_float(f32::MAX), vec![0x7f, 0xff, 0x7f, 0xff]);
        // A negative float: writeByte(0xFF) then writeInt(bits) LE.
        assert_eq!(write_float(-1.5), vec![0xff, 0x00, 0x00, 0xc0, 0xbf]);
        // Double.MAX_VALUE: writeByte(bits>>56), writeInt(bits>>>24) LE,
        // writeShort(bits>>>8) LE, writeByte(bits).
        assert_eq!(
            write_double(f64::MAX),
            vec![0x7f, 0xff, 0xff, 0xff, 0xef, 0xff, 0xff, 0xff]
        );
        // A negative double with no exact float form: writeByte(0xFF) then
        // writeLong(bits) LE.
        assert_eq!(
            write_double(-0.1),
            vec![0xff, 0x9a, 0x99, 0x99, 0x99, 0x99, 0x99, 0xb9, 0xbf]
        );

        // And every one of them still reads back to the value written.
        for value in [f32::MAX, f32::MIN, -1.5f32, 0.0f32, -0.0f32, 1.0f32] {
            let bytes = write_float(value);
            let mut input = ByteArrayDataInput::new(&bytes);
            let read = Lucene90CompressingStoredFieldsReader::read_z_float(&mut input).unwrap();
            assert_eq!(read.to_bits(), value.to_bits(), "float {value}");
        }
        for value in [
            f64::MAX,
            f64::MIN,
            -0.1f64,
            0.0f64,
            -0.0f64,
            1.0f64,
            -1.5f64,
        ] {
            let bytes = write_double(value);
            let mut input = ByteArrayDataInput::new(&bytes);
            let read = Lucene90CompressingStoredFieldsReader::read_z_double(&mut input).unwrap();
            assert_eq!(read.to_bits(), value.to_bits(), "double {value}");
        }
    }

    /// Regression test for the same byte-order bug in `StoredFieldsInts`.
    ///
    /// A block of 128 values is transposed into sixteen longs and written with
    /// `DataOutput.writeLong`, which is little-endian; the first byte on disk is
    /// therefore `values[112]`, not `values[0]`. Writing the long big-endian
    /// reversed every group of eight values.
    #[test]
    fn stored_fields_ints_writes_full_blocks_little_endian() {
        let values: Vec<i32> = (0..128).collect();
        let mut out = crate::store::VecDataOutput::new();
        StoredFieldsInts::write_ints(&values, 0, values.len(), &mut out).unwrap();
        let bytes = out.into_inner();

        assert_eq!(bytes[0], 8, "the maximum is 127, so eight bits per value");
        // First long: values[0]<<56 | values[16]<<48 | ... | values[112],
        // written little-endian.
        assert_eq!(
            &bytes[1..9],
            &[112u8, 96, 80, 64, 48, 32, 16, 0],
            "the first byte written must be values[112]"
        );

        let mut input = ByteArrayDataInput::new(&bytes);
        let mut read = vec![0i64; 128];
        StoredFieldsInts::read_ints(&mut input, 128, &mut read, 0).unwrap();
        let expected: Vec<i64> = (0..128).collect();
        assert_eq!(read, expected);
    }

    /// Regression test: the reader must use the directory it is handed.
    ///
    /// `Lucene90CompressingStoredFieldsFormat.fieldsReader` used to read from
    /// `segmentInfo.directory` instead of the `Directory` argument. That is
    /// wrong for a compound-file segment, whose files exist only inside the
    /// `.cfs`, and it made every Java-written index unreadable: Rucene's
    /// `SegmentInfoFormat` deliberately attaches an empty placeholder directory
    /// when it parses a `.si`, precisely because the real files are reached
    /// through the directory passed to the format.
    #[test]
    fn the_reader_uses_the_directory_it_is_given_not_the_one_on_the_segment_info() {
        let data_dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = make_field_infos();
        let segment_info = make_segment_info(Arc::clone(&data_dir), "_0", 1);
        let format = Lucene90StoredFieldsFormat::new();
        {
            let mut writer = format
                .fields_writer(&data_dir, &segment_info, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            writer.start_document().unwrap();
            writer
                .write_field_string(field_infos.field_info("id").unwrap(), "only")
                .unwrap();
            writer.finish_document().unwrap();
            writer.finish(1).unwrap();
            writer.close().unwrap();
        }

        // The segment info now points at an unrelated, empty directory, exactly
        // as it does after `SegmentInfos.readLatestCommit`.
        let elsewhere: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let detached = make_segment_info(Arc::clone(&elsewhere), "_0", 1);
        detached.put_attribute(
            Lucene90StoredFieldsFormat::MODE_KEY.to_string(),
            "BEST_SPEED".to_string(),
        );
        assert!(
            elsewhere.list_all().unwrap().is_empty(),
            "the placeholder directory must really be empty"
        );

        let reader = format
            .fields_reader(
                data_dir.as_ref(),
                &detached,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .expect("the reader must read from the directory it is given");
        let mut visitor = CollectingVisitor::default();
        reader.document(0, &mut visitor).unwrap();
        assert_eq!(
            visitor.fields,
            vec![("id".to_string(), StoredValue::String("only".to_string()))]
        );
        reader.check_integrity().unwrap();
    }

    /// Regression test: a block whose header parsed but whose body is corrupt
    /// must not stay cached.
    ///
    /// `Lucene90CompressingStoredFieldsReader.BlockState.reset` wraps `doReset`
    /// and sets `chunkDocs = 0` on failure, so `contains` stops matching and
    /// the header is decoded again. Without that, the first read reported the
    /// corruption and every later read of the same block was served from the
    /// half-decoded state — returning an empty document instead of an error.
    #[test]
    fn a_corrupt_chunk_reports_the_same_error_on_every_read() {
        // Five documents whose stored lengths are all different, so the chunk
        // header records them one byte each and the sequence can be located
        // unambiguously in the uncompressed part of the `.fdt`.
        let values: Vec<String> = (0..5).map(|i| "x".repeat(10 + i)).collect();
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = make_field_infos();
        let id_field = field_infos.field_info("id").unwrap();
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", 5);
        let format = Lucene90StoredFieldsFormat::new();
        {
            let mut writer = format
                .fields_writer(&dir, &segment_info, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            for value in &values {
                writer.start_document().unwrap();
                writer.write_field_string(id_field, value).unwrap();
                writer.finish_document().unwrap();
            }
            writer.finish(5).unwrap();
            writer.close().unwrap();
        }

        let mut bytes = {
            let mut input = dir.open_input("_0.fdt", &*DEFAULT_IO_CONTEXT).unwrap();
            let length = input.length() as usize;
            let mut buffer = vec![0u8; length];
            input.read_bytes(&mut buffer, 0, length).unwrap();
            buffer
        };

        // `saveInts` writes `08` (eight bits per value) followed by one byte per
        // document; each stored value is a one-byte field header, a one-byte
        // length and the characters.
        let lengths: Vec<u8> = values.iter().map(|v| (v.len() + 2) as u8).collect();
        let mut needle = vec![8u8];
        needle.extend_from_slice(&lengths);
        let position = bytes
            .windows(needle.len())
            .position(|window| window == needle)
            .expect("the per-document lengths must appear verbatim in the chunk header");
        assert!(
            bytes[position + 1..]
                .windows(needle.len())
                .all(|window| window != needle),
            "the pattern must be unique, or the corruption would be ambiguous"
        );
        // Doc 4 now claims zero bytes while still claiming one stored field,
        // which is exactly the inconsistency `doReset` validates.
        let last = position + needle.len() - 1;
        bytes[last] = 0;

        let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        {
            let mut output = corrupt
                .create_output("_0.fdt", &*DEFAULT_IO_CONTEXT)
                .unwrap();
            output.write_bytes(&bytes, 0, bytes.len()).unwrap();
            output.close().unwrap();
        }
        for name in ["_0.fdx", "_0.fdm"] {
            let mut input = dir.open_input(name, &*DEFAULT_IO_CONTEXT).unwrap();
            let length = input.length() as usize;
            let mut buffer = vec![0u8; length];
            input.read_bytes(&mut buffer, 0, length).unwrap();
            let mut output = corrupt.create_output(name, &*DEFAULT_IO_CONTEXT).unwrap();
            output.write_bytes(&buffer, 0, buffer.len()).unwrap();
            output.close().unwrap();
        }

        let reader = format
            .fields_reader(
                corrupt.as_ref(),
                &segment_info,
                &field_infos,
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap();

        let mut messages = Vec::new();
        for attempt in 0..3 {
            let mut visitor = CollectingVisitor::default();
            match reader.document(0, &mut visitor) {
                Ok(()) => panic!(
                    "read {attempt} of a corrupt block must fail, \
                     not return {:?}",
                    visitor.fields
                ),
                Err(error) => messages.push(error.to_string()),
            }
        }
        assert_eq!(messages.len(), 3);
        assert!(
            messages.iter().all(|message| *message == messages[0]),
            "every read must report the same corruption: {messages:?}"
        );
        assert!(
            messages[0].contains("num_stored_fields"),
            "unexpected error: {}",
            messages[0]
        );
    }

    /// Regression test: `close()` must release every resource, not stop at the
    /// first failure, and must include the fields index.
    ///
    /// Equivalent to `IOUtils.close(metaStream, fieldsStream, indexWriter,
    /// compressor)`. `FieldsIndexWriter` streams the index to two temporary
    /// outputs (`FieldsIndexWriter.java:83-87`); a writer closed without being
    /// finished — the abort path — closes and deletes them
    /// (`FieldsIndexWriter.java:183-201`), so no `.fdx` and no temporary file
    /// is left behind.
    #[test]
    fn closing_the_writer_releases_every_stream() {
        let dir = Arc::new(CountingDirectory::new(RamDirectory::new()));
        let field_infos = make_field_infos();
        let segment_info = make_segment_info(Arc::clone(&dir) as Arc<dyn Directory>, "_0", 1);
        let format = Lucene90StoredFieldsFormat::new();
        let mut writer = format
            .fields_writer(
                &(Arc::clone(&dir) as Arc<dyn Directory>),
                &segment_info,
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap();
        writer.start_document().unwrap();
        writer
            .write_field_string(field_infos.field_info("id").unwrap(), "abandoned")
            .unwrap();
        writer.finish_document().unwrap();
        assert_eq!(
            dir.opened(),
            4,
            "the .fdm, the .fdt and the index's two temporary outputs are all created"
        );
        assert_eq!(dir.closed(), 0);

        // Abort: close without finishing, exactly as `StoredFieldsConsumer.abort`
        // does.
        writer.close().unwrap();
        assert_eq!(
            dir.closed(),
            4,
            "the fields index's outputs must be closed alongside the two streams"
        );
        let mut files = dir.list_all().unwrap();
        files.sort();
        assert_eq!(
            files,
            vec!["_0.fdm".to_string(), "_0.fdt".to_string()],
            "the aborted fields index must delete its temporary files and write no .fdx"
        );
    }

    /// A `Directory` that counts how many outputs it handed out and how many
    /// of them were closed.
    #[derive(Debug)]
    struct CountingDirectory {
        inner: RamDirectory,
        opened: Arc<std::sync::atomic::AtomicUsize>,
        closed: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl CountingDirectory {
        fn new(inner: RamDirectory) -> Self {
            Self {
                inner,
                opened: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                closed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }
        fn opened(&self) -> usize {
            self.opened.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn closed(&self) -> usize {
            self.closed.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    /// Wraps an `IndexOutput` and records the close.
    struct CountingOutput {
        inner: Box<dyn IndexOutput>,
        closed: Arc<std::sync::atomic::AtomicUsize>,
        already: bool,
    }

    impl crate::store::DataOutput for CountingOutput {
        fn write_byte(&mut self, b: u8) -> Result<()> {
            self.inner.write_byte(b)
        }
        fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
            self.inner.write_bytes(b, offset, len)
        }
    }

    impl fmt::Debug for CountingOutput {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CountingOutput").finish_non_exhaustive()
        }
    }

    impl IndexOutput for CountingOutput {
        fn resource_description(&self) -> &str {
            self.inner.resource_description()
        }

        fn close(&mut self) -> Result<()> {
            if !self.already {
                self.already = true;
                self.closed
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            self.inner.close()
        }
        fn file_pointer(&self) -> i64 {
            self.inner.file_pointer()
        }
        fn checksum(&self) -> Result<i64> {
            self.inner.checksum()
        }
        fn name(&self) -> &str {
            self.inner.name()
        }
    }

    impl Directory for CountingDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }
        fn delete_file(&self, name: &str) -> Result<()> {
            self.inner.delete_file(name)
        }
        fn file_length(&self, name: &str) -> Result<i64> {
            self.inner.file_length(name)
        }
        fn create_output(
            &self,
            name: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.opened
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::new(CountingOutput {
                inner: self.inner.create_output(name, context)?,
                closed: Arc::clone(&self.closed),
                already: false,
            }))
        }
        fn create_temp_output(
            &self,
            prefix: &str,
            suffix: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.opened
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::new(CountingOutput {
                inner: self.inner.create_temp_output(prefix, suffix, context)?,
                closed: Arc::clone(&self.closed),
                already: false,
            }))
        }
        fn sync(&self, names: &[String]) -> Result<()> {
            self.inner.sync(names)
        }
        fn sync_metadata(&self) -> Result<()> {
            self.inner.sync_metadata()
        }
        fn rename(&self, source: &str, dest: &str) -> Result<()> {
            self.inner.rename(source, dest)
        }
        fn open_input(
            &self,
            name: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn crate::store::IndexInput>> {
            self.inner.open_input(name, context)
        }
        fn obtain_lock(&self, name: &str) -> Result<Box<dyn crate::store::Lock>> {
            self.inner.obtain_lock(name)
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
        fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
            self.inner.get_pending_deletions()
        }
    }

    /// Regression test for the single-byte small-integer form of `ZFloat` and
    /// `ZDouble`.
    ///
    /// `readZFloat`/`readZDouble` decode it as `(b & 0x7f) - 1` in a *signed*
    /// `int`. The value `-1` is written with header byte `0x80`, so `b & 0x7f`
    /// is zero and unsigned arithmetic underflows: a debug build panicked and a
    /// release build only got the right answer by wrapping around.
    #[test]
    fn the_small_integer_form_decodes_minus_one() {
        // -1 is the boundary; the rest of the range and the wide forms are
        // checked alongside it so the fix cannot narrow the encoding.
        for value in [-1.0f32, 0.0f32, 1.0f32, 125.0f32, 126.0f32, -2.0f32] {
            let mut out = crate::store::VecDataOutput::new();
            Lucene90CompressingStoredFieldsWriter::write_z_float(&mut out, value).unwrap();
            let encoded = out.into_inner();
            if (-1.0..=125.0).contains(&value) && value.fract() == 0.0 {
                assert_eq!(encoded.len(), 1, "{value} must use the one-byte form");
            }
            let mut input = ByteArrayDataInput::new(&encoded);
            let read = Lucene90CompressingStoredFieldsReader::read_z_float(&mut input).unwrap();
            assert_eq!(read.to_bits(), value.to_bits(), "float {value}");
        }
        for value in [-1.0f64, 0.0f64, 1.0f64, 124.0f64, 125.0f64, -2.0f64] {
            let mut out = crate::store::VecDataOutput::new();
            Lucene90CompressingStoredFieldsWriter::write_z_double(&mut out, value).unwrap();
            let encoded = out.into_inner();
            if (-1.0..=124.0).contains(&value) && value.fract() == 0.0 {
                assert_eq!(encoded.len(), 1, "{value} must use the one-byte form");
            }
            let mut input = ByteArrayDataInput::new(&encoded);
            let read = Lucene90CompressingStoredFieldsReader::read_z_double(&mut input).unwrap();
            assert_eq!(read.to_bits(), value.to_bits(), "double {value}");
        }
        // The one-byte encoding of -1 really is 0x80, which is what underflows.
        let mut out = crate::store::VecDataOutput::new();
        Lucene90CompressingStoredFieldsWriter::write_z_float(&mut out, -1.0).unwrap();
        assert_eq!(out.into_inner(), vec![0x80]);
    }

    /// Regression test: a chunk large enough to be *sliced* must read back.
    ///
    /// The writer slices once the buffered bytes reach twice the chunk size and
    /// compresses each slice independently, framing it with its own
    /// uncompressed length. Passing the whole chunk's length to the
    /// decompressor made a framed codec read compressed payload as framing.
    #[test]
    fn a_sliced_chunk_round_trips_in_both_modes() {
        for mode in [Mode::BestSpeed, Mode::BestCompression] {
            // BEST_SPEED chunks are 80 KiB, BEST_COMPRESSION 480 KiB; three
            // times the larger guarantees slicing in both.
            let payload: String = (0..1_500_000 / 12)
                .map(|i: usize| format!("slice{:06} ", i % 99991))
                .collect();
            let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let field_infos = make_field_infos();
            let id_field = field_infos.field_info("id").unwrap();
            let segment_info = make_segment_info(Arc::clone(&dir), "_0", 3);
            let format = Lucene90StoredFieldsFormat::with_mode(mode);
            {
                let mut writer = format
                    .fields_writer(&dir, &segment_info, &*DEFAULT_IO_CONTEXT)
                    .unwrap();
                writer.start_document().unwrap();
                writer.write_field_string(id_field, "before").unwrap();
                writer.finish_document().unwrap();
                writer.start_document().unwrap();
                writer.write_field_string(id_field, &payload).unwrap();
                writer.finish_document().unwrap();
                writer.start_document().unwrap();
                writer.write_field_string(id_field, "after").unwrap();
                writer.finish_document().unwrap();
                writer.finish(3).unwrap();
                writer.close().unwrap();
            }

            for merging in [false, true] {
                let base = format
                    .fields_reader(
                        dir.as_ref(),
                        &segment_info,
                        &field_infos,
                        &*DEFAULT_IO_CONTEXT,
                    )
                    .unwrap();
                let reader = if merging {
                    base.get_merge_instance()
                } else {
                    base
                };
                for (doc_id, expected) in [(0i32, "before"), (1, payload.as_str()), (2, "after")] {
                    let mut visitor = CollectingVisitor::default();
                    reader.document(doc_id, &mut visitor).unwrap();
                    assert_eq!(
                        visitor.fields,
                        vec![("id".to_string(), StoredValue::String(expected.to_string()))],
                        "mode={mode} merging={merging} doc={doc_id}"
                    );
                }
            }
        }
    }

    /// Regression test: the `TLong` scaling step must wrap, as Java's does.
    ///
    /// `readTLong` multiplies the decoded value by 1000, 3 600 000 or
    /// 86 400 000 depending on the header
    /// (`Lucene90CompressingStoredFieldsReader.java:373-382`). Java does that in
    /// a `long` and wraps silently, returning a nonsensical timestamp; a
    /// checked multiply aborts a debug build on bytes Lucene reads without
    /// complaint. The expected values below are computed from Java's semantics,
    /// not read off this port's output.
    #[test]
    fn the_t_long_scaling_step_wraps_like_java() {
        /// Encodes the header and zig-zag payload `readTLong` expects.
        fn encode(encoding: i32, zigzag: i64) -> Vec<u8> {
            let mut out = crate::store::VecDataOutput::new();
            let upper = ((zigzag as u64) >> 5) as i64;
            let mut header = (encoding as u8) | ((zigzag & 0x1F) as u8);
            if upper != 0 {
                header |= 0x20;
            }
            out.write_byte(header).unwrap();
            if upper != 0 {
                out.write_v_long(upper).unwrap();
            }
            out.into_inner()
        }

        for (encoding, scale) in [
            (SECOND_ENCODING, SECOND_MS),
            (HOUR_ENCODING, HOUR_MS),
            (DAY_ENCODING, DAY_MS),
        ] {
            for value in [i64::MAX / 3, i64::MIN / 3, 1 << 60, -(1 << 60)] {
                let zigzag = BitUtil::zig_zag_encode_long(value);
                let input_src = encode(encoding, zigzag);
                let mut input = ByteArrayDataInput::new(&input_src);
                let read = Lucene90CompressingStoredFieldsReader::read_t_long(&mut input)
                    .expect("a corrupt magnitude is a value, not an error");
                assert_eq!(
                    read,
                    value.wrapping_mul(scale),
                    "encoding {encoding:#x}, value {value}"
                );
            }
        }

        // And the ordinary values still round-trip through the writer.
        for value in [0i64, 1, -1, 1_000, 3_600_000, 86_400_000, -86_400_000] {
            let mut out = crate::store::VecDataOutput::new();
            Lucene90CompressingStoredFieldsWriter::write_t_long(&mut out, value).unwrap();
            let input_src = out.into_inner();
            let mut input = ByteArrayDataInput::new(&input_src);
            assert_eq!(
                Lucene90CompressingStoredFieldsReader::read_t_long(&mut input).unwrap(),
                value
            );
        }
    }

    /// Regression test: non-monotonic chunk offsets must be reported, not
    /// turned into a wild slice index.
    ///
    /// The per-document offsets are a running sum of the `lengths` array, which
    /// `StoredFieldsInts` can write with 32 bits per value; a corrupt entry is
    /// then a negative number and the sum goes backwards. Java computes the
    /// length as `Math.toIntExact(offsets[index + 1]) - offset`, so a negative
    /// length trips its array bounds and raises an exception. Casting the
    /// difference to `usize` produced a length near 2^64 and a panicking slice
    /// instead.
    ///
    /// The block state is built directly rather than through a corrupted file:
    /// the reader condemns a block as soon as any earlier check fails, so a
    /// file-level corruption reaches this particular guard only by accident.
    #[test]
    fn a_chunk_whose_offsets_go_backwards_is_reported() {
        let mut state = BlockState::new();
        state.doc_base = 0;
        state.chunk_docs = 2;
        state.sliced = false;
        // Document 1 would span [10, 5): a negative length.
        state.offsets = vec![0, 10, 5];
        state.num_stored_fields = vec![1, 1];
        state.start_pointer = 0;
        state.bytes = vec![0u8; 32];

        let mut decompressor = CompressionMode::LZ4_WITH_PRESET_DICT.new_decompressor();
        let error = match state.document(1, None, 1024, &mut *decompressor, true) {
            Ok(_) => panic!("a document cannot have a negative length"),
            Err(error) => error,
        };
        assert!(matches!(error, LuceneError::CorruptIndex(_)), "{error:?}");

        // An offset past the range an `int` can hold is the other half of the
        // same guard: Java's `Math.toIntExact` raises `ArithmeticException`.
        let mut huge = BlockState::new();
        huge.doc_base = 0;
        huge.chunk_docs = 1;
        huge.offsets = vec![0, i64::from(i32::MAX) + 1];
        huge.num_stored_fields = vec![1];
        huge.bytes = vec![0u8; 32];
        let error = match huge.document(0, None, 1024, &mut *decompressor, true) {
            Ok(_) => panic!("an offset past int range is corrupt"),
            Err(error) => error,
        };
        assert!(matches!(error, LuceneError::CorruptIndex(_)), "{error:?}");

        // And a well-formed block still reads.
        let mut good = BlockState::new();
        good.doc_base = 0;
        good.chunk_docs = 1;
        good.offsets = vec![0, 4];
        good.num_stored_fields = vec![0];
        good.bytes = vec![1u8, 2, 3, 4];
        assert!(
            good.document(0, None, 1024, &mut *decompressor, true)
                .is_ok(),
            "a valid block must still read"
        );
    }

    #[test]
    fn compressing_format_validates_parameters() {
        assert!(Lucene90CompressingStoredFieldsFormat::new(
            "Test",
            CompressionMode::FAST,
            0,
            1,
            10
        )
        .is_err());
        assert!(Lucene90CompressingStoredFieldsFormat::new(
            "Test",
            CompressionMode::FAST,
            1,
            0,
            10
        )
        .is_err());
        assert!(Lucene90CompressingStoredFieldsFormat::new(
            "Test",
            CompressionMode::FAST,
            1,
            1,
            10
        )
        .is_ok());
    }

    // -----------------------------------------------------------------------
    // Bulk merge — Lucene90CompressingStoredFieldsWriter#merge
    // -----------------------------------------------------------------------

    /// The format the merge tests use: chunks of four documents, so that a
    /// handful of tiny documents already makes several chunks and a merge has
    /// real chunk boundaries to copy. The production formats hold 1024 or 4096
    /// documents per chunk, which would take that many documents to exercise.
    const MERGE_TEST_FORMAT_NAME: &str = "TestBulkMergeStoredFields";
    const MERGE_TEST_CHUNK_SIZE: usize = 1024;
    const MERGE_TEST_DOCS_PER_CHUNK: usize = 4;

    fn merge_test_format() -> Lucene90CompressingStoredFieldsFormat {
        Lucene90CompressingStoredFieldsFormat::new(
            MERGE_TEST_FORMAT_NAME,
            CompressionMode::FAST,
            MERGE_TEST_CHUNK_SIZE,
            MERGE_TEST_DOCS_PER_CHUNK,
            10,
        )
        .unwrap()
    }

    /// A writer of the merge-test format with the given compression
    /// parameters, writing segment `name` into `dir`.
    fn merge_test_writer(
        dir: &Arc<dyn Directory>,
        name: &str,
        compression_mode: CompressionMode,
        chunk_size: usize,
    ) -> Lucene90CompressingStoredFieldsWriter {
        Lucene90CompressingStoredFieldsWriter::new(
            dir,
            &make_segment_info(Arc::clone(dir), name, 0),
            "",
            &*DEFAULT_IO_CONTEXT,
            MERGE_TEST_FORMAT_NAME,
            compression_mode,
            chunk_size,
            MERGE_TEST_DOCS_PER_CHUNK,
            10,
        )
        .unwrap()
    }

    /// One merge-test document: every stored field, in stored order.
    type MergeDoc = Vec<(String, StoredValue)>;

    /// The stored fields of merge-test document `n`.
    ///
    /// Every value type the format encodes appears, as do a multi-valued field
    /// and documents that lack the optional field altogether, so comparing two
    /// merged segments document by document compares every kind of stored
    /// field. The documents stay far below [`MERGE_TEST_CHUNK_SIZE`], so a
    /// chunk closes on the document count alone.
    fn merge_doc(n: i32) -> MergeDoc {
        let text = |value: StoredValue| ("text".to_string(), value);
        let mut fields = vec![("id".to_string(), StoredValue::String(format!("doc-{n}")))];
        match n.rem_euclid(6) {
            0 => {}
            1 => fields.push(text(StoredValue::Long(i64::from(n) * 1_000_000_007))),
            2 => fields.push(text(StoredValue::Float(n as f32 / 4.0))),
            3 => fields.push(text(StoredValue::Double(-f64::from(n) / 3.0))),
            4 => fields.push(text(StoredValue::Bytes(vec![
                (n & 0xff) as u8;
                (n % 17) as usize + 1
            ]))),
            _ => {
                fields.push(text(StoredValue::String(format!("first of {n}"))));
                fields.push(text(StoredValue::String(format!("second of {n}"))));
            }
        }
        fields.push(("value".to_string(), StoredValue::Int(n)));
        fields
    }

    /// The merge-test documents numbered by `numbers`, in that order.
    fn merge_docs(numbers: impl IntoIterator<Item = i32>) -> Vec<MergeDoc> {
        numbers.into_iter().map(merge_doc).collect()
    }

    /// Writes one source segment holding merge-test documents
    /// `first..first + count`, and returns its segment info.
    fn write_merge_source(
        dir: &Arc<dyn Directory>,
        name: &str,
        first: i32,
        count: i32,
    ) -> SegmentInfo {
        let field_infos = make_field_infos();
        let segment_info = make_segment_info(Arc::clone(dir), name, count);
        let mut writer = merge_test_format()
            .fields_writer(dir, &segment_info, &*DEFAULT_IO_CONTEXT)
            .unwrap();
        for n in first..first + count {
            writer.start_document().unwrap();
            for (name, value) in merge_doc(n) {
                let info = field_infos.field_info(&name).unwrap();
                match value {
                    StoredValue::Int(v) => writer.write_field_i32(info, v),
                    StoredValue::Long(v) => writer.write_field_i64(info, v),
                    StoredValue::Float(v) => writer.write_field_f32(info, v),
                    StoredValue::Double(v) => writer.write_field_f64(info, v),
                    StoredValue::Bytes(v) => writer.write_field_bytes(info, &v),
                    StoredValue::String(v) => writer.write_field_string(info, &v),
                }
                .unwrap();
            }
            writer.finish_document().unwrap();
        }
        writer.finish(count).unwrap();
        writer.close().unwrap();
        segment_info
    }

    /// Opens one source segment for merging: its merge instance, which is
    /// what `MergeState` holds for every reader (`MergeState.java:134-168`).
    fn open_merge_source(
        dir: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
    ) -> Box<dyn StoredFieldsReader> {
        merge_test_format()
            .fields_reader(
                dir.as_ref(),
                segment_info,
                &make_field_infos(),
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap()
            .get_merge_instance()
    }

    /// Opens one source segment as this format's concrete reader, whose
    /// recorded metadata a test may then alter.
    fn concrete_merge_source(
        dir: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
    ) -> Lucene90CompressingStoredFieldsReader {
        Lucene90CompressingStoredFieldsReader::new(
            dir.as_ref(),
            segment_info,
            "",
            &make_field_infos(),
            &*DEFAULT_IO_CONTEXT,
            MERGE_TEST_FORMAT_NAME,
            CompressionMode::FAST,
        )
        .unwrap()
    }

    /// `(numChunks, numDirtyChunks, numDirtyDocs)` of a source segment, as its
    /// own reader reports them.
    fn source_chunk_counters(
        dir: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
    ) -> (i64, i64, i64) {
        let reader = concrete_merge_source(dir, segment_info);
        (
            reader.num_chunks(),
            reader.num_dirty_chunks(),
            reader.num_dirty_docs(),
        )
    }

    /// A stored-fields reader that hides the concrete type it wraps.
    ///
    /// `get_merge_strategy` recognises this format's own reader through
    /// [`StoredFieldsReader::as_any`], whose default answer is `None`; a reader
    /// that keeps the default therefore takes the route Java takes for a
    /// foreign codec's reader, the visitor
    /// (`Lucene90CompressingStoredFieldsWriter.java:684-688`). That is what
    /// makes the merge routes comparable over the very same documents.
    #[derive(Debug)]
    struct OpaqueStoredFieldsReader(Box<dyn StoredFieldsReader>);

    impl StoredFieldsReader for OpaqueStoredFieldsReader {
        fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
            self.0.document(doc_id, visitor)
        }

        fn check_integrity(&self) -> Result<()> {
            self.0.check_integrity()
        }

        fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
            Box::new(Self(self.0.clone_reader()))
        }

        fn close(&mut self) -> Result<()> {
            self.0.close()
        }
    }

    /// Hides `reader`'s concrete type, forcing the visitor route for it.
    fn opaque(reader: Box<dyn StoredFieldsReader>) -> Box<dyn StoredFieldsReader> {
        Box::new(OpaqueStoredFieldsReader(reader))
    }

    /// The live documents of a `max_doc`-document segment whose `deleted`
    /// documents are deleted.
    fn live_docs_without(max_doc: i32, deleted: &[i32]) -> Box<dyn Bits> {
        let mut live = crate::util::FixedBitSet::new(max_doc as usize);
        for doc in 0..max_doc {
            if !deleted.contains(&doc) {
                live.set(doc as usize);
            }
        }
        Box::new(live)
    }

    /// The doc map of a `max_doc`-document segment placed at `doc_base` in the
    /// merged segment, its `deleted` documents dropped and the rest packed.
    fn doc_map_without(max_doc: i32, doc_base: i32, deleted: &[i32]) -> DocMap {
        let mut mapping = Vec::with_capacity(max_doc as usize);
        let mut next = doc_base;
        for doc in 0..max_doc {
            if deleted.contains(&doc) {
                mapping.push(-1);
            } else {
                mapping.push(next);
                next += 1;
            }
        }
        Arc::new(move |doc: i32| mapping[doc as usize])
    }

    /// The doc map that sends each `(from, to, merged)` run of a segment's
    /// documents `from..to` to `merged..`, as an index-sorted merge does when
    /// it interleaves segments.
    fn interleaving_doc_map(runs: &[(i32, i32, i32)]) -> DocMap {
        let runs = runs.to_vec();
        Arc::new(move |doc: i32| {
            runs.iter()
                .find(|&&(from, to, _)| from <= doc && doc < to)
                .map_or(-1, |&(from, _, merged)| merged + doc - from)
        })
    }

    /// Builds the merge state for the given source readers.
    fn stored_fields_merge_state_with(
        readers: Vec<Box<dyn StoredFieldsReader>>,
        doc_maps: Vec<DocMap>,
        max_docs: Vec<i32>,
        live_docs: Vec<Option<Box<dyn Bits>>>,
        needs_index_sort: bool,
    ) -> MergeState {
        let n = readers.len();
        let field_infos = make_field_infos();
        let merged_info =
            make_segment_info(Arc::new(RamDirectory::new()), "_m", max_docs.iter().sum());
        MergeState::new(
            doc_maps,
            merged_info,
            field_infos.clone(),
            readers.into_iter().map(Some).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| field_infos.clone()).collect(),
            live_docs,
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            (0..n).map(|_| None).collect(),
            max_docs,
            needs_index_sort,
        )
    }

    /// Builds the merge state of a plain, unsorted merge of segments without
    /// deletions, laid end to end.
    fn stored_fields_merge_state(
        readers: Vec<Box<dyn StoredFieldsReader>>,
        max_docs: Vec<i32>,
    ) -> MergeState {
        let mut doc_maps = Vec::with_capacity(max_docs.len());
        let mut doc_base = 0;
        for &max_doc in &max_docs {
            doc_maps.push(crate::index::identity_doc_map(max_doc, doc_base));
            doc_base += max_doc;
        }
        let live_docs = max_docs.iter().map(|_| None).collect();
        stored_fields_merge_state_with(readers, doc_maps, max_docs, live_docs, false)
    }

    /// What one merge produced.
    struct MergeOutcome {
        /// The directory holding the merged segment `_m`.
        dir: Arc<dyn Directory>,
        /// The merged segment's info.
        info: SegmentInfo,
        /// Every file of the merged segment, by name, sorted.
        files: Vec<(String, Vec<u8>)>,
        /// Every stored field of every merged document, in order.
        documents: Vec<MergeDoc>,
        /// `(numChunks, numDirtyChunks, numDirtyDocs)` as the merged segment
        /// records them in its `.fdm`.
        counters: (i64, i64, i64),
    }

    /// Runs one merge into a fresh directory and reports everything about it.
    fn run_stored_fields_merge(merge_state: &MergeState) -> MergeOutcome {
        let total_docs: i32 = merge_state
            .max_docs
            .iter()
            .zip(&merge_state.live_docs)
            .map(|(&max_doc, live)| match live {
                None => max_doc,
                Some(live) => (0..max_doc).filter(|&doc| live.get(doc as usize)).count() as i32,
            })
            .sum();
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let info = make_segment_info(Arc::clone(&dir), "_m", total_docs);
        let stored_fields = merge_test_format();
        {
            let mut writer = stored_fields
                .fields_writer(&dir, &info, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            assert_eq!(
                writer.merge(merge_state).unwrap(),
                total_docs,
                "the merge must write every live document of every source segment"
            );
            writer.close().unwrap();
        }

        let reader = concrete_merge_source(&dir, &info);
        let documents = (0..total_docs)
            .map(|doc_id| {
                let mut visitor = CollectingVisitor::default();
                reader.document(doc_id, &mut visitor).unwrap();
                visitor.fields
            })
            .collect();
        let counters = (
            reader.num_chunks(),
            reader.num_dirty_chunks(),
            reader.num_dirty_docs(),
        );

        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for name in dir.list_all().unwrap() {
            let mut input = dir.open_input(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            let length = input.length() as usize;
            let mut bytes = vec![0u8; length];
            input.read_bytes(&mut bytes, 0, length).unwrap();
            files.push((name, bytes));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));

        MergeOutcome {
            dir,
            info,
            files,
            documents,
            counters,
        }
    }

    /// `getMergeStrategy` picks BULK only for this format's own reader, at the
    /// current version, written with the writer's compression mode and chunk
    /// size, whose fields the merged segment numbers exactly as the source did,
    /// which carries no deletions and is not too dirty; DOC for such a reader
    /// that fails any of the last four conditions; and VISITOR otherwise
    /// (`Lucene90CompressingStoredFieldsWriter.java:681-701`, `663-668`).
    #[test]
    fn the_stored_fields_merge_strategy_is_lucenes() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let source = write_merge_source(&dir, "_0", 0, 10);
        let out: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = merge_test_writer(&out, "_w0", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
        let strategy_for = |writer: &Lucene90CompressingStoredFieldsWriter,
                            reader: Box<dyn StoredFieldsReader>,
                            live_docs: Option<Box<dyn Bits>>| {
            let state = stored_fields_merge_state_with(
                vec![reader],
                vec![crate::index::identity_doc_map(10, 0)],
                vec![10],
                vec![live_docs],
                false,
            );
            writer.get_merge_strategy(&state, &MatchingReaders::new(&state), 0)
        };

        assert_eq!(
            strategy_for(&writer, open_merge_source(&dir, &source), None),
            MergeStrategy::Bulk,
            "a matching, undeleted, clean segment of this very format is bulk merged"
        );

        // DOC: this format's reader, but the chunks cannot be reused as they are.
        assert_eq!(
            strategy_for(
                &writer,
                open_merge_source(&dir, &source),
                Some(live_docs_without(10, &[3]))
            ),
            MergeStrategy::Doc,
            "it's not worth fine-graining this if there are deletions"
        );
        let other_mode = merge_test_writer(
            &out,
            "_w1",
            CompressionMode::HIGH_COMPRESSION,
            MERGE_TEST_CHUNK_SIZE,
        );
        assert_eq!(
            strategy_for(&other_mode, open_merge_source(&dir, &source), None),
            MergeStrategy::Doc,
            "chunks compressed another way cannot be copied"
        );
        let other_chunk_size = merge_test_writer(
            &out,
            "_w2",
            CompressionMode::FAST,
            MERGE_TEST_CHUNK_SIZE * 2,
        );
        assert_eq!(
            strategy_for(&other_chunk_size, open_merge_source(&dir, &source), None),
            MergeStrategy::Doc,
            "chunks of another size cannot be copied"
        );

        // tooDirty: more dirty documents than one chunk holds AND more than one
        // per cent of the chunks dirty; each boundary on its own is not enough.
        for (dirty_docs, dirty_chunks, chunks, too_dirty) in [
            (MERGE_TEST_DOCS_PER_CHUNK as i64, 100, 100, false),
            (MERGE_TEST_DOCS_PER_CHUNK as i64 + 1, 1, 100, false),
            (MERGE_TEST_DOCS_PER_CHUNK as i64 + 1, 2, 100, true),
        ] {
            let mut reader = concrete_merge_source(&dir, &source);
            reader.num_dirty_docs = dirty_docs;
            reader.num_dirty_chunks = dirty_chunks;
            reader.num_chunks = chunks;
            assert_eq!(writer.too_dirty(&reader), too_dirty);
            assert_eq!(
                strategy_for(&writer, Box::new(reader), None),
                if too_dirty {
                    MergeStrategy::Doc
                } else {
                    MergeStrategy::Bulk
                },
                "dirty docs {dirty_docs}, dirty chunks {dirty_chunks} of {chunks}"
            );
        }

        // VISITOR: the documents have to be decoded field by field.
        assert_eq!(
            strategy_for(&writer, opaque(open_merge_source(&dir, &source)), None),
            MergeStrategy::Visitor,
            "a foreign reader has to be read field by field"
        );
        // The only format version there is today is the current one, so the
        // version check is reached by altering what the reader recorded.
        let mut old_version = concrete_merge_source(&dir, &source);
        old_version.version = DATA_VERSION_CURRENT - 1;
        assert_eq!(
            strategy_for(&writer, Box::new(old_version), None),
            MergeStrategy::Visitor,
            "a segment of another format version is re-encoded"
        );
        let mut renumbered = RealFieldInfo::new("id", 7);
        renumbered.index_options = crate::index::IndexOptions::DOCS;
        let mut state = stored_fields_merge_state(vec![open_merge_source(&dir, &source)], vec![10]);
        state.field_infos = vec![FieldInfos::new(vec![renumbered]).unwrap()];
        let matching = MatchingReaders::new(&state);
        assert!(!matching.matching_readers[0]);
        assert_eq!(
            writer.get_merge_strategy(&state, &matching, 0),
            MergeStrategy::Visitor,
            "a segment whose fields are numbered differently has to be remapped"
        );
    }

    /// The bulk route is the one `merge` actually takes, and it leaves a mark
    /// in the merged segment's `.fdm` that the other routes cannot leave.
    ///
    /// Copying chunks verbatim preserves the *source* chunk boundaries
    /// (`Lucene90CompressingStoredFieldsWriter.java:555-592`): two segments of
    /// ten and six documents, chunked four at a time, hold three and two
    /// chunks, the last of each a partial one the writer had to force out. A
    /// bulk merge therefore ends with five chunks, two of them dirty.
    /// Re-encoding the same sixteen documents packs them four at a time with
    /// nothing left over: four chunks, none dirty.
    ///
    /// The two merged segments differ on disk for that reason, yet must hold
    /// exactly the same documents.
    #[test]
    fn a_bulk_stored_fields_merge_copies_the_source_chunks_verbatim() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let first = write_merge_source(&dir, "_0", 0, 10);
        let second = write_merge_source(&dir, "_1", 100, 6);
        assert_eq!(
            source_chunk_counters(&dir, &first),
            (3, 1, 2),
            "ten documents chunked four at a time make three chunks, the last forced out"
        );
        assert_eq!(source_chunk_counters(&dir, &second), (2, 1, 2));

        let bulk = run_stored_fields_merge(&stored_fields_merge_state(
            vec![
                open_merge_source(&dir, &first),
                open_merge_source(&dir, &second),
            ],
            vec![10, 6],
        ));
        assert_eq!(
            bulk.counters,
            (5, 2, 4),
            "the bulk merge must carry over both source segments' chunk boundaries"
        );

        let visitor = run_stored_fields_merge(&stored_fields_merge_state(
            vec![
                opaque(open_merge_source(&dir, &first)),
                opaque(open_merge_source(&dir, &second)),
            ],
            vec![10, 6],
        ));
        assert_eq!(
            visitor.counters,
            (4, 0, 0),
            "re-encoding the documents repacks them into full chunks"
        );

        // Same information by two routes.
        let expected = merge_docs((0..10).chain(100..106));
        assert_eq!(bulk.documents, expected);
        assert_eq!(visitor.documents, expected);
    }

    /// Where the writer would have drawn the same chunk boundaries anyway, the
    /// bulk and the visitor route must produce the same index, byte for byte.
    /// This is the property Lucene's own comment says bulk merge broke in the
    /// past: "bulk merge is scary: its caused corruption bugs"
    /// (`Lucene90CompressingStoredFieldsWriter.java:493-495`).
    ///
    /// Two shapes are checked. A single source segment is copied chunk for
    /// chunk, so re-encoding it lands on the same boundaries; and two segments
    /// of exactly two full chunks each leave nothing buffered at the seam, so
    /// the copied and the re-encoded segment agree there too.
    #[test]
    fn bulk_and_visitor_stored_fields_merges_produce_the_same_index() {
        for sources in [vec![(0i32, 10i32)], vec![(0i32, 8i32), (100i32, 8i32)]] {
            let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let infos: Vec<SegmentInfo> = sources
                .iter()
                .enumerate()
                .map(|(i, &(first, count))| {
                    write_merge_source(&dir, &format!("_{i}"), first, count)
                })
                .collect();
            let max_docs: Vec<i32> = sources.iter().map(|&(_, count)| count).collect();
            let expected = merge_docs(
                sources
                    .iter()
                    .flat_map(|&(first, count)| first..first + count),
            );

            // The comparison only means something if the first merge really
            // copies chunks, so every source must be eligible for BULK.
            let bulk_state = stored_fields_merge_state(
                infos
                    .iter()
                    .map(|info| open_merge_source(&dir, info))
                    .collect(),
                max_docs.clone(),
            );
            let out: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let writer =
                merge_test_writer(&out, "_w", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
            let matching = MatchingReaders::new(&bulk_state);
            for i in 0..infos.len() {
                assert_eq!(
                    writer.get_merge_strategy(&bulk_state, &matching, i),
                    MergeStrategy::Bulk,
                    "sources {sources:?}: source {i} must be bulk merged"
                );
            }

            let bulk = run_stored_fields_merge(&bulk_state);
            let visitor = run_stored_fields_merge(&stored_fields_merge_state(
                infos
                    .iter()
                    .map(|info| opaque(open_merge_source(&dir, info)))
                    .collect(),
                max_docs.clone(),
            ));

            assert_eq!(bulk.documents, expected, "sources {sources:?}");
            assert_eq!(visitor.documents, expected, "sources {sources:?}");
            assert_eq!(
                bulk.counters, visitor.counters,
                "sources {sources:?}: both routes must draw the same chunk boundaries here"
            );
            assert_eq!(
                bulk.files.iter().map(|(name, _)| name).collect::<Vec<_>>(),
                visitor
                    .files
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
                "sources {sources:?}: both routes must write the same files"
            );
            for ((name, bulk_bytes), (_, visitor_bytes)) in
                bulk.files.iter().zip(visitor.files.iter())
            {
                assert_eq!(
                    bulk_bytes, visitor_bytes,
                    "sources {sources:?}: {name} differs between the bulk and the visitor merge"
                );
            }
        }
    }

    /// `merge` chooses a strategy per source segment and mixes them in one
    /// merge (`Lucene90CompressingStoredFieldsWriter.java:608-650`).
    ///
    /// Four segments: a clean one (BULK), one with a deleted document (DOC),
    /// one behind a foreign reader (VISITOR), and another clean one (BULK).
    /// The first copy keeps its three chunks `[4][4][2]`; the five live
    /// documents of the second and the six of the third are re-chunked as
    /// `[4][4]` with three left over, which the last copy forces out as a dirty
    /// chunk before carrying over its own `[4][2]`
    /// (`Lucene90CompressingStoredFieldsWriter.java:550-552`): eight chunks,
    /// three of them dirty, holding 2 + 3 + 2 documents.
    #[test]
    fn a_stored_fields_merge_picks_a_strategy_per_segment() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let a = write_merge_source(&dir, "_0", 0, 10);
        let b = write_merge_source(&dir, "_1", 100, 6);
        let c = write_merge_source(&dir, "_2", 200, 6);
        let d = write_merge_source(&dir, "_3", 300, 6);
        let state = || {
            stored_fields_merge_state_with(
                vec![
                    open_merge_source(&dir, &a),
                    open_merge_source(&dir, &b),
                    opaque(open_merge_source(&dir, &c)),
                    open_merge_source(&dir, &d),
                ],
                vec![
                    crate::index::identity_doc_map(10, 0),
                    doc_map_without(6, 10, &[2]),
                    crate::index::identity_doc_map(6, 15),
                    crate::index::identity_doc_map(6, 21),
                ],
                vec![10, 6, 6, 6],
                vec![None, Some(live_docs_without(6, &[2])), None, None],
                false,
            )
        };

        let out: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = merge_test_writer(&out, "_w", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
        let probe = state();
        let matching = MatchingReaders::new(&probe);
        assert_eq!(
            (0..4)
                .map(|i| writer.get_merge_strategy(&probe, &matching, i))
                .collect::<Vec<_>>(),
            vec![
                MergeStrategy::Bulk,
                MergeStrategy::Doc,
                MergeStrategy::Visitor,
                MergeStrategy::Bulk
            ]
        );

        let merged = run_stored_fields_merge(&state());
        assert_eq!(merged.counters, (8, 3, 7));
        assert_eq!(
            merged.documents,
            merge_docs(
                (0..10)
                    .chain([100, 101, 103, 104, 105])
                    .chain(200..206)
                    .chain(300..306)
            )
        );
    }

    /// A segment that is too dirty is recompressed instead of copied, so that
    /// a run of tiny flushes cannot degrade the compression ratio for ever
    /// (`Lucene90CompressingStoredFieldsWriter.java:655-668`).
    ///
    /// Bulk-merging three segments of two documents each carries over three
    /// dirty chunks: six dirty documents, more than one chunk holds, in every
    /// one of the chunks. Merging that segment again must take the DOC route,
    /// which repacks the six documents as `[4][2]`.
    #[test]
    fn a_too_dirty_stored_fields_segment_is_recompressed() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let sources: Vec<SegmentInfo> = (0..3)
            .map(|i| write_merge_source(&dir, &format!("_{i}"), 10 * i, 2))
            .collect();
        let expected = merge_docs([0, 1, 10, 11, 20, 21]);

        let dirty = run_stored_fields_merge(&stored_fields_merge_state(
            sources
                .iter()
                .map(|info| open_merge_source(&dir, info))
                .collect(),
            vec![2, 2, 2],
        ));
        assert_eq!(dirty.counters, (3, 3, 6), "each tiny chunk is carried over");
        assert_eq!(dirty.documents, expected);
        let out: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = merge_test_writer(&out, "_w", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
        assert!(writer.too_dirty(&concrete_merge_source(&dirty.dir, &dirty.info)));

        let recompressed = run_stored_fields_merge(&stored_fields_merge_state(
            vec![open_merge_source(&dirty.dir, &dirty.info)],
            vec![6],
        ));
        assert_eq!(
            recompressed.counters,
            (2, 1, 2),
            "a too dirty segment must be repacked, not copied"
        );
        assert_eq!(recompressed.documents, expected);
    }

    /// An index-sorted merge interleaves the source segments, so the bulk
    /// route copies each segment in several runs, and a run may start inside
    /// the chunk the previous run of the same segment ended in. Those documents
    /// are copied one by one until the next chunk boundary, then whole chunks
    /// are copied, then the leftovers one by one
    /// (`Lucene90CompressingStoredFieldsWriter.java:538-599`); that relies on
    /// the reader remembering the chunk it last decoded.
    ///
    /// Segments A and B of ten documents each, chunked `[4][4][2]`, are merged
    /// as A0-2, B0-2, A3-8, B3-9, A9. The chunks the bulk route writes are
    /// `[A0 A1 A2 B0]`, `[B1 B2 A3]` (forced), `[A4-A7]` (copied), `[A8 B3]`
    /// (forced), `[B4-B7]` and `[B8 B9]` (both copied, the latter dirty) and
    /// `[A9]` (forced at the end): seven chunks, four dirty, eight dirty
    /// documents. Re-encoding packs the twenty documents into five full
    /// chunks.
    #[test]
    fn an_index_sorted_stored_fields_merge_copies_interleaved_runs() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let a = write_merge_source(&dir, "_0", 0, 10);
        let b = write_merge_source(&dir, "_1", 100, 10);
        let state = |wrap: fn(Box<dyn StoredFieldsReader>) -> Box<dyn StoredFieldsReader>| {
            stored_fields_merge_state_with(
                vec![
                    wrap(open_merge_source(&dir, &a)),
                    wrap(open_merge_source(&dir, &b)),
                ],
                vec![
                    interleaving_doc_map(&[(0, 3, 0), (3, 9, 6), (9, 10, 19)]),
                    interleaving_doc_map(&[(0, 3, 3), (3, 10, 12)]),
                ],
                vec![10, 10],
                vec![None, None],
                true,
            )
        };
        let expected = merge_docs(
            (0..3)
                .chain(100..103)
                .chain(3..9)
                .chain(103..110)
                .chain(9..10),
        );

        let bulk = run_stored_fields_merge(&state(|reader| reader));
        assert_eq!(bulk.documents, expected);
        assert_eq!(
            bulk.counters,
            (7, 4, 8),
            "the whole chunks between the runs' ends must be copied verbatim"
        );

        let visitor = run_stored_fields_merge(&state(opaque));
        assert_eq!(visitor.documents, expected);
        assert_eq!(visitor.counters, (5, 0, 0));
    }

    /// Visits every field of a document and keeps nothing.
    struct SkipAllVisitor;

    impl StoredFieldVisitor for SkipAllVisitor {
        fn needs_field(&mut self, _info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
            Ok(StoredFieldVisitorStatus::Yes)
        }
    }

    /// `getNumDocs` is the segment's `maxDoc` — the constructor's
    /// `numDocs = si.maxDoc()` — and a merge instance, which Java builds with
    /// the copy constructor's `this.numDocs = reader.numDocs`, reports the same
    /// (`Lucene90CompressingStoredFieldsReader.java:113,138,759-761`). The
    /// counts straddle the test format's chunk of four documents.
    #[test]
    fn get_num_docs_is_the_segment_max_doc() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let per_chunk = MERGE_TEST_DOCS_PER_CHUNK as i32;
        for (name, count) in [("_n1", 1), ("_n4", per_chunk), ("_n5", per_chunk + 1)] {
            let source = write_merge_source(&dir, name, 0, count);
            let search = concrete_merge_source(&dir, &source);
            assert_eq!(search.get_num_docs(), count, "{name}");
            let merge_box = search.get_merge_instance();
            let merge = merge_box
                .as_any()
                .and_then(|any| any.downcast_ref::<Lucene90CompressingStoredFieldsReader>())
                .expect("the merge instance is this format's reader");
            assert_eq!(merge.get_num_docs(), count, "{name} merge instance");
        }
    }

    /// `isLoaded` only answers a merge instance of a reader on the current
    /// format version, and then answers for exactly the block that instance
    /// last decoded.
    ///
    /// Java throws `IllegalStateException` with two distinct messages for the
    /// two guards (`Lucene90CompressingStoredFieldsReader.java:655-664`).
    #[test]
    fn is_loaded_answers_only_a_merge_instance_on_the_current_version() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let source = write_merge_source(&dir, "_a", 0, 10);

        let search = concrete_merge_source(&dir, &source);
        let merge_box = search.get_merge_instance();
        let merge = merge_box
            .as_any()
            .and_then(|any| any.downcast_ref::<Lucene90CompressingStoredFieldsReader>())
            .expect("the merge instance is this format's reader");
        assert!(
            !merge.is_loaded(0).unwrap(),
            "nothing is decoded before the first read"
        );
        merge.document(1, &mut SkipAllVisitor).unwrap();
        for doc in 0..MERGE_TEST_DOCS_PER_CHUNK as i32 {
            assert!(merge.is_loaded(doc).unwrap(), "doc {doc} is in block 0");
        }
        assert!(
            !merge.is_loaded(MERGE_TEST_DOCS_PER_CHUNK as i32).unwrap(),
            "the next block has not been decoded"
        );

        match search.is_loaded(0) {
            Err(LuceneError::IllegalState(message)) => assert_eq!(
                message,
                "isLoaded should only ever get called on a merge instance"
            ),
            other => panic!("a search instance must refuse, got {other:?}"),
        }

        let mut old_version = concrete_merge_source(&dir, &source);
        old_version.merging = true;
        old_version.version = DATA_VERSION_CURRENT - 1;
        match old_version.is_loaded(0) {
            Err(LuceneError::IllegalState(message)) => assert_eq!(
                message,
                "isLoaded should only ever get called when the reader is on the current version"
            ),
            other => panic!("a reader on another version must refuse, got {other:?}"),
        }
    }

    /// An input that records every `prefetch` request and otherwise reads
    /// through to the input it wraps.
    struct PrefetchRecordingInput {
        inner: Box<dyn IndexInput>,
        requests: Arc<Mutex<Vec<(i64, i64)>>>,
    }

    impl fmt::Debug for PrefetchRecordingInput {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PrefetchRecordingInput")
                .finish_non_exhaustive()
        }
    }

    impl DataInput for PrefetchRecordingInput {
        fn read_byte(&mut self) -> Result<u8> {
            self.inner.read_byte()
        }

        fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
            self.inner.read_bytes(b, offset, len)
        }

        fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
            self.inner.skip_bytes(num_bytes)
        }
    }

    impl IndexInput for PrefetchRecordingInput {
        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }

        fn file_pointer(&self) -> i64 {
            self.inner.file_pointer()
        }

        fn length(&self) -> i64 {
            self.inner.length()
        }

        fn seek(&mut self, pos: i64) -> Result<()> {
            self.inner.seek(pos)
        }

        fn slice(
            &self,
            slice_description: &str,
            offset: i64,
            length: i64,
        ) -> Result<Box<dyn IndexInput>> {
            self.inner.slice(slice_description, offset, length)
        }

        fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
            self.inner.clone_input()
        }

        fn resource_description(&self) -> &str {
            self.inner.resource_description()
        }

        fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
            self.requests.lock().unwrap().push((offset, length));
            self.inner.prefetch(offset, length)
        }
    }

    /// A `.fdm` whose fields index declares no chunks for a non-empty segment
    /// opens, `prefetch` answers from the cache, and reading the document is a
    /// corrupt-index error rather than a panic.
    ///
    /// Lucene 10.5.0 behaves the same way up to the read: `getBlockID` answers
    /// `-1` (`FieldsIndexReader.java:119-127`), which the `-1`-filled prefetch
    /// cache already holds (`Lucene90CompressingStoredFieldsReader.java:118,
    /// 629-643`), and `document(0)` throws `ArrayIndexOutOfBoundsException`
    /// from `getBlockStartPointer(-1)` (measured with `StoredProbe` on the same
    /// patched index: "open ok", "prefetch ok", "document threw").
    #[test]
    fn a_fields_index_without_chunks_is_a_corrupt_index_not_a_panic() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let source = write_merge_source(&dir, "_h", 0, 3);
        // Positive control: the intact index serves the same calls.
        {
            let reader = concrete_merge_source(&dir, &source);
            assert_eq!(reader.index_reader().get_block_id(0).unwrap(), 0);
            reader.prefetch(0).unwrap();
            reader.serialized_document(0).unwrap();
        }

        super::declare_no_fields_index_chunks(dir.as_ref(), "_h.fdm", 1);
        let reader = concrete_merge_source(&dir, &source);
        let index = reader.index_reader();
        assert_eq!(index.get_block_id(0).unwrap(), -1);
        assert!(matches!(
            index.get_block_start_pointer(-1),
            Err(LuceneError::CorruptIndex(_))
        ));
        assert!(matches!(
            index.get_block_length(-1),
            Err(LuceneError::CorruptIndex(_))
        ));
        reader.prefetch(0).unwrap();
        assert!(matches!(
            reader.serialized_document(0),
            Err(LuceneError::CorruptIndex(_))
        ));
    }

    /// `prefetch` asks the `.fdt` input for the whole block that holds the
    /// document — from `getBlockStartPointer` for `getBlockLength` bytes — and
    /// asks only once while that block is among the last sixteen prefetched.
    ///
    /// Equivalent to `Lucene90CompressingStoredFieldsReader.prefetch(int)`
    /// (`Lucene90CompressingStoredFieldsReader.java:629-643`) over
    /// `FieldsIndexReader.getBlockLength(long)`
    /// (`FieldsIndexReader.java:135-143`), whose last block ends at the
    /// reader's `maxPointer`. Eighteen blocks overflow the sixteen-slot cache,
    /// so the round-robin overwrite is pinned too.
    #[test]
    fn prefetch_asks_for_the_whole_block_once_while_it_is_cached() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let per_block = MERGE_TEST_DOCS_PER_CHUNK as i32;
        // Seventeen full blocks and a last partial one.
        let source = write_merge_source(&dir, "_p", 0, 17 * per_block + 1);
        let mut reader = concrete_merge_source(&dir, &source);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let inner = reader.fields_stream.clone_input().unwrap();
        reader.fields_stream = Box::new(PrefetchRecordingInput {
            inner,
            requests: Arc::clone(&requests),
        });

        let index = reader.index_reader();
        let block = |b: i64| {
            let start = index.get_block_start_pointer(b).unwrap();
            let end = if b == 17 {
                reader.max_pointer()
            } else {
                index.get_block_start_pointer(b + 1).unwrap()
            };
            assert_eq!(index.get_block_length(b).unwrap(), end - start);
            (start, end - start)
        };

        reader.prefetch(0).unwrap();
        reader.prefetch(per_block - 1).unwrap(); // same block: cached
        reader.prefetch(17 * per_block).unwrap(); // the last, partial block
        for b in 1..=16 {
            reader.prefetch(b * per_block + 1).unwrap();
        }
        // Eighteen distinct blocks went through sixteen slots, filled in
        // order: blocks 15 and 16 overwrote blocks 0 and 17 in slots 0 and 1.
        // Block 0 is therefore asked for again, and takes slot 2 from block 1;
        // block 2, in slot 3, is still cached; block 1 is asked for again.
        reader.prefetch(2).unwrap();
        reader.prefetch(2 * per_block + 1).unwrap();
        reader.prefetch(per_block + 2).unwrap();

        let mut expected = vec![block(0), block(17)];
        expected.extend((1..=16).map(block));
        expected.push(block(0));
        expected.push(block(1));
        assert_eq!(*requests.lock().unwrap(), expected);
    }

    /// A forced flush is the statement that the chunk is *not* full, so Java
    /// asserts `triggerFlush() != force`
    /// (`Lucene90CompressingStoredFieldsWriter.java:235`): forcing a chunk that
    /// has already reached its size limit is a bug in the caller.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "self.trigger_flush() != force")]
    fn forcing_the_flush_of_a_full_chunk_trips_the_assertion() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut writer =
            merge_test_writer(&dir, "_f", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
        let field_infos = make_field_infos();
        let info = field_infos.field_info("text").unwrap();
        writer.start_document().unwrap();
        writer
            .write_field_bytes(info, &vec![7u8; MERGE_TEST_CHUNK_SIZE])
            .unwrap();
        assert!(writer.trigger_flush(), "the chunk is full");
        let _ = writer.flush(true);
    }

    /// The other half of the same assertion: an unforced flush of a chunk that
    /// is not full.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "self.trigger_flush() != force")]
    fn an_unforced_flush_of_a_partial_chunk_trips_the_assertion() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut writer =
            merge_test_writer(&dir, "_g", CompressionMode::FAST, MERGE_TEST_CHUNK_SIZE);
        let field_infos = make_field_infos();
        writer.start_document().unwrap();
        writer
            .write_field_i32(field_infos.field_info("value").unwrap(), 1)
            .unwrap();
        writer.finish_document().unwrap();
        assert!(!writer.trigger_flush(), "one small document fills nothing");
        let _ = writer.flush(false);
    }
}

impl fmt::Display for Lucene90CompressingStoredFieldsFormat {
    /// Port of `Lucene90CompressingStoredFieldsFormat.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Lucene90CompressingStoredFieldsFormat(compressionMode={}, chunkSize={}, maxDocsPerChunk={}, blockShift={})",
            self.compression_mode, self.chunk_size, self.max_docs_per_chunk, self.block_shift
        )
    }
}

impl fmt::Display for Lucene90CompressingStoredFieldsReader {
    /// Port of `Lucene90CompressingStoredFieldsReader.toString()`, which uses
    /// `chunksize` in lower case and no space after the comma, unlike the
    /// format's own string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Lucene90CompressingStoredFieldsReader(mode={},chunksize={})",
            self.compression_mode, self.chunk_size
        )
    }
}
