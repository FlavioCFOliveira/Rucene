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
//! * `Mode::BestSpeed` – LZ4 (`CompressionMode::FAST`), 80 KiB target chunks.
//! * `Mode::BestCompression` – Deflate (`CompressionMode::HIGH_COMPRESSION`),
//!   480 KiB target chunks.

use std::cmp;
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::codecs::codec_util::{
    check_footer, check_index_header, checksum_entire_file, retrieve_checksum, write_footer,
    write_index_header,
};
use crate::codecs::compressing::{CompressionMode, Compressor, Decompressor};
use crate::codecs::stored_fields::{StoredFieldsFormat, StoredFieldsReader, StoredFieldsWriter};
use crate::codecs::stub::{
    FieldInfo, FieldInfos, SegmentInfo, StoredFieldVisitor, StoredFieldVisitorStatus,
};
use crate::error::{LuceneError, Result};
use crate::index::segment_file_name;
use crate::store::{
    ByteArrayDataInput, ByteBuffersDataOutput, DataInput, DataOutput, Directory, IOContext,
    IndexInput, IndexOutput,
};
use crate::util::extra::LongValues;
use crate::util::packed::{DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter};
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
                Self::write_be_long(out, l)?;
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
                Self::write_be_long(out, l)?;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            Self::write_be_short(out, values[offset + k] as u16)?;
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
                Self::write_be_long(out, l)?;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            Self::write_be_int(out, values[offset + k] as u32)?;
        }
        Ok(())
    }

    fn write_be_short(out: &mut dyn DataOutput, s: u16) -> Result<()> {
        out.write_byte((s >> 8) as u8)?;
        out.write_byte(s as u8)?;
        Ok(())
    }

    fn write_be_int(out: &mut dyn DataOutput, i: u32) -> Result<()> {
        out.write_byte((i >> 24) as u8)?;
        out.write_byte((i >> 16) as u8)?;
        out.write_byte((i >> 8) as u8)?;
        out.write_byte(i as u8)?;
        Ok(())
    }

    fn write_be_long(out: &mut dyn DataOutput, l: u64) -> Result<()> {
        for shift in (0..64).step_by(8).rev() {
            out.write_byte((l >> shift) as u8)?;
        }
        Ok(())
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
                let l = Self::read_be_long(input)?;
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
                let l = Self::read_be_long(input)?;
                values[step + i] = ((l >> 48) & 0xFFFF) as i64;
                values[step + 32 + i] = ((l >> 32) & 0xFFFF) as i64;
                values[step + 64 + i] = ((l >> 16) & 0xFFFF) as i64;
                values[step + 96 + i] = (l & 0xFFFF) as i64;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            values[offset + k] = Self::read_be_short(input)? as i64;
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
                let l = Self::read_be_long(input)?;
                values[step + i] = ((l >> 32) & 0xFFFFFFFF) as i64;
                values[step + 64 + i] = (l & 0xFFFFFFFF) as i64;
            }
            k += STORED_FIELDS_INTS_BLOCK_SIZE;
        }
        for k in k..count {
            values[offset + k] = Self::read_be_int(input)? as i64;
        }
        Ok(())
    }

    fn read_be_short(input: &mut dyn DataInput) -> Result<u16> {
        let b1 = input.read_byte()? as u16;
        let b2 = input.read_byte()? as u16;
        Ok((b1 << 8) | b2)
    }

    fn read_be_int(input: &mut dyn DataInput) -> Result<u32> {
        let b1 = input.read_byte()? as u32;
        let b2 = input.read_byte()? as u32;
        let b3 = input.read_byte()? as u32;
        let b4 = input.read_byte()? as u32;
        Ok((b1 << 24) | (b2 << 16) | (b3 << 8) | b4)
    }

    fn read_be_long(input: &mut dyn DataInput) -> Result<u64> {
        let mut l: u64 = 0;
        for _ in 0..8 {
            l = (l << 8) | (input.read_byte()? as u64);
        }
        Ok(l)
    }
}

// -----------------------------------------------------------------------------
// FieldsIndex
// -----------------------------------------------------------------------------

/// Doc-id to block-offset mapping used by the stored-fields reader.
pub(crate) trait FieldsIndex {
    /// Returns the block that contains `doc_id`.
    fn get_block_id(&self, doc_id: i32) -> Result<i64>;

    /// Returns the file pointer where the given block starts.
    fn get_block_start_pointer(&self, block_id: i64) -> i64;

    /// Returns the byte length of the given block.
    fn get_block_length(&self, block_id: i64) -> i64;

    /// Returns the file pointer where the block containing `doc_id` starts.
    fn get_start_pointer(&self, doc_id: i32) -> Result<i64> {
        let block_id = self.get_block_id(doc_id)?;
        Ok(self.get_block_start_pointer(block_id))
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
    directory: Arc<dyn Directory>,
    index_file: String,
    io_context: Box<dyn IOContext>,
}

impl FieldsIndexReader {
    fn new(
        directory: Arc<dyn Directory>,
        segment: &str,
        suffix: &str,
        extension: &str,
        codec_name: &str,
        id: [u8; 16],
        meta_in: &mut dyn DataInput,
        io_context: Box<dyn IOContext>,
    ) -> Result<Self> {
        let max_doc = meta_in.read_int()?;
        let block_shift = meta_in.read_int()?;
        let num_values = meta_in.read_int()? as usize;
        let docs_start_pointer = meta_in.read_long()?;
        let docs_meta = DirectMonotonicMeta::load(meta_in, num_values as i64, block_shift)?;
        let start_pointers_start_pointer = meta_in.read_long()?;
        let start_pointers_meta =
            DirectMonotonicMeta::load(meta_in, num_values as i64, block_shift)?;
        let start_pointers_end_pointer = meta_in.read_long()?;
        let max_pointer = meta_in.read_long()?;

        let index_file = segment_file_name(segment, suffix, extension);
        let mut index_input = directory.open_input(&index_file, io_context.as_ref())?;

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

        let mut docs = Vec::with_capacity(num_values);
        let mut start_pointers = Vec::with_capacity(num_values);
        for i in 0..num_values {
            docs.push(docs_reader.get(i as i64));
            start_pointers.push(start_pointers_reader.get(i as i64));
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
            directory,
            index_file,
            io_context,
        })
    }

    fn read_slice(input: &mut dyn IndexInput, start: i64, end: i64) -> Result<Vec<u8>> {
        if end < start {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid slice: start={start}, end={end}"
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

impl FieldsIndex for FieldsIndexReader {
    fn get_block_id(&self, doc_id: i32) -> Result<i64> {
        if doc_id < 0 || doc_id >= self.max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "doc_id {doc_id} out of bounds [0, {})",
                self.max_doc
            )));
        }

        // Binary search for the largest index with docs[idx] <= doc_id.
        let mut lo: i64 = 0;
        let mut hi: i64 = (self.num_blocks - 1) as i64;
        let mut ans: i64 = 0;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let v = self.docs[mid as usize];
            if v <= doc_id as i64 {
                ans = mid;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        Ok(ans)
    }

    fn get_block_start_pointer(&self, block_id: i64) -> i64 {
        self.start_pointers[block_id as usize]
    }

    fn get_block_length(&self, block_id: i64) -> i64 {
        let end = if block_id as usize == self.num_blocks - 1 {
            self.max_pointer
        } else {
            self.start_pointers[block_id as usize + 1]
        };
        end - self.get_block_start_pointer(block_id)
    }

    fn check_integrity(&self) -> Result<()> {
        let mut input = self
            .directory
            .open_input(&self.index_file, self.io_context.as_ref())?;
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
            directory: Arc::clone(&self.directory),
            index_file: self.index_file.clone(),
            io_context: dyn_io_context_clone(self.io_context.as_ref()),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Returns an owned clone of an [`IOContext`] trait object.
///
/// The `IOContext` trait does not require `Clone`, but every implementation
/// provides `with_hints` which returns a `Box<dyn IOContext>`. Re-hinting with
/// the same hints yields an equivalent owned context.
fn dyn_io_context_clone(ctx: &dyn IOContext) -> Box<dyn IOContext> {
    ctx.with_hints(ctx.hints())
}

// -----------------------------------------------------------------------------
// FieldsIndexWriter
// -----------------------------------------------------------------------------

/// Writer side of the block index. Accumulates per-chunk first doc IDs and
/// start pointers in memory and flushes them as monotonic arrays when the
/// segment is finished.
pub(crate) struct FieldsIndexWriter {
    data_out: Box<dyn IndexOutput>,
    block_shift: i32,
    doc_counts: Vec<i32>,
    start_pointer_deltas: Vec<i64>,
    total_docs: i32,
    previous_fp: i64,
}

impl fmt::Debug for FieldsIndexWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldsIndexWriter")
            .field("block_shift", &self.block_shift)
            .field("num_chunks", &self.doc_counts.len())
            .field("total_docs", &self.total_docs)
            .finish_non_exhaustive()
    }
}

impl FieldsIndexWriter {
    fn new(
        directory: &dyn Directory,
        segment: &str,
        suffix: &str,
        extension: &str,
        codec_name: &str,
        id: [u8; 16],
        block_shift: i32,
        context: &dyn IOContext,
    ) -> Result<Self> {
        let index_file = segment_file_name(segment, suffix, extension);
        let mut data_out = directory.create_output(&index_file, context)?;
        write_index_header(
            data_out.as_mut(),
            &format!("{codec_name}Idx"),
            INDEX_VERSION_CURRENT,
            &id,
            suffix,
        )?;

        Ok(Self {
            data_out,
            block_shift,
            doc_counts: Vec::new(),
            start_pointer_deltas: Vec::new(),
            total_docs: 0,
            previous_fp: 0,
        })
    }

    fn write_index(&mut self, num_docs: i32, start_pointer: i64) -> Result<()> {
        debug_assert!(start_pointer >= self.previous_fp);
        self.doc_counts.push(num_docs);
        self.start_pointer_deltas
            .push(start_pointer - self.previous_fp);
        self.previous_fp = start_pointer;
        self.total_docs += num_docs;
        Ok(())
    }

    fn finish(
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
        let total_chunks = self.doc_counts.len();
        let num_values = total_chunks as i64 + 1;

        let docs_start_pointer = self.data_out.file_pointer();
        meta_out.write_int(num_docs)?;
        meta_out.write_int(self.block_shift)?;
        meta_out.write_int(num_values as i32)?;
        meta_out.write_long(docs_start_pointer)?;

        {
            let mut writer = DirectMonotonicWriter::new(
                meta_out,
                self.data_out.as_mut(),
                num_values,
                self.block_shift,
            )?;
            let mut doc = 0i64;
            writer.add(doc)?;
            for &count in &self.doc_counts {
                doc += count as i64;
                writer.add(doc)?;
            }
            writer.finish()?;
        }

        let start_pointers_start_pointer = self.data_out.file_pointer();
        meta_out.write_long(start_pointers_start_pointer)?;
        {
            let mut writer = DirectMonotonicWriter::new(
                meta_out,
                self.data_out.as_mut(),
                num_values,
                self.block_shift,
            )?;
            let mut fp = 0i64;
            for &delta in &self.start_pointer_deltas {
                fp += delta;
                writer.add(fp)?;
            }
            if max_pointer < fp {
                return Err(LuceneError::CorruptIndex(
                    "File pointers don't add up".to_string(),
                ));
            }
            writer.add(max_pointer)?;
            writer.finish()?;
        }

        let start_pointers_end_pointer = self.data_out.file_pointer();
        meta_out.write_long(start_pointers_end_pointer)?;
        meta_out.write_long(max_pointer)?;

        write_footer(self.data_out.as_mut())?;
        self.data_out.close()?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Lucene90CompressingStoredFieldsWriter
// -----------------------------------------------------------------------------

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
        directory: &dyn Directory,
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

        let meta_file = segment_file_name(&segment, segment_suffix, META_EXTENSION);
        let mut meta_stream = directory.create_output(&meta_file, context)?;
        write_index_header(
            meta_stream.as_mut(),
            &format!("{INDEX_CODEC_NAME}Meta"),
            INDEX_VERSION_CURRENT,
            &id,
            segment_suffix,
        )?;

        let fields_file = segment_file_name(&segment, segment_suffix, FIELDS_EXTENSION);
        let mut fields_stream = directory.create_output(&fields_file, context)?;
        write_index_header(
            fields_stream.as_mut(),
            format_name,
            DATA_VERSION_CURRENT,
            &id,
            segment_suffix,
        )?;

        let index_writer = FieldsIndexWriter::new(
            directory,
            &segment,
            segment_suffix,
            INDEX_EXTENSION,
            INDEX_CODEC_NAME,
            id,
            block_shift,
            context,
        )?;

        meta_stream.write_v_int(chunk_size as i32)?;

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

    fn flush(&mut self, force: bool) -> Result<()> {
        debug_assert!(self.trigger_flush() || force);
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
            StoredFieldsInts::write_be_short(out, (float_bits >> 8) as u16)?;
            out.write_byte(float_bits as u8)
        } else {
            out.write_byte(0xFF)?;
            StoredFieldsInts::write_be_int(out, float_bits)
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
            StoredFieldsInts::write_be_int(out, (value as f32).to_bits())
        } else if (double_bits >> 63) == 0 {
            out.write_byte((double_bits >> 56) as u8)?;
            StoredFieldsInts::write_be_int(out, (double_bits >> 24) as u32)?;
            StoredFieldsInts::write_be_short(out, (double_bits >> 8) as u16)?;
            out.write_byte(double_bits as u8)
        } else {
            out.write_byte(0xFF)?;
            StoredFieldsInts::write_be_long(out, double_bits)
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
        let upper = zigzag >> 5;
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

    fn close(&mut self) -> Result<()> {
        self.meta_stream.close()?;
        self.fields_stream.close()?;
        self.compressor.close()
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
    directory: Arc<dyn Directory>,
    fields_file: String,
    io_context: Box<dyn IOContext>,
    merging: bool,
    state: Mutex<BlockState>,
}

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
        doc_id >= self.doc_base && doc_id < self.doc_base + self.chunk_docs
    }

    fn reset(
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
                    decompressor.decompress(
                        fields_stream,
                        total_length,
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
        let offset = self.offsets[index] as usize;
        let length = (self.offsets[index + 1] - self.offsets[index]) as usize;
        let num_stored_fields = self.num_stored_fields[index] as i32;

        if length == 0 {
            return Ok(SerializedDocument {
                input: Box::new(ByteArrayDataInput::new(Vec::new())),
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
            if self.sliced {
                let mut full = Vec::with_capacity(total_length);
                let mut decompressed = 0;
                while decompressed < total_length {
                    let to_decompress = cmp::min(total_length - decompressed, chunk_size);
                    let mut spare = BytesRef::default();
                    decompressor.decompress(input, total_length, 0, to_decompress, &mut spare)?;
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
            input: Box::new(ByteArrayDataInput::new(data)),
            length,
            num_stored_fields,
        })
    }
}

/// A document as it appears inside a decompressed chunk.
struct SerializedDocument {
    input: Box<dyn DataInput>,
    length: usize,
    num_stored_fields: i32,
}

impl Lucene90CompressingStoredFieldsReader {
    fn new(
        directory: Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        field_infos: &FieldInfos,
        io_context: Box<dyn IOContext>,
        format_name: &str,
        compression_mode: CompressionMode,
    ) -> Result<Self> {
        let segment = segment_info.name.clone();
        let id = segment_info.id();
        let num_docs = segment_info.max_doc()?;

        let fields_file = segment_file_name(&segment, segment_suffix, FIELDS_EXTENSION);
        let mut fields_stream = directory.open_input(&fields_file, io_context.as_ref())?;
        let version = check_index_header(
            fields_stream.as_mut(),
            format_name,
            DATA_VERSION_START,
            DATA_VERSION_CURRENT,
            &id,
            segment_suffix,
        )?;
        retrieve_checksum(fields_stream.as_mut())?;
        fields_stream.close()?;

        let meta_file = segment_file_name(&segment, segment_suffix, META_EXTENSION);
        let mut meta_in = directory.open_checksum_input(&meta_file)?;
        check_index_header(
            meta_in.as_mut(),
            &format!("{INDEX_CODEC_NAME}Meta"),
            INDEX_VERSION_START,
            version,
            &id,
            segment_suffix,
        )?;

        let chunk_size = meta_in.read_v_int()? as usize;

        let index_reader = FieldsIndexReader::new(
            Arc::clone(&directory),
            &segment,
            segment_suffix,
            INDEX_EXTENSION,
            INDEX_CODEC_NAME,
            id,
            meta_in.as_mut(),
            dyn_io_context_clone(io_context.as_ref()),
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
            directory,
            fields_file,
            io_context,
            merging: false,
            state: Mutex::new(BlockState::new()),
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
        let mut state = self.state.lock().unwrap();
        if !state.contains(doc_id) {
            let mut input = self
                .directory
                .open_input(&self.fields_file, self.io_context.as_ref())?;
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

        let mut decompressor = self.decompressor.lock().unwrap();
        let doc = if self.merging {
            state.document(doc_id, None, self.chunk_size, &mut **decompressor, true)?
        } else {
            let mut inp = self
                .directory
                .open_input(&self.fields_file, self.io_context.as_ref())?;
            let start_pointer = self.index_reader.get_start_pointer(doc_id)?;
            inp.seek(start_pointer)?;
            state.document(
                doc_id,
                Some(inp.as_mut()),
                self.chunk_size,
                &mut **decompressor,
                false,
            )?
        };
        drop(state);
        drop(decompressor);

        let mut input = doc.input;
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

            match visitor.needs_field(field_info) {
                StoredFieldVisitorStatus::Yes => {
                    Self::read_field(input.as_mut(), visitor, field_info, bits)?;
                }
                StoredFieldVisitorStatus::No => {
                    if field_idx == doc.num_stored_fields - 1 {
                        return Ok(());
                    }
                    Self::skip_field(input.as_mut(), bits)?;
                }
                StoredFieldVisitorStatus::Stop => return Ok(()),
            }
        }
        Ok(())
    }

    fn check_integrity(&self) -> Result<()> {
        self.index_reader.check_integrity()?;
        let mut input = self
            .directory
            .open_input(&self.fields_file, self.io_context.as_ref())?;
        input.seek(0)?;
        checksum_entire_file(input.as_mut())?;
        Ok(())
    }

    fn clone_reader(&self) -> Box<dyn StoredFieldsReader> {
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
            directory: Arc::clone(&self.directory),
            fields_file: self.fields_file.clone(),
            io_context: dyn_io_context_clone(self.io_context.as_ref()),
            merging: false,
            state: Mutex::new(BlockState::new()),
        })
    }

    fn get_merge_instance(&self) -> Box<dyn StoredFieldsReader> {
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
            directory: Arc::clone(&self.directory),
            fields_file: self.fields_file.clone(),
            io_context: dyn_io_context_clone(self.io_context.as_ref()),
            merging: true,
            state: Mutex::new(BlockState::new()),
        })
    }
}

impl Lucene90CompressingStoredFieldsReader {
    fn read_field(
        input: &mut dyn DataInput,
        visitor: &mut dyn StoredFieldVisitor,
        info: &FieldInfo,
        bits: i32,
    ) -> Result<()> {
        match bits & TYPE_MASK {
            TYPE_BYTE_ARR => {
                let length = input.read_v_int()? as usize;
                let mut buf = vec![0u8; length];
                input.read_bytes(&mut buf, 0, length)?;
                visitor.binary_field(info, &buf)
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
            Ok(f32::from_bits(StoredFieldsInts::read_be_int(input)?))
        } else if (b & 0x80) != 0 {
            Ok(((b & 0x7F) - 1) as i32 as f32)
        } else {
            let bits = (b << 24)
                | ((StoredFieldsInts::read_be_short(input)? as u32) << 8)
                | (input.read_byte()? as u32);
            Ok(f32::from_bits(bits))
        }
    }

    fn read_z_double(input: &mut dyn DataInput) -> Result<f64> {
        let b = input.read_byte()? as u64;
        if b == 0xFF {
            Ok(f64::from_bits(StoredFieldsInts::read_be_long(input)?))
        } else if b == 0xFE {
            let float_bits = StoredFieldsInts::read_be_int(input)?;
            Ok(f64::from(f32::from_bits(float_bits)))
        } else if (b & 0x80) != 0 {
            Ok(((b & 0x7F) - 1) as i32 as f64)
        } else {
            let bits = (b << 56)
                | ((StoredFieldsInts::read_be_int(input)? as u64) << 24)
                | ((StoredFieldsInts::read_be_short(input)? as u64) << 8)
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
        match header as i32 & DAY_ENCODING {
            SECOND_ENCODING => l *= SECOND_MS,
            HOUR_ENCODING => l *= HOUR_MS,
            DAY_ENCODING => l *= DAY_MS,
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
        _directory: &dyn Directory,
        segment_info: &SegmentInfo,
        field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        let _ = _directory;
        Ok(Box::new(Lucene90CompressingStoredFieldsReader::new(
            Arc::clone(&segment_info.directory),
            segment_info,
            "",
            field_infos,
            dyn_io_context_clone(context),
            self.format_name,
            self.compression_mode,
        )?))
    }

    fn fields_writer(
        &self,
        directory: &dyn Directory,
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
        impl_for_mode(mode).fields_reader(directory, segment_info, field_infos, context)
    }

    fn fields_writer(
        &self,
        directory: &dyn Directory,
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
        impl_for_mode(self.mode).fields_writer(directory, segment_info, context)
    }
}

fn impl_for_mode(mode: Mode) -> Lucene90CompressingStoredFieldsFormat {
    match mode {
        Mode::BestSpeed => Lucene90CompressingStoredFieldsFormat {
            format_name: "Lucene90StoredFieldsFastData",
            compression_mode: CompressionMode::FAST,
            chunk_size: 10 * 8 * 1024,
            max_docs_per_chunk: 1024,
            block_shift: 10,
        },
        Mode::BestCompression => Lucene90CompressingStoredFieldsFormat {
            format_name: "Lucene90StoredFieldsHighData",
            compression_mode: CompressionMode::HIGH_COMPRESSION,
            chunk_size: 10 * 48 * 1024,
            max_docs_per_chunk: 4096,
            block_shift: 10,
        },
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

        fn needs_field(&mut self, _info: &FieldInfo) -> StoredFieldVisitorStatus {
            StoredFieldVisitorStatus::Yes
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
                .fields_writer(dir.as_ref(), &segment_info, &*DEFAULT_IO_CONTEXT)
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
                .write_field_f32(field_infos.field_info("value").unwrap(), std::f32::consts::PI)
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
                ("value".to_string(), StoredValue::Float(std::f32::consts::PI)),
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

    #[test]
    fn round_trip_best_compression() {
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
}
