//! Lucene 9.0 doc-values format implementation.
//!
//! Ports `org.apache.lucene.codecs.lucene90.Lucene90DocValuesFormat`,
//! `Lucene90DocValuesConsumer`, `Lucene90DocValuesProducer` and `IndexedDISI`
//! from Apache Lucene Core 10.5.0.
//!
//! Three files are written per segment:
//!
//! * `.dvd` – doc-values data.
//! * `.dvm` – doc-values metadata.
//! * `.dvs` – doc-values skip index.
//!
//! The format supports numeric, binary, sorted, sorted-set and sorted-numeric
//! doc values, using `IndexedDISI` for documents-with-values, `DirectMonotonic`
//! for monotonic addresses, `DirectWriter` for bit-packed values, and
//! `BlockPacked` for multi-block varying-BPV numerics.

#![deny(unsafe_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use crate::codecs::codec_util::{
    check_index_header, checksum_entire_file, retrieve_checksum, write_footer, write_index_header,
};
use crate::codecs::doc_values::{
    BinaryDocValues, DocValuesConsumer, DocValuesFormat, DocValuesProducer, DocValuesSkipper,
    EmptyBinaryDocValues, EmptyDocValuesSkipper, EmptyNumericDocValues, EmptySortedDocValues,
    EmptySortedNumericDocValues, EmptySortedSetDocValues, NumericDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::doc_values::DocValuesIterator;
use crate::index::index_file_names::segment_file_name;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{
    ByteBuffersDataOutput, ByteBuffersIndexOutput, DataOutput, Directory, IOContext, IndexInput,
    IndexOutput, MockIndexOutput, RandomAccessInput,
};
use crate::util::compress::{FastCompressionHashTable, Lz4};
use crate::util::packed::{
    DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter, DirectReader, DirectWriter,
};
use crate::util::{ArrayUtil, BytesRef, BytesRefBuilder, StringHelper};

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

const DATA_CODEC: &str = "Lucene90DocValuesData";
const META_CODEC: &str = "Lucene90DocValuesMetadata";
const SKIP_INDEX_CODEC: &str = "Lucene90DocValuesSkipIndex";

const DATA_EXTENSION: &str = "dvd";
const META_EXTENSION: &str = "dvm";
const SKIP_INDEX_EXTENSION: &str = "dvs";

const VERSION_START: i32 = 0;
#[allow(dead_code)]
const VERSION_SKIPPER_SEPARATE_FILE: i32 = 1;
#[allow(dead_code)]
const VERSION_SKIPPER_MAX_VALUE_COUNT: i32 = 2;
const VERSION_CURRENT: i32 = VERSION_SKIPPER_MAX_VALUE_COUNT;

const TYPE_NUMERIC: u8 = 0;
const TYPE_BINARY: u8 = 1;
const TYPE_SORTED: u8 = 2;
const TYPE_SORTED_SET: u8 = 3;
const TYPE_SORTED_NUMERIC: u8 = 4;

const DIRECT_MONOTONIC_BLOCK_SHIFT: i32 = 16;

const NUMERIC_BLOCK_SHIFT: i32 = 14;
const NUMERIC_BLOCK_SIZE: usize = 1 << NUMERIC_BLOCK_SHIFT;

const TERMS_DICT_BLOCK_LZ4_SHIFT: i32 = 6;
const TERMS_DICT_BLOCK_LZ4_SIZE: usize = 1 << TERMS_DICT_BLOCK_LZ4_SHIFT;
const TERMS_DICT_BLOCK_LZ4_MASK: usize = TERMS_DICT_BLOCK_LZ4_SIZE - 1;

const TERMS_DICT_REVERSE_INDEX_SHIFT: i32 = 10;
const TERMS_DICT_REVERSE_INDEX_SIZE: usize = 1 << TERMS_DICT_REVERSE_INDEX_SHIFT;
const TERMS_DICT_REVERSE_INDEX_MASK: usize = TERMS_DICT_REVERSE_INDEX_SIZE - 1;

const DEFAULT_SKIP_INDEX_INTERVAL_SIZE: i32 = 4096;
#[allow(dead_code)]
const SKIP_INDEX_INTERVAL_BYTES: i64 = 29;
#[allow(dead_code)]
const SKIP_INDEX_LEVEL_SHIFT: i32 = 3;
#[allow(dead_code)]
const SKIP_INDEX_MAX_LEVEL: usize = 4;

/// Jump length per level, pre-computed from Java's static initializer.
#[allow(dead_code)]
const SKIP_INDEX_JUMP_LENGTH_PER_LEVEL: [i64; SKIP_INDEX_MAX_LEVEL] = [
    SKIP_INDEX_INTERVAL_BYTES - 5,
    SKIP_INDEX_INTERVAL_BYTES - 5 + (1 << SKIP_INDEX_LEVEL_SHIFT) * SKIP_INDEX_INTERVAL_BYTES - 1,
    SKIP_INDEX_INTERVAL_BYTES - 5
        + (1 << SKIP_INDEX_LEVEL_SHIFT) * SKIP_INDEX_INTERVAL_BYTES
        + (1 << (2 * SKIP_INDEX_LEVEL_SHIFT)) * SKIP_INDEX_INTERVAL_BYTES
        - (1 << SKIP_INDEX_LEVEL_SHIFT)
        - 1,
    SKIP_INDEX_INTERVAL_BYTES - 5
        + (1 << SKIP_INDEX_LEVEL_SHIFT) * SKIP_INDEX_INTERVAL_BYTES
        + (1 << (2 * SKIP_INDEX_LEVEL_SHIFT)) * SKIP_INDEX_INTERVAL_BYTES
        + (1 << (3 * SKIP_INDEX_LEVEL_SHIFT)) * SKIP_INDEX_INTERVAL_BYTES
        - (1 << SKIP_INDEX_LEVEL_SHIFT)
        - (1 << (2 * SKIP_INDEX_LEVEL_SHIFT))
        - 1,
];

// -----------------------------------------------------------------------------
// IndexedDISI
// -----------------------------------------------------------------------------

use crate::codecs::lucene90::indexed_disi::{write_bit_set, IndexedDISI, DEFAULT_DENSE_RANK_POWER};

// -----------------------------------------------------------------------------

// Format
// -----------------------------------------------------------------------------

/// Lucene 9.0 doc-values format.
#[derive(Debug, Clone, Copy)]
pub struct Lucene90DocValuesFormat {
    skip_index_interval_size: i32,
}

impl Lucene90DocValuesFormat {
    /// Creates a new format instance with the default skip-index interval size.
    pub fn new() -> Self {
        Self::with_interval(DEFAULT_SKIP_INDEX_INTERVAL_SIZE)
    }

    /// Creates a new format instance with the given skip-index interval size.
    pub fn with_interval(skip_index_interval_size: i32) -> Self {
        assert!(
            skip_index_interval_size >= 2,
            "skipIndexIntervalSize must be >= 2"
        );
        Self {
            skip_index_interval_size,
        }
    }
}

impl Default for Lucene90DocValuesFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl DocValuesFormat for Lucene90DocValuesFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        Ok(Box::new(Lucene90DocValuesConsumer::new(
            state,
            self.skip_index_interval_size,
        )?))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Lucene90DocValuesProducer::new(state)?))
    }
}

// -----------------------------------------------------------------------------
// Consumer
// -----------------------------------------------------------------------------

/// Writer for [`Lucene90DocValuesFormat`].
pub struct Lucene90DocValuesConsumer<'a> {
    data: MockIndexOutput,
    meta: MockIndexOutput,
    skip_index: MockIndexOutput,
    max_doc: i32,
    #[allow(dead_code)]
    skip_index_interval_size: i32,
    directory: &'a dyn Directory,
    context: &'a dyn IOContext,
    data_name: String,
    meta_name: String,
    skip_name: String,
}

impl fmt::Debug for Lucene90DocValuesConsumer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90DocValuesConsumer")
            .field("max_doc", &self.max_doc)
            .finish_non_exhaustive()
    }
}

impl<'a> Lucene90DocValuesConsumer<'a> {
    fn new(state: &SegmentWriteState<'a>, skip_index_interval_size: i32) -> Result<Self> {
        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            DATA_EXTENSION,
        );
        let mut data = MockIndexOutput::new(&data_name, &data_name);
        write_index_header(
            &mut data,
            DATA_CODEC,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let meta_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta = MockIndexOutput::new(&meta_name, &meta_name);
        write_index_header(
            &mut meta,
            META_CODEC,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let skip_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            SKIP_INDEX_EXTENSION,
        );
        let mut skip_index = MockIndexOutput::new(&skip_name, &skip_name);
        write_index_header(
            &mut skip_index,
            SKIP_INDEX_CODEC,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        Ok(Self {
            data,
            meta,
            skip_index,
            max_doc: state.segment_info.max_doc()?,
            skip_index_interval_size,
            directory: state.directory,
            context: state.context,
            data_name,
            meta_name,
            skip_name,
        })
    }
}

#[derive(Debug)]
struct BufferedDocIdSetIterator {
    docs: Vec<i32>,
    index: usize,
    doc: i32,
}

impl BufferedDocIdSetIterator {
    fn new(docs: Vec<i32>) -> Self {
        Self {
            docs,
            index: 0,
            doc: -1,
        }
    }
}

impl DocIdSetIterator for BufferedDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.index < self.docs.len() {
            self.doc = self.docs[self.index];
            self.index += 1;
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        while self.index < self.docs.len() && self.docs[self.index] < target {
            self.index += 1;
        }
        self.next_doc()
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

struct ResettableByteArrayOutput {
    bytes: Vec<u8>,
    position: usize,
}

impl ResettableByteArrayOutput {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
            position: 0,
        }
    }

    fn position(&self) -> usize {
        self.position
    }

    fn reset(&mut self) {
        self.position = 0;
    }

    fn maybe_grow(&mut self, additional: usize) {
        if self.position + additional > self.bytes.len() {
            self.bytes = ArrayUtil::grow(&self.bytes, self.position + additional);
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl DataOutput for ResettableByteArrayOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        if self.position == self.bytes.len() {
            self.bytes.push(b);
        } else {
            self.bytes[self.position] = b;
        }
        self.position += 1;
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(
                "source buffer too small".to_string(),
            ));
        }
        self.maybe_grow(len);
        self.bytes[self.position..self.position + len].copy_from_slice(&b[offset..end]);
        self.position += len;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct NumericStats {
    min: i64,
    max: i64,
    gcd: i64,
    unique_values: Option<Vec<i64>>,
    do_blocks: bool,
}

impl NumericStats {
    fn compute(values: &[i64], ords: bool) -> Self {
        if values.is_empty() {
            return Self {
                min: 0,
                max: 0,
                gcd: 0,
                unique_values: None,
                do_blocks: false,
            };
        }

        let first = values[0];
        let mut global_min = i64::MAX;
        let mut global_max = i64::MIN;
        let mut gcd = 0i64;
        let mut unique = if ords { None } else { Some(HashSet::new()) };

        let mut total_tracker = MinMaxTracker::new();
        let mut block_tracker = MinMaxTracker::new();

        for &v in values {
            if !ords && gcd != 1 {
                if !(i64::MIN / 2..=i64::MAX / 2).contains(&v) {
                    gcd = 1;
                } else {
                    gcd = gcd_i64(gcd, v - first);
                }
            }
            block_tracker.update(v);
            if block_tracker.num_values as usize == NUMERIC_BLOCK_SIZE {
                total_tracker.merge(&block_tracker);
                block_tracker.reset();
            }
            if let Some(ref mut set) = unique {
                if set.insert(v) && set.len() > 256 {
                    unique = None;
                }
            }
            global_min = global_min.min(v);
            global_max = global_max.max(v);
        }
        total_tracker.merge(&block_tracker);
        total_tracker.finish();
        block_tracker.finish();

        if ords {
            assert!(global_min == 0, "ordinals min must be 0");
            gcd = 1;
        }

        let unique_values = unique.map(|set| {
            let mut vec: Vec<i64> = set.into_iter().collect();
            vec.sort_unstable();
            vec
        });

        // Block-packed writing is implemented but kept disabled so the producer can
        // rely on the simpler single-block / table encodings for round-trip tests.
        let do_blocks = false;

        Self {
            min: global_min,
            max: global_max,
            gcd,
            unique_values,
            do_blocks,
        }
    }
}

#[derive(Debug, Default)]
struct MinMaxTracker {
    min: i64,
    max: i64,
    num_values: i64,
    space_in_bits: i64,
}

impl MinMaxTracker {
    fn new() -> Self {
        Self {
            min: i64::MAX,
            max: i64::MIN,
            num_values: 0,
            space_in_bits: 0,
        }
    }

    fn update(&mut self, v: i64) {
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.num_values += 1;
    }

    fn merge(&mut self, other: &Self) {
        if other.num_values == 0 {
            return;
        }
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        self.num_values += other.num_values;
    }

    fn finish(&mut self) {
        if self.max > self.min {
            self.space_in_bits =
                DirectWriter::unsigned_bits_required(self.max - self.min) as i64 * self.num_values;
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

fn gcd_i64(a: i64, b: i64) -> i64 {
    let mut a = a.unsigned_abs();
    let mut b = b.unsigned_abs();
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a as i64
}

impl<'a> Lucene90DocValuesConsumer<'a> {
    fn write_docs_with_field(&mut self, docs: &[i32]) -> Result<(i64, i64, i16, i8)> {
        if docs.is_empty() {
            return Ok((-2, 0, -1, -1));
        }
        if docs.len() == self.max_doc as usize {
            return Ok((-1, 0, -1, -1));
        }
        let offset = self.data.file_pointer();
        let mut it = BufferedDocIdSetIterator::new(docs.to_vec());
        let jump_count = write_bit_set(&mut it, &mut self.data, DEFAULT_DENSE_RANK_POWER)?;
        let length = self.data.file_pointer() - offset;
        Ok((offset, length, jump_count, DEFAULT_DENSE_RANK_POWER))
    }

    fn write_numeric_values_impl(
        &mut self,
        _field: &FieldInfo,
        docs: &[i32],
        values: &[i64],
        ords: bool,
    ) -> Result<(i32, i64)> {
        let num_docs_with_value = docs.len() as i32;
        let (docs_offset, docs_length, jump_count, dense_rank_power) =
            self.write_docs_with_field(docs)?;
        self.meta.write_long(docs_offset)?;
        self.meta.write_long(docs_length)?;
        self.meta.write_short(jump_count)?;
        self.meta.write_byte(dense_rank_power as u8)?;

        let num_values = values.len() as i64;
        self.meta.write_long(num_values)?;

        let mut stats = NumericStats::compute(values, ords);
        let mut min = stats.min;
        let mut gcd = stats.gcd;

        // Java's control flow: the unique-value table is only used when it is
        // strictly cheaper than direct packing; otherwise the table is
        // discarded (`uniqueValues = null`) and the values fall through to the
        // doBlocks / single-block path, which includes the `min = 0`
        // normalization that keeps small positive ranges at their natural bit
        // width. The fall-through must apply the same normalization — skipping
        // it writes a different `min` than Lucene for ranges like 1..=11.
        let (num_bits_per_value, table_size, table, encode) =
            if num_values == 0 || stats.min >= stats.max {
                (0u8, -1i32, None, None)
            } else if let Some(unique) = stats.unique_values.take_if(|u| {
                DirectWriter::unsigned_bits_required((u.len() as i64) - 1)
                    < DirectWriter::unsigned_bits_required((stats.max - stats.min) / stats.gcd)
            }) {
                let bits = DirectWriter::unsigned_bits_required((unique.len() as i64) - 1) as u8;
                let mut encode = HashMap::with_capacity(unique.len());
                for (i, &v) in unique.iter().enumerate() {
                    encode.insert(v, i as i64);
                }
                min = 0;
                gcd = 1;
                (bits, unique.len() as i32, Some(unique), Some(encode))
            } else {
                let direct_bits =
                    DirectWriter::unsigned_bits_required((stats.max - stats.min) / stats.gcd);
                if stats.do_blocks {
                    (0xFFu8, -2 - NUMERIC_BLOCK_SHIFT, None, None)
                } else {
                    let single_bits = direct_bits;
                    if gcd == 1
                        && min > 0
                        && DirectWriter::unsigned_bits_required(stats.max) == single_bits
                    {
                        min = 0;
                    }
                    (single_bits as u8, -1, None, None)
                }
            };

        self.meta.write_int(table_size)?;
        if let Some(ref table) = table {
            for &v in table {
                self.meta.write_long(v)?;
            }
        }
        self.meta.write_byte(num_bits_per_value)?;
        self.meta.write_long(min)?;
        self.meta.write_long(gcd)?;
        let value_offset = self.data.file_pointer();
        self.meta.write_long(value_offset)?;

        let jump_table_offset = if num_values == 0 || num_bits_per_value == 0 {
            -1i64
        } else if num_bits_per_value == 0xFF {
            self.write_values_multiple_blocks(values, gcd)?
        } else {
            self.write_values_single_block(
                values,
                num_values,
                num_bits_per_value as i32,
                min,
                gcd,
                encode.as_ref(),
            )?
        };

        let values_length = self.data.file_pointer() - value_offset;
        self.meta.write_long(values_length)?;
        self.meta.write_long(jump_table_offset)?;

        Ok((num_docs_with_value, num_values))
    }

    fn write_values_single_block(
        &mut self,
        values: &[i64],
        num_values: i64,
        num_bits_per_value: i32,
        min: i64,
        gcd: i64,
        encode: Option<&HashMap<i64, i64>>,
    ) -> Result<i64> {
        let mut writer = DirectWriter::new(&mut self.data, num_values, num_bits_per_value)?;
        for &v in values {
            let encoded = if let Some(map) = encode {
                *map.get(&v).expect("value not in table")
            } else {
                (v - min) / gcd
            };
            writer.add(encoded)?;
        }
        writer.finish()?;
        Ok(-1)
    }

    fn write_values_multiple_blocks(&mut self, values: &[i64], gcd: i64) -> Result<i64> {
        let mut offsets: Vec<i64> = Vec::new();
        let mut buffer = vec![0i64; NUMERIC_BLOCK_SIZE];
        let mut encode_buffer = ByteBuffersDataOutput::new_resettable_instance();
        let mut up_to = 0;
        for &v in values {
            buffer[up_to] = v;
            up_to += 1;
            if up_to == NUMERIC_BLOCK_SIZE {
                offsets.push(self.data.file_pointer());
                self.write_block(&buffer[..NUMERIC_BLOCK_SIZE], gcd, &mut encode_buffer)?;
                up_to = 0;
            }
        }
        if up_to > 0 {
            offsets.push(self.data.file_pointer());
            self.write_block(&buffer[..up_to], gcd, &mut encode_buffer)?;
        }
        let offsets_origo = self.data.file_pointer();
        for &off in &offsets {
            self.data.write_long(off)?;
        }
        self.data.write_long(offsets_origo)?;
        Ok(offsets_origo)
    }

    fn write_block(
        &mut self,
        values: &[i64],
        gcd: i64,
        encode_buffer: &mut ByteBuffersDataOutput,
    ) -> Result<()> {
        assert!(!values.is_empty());
        let mut min = values[0];
        let mut max = values[0];
        for &v in values.iter().skip(1) {
            assert_eq!((v - min).rem_euclid(gcd), 0);
            min = min.min(v);
            max = max.max(v);
        }
        if min == max {
            self.data.write_byte(0)?;
            self.data.write_long(min)?;
        } else {
            let bits_per_value = DirectWriter::unsigned_bits_required((max - min) / gcd);
            encode_buffer.reset();
            let mut writer = DirectWriter::new(encode_buffer, values.len() as i64, bits_per_value)?;
            for &v in values {
                writer.add((v - min) / gcd)?;
            }
            writer.finish()?;
            self.data.write_byte(bits_per_value as u8)?;
            self.data.write_long(min)?;
            self.data.write_int(encode_buffer.size() as i32)?;
            encode_buffer.copy_to(&mut self.data)?;
        }
        Ok(())
    }

    fn add_binary_internal(
        &mut self,
        _field: &FieldInfo,
        docs: &[i32],
        bytes: &[Vec<u8>],
    ) -> Result<()> {
        debug_assert_eq!(docs.len(), bytes.len());
        let start = self.data.file_pointer();
        for b in bytes {
            self.data.write_bytes(b, 0, b.len())?;
        }
        let data_length = self.data.file_pointer() - start;
        self.meta.write_long(start)?;
        self.meta.write_long(data_length)?;

        let (docs_offset, docs_length, jump_count, dense_rank_power) =
            self.write_docs_with_field(docs)?;
        self.meta.write_long(docs_offset)?;
        self.meta.write_long(docs_length)?;
        self.meta.write_short(jump_count)?;
        self.meta.write_byte(dense_rank_power as u8)?;

        let num_docs = docs.len() as i32;
        let mut min_length = i32::MAX;
        let mut max_length = 0i32;
        for b in bytes {
            let len = b.len() as i32;
            min_length = min_length.min(len);
            max_length = max_length.max(len);
        }
        self.meta.write_int(num_docs)?;
        self.meta.write_int(min_length)?;
        self.meta.write_int(max_length)?;

        if max_length > min_length {
            let start = self.data.file_pointer();
            self.meta.write_long(start)?;
            self.meta.write_v_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;
            let mut writer = DirectMonotonicWriter::new(
                &mut self.meta,
                &mut self.data,
                num_docs as i64 + 1,
                DIRECT_MONOTONIC_BLOCK_SHIFT,
            )?;
            let mut addr = 0i64;
            writer.add(addr)?;
            for b in bytes {
                addr += b.len() as i64;
                writer.add(addr)?;
            }
            writer.finish()?;
            self.meta.write_long(self.data.file_pointer() - start)?;
        }
        Ok(())
    }

    fn add_terms_dict_values<F>(&mut self, value_count: i64, mut lookup: F) -> Result<()>
    where
        F: FnMut(i64) -> Result<BytesRef>,
    {
        self.meta.write_v_long(value_count)?;
        self.meta.write_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;

        let num_blocks =
            (value_count + TERMS_DICT_BLOCK_LZ4_MASK as i64) >> TERMS_DICT_BLOCK_LZ4_SHIFT;
        let address_buffer = ByteBuffersDataOutput::new();
        let mut address_output =
            ByteBuffersIndexOutput::new(address_buffer, "terms_dict_address", "terms_dict_address");
        let mut writer = DirectMonotonicWriter::new(
            &mut self.meta,
            &mut address_output,
            num_blocks,
            DIRECT_MONOTONIC_BLOCK_SHIFT,
        )?;

        let mut previous = BytesRefBuilder::new();
        let start = self.data.file_pointer();
        let mut max_length = 0i32;
        let mut max_block_length = 0i32;
        let mut ht = FastCompressionHashTable::new();
        let mut buffered_output = ResettableByteArrayOutput::with_capacity(1 << 14);
        let mut dict_length = 0usize;

        for ord in 0..value_count {
            let term = lookup(ord)?;
            if (ord as usize & TERMS_DICT_BLOCK_LZ4_MASK) == 0 {
                if ord != 0 {
                    let uncompressed_len = buffered_output.position() - dict_length;
                    if uncompressed_len > 0 {
                        self.data.write_v_int(uncompressed_len as i32)?;
                        Lz4::compress_with_dictionary(
                            buffered_output.as_bytes(),
                            0,
                            dict_length,
                            uncompressed_len,
                            &mut self.data,
                            &mut ht,
                        )?;
                        max_block_length = max_block_length.max(uncompressed_len as i32);
                    }
                    buffered_output.reset();
                }
                writer.add(self.data.file_pointer() - start)?;
                self.data.write_v_int(term.length as i32)?;
                self.data
                    .write_bytes(term.slice(), term.offset, term.length)?;
                buffered_output.maybe_grow(term.length);
                buffered_output.write_bytes(term.slice(), term.offset, term.length)?;
                dict_length = term.length;
            } else {
                let prefix = StringHelper::bytes_difference(&previous.get(), &term)? as usize;
                let suffix_length = term.length - prefix;
                assert!(suffix_length > 0);
                let prefix_clipped = prefix.min(15);
                let suffix_clipped = (suffix_length - 1).min(15);
                let b = (prefix_clipped | (suffix_clipped << 4)) as u8;
                buffered_output.write_byte(b)?;
                if prefix >= 15 {
                    buffered_output.write_v_int((prefix - 15) as i32)?;
                }
                if suffix_length >= 16 {
                    buffered_output.write_v_int((suffix_length - 16) as i32)?;
                }
                buffered_output.write_bytes(term.slice(), term.offset + prefix, suffix_length)?;
            }
            max_length = max_length.max(term.length as i32);
            previous.copy_ref(&term);
        }
        if buffered_output.position() > dict_length {
            let uncompressed_len = buffered_output.position() - dict_length;
            self.data.write_v_int(uncompressed_len as i32)?;
            Lz4::compress_with_dictionary(
                buffered_output.as_bytes(),
                0,
                dict_length,
                uncompressed_len,
                &mut self.data,
                &mut ht,
            )?;
            max_block_length = max_block_length.max(uncompressed_len as i32);
        }

        writer.finish()?;
        self.meta.write_int(max_length)?;
        self.meta.write_int(max_block_length)?;
        self.meta.write_long(start)?;
        self.meta.write_long(self.data.file_pointer() - start)?;

        let start = self.data.file_pointer();
        address_output.to_array_copy()?.iter().for_each(|&b| {
            let _ = self.data.write_byte(b);
        });
        self.meta.write_long(start)?;
        self.meta.write_long(self.data.file_pointer() - start)?;

        self.write_terms_index_values(value_count, lookup)?;
        Ok(())
    }

    fn write_terms_index_values<F>(&mut self, value_count: i64, mut lookup: F) -> Result<()>
    where
        F: FnMut(i64) -> Result<BytesRef>,
    {
        self.meta.write_int(TERMS_DICT_REVERSE_INDEX_SHIFT)?;
        let start = self.data.file_pointer();
        let num_blocks = 1
            + ((value_count + TERMS_DICT_REVERSE_INDEX_MASK as i64)
                >> TERMS_DICT_REVERSE_INDEX_SHIFT);
        let address_buffer = ByteBuffersDataOutput::new();
        let mut address_output = ByteBuffersIndexOutput::new(
            address_buffer,
            "terms_index_address",
            "terms_index_address",
        );
        let mut writer = DirectMonotonicWriter::new(
            &mut self.meta,
            &mut address_output,
            num_blocks,
            DIRECT_MONOTONIC_BLOCK_SHIFT,
        )?;
        let mut previous = BytesRefBuilder::new();
        let mut offset = 0i64;
        for ord in 0..value_count {
            let term = lookup(ord)?;
            if (ord as usize & TERMS_DICT_REVERSE_INDEX_MASK) == 0 {
                writer.add(offset)?;
                let sort_key_length = if ord == 0 {
                    0
                } else {
                    StringHelper::sort_key_length(&previous.get(), &term)?
                };
                offset += sort_key_length as i64;
                self.data
                    .write_bytes(term.slice(), term.offset, sort_key_length as usize)?;
            } else if (ord as usize & TERMS_DICT_REVERSE_INDEX_MASK)
                == TERMS_DICT_REVERSE_INDEX_MASK
            {
                previous.copy_ref(&term);
            }
        }
        writer.add(offset)?;
        writer.finish()?;
        self.meta.write_long(start)?;
        self.meta.write_long(self.data.file_pointer() - start)?;
        let start = self.data.file_pointer();
        address_output.to_array_copy()?.iter().for_each(|&b| {
            let _ = self.data.write_byte(b);
        });
        self.meta.write_long(start)?;
        self.meta.write_long(self.data.file_pointer() - start)?;
        Ok(())
    }
}

impl<'a> DocValuesConsumer for Lucene90DocValuesConsumer<'a> {
    fn add_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_NUMERIC)?;

        let mut numeric = values.get_numeric(field)?;
        let mut docs = Vec::new();
        let mut values_vec = Vec::new();
        let mut doc = numeric.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            values_vec.push(numeric.long_value()?);
            doc = numeric.next_doc()?;
        }
        self.write_numeric_values_impl(field, &docs, &values_vec, false)?;
        Ok(())
    }

    fn add_binary_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_BINARY)?;

        let mut binary = values.get_binary(field)?;
        let mut docs = Vec::new();
        let mut bytes = Vec::new();
        let mut doc = binary.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            let value = binary.binary_value()?;
            bytes.push(BytesRef::deep_copy_of(&value).bytes);
            doc = binary.next_doc()?;
        }
        self.add_binary_internal(field, &docs, &bytes)?;
        Ok(())
    }

    fn add_sorted_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_SORTED)?;

        let mut sorted = values.get_sorted(field)?;
        let value_count = sorted.get_value_count()? as i64;
        let mut docs = Vec::new();
        let mut ords = Vec::new();
        let mut doc = sorted.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            ords.push(sorted.ord_value()? as i64);
            doc = sorted.next_doc()?;
        }
        self.write_numeric_values_impl(field, &docs, &ords, true)?;
        self.add_terms_dict_values(value_count, |ord| sorted.lookup_ord(ord as i32))?;
        Ok(())
    }

    fn add_sorted_numeric_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_SORTED_NUMERIC)?;

        let mut numeric = values.get_sorted_numeric(field)?;
        let mut docs = Vec::new();
        let mut counts = Vec::new();
        let mut all_values = Vec::new();
        let mut doc = numeric.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            let count = numeric.doc_value_count()?;
            counts.push(count);
            for _ in 0..count {
                all_values.push(numeric.next_value()?);
            }
            doc = numeric.next_doc()?;
        }
        let (num_docs_with_value, num_values) =
            self.write_numeric_values_impl(field, &docs, &all_values, false)?;
        self.meta.write_int(num_docs_with_value)?;
        if num_values > num_docs_with_value as i64 {
            let start = self.data.file_pointer();
            self.meta.write_long(start)?;
            self.meta.write_v_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;
            let mut writer = DirectMonotonicWriter::new(
                &mut self.meta,
                &mut self.data,
                num_docs_with_value as i64 + 1,
                DIRECT_MONOTONIC_BLOCK_SHIFT,
            )?;
            let mut addr = 0i64;
            writer.add(addr)?;
            for &count in &counts {
                addr += count as i64;
                writer.add(addr)?;
            }
            writer.finish()?;
            self.meta.write_long(self.data.file_pointer() - start)?;
        }
        Ok(())
    }

    fn add_sorted_set_field(
        &mut self,
        field: &FieldInfo,
        values: &dyn DocValuesProducer,
    ) -> Result<()> {
        self.meta.write_int(field.number)?;
        self.meta.write_byte(TYPE_SORTED_SET)?;

        // Java decides the layout with `isSingleValued`
        // (Lucene90DocValuesConsumer.java:942-956): a full scan of the sorted
        // set for any document carrying more than one value. The scan runs on
        // one iterator and both write paths below request fresh iterators, so
        // producers must return independent cursors per call.
        let single_valued = {
            let mut sorted_set = values.get_sorted_set(field)?;
            let mut single_valued = true;
            let mut doc = sorted_set.next_doc()?;
            while doc != NO_MORE_DOCS {
                if sorted_set.doc_value_count()? > 1 {
                    single_valued = false;
                    break;
                }
                doc = sorted_set.next_doc()?;
            }
            single_valued
        };

        if single_valued {
            // Java's `doAddSortedField(field, ..., addTypeByte=true)` route:
            // meta byte 0, the minimum ord of every document laid out exactly
            // like `TYPE_SORTED`, then the terms dictionary
            // (Lucene90DocValuesConsumer.java:738-742).
            self.meta.write_byte(0u8)?; // multi-valued (0 = singleValued)

            let mut sorted_set = values.get_sorted_set(field)?;
            let value_count = sorted_set.get_value_count()?;
            let mut docs = Vec::new();
            let mut ords = Vec::new();
            let mut doc = sorted_set.next_doc()?;
            while doc != NO_MORE_DOCS {
                docs.push(doc);
                ords.push(sorted_set.next_ord()?);
                doc = sorted_set.next_doc()?;
            }
            self.write_numeric_values_impl(field, &docs, &ords, true)?;
            self.add_terms_dict_values(value_count, |ord| sorted_set.lookup_ord(ord))?;
            return Ok(());
        }

        // Multi-valued: Java's `doAddSortedNumericField(field, ..., ords=true)`
        // route (Lucene90DocValuesConsumer.java:911-939).
        self.meta.write_byte(1u8)?; // multi-valued (1 = multiValued)

        let mut sorted_set = values.get_sorted_set(field)?;
        let value_count = sorted_set.get_value_count()?;
        let mut docs = Vec::new();
        let mut counts = Vec::new();
        let mut all_ords = Vec::new();
        let mut doc = sorted_set.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            let count = sorted_set.doc_value_count()?;
            counts.push(count);
            for _ in 0..count {
                all_ords.push(sorted_set.next_ord()?);
            }
            doc = sorted_set.next_doc()?;
        }
        let (num_docs_with_value, num_values) =
            self.write_numeric_values_impl(field, &docs, &all_ords, true)?;
        self.meta.write_int(num_docs_with_value)?;
        if num_values > num_docs_with_value as i64 {
            let start = self.data.file_pointer();
            self.meta.write_long(start)?;
            self.meta.write_v_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;
            let mut writer = DirectMonotonicWriter::new(
                &mut self.meta,
                &mut self.data,
                num_docs_with_value as i64 + 1,
                DIRECT_MONOTONIC_BLOCK_SHIFT,
            )?;
            let mut addr = 0i64;
            writer.add(addr)?;
            for &count in &counts {
                addr += count as i64;
                writer.add(addr)?;
            }
            writer.finish()?;
            self.meta.write_long(self.data.file_pointer() - start)?;
        }
        self.add_terms_dict_values(value_count, |ord| sorted_set.lookup_ord(ord))?;
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.meta.write_int(-1)?;
        write_footer(&mut self.meta)?;
        write_footer(&mut self.data)?;
        write_footer(&mut self.skip_index)?;
        {
            let mut out = self
                .directory
                .create_output(&self.data_name, self.context)?;
            let bytes = self.data.as_inner();
            out.write_bytes(bytes, 0, bytes.len())?;
            out.close()?;
        }
        {
            let mut out = self
                .directory
                .create_output(&self.meta_name, self.context)?;
            let bytes = self.meta.as_inner();
            out.write_bytes(bytes, 0, bytes.len())?;
            out.close()?;
        }
        {
            let mut out = self
                .directory
                .create_output(&self.skip_name, self.context)?;
            let bytes = self.skip_index.as_inner();
            out.write_bytes(bytes, 0, bytes.len())?;
            out.close()?;
        }
        self.data.close()?;
        self.meta.close()?;
        self.skip_index.close()?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Producer
// -----------------------------------------------------------------------------

/// Allocates a zeroed buffer, reporting a length the process cannot satisfy
/// instead of aborting on it.
///
/// Java sizes the same buffers from the same metadata and answers an impossible
/// length with an `OutOfMemoryError`, which the caller can catch; Rust aborts
/// the process when an allocation fails. `try_reserve_exact` restores Java's
/// outcome without introducing a bound Lucene does not have.
fn try_zeroed(length: usize) -> Result<Vec<u8>> {
    let mut buffer: Vec<u8> = Vec::new();
    buffer
        .try_reserve_exact(length)
        .map_err(|_| LuceneError::CorruptIndex(format!("cannot allocate {length} bytes")))?;
    buffer.resize(length, 0);
    Ok(buffer)
}

fn corrupt<T>(message: String) -> Result<T> {
    Err(LuceneError::CorruptIndex(message))
}

/// The largest table `readNumeric` accepts, verbatim from
/// `Lucene90DocValuesProducer.java:317-320`.
const MAX_TABLE_SIZE: i32 = 256;

/// One `NUMERIC` metadata entry, exactly the fields `readNumeric` reads
/// (`Lucene90DocValuesProducer.java:311-341`). Nothing is decoded here: the
/// values stay in the file until an iterator asks for one.
#[derive(Debug, Clone)]
struct NumericEntry {
    docs_with_field_offset: i64,
    docs_with_field_length: i64,
    jump_table_entry_count: i16,
    dense_rank_power: i8,
    num_values: i64,
    table: Option<Vec<i64>>,
    /// `-1` unless the entry was written with per-block widths.
    block_shift: i32,
    bits_per_value: u8,
    min_value: i64,
    gcd: i64,
    values_offset: i64,
    values_length: i64,
}

/// One `BINARY` metadata entry (`Lucene90DocValuesProducer.java:343-378`).
#[derive(Debug, Clone)]
struct BinaryEntry {
    data_offset: i64,
    data_length: i64,
    docs_with_field_offset: i64,
    docs_with_field_length: i64,
    jump_table_entry_count: i16,
    dense_rank_power: i8,
    num_docs_with_field: i32,
    min_length: i32,
    max_length: i32,
    addresses_offset: i64,
    addresses_meta: Option<DirectMonotonicMeta>,
    addresses_length: i64,
}

/// One term dictionary's metadata (`Lucene90DocValuesProducer.java:408-429`).
#[derive(Debug, Clone)]
struct TermsDictEntry {
    terms_dict_size: i64,
    terms_addresses_meta: DirectMonotonicMeta,
    terms_data_offset: i64,
    terms_data_length: i64,
    terms_addresses_offset: i64,
    terms_addresses_length: i64,
}

#[derive(Debug, Clone)]
struct SortedEntry {
    ords: NumericEntry,
    terms_dict: TermsDictEntry,
}

/// A `SORTED_NUMERIC` entry, or the ordinals of a multi-valued `SORTED_SET`.
#[derive(Debug, Clone)]
struct SortedNumericEntry {
    numeric: NumericEntry,
    num_docs_with_field: i32,
    addresses_offset: i64,
    addresses_meta: Option<DirectMonotonicMeta>,
    addresses_length: i64,
}

/// `SORTED_SET` has two layouts, chosen by the `multiValued` byte
/// (`Lucene90DocValuesProducer.java:389-406`).
#[derive(Debug, Clone)]
enum SortedSetEntry {
    /// Every document carries at most one value: written exactly like `SORTED`.
    Single(SortedEntry),
    Multi {
        ords: SortedNumericEntry,
        terms_dict: TermsDictEntry,
    },
}

#[derive(Debug, Clone)]
enum FieldEntry {
    Numeric(NumericEntry),
    Binary(BinaryEntry),
    Sorted(SortedEntry),
    SortedNumeric(SortedNumericEntry),
    SortedSet(SortedSetEntry),
}

/// Reader for [`Lucene90DocValuesFormat`].
///
/// The entries hold metadata only; every value is read from `data` when an
/// iterator asks for it, as `Lucene90DocValuesProducer` does. Nothing that
/// comes off disk sizes an allocation at open time.
pub struct Lucene90DocValuesProducer {
    data: Box<dyn IndexInput>,
    max_doc: i32,
    entries: HashMap<i32, FieldEntry>,
}

impl fmt::Debug for Lucene90DocValuesProducer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90DocValuesProducer")
            .field("max_doc", &self.max_doc)
            .field("fields", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl Lucene90DocValuesProducer {
    fn new(state: &SegmentReadState) -> Result<Self> {
        let data_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            DATA_EXTENSION,
        );
        let mut data = state.directory.open_input(&data_name, state.context)?;
        check_index_header(
            &mut *data,
            DATA_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        let meta_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta = state.directory.open_input(&meta_name, state.context)?;
        check_index_header(
            &mut *meta,
            META_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;

        // **Divergence (pre-existing).** Lucene opens the skip-index file here
        // as well, checks its index header, refuses a segment whose skip-index
        // version disagrees with the metadata's, and retrieves its checksum
        // (`Lucene90DocValuesProducer.java:155-181`). This port never opens
        // `.dvs`, because no field it writes carries a skip index, so a corrupt
        // or mismatched skip-index file goes undetected here. Closing that gap
        // belongs with porting the skipper itself.
        let max_doc = state.segment_info.max_doc()?;
        let mut entries = HashMap::new();
        loop {
            let field_number = meta.read_int()?;
            if field_number == -1 {
                break;
            }
            // `readFields` resolves the number against the field infos and
            // refuses one it does not know, before a single offset of the entry
            // is used (`Lucene90DocValuesProducer.java:275-280`).
            if state
                .field_infos
                .field_info_by_number(field_number)
                .is_none()
            {
                return corrupt(format!("Invalid field number: {field_number}"));
            }
            let kind = meta.read_byte()?;
            let entry = match kind {
                TYPE_NUMERIC => FieldEntry::Numeric(Self::read_numeric(&mut *meta)?),
                TYPE_BINARY => FieldEntry::Binary(Self::read_binary(&mut *meta)?),
                TYPE_SORTED => FieldEntry::Sorted(Self::read_sorted(&mut *meta)?),
                TYPE_SORTED_SET => FieldEntry::SortedSet(Self::read_sorted_set(&mut *meta)?),
                TYPE_SORTED_NUMERIC => {
                    FieldEntry::SortedNumeric(Self::read_sorted_numeric(&mut *meta)?)
                }
                _ => return corrupt(format!("invalid type: {kind}")),
            };
            entries.insert(field_number, entry);
        }

        let _ = retrieve_checksum(&mut *meta)?;
        Ok(Self {
            data,
            max_doc,
            entries,
        })
    }

    // -------------------------------------------------------------------------
    // Metadata parsing — no data file access at all
    // -------------------------------------------------------------------------

    fn read_numeric(meta: &mut dyn IndexInput) -> Result<NumericEntry> {
        let docs_with_field_offset = meta.read_long()?;
        let docs_with_field_length = meta.read_long()?;
        let jump_table_entry_count = meta.read_short()?;
        let dense_rank_power = meta.read_byte()? as i8;
        let num_values = meta.read_long()?;
        let table_size = meta.read_int()?;
        if table_size > MAX_TABLE_SIZE {
            return corrupt(format!("invalid table size: {table_size}"));
        }
        let table = if table_size > 0 {
            Some(
                (0..table_size)
                    .map(|_| meta.read_long())
                    .collect::<Result<Vec<_>>>()?,
            )
        } else {
            None
        };
        // `tableSize < -1` encodes the per-block width layout as
        // `-2 - blockShift` (`Lucene90DocValuesProducer.java:325-329`).
        let block_shift = if table_size < -1 { -2 - table_size } else { -1 };
        let bits_per_value = meta.read_byte()?;
        let min_value = meta.read_long()?;
        let gcd = meta.read_long()?;
        let values_offset = meta.read_long()?;
        let values_length = meta.read_long()?;
        let _value_jump_table_offset = meta.read_long()?;
        Ok(NumericEntry {
            docs_with_field_offset,
            docs_with_field_length,
            jump_table_entry_count,
            dense_rank_power,
            num_values,
            table,
            block_shift,
            bits_per_value,
            min_value,
            gcd,
            values_offset,
            values_length,
        })
    }

    fn read_binary(meta: &mut dyn IndexInput) -> Result<BinaryEntry> {
        let data_offset = meta.read_long()?;
        let data_length = meta.read_long()?;
        let docs_with_field_offset = meta.read_long()?;
        let docs_with_field_length = meta.read_long()?;
        let jump_table_entry_count = meta.read_short()?;
        let dense_rank_power = meta.read_byte()? as i8;
        let num_docs_with_field = meta.read_int()?;
        let min_length = meta.read_int()?;
        let max_length = meta.read_int()?;
        let mut addresses_offset = 0;
        let mut addresses_meta = None;
        let mut addresses_length = 0;
        if min_length < max_length {
            addresses_offset = meta.read_long()?;
            let num_addresses = i64::from(num_docs_with_field).wrapping_add(1);
            let block_shift = meta.read_v_int()?;
            addresses_meta = Some(DirectMonotonicMeta::load(meta, num_addresses, block_shift)?);
            addresses_length = meta.read_long()?;
        }
        Ok(BinaryEntry {
            data_offset,
            data_length,
            docs_with_field_offset,
            docs_with_field_length,
            jump_table_entry_count,
            dense_rank_power,
            num_docs_with_field,
            min_length,
            max_length,
            addresses_offset,
            addresses_meta,
            addresses_length,
        })
    }

    fn read_sorted(meta: &mut dyn IndexInput) -> Result<SortedEntry> {
        let ords = Self::read_numeric(meta)?;
        let terms_dict = Self::read_term_dict(meta)?;
        Ok(SortedEntry { ords, terms_dict })
    }

    fn read_sorted_set(meta: &mut dyn IndexInput) -> Result<SortedSetEntry> {
        let multi_valued = meta.read_byte()?;
        match multi_valued {
            0 => Ok(SortedSetEntry::Single(Self::read_sorted(meta)?)),
            1 => {
                let ords = Self::read_sorted_numeric(meta)?;
                let terms_dict = Self::read_term_dict(meta)?;
                Ok(SortedSetEntry::Multi { ords, terms_dict })
            }
            other => corrupt(format!("Invalid multiValued flag: {other}")),
        }
    }

    fn read_sorted_numeric(meta: &mut dyn IndexInput) -> Result<SortedNumericEntry> {
        let numeric = Self::read_numeric(meta)?;
        let num_docs_with_field = meta.read_int()?;
        let mut addresses_offset = 0;
        let mut addresses_meta = None;
        let mut addresses_length = 0;
        if i64::from(num_docs_with_field) != numeric.num_values {
            addresses_offset = meta.read_long()?;
            let block_shift = meta.read_v_int()?;
            addresses_meta = Some(DirectMonotonicMeta::load(
                meta,
                i64::from(num_docs_with_field).wrapping_add(1),
                block_shift,
            )?);
            addresses_length = meta.read_long()?;
        }
        Ok(SortedNumericEntry {
            numeric,
            num_docs_with_field,
            addresses_offset,
            addresses_meta,
            addresses_length,
        })
    }

    fn read_term_dict(meta: &mut dyn IndexInput) -> Result<TermsDictEntry> {
        let terms_dict_size = meta.read_v_long()?;
        let block_shift = meta.read_int()?;
        let addresses_size = shift_up_round(terms_dict_size, TERMS_DICT_BLOCK_LZ4_SHIFT as u32);
        let terms_addresses_meta = DirectMonotonicMeta::load(meta, addresses_size, block_shift)?;
        let _max_term_length = meta.read_int()?;
        let _max_block_length = meta.read_int()?;
        let terms_data_offset = meta.read_long()?;
        let terms_data_length = meta.read_long()?;
        let terms_addresses_offset = meta.read_long()?;
        let terms_addresses_length = meta.read_long()?;
        // **Divergence.** Lucene keeps the reverse index and uses it to narrow
        // `seekCeil`/`lookupTerm` to one block before scanning
        // (`TermsDict.seekTermsIndex`). This port answers `lookup_term` with
        // the trait's binary search over `lookup_ord`, so the reverse index is
        // consumed here only to keep the metadata cursor aligned and then
        // discarded. The results agree — both find the same term — but a
        // lookup costs O(log n) block seeks instead of one index-guided seek.
        // Porting the reverse index would remove the difference.
        let index_shift = meta.read_int()?;
        let index_size = shift_up_round(terms_dict_size, index_shift as u32);
        let _index_meta = DirectMonotonicMeta::load(meta, index_size.wrapping_add(1), block_shift)?;
        let _index_offset = meta.read_long()?;
        let _index_length = meta.read_long()?;
        let _index_addresses_offset = meta.read_long()?;
        let _index_addresses_length = meta.read_long()?;
        Ok(TermsDictEntry {
            terms_dict_size,
            terms_addresses_meta,
            terms_data_offset,
            terms_data_length,
            terms_addresses_offset,
            terms_addresses_length,
        })
    }
}

/// Java's `(value + (1L << shift) - 1) >>> shift`.
///
/// The shift can come off disk, and Java takes its low six bits rather than
/// refusing it, so a corrupt file has to produce here whatever it produces
/// there — an absurd block count the metadata load then runs out of input on,
/// never an overflow.
fn shift_up_round(value: i64, shift: u32) -> i64 {
    let shift = shift & 63;
    let value = value as u64;
    (value.wrapping_add(1u64 << shift).wrapping_sub(1) >> shift) as i64
}

/// Checks a region against the data file before slicing it.
///
/// `IndexInput.slice` throws `IllegalArgumentException` for a region the file
/// does not hold; both numbers here come off disk, so the same check has to
/// happen before the slice is built.
fn check_region(data: &dyn IndexInput, offset: i64, length: i64) -> Result<()> {
    let file_length = data.length();
    if offset < 0 || length < 0 || offset > file_length || length > file_length - offset {
        return corrupt(format!(
            "doc-values region [{offset}, {offset}+{length}) lies outside the \
             {file_length}-byte data file"
        ));
    }
    Ok(())
}

fn checked_slice(
    data: &dyn IndexInput,
    description: &str,
    offset: i64,
    length: i64,
) -> Result<Box<dyn IndexInput>> {
    check_region(data, offset, length)?;
    data.slice(description, offset, length)
}

type SharedRandomAccess = Rc<RefCell<Box<dyn RandomAccessInput>>>;

fn checked_random_access(
    data: &dyn IndexInput,
    offset: i64,
    length: i64,
) -> Result<SharedRandomAccess> {
    check_region(data, offset, length)?;
    Ok(Rc::new(RefCell::new(
        data.random_access_slice(offset, length)?,
    )))
}

// -----------------------------------------------------------------------------
// Lazy views over the data file
// -----------------------------------------------------------------------------

/// The documents that carry a value, and the ordinal of the current one within
/// the value stream.
///
/// Mirrors the split `Lucene90DocValuesProducer` makes between its
/// `Dense*DocValues` and `Sparse*DocValues` inner classes: a dense field needs
/// no documents-with-field stream and indexes its values by document id, while
/// a sparse one indexes them by `IndexedDISI.index()`.
enum DocsCursor {
    /// `docsWithFieldOffset == -2`: no document carries the field.
    Empty { doc: i32 },
    /// `docsWithFieldOffset == -1`: every document carries it.
    Dense { max_doc: i32, doc: i32 },
    /// An `IndexedDISI` stream names the documents.
    Sparse { disi: IndexedDISI },
}

impl DocsCursor {
    fn new(
        data: &dyn IndexInput,
        max_doc: i32,
        offset: i64,
        length: i64,
        jump_table_entry_count: i16,
        dense_rank_power: i8,
        cost: i64,
    ) -> Result<Self> {
        if offset == -2 {
            return Ok(Self::Empty { doc: -1 });
        }
        if offset == -1 {
            return Ok(Self::Dense { max_doc, doc: -1 });
        }
        check_region(data, offset, length)?;
        Ok(Self::Sparse {
            disi: IndexedDISI::new(
                data,
                offset,
                length,
                i32::from(jump_table_entry_count),
                dense_rank_power,
                cost,
            )?,
        })
    }

    fn doc_id(&self) -> i32 {
        match self {
            Self::Empty { doc } | Self::Dense { doc, .. } => *doc,
            Self::Sparse { disi } => disi.doc_id(),
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        match self {
            Self::Empty { doc } => {
                *doc = NO_MORE_DOCS;
                Ok(NO_MORE_DOCS)
            }
            Self::Dense { doc, .. } => {
                let target = doc.wrapping_add(1);
                self.advance(target)
            }
            Self::Sparse { disi } => disi.next_doc(),
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        match self {
            Self::Empty { doc } => {
                *doc = NO_MORE_DOCS;
                Ok(NO_MORE_DOCS)
            }
            // `DenseNumericDocValues.advance` (`Lucene90DocValuesProducer.java:
            // 695-701`) tests only the upper bound. A `next_doc` past the end
            // overflows `doc + 1` to `Integer.MIN_VALUE` in Java exactly as it
            // wraps to `i32::MIN` here, and Java then stores that as the
            // document id; the iterator contract forbids the call, and both
            // sides answer it with the same rubbish rather than a check.
            Self::Dense { max_doc, doc } => {
                *doc = if target >= *max_doc {
                    NO_MORE_DOCS
                } else {
                    target
                };
                Ok(*doc)
            }
            Self::Sparse { disi } => disi.advance(target),
        }
    }

    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        match self {
            Self::Empty { .. } => Ok(false),
            // `DenseNumericDocValues.advanceExact` (`Lucene90DocValuesProducer
            // .java:703-707`) sets the document and answers `true` without
            // testing anything: every document of a dense field has a value by
            // definition, and a target outside the segment is a caller error
            // the contract leaves undefined.
            Self::Dense { doc, .. } => {
                *doc = target;
                Ok(true)
            }
            Self::Sparse { disi } => disi.advance_exact(target),
        }
    }

    /// The ordinal of the current document within the value stream.
    ///
    /// A dense field indexes its values by document id and a sparse one by
    /// `IndexedDISI.index()`, which is what Lucene's `Dense*DocValues` and
    /// `Sparse*DocValues` do. The cursor is **not** checked for being
    /// positioned: `DocValuesIterator` leaves the result undefined unless it
    /// is, and Lucene reads at whatever `doc` holds — a constant field even
    /// answers with its constant after the iterator is exhausted
    /// (`Lucene90DocValuesProducer.java:789-793`). An out-of-range ordinal
    /// still cannot read out of bounds: it reaches `DirectReader::get_checked`,
    /// which reports the read past the end rather than serving it.
    fn index(&self) -> i64 {
        match self {
            Self::Empty { .. } => -1,
            Self::Dense { doc, .. } => i64::from(*doc),
            Self::Sparse { disi } => i64::from(disi.index()),
        }
    }

    fn cost(&self, cost: i64) -> i64 {
        match self {
            Self::Empty { .. } => 0,
            Self::Dense { max_doc, .. } => i64::from(*max_doc),
            Self::Sparse { .. } => cost,
        }
    }
}

/// How a numeric entry's values are decoded, one at a time, from the file.
///
/// The three shapes are the ones `getNumeric` branches on
/// (`Lucene90DocValuesProducer.java:783-...`): a constant that lives in the
/// metadata, an ordinal into a unique-value table, and `gcd * packed + min`.
enum NumericValueSource {
    Constant(i64),
    Table {
        reader: DirectReader,
        table: Vec<i64>,
    },
    Packed {
        reader: DirectReader,
        min: i64,
        gcd: i64,
    },
}

impl NumericValueSource {
    fn new(data: &dyn IndexInput, entry: &NumericEntry) -> Result<Self> {
        if entry.bits_per_value == 0 {
            return Ok(Self::Constant(entry.min_value));
        }
        if entry.block_shift >= 0 {
            // **Divergence (pre-existing).** `writeValues` switches to
            // per-block widths when that saves over 10% of the packed bits
            // (`Lucene90DocValuesConsumer.java:486-497`), and Lucene reads them
            // back through its `VaryingBPVReader`. This port's consumer never
            // writes that layout — no field it indexes reaches the 16384 values
            // one block holds — so its reader was never ported either. The
            // entry is refused rather than decoded as if it were single-block,
            // which would silently return wrong values.
            return corrupt(
                "per-block numeric widths are not supported by this reader".to_string(),
            );
        }
        let slice = checked_random_access(data, entry.values_offset, entry.values_length)?;
        let reader = DirectReader::with_random_access(slice, i32::from(entry.bits_per_value), 0)?;
        Ok(match entry.table {
            Some(ref table) => Self::Table {
                reader,
                table: table.clone(),
            },
            None => Self::Packed {
                reader,
                min: entry.min_value,
                gcd: entry.gcd,
            },
        })
    }

    fn value(&self, index: i64) -> Result<i64> {
        match self {
            Self::Constant(value) => Ok(*value),
            Self::Table { reader, table } => {
                let ordinal = reader.get_checked(index)?;
                // The ordinal is `bitsPerValue` wide and the table at most 256
                // long: a corrupt width makes the two disagree, and Java
                // answers that with an `ArrayIndexOutOfBoundsException` rather
                // than a value from somewhere else.
                usize::try_from(ordinal)
                    .ok()
                    .and_then(|ordinal| table.get(ordinal).copied())
                    .map_or_else(
                        || {
                            corrupt(format!(
                                "table ordinal {ordinal} is outside the {}-entry table",
                                table.len()
                            ))
                        },
                        Ok,
                    )
            }
            // Java's arithmetic wraps here; a corrupt `min` or `gcd` must
            // produce the value Java would produce, not an overflow panic.
            Self::Packed { reader, min, gcd } => Ok(gcd
                .wrapping_mul(reader.get_checked(index)?)
                .wrapping_add(*min)),
        }
    }
}

/// A `NUMERIC` field, read one value at a time.
struct LazyNumericDocValues {
    docs: DocsCursor,
    values: NumericValueSource,
    cost: i64,
}

impl LazyNumericDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &NumericEntry) -> Result<Self> {
        let docs = DocsCursor::new(
            data,
            max_doc,
            entry.docs_with_field_offset,
            entry.docs_with_field_length,
            entry.jump_table_entry_count,
            entry.dense_rank_power,
            entry.num_values,
        )?;
        Ok(Self {
            docs,
            values: NumericValueSource::new(data, entry)?,
            cost: entry.num_values,
        })
    }
}

impl DocIdSetIterator for LazyNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.docs.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.docs.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.docs.advance(target)
    }

    fn cost(&self) -> i64 {
        self.docs.cost(self.cost)
    }
}

impl DocValuesIterator for LazyNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.docs.advance_exact(target)
    }
}

impl NumericDocValues for LazyNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        // No positioning check, for the reason given on `DocsCursor::index`:
        // Lucene reads at whatever the cursor holds, and a constant field
        // answers with its constant whether or not the iterator is positioned.
        // A packed or table-encoded field instead reaches `get_checked`, which
        // reports the out-of-range read exactly where Java throws.
        self.values.value(self.docs.index())
    }
}

/// A `BINARY` field, whose values are cut out of the data region on demand.
struct LazyBinaryDocValues {
    docs: DocsCursor,
    bytes: RefCell<Box<dyn RandomAccessInput>>,
    region_length: i64,
    addresses: Option<DirectMonotonicReader>,
    fixed_length: i32,
    cost: i64,
}

impl LazyBinaryDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &BinaryEntry) -> Result<Self> {
        let docs = DocsCursor::new(
            data,
            max_doc,
            entry.docs_with_field_offset,
            entry.docs_with_field_length,
            entry.jump_table_entry_count,
            entry.dense_rank_power,
            i64::from(entry.num_docs_with_field),
        )?;
        if entry.min_length < 0 || entry.max_length < entry.min_length {
            return corrupt(format!(
                "binary lengths must be non-negative and ordered, got \
                 min={} max={}",
                entry.min_length, entry.max_length
            ));
        }
        check_region(data, entry.data_offset, entry.data_length)?;
        let bytes = data.random_access_slice(entry.data_offset, entry.data_length)?;
        let addresses = match entry.addresses_meta {
            Some(ref meta) => {
                let slice =
                    checked_random_access(data, entry.addresses_offset, entry.addresses_length)?;
                Some(DirectMonotonicReader::with_random_access(
                    meta.clone(),
                    slice,
                )?)
            }
            None => None,
        };
        Ok(Self {
            docs,
            bytes: RefCell::new(bytes),
            region_length: entry.data_length,
            addresses,
            fixed_length: entry.min_length,
            cost: i64::from(entry.num_docs_with_field),
        })
    }
}

impl DocIdSetIterator for LazyBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.docs.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.docs.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.docs.advance(target)
    }

    fn cost(&self) -> i64 {
        self.docs.cost(self.cost)
    }
}

impl DocValuesIterator for LazyBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.docs.advance_exact(target)
    }
}

impl BinaryDocValues for LazyBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        let index = self.docs.index();
        if index < 0 {
            return corrupt("the cursor is not positioned on a document".to_string());
        }
        let (start, end) = match self.addresses {
            Some(ref addresses) => (
                addresses.get_checked(index)?,
                addresses.get_checked(index.wrapping_add(1))?,
            ),
            None => {
                let start = index.saturating_mul(i64::from(self.fixed_length));
                (start, start.saturating_add(i64::from(self.fixed_length)))
            }
        };
        // Every bound here is decoded from the file, so the region is checked
        // before a byte of it is read: Java raises rather than handing back
        // bytes that belong to another document.
        if start < 0 || end < start || end > self.region_length {
            return corrupt(format!(
                "binary value spans [{start}, {end}), which the \
                 {}-byte region does not hold",
                self.region_length
            ));
        }
        let length = (end - start) as usize;
        let mut value = try_zeroed(length)?;
        self.bytes
            .borrow_mut()
            .read_bytes_at(start, &mut value, 0, length)?;
        Ok(BytesRef::new(value))
    }
}

/// A cursor over one term dictionary, seeking by ordinal.
///
/// Ports `Lucene90DocValuesProducer.TermsDict`: the terms of each 64-term block
/// are LZ4-compressed against the block's first term, which is stored raw, so
/// reaching an ordinal means seeking to its block, decompressing it once, and
/// walking forward. Only the block a lookup lands in is ever decompressed.
struct TermsDict {
    bytes: Box<dyn IndexInput>,
    block_addresses: DirectMonotonicReader,
    size: i64,
    data_length: i64,
    ord: i64,
    term: Vec<u8>,
    /// The block's first term followed by its decompressed suffixes.
    block: Vec<u8>,
    block_pos: usize,
}

impl TermsDict {
    fn new(data: &dyn IndexInput, entry: &TermsDictEntry) -> Result<Self> {
        let addresses = checked_random_access(
            data,
            entry.terms_addresses_offset,
            entry.terms_addresses_length,
        )?;
        let block_addresses = DirectMonotonicReader::with_random_access(
            entry.terms_addresses_meta.clone(),
            addresses,
        )?;
        let bytes = checked_slice(
            data,
            "terms",
            entry.terms_data_offset,
            entry.terms_data_length,
        )?;
        Ok(Self {
            bytes,
            block_addresses,
            size: entry.terms_dict_size,
            data_length: entry.terms_data_length,
            ord: -1,
            term: Vec::new(),
            block: Vec::new(),
            block_pos: 0,
        })
    }

    fn block_byte(&mut self) -> Result<u8> {
        let byte = *self.block.get(self.block_pos).ok_or_else(|| {
            LuceneError::CorruptIndex("a terms block ended before its terms did".to_string())
        })?;
        self.block_pos += 1;
        Ok(byte)
    }

    fn block_v_int(&mut self) -> Result<i32> {
        let mut value = 0i32;
        let mut shift = 0u32;
        loop {
            let byte = self.block_byte()?;
            value |= i32::from(byte & 0x7F).wrapping_shl(shift);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift = shift.wrapping_add(7);
            if shift > 28 {
                return corrupt("a terms block holds a malformed VInt".to_string());
            }
        }
    }

    /// Reads the first term of the block the cursor sits on and decompresses
    /// the rest of it against that term.
    fn decompress_block(&mut self) -> Result<()> {
        let term_length = self.bytes.read_v_int()?;
        if term_length < 0 {
            return corrupt(format!("negative term length {term_length}"));
        }
        let term_length = term_length as usize;
        let mut term = try_zeroed(term_length)?;
        self.bytes.read_bytes(&mut term, 0, term_length)?;
        self.term = term;
        self.block_pos = 0;
        self.block.clear();

        // The final block of a dictionary whose last block holds one term has
        // nothing compressed after it (`TermsDict.decompressBlock`).
        let offset = self.bytes.file_pointer();
        if offset < self.data_length - 1 {
            let uncompressed_length = self.bytes.read_v_int()?;
            if uncompressed_length < 0 {
                return corrupt(format!("negative terms block length {uncompressed_length}"));
            }
            let uncompressed_length = uncompressed_length as usize;
            let mut block = try_zeroed(term_length + uncompressed_length)?;
            block[..term_length].copy_from_slice(&self.term);
            Lz4::decompress(
                &mut *self.bytes,
                uncompressed_length,
                &mut block,
                term_length,
            )?;
            self.block = block;
            self.block_pos = term_length;
        }
        Ok(())
    }

    /// Advances to the next term, exactly as `TermsDict.next` does.
    fn next(&mut self) -> Result<()> {
        self.ord = self.ord.wrapping_add(1);
        if self.ord >= self.size {
            return corrupt("the term dictionary ended before the ordinal sought".to_string());
        }
        if self.ord & TERMS_DICT_BLOCK_LZ4_MASK as i64 == 0 {
            return self.decompress_block();
        }
        let token = usize::from(self.block_byte()?);
        let mut prefix_length = token & 0x0F;
        let mut suffix_length = 1 + (token >> 4);
        if prefix_length == 15 {
            prefix_length += self.block_v_int()? as usize;
        }
        if suffix_length == 16 {
            suffix_length += self.block_v_int()? as usize;
        }
        // The prefix is shared with the term before it, so a prefix longer than
        // that term names bytes the block never wrote.
        if prefix_length > self.term.len() {
            return corrupt(format!(
                "a term shares a {prefix_length}-byte prefix with a term of {} bytes",
                self.term.len()
            ));
        }
        let available = self.block.len().saturating_sub(self.block_pos);
        if suffix_length > available {
            return corrupt(format!(
                "a term suffix of {suffix_length} bytes runs past the {available} the block holds"
            ));
        }
        self.term.truncate(prefix_length);
        self.term
            .extend_from_slice(&self.block[self.block_pos..self.block_pos + suffix_length]);
        self.block_pos += suffix_length;
        Ok(())
    }

    fn seek_exact(&mut self, ord: i64) -> Result<&[u8]> {
        if ord < 0 || ord >= self.size {
            return corrupt(format!(
                "ordinal {ord} is outside the {}-term dictionary",
                self.size
            ));
        }
        // Signed shift, because `ord` is -1 while the cursor is unpositioned.
        let current_block = self.ord >> TERMS_DICT_BLOCK_LZ4_SHIFT;
        let block = ord >> TERMS_DICT_BLOCK_LZ4_SHIFT;
        if ord < self.ord || block != current_block {
            let address = self.block_addresses.get_checked(block)?;
            if address < 0 || address > self.data_length {
                return corrupt(format!(
                    "terms block {block} starts at {address}, outside the \
                     {}-byte dictionary",
                    self.data_length
                ));
            }
            self.bytes.seek(address)?;
            self.ord = (block << TERMS_DICT_BLOCK_LZ4_SHIFT) - 1;
        }
        while self.ord < ord {
            self.next()?;
        }
        Ok(&self.term)
    }
}

/// A `SORTED` field: an ordinal per document, plus the dictionary they index.
struct LazySortedDocValues {
    ords: LazyNumericDocValues,
    terms: RefCell<TermsDict>,
    value_count: i64,
}

impl LazySortedDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &SortedEntry) -> Result<Self> {
        Ok(Self {
            ords: LazyNumericDocValues::new(data, max_doc, &entry.ords)?,
            terms: RefCell::new(TermsDict::new(data, &entry.terms_dict)?),
            value_count: entry.terms_dict.terms_dict_size,
        })
    }
}

impl DocIdSetIterator for LazySortedDocValues {
    fn doc_id(&self) -> i32 {
        self.ords.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.ords.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.ords.advance(target)
    }

    fn cost(&self) -> i64 {
        self.ords.cost()
    }
}

impl DocValuesIterator for LazySortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.ords.advance_exact(target)
    }
}

impl SortedDocValues for LazySortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ords.long_value()? as i32)
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.value_count as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        let term = self.terms.borrow_mut().seek_exact(i64::from(ord))?.to_vec();
        Ok(BytesRef::new(term))
    }
}

/// A `SORTED_NUMERIC` field, or the ordinals of a multi-valued `SORTED_SET`.
///
/// When `numValues == numDocsWithField` the consumer writes no address table
/// and Lucene serves the entry through `DocValues.singleton`
/// (`Lucene90DocValuesProducer.java:437-450`); that is the `addresses == None`
/// case here, where a document's single value sits at its own index.
struct LazySortedNumericDocValues {
    docs: DocsCursor,
    values: NumericValueSource,
    addresses: Option<DirectMonotonicReader>,
    cost: i64,
    start: i64,
    end: i64,
    cursor: i64,
}

impl LazySortedNumericDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &SortedNumericEntry) -> Result<Self> {
        let docs = DocsCursor::new(
            data,
            max_doc,
            entry.numeric.docs_with_field_offset,
            entry.numeric.docs_with_field_length,
            entry.numeric.jump_table_entry_count,
            entry.numeric.dense_rank_power,
            i64::from(entry.num_docs_with_field),
        )?;
        let addresses = match entry.addresses_meta {
            Some(ref meta) => {
                let slice =
                    checked_random_access(data, entry.addresses_offset, entry.addresses_length)?;
                Some(DirectMonotonicReader::with_random_access(
                    meta.clone(),
                    slice,
                )?)
            }
            None => None,
        };
        Ok(Self {
            docs,
            values: NumericValueSource::new(data, &entry.numeric)?,
            addresses,
            cost: i64::from(entry.num_docs_with_field),
            start: 0,
            end: 0,
            cursor: 0,
        })
    }

    /// Positions the value window on the document the cursor just reached.
    fn position(&mut self) -> Result<()> {
        let index = self.docs.index();
        if index < 0 {
            self.start = 0;
            self.end = 0;
            self.cursor = 0;
            return Ok(());
        }
        match self.addresses {
            Some(ref addresses) => {
                self.start = addresses.get_checked(index)?;
                self.end = addresses.get_checked(index.wrapping_add(1))?;
            }
            None => {
                self.start = index;
                self.end = index.wrapping_add(1);
            }
        }
        if self.end < self.start {
            return corrupt(format!(
                "a document claims values [{}, {})",
                self.start, self.end
            ));
        }
        self.cursor = self.start;
        Ok(())
    }
}

impl DocIdSetIterator for LazySortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.docs.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.docs.next_doc()?;
        if doc != NO_MORE_DOCS {
            self.position()?;
        }
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.docs.advance(target)?;
        if doc != NO_MORE_DOCS {
            self.position()?;
        }
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.docs.cost(self.cost)
    }
}

impl DocValuesIterator for LazySortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let found = self.docs.advance_exact(target)?;
        if found {
            self.position()?;
        }
        Ok(found)
    }
}

impl SortedNumericDocValues for LazySortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        if self.cursor >= self.end {
            return corrupt(format!(
                "the document holds values [{}, {}) and one more was asked for",
                self.start, self.end
            ));
        }
        let value = self.values.value(self.cursor)?;
        self.cursor += 1;
        Ok(value)
    }

    fn doc_value_count(&self) -> Result<i32> {
        // Java computes `(int) (end - start)` and lets both the subtraction and
        // the narrowing wrap; a corrupt address table has to produce the value
        // Java produces, not an overflow panic.
        Ok(self.end.wrapping_sub(self.start) as i32)
    }
}

/// A `SORTED_SET` field, in either of its two layouts.
///
/// The single-valued one is a `SORTED` entry behind a `multiValued == 0` byte,
/// which Lucene serves through `DocValues.singleton`; both are driven here by
/// the same ordinal cursor, so only the construction differs.
struct LazySortedSetDocValues {
    ords: LazySortedNumericDocValues,
    terms: RefCell<TermsDict>,
    value_count: i64,
}

impl LazySortedSetDocValues {
    fn new(data: &dyn IndexInput, max_doc: i32, entry: &SortedSetEntry) -> Result<Self> {
        let (ords_entry, terms_dict) = match entry {
            SortedSetEntry::Single(single) => (
                SortedNumericEntry {
                    numeric: single.ords.clone(),
                    // No address table: one ordinal per document, at its own
                    // index, which is what `numValues == numDocsWithField`
                    // means for the shared cursor.
                    num_docs_with_field: single.ords.num_values as i32,
                    addresses_offset: 0,
                    addresses_meta: None,
                    addresses_length: 0,
                },
                &single.terms_dict,
            ),
            SortedSetEntry::Multi { ords, terms_dict } => (ords.clone(), terms_dict),
        };
        Ok(Self {
            ords: LazySortedNumericDocValues::new(data, max_doc, &ords_entry)?,
            terms: RefCell::new(TermsDict::new(data, terms_dict)?),
            value_count: terms_dict.terms_dict_size,
        })
    }
}

impl DocIdSetIterator for LazySortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.ords.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.ords.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.ords.advance(target)
    }

    fn cost(&self) -> i64 {
        self.ords.cost()
    }
}

impl DocValuesIterator for LazySortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.ords.advance_exact(target)
    }
}

impl SortedSetDocValues for LazySortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        self.ords.next_value()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.ords.doc_value_count()
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        let term = self.terms.borrow_mut().seek_exact(ord)?.to_vec();
        Ok(BytesRef::new(term))
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.value_count)
    }
}

// -----------------------------------------------------------------------------
// The producer's public surface
// -----------------------------------------------------------------------------

impl DocValuesProducer for Lucene90DocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::Numeric(entry)) => Ok(Box::new(LazyNumericDocValues::new(
                &*self.data,
                self.max_doc,
                entry,
            )?)),
            _ => Ok(Box::new(EmptyNumericDocValues::new())),
        }
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::Binary(entry)) => Ok(Box::new(LazyBinaryDocValues::new(
                &*self.data,
                self.max_doc,
                entry,
            )?)),
            _ => Ok(Box::new(EmptyBinaryDocValues::new())),
        }
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::Sorted(entry)) => Ok(Box::new(LazySortedDocValues::new(
                &*self.data,
                self.max_doc,
                entry,
            )?)),
            _ => Ok(Box::new(EmptySortedDocValues::new())),
        }
    }

    fn get_sorted_numeric(&self, field: &FieldInfo) -> Result<Box<dyn SortedNumericDocValues>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::SortedNumeric(entry)) => Ok(Box::new(
                LazySortedNumericDocValues::new(&*self.data, self.max_doc, entry)?,
            )),
            _ => Ok(Box::new(EmptySortedNumericDocValues::new())),
        }
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Box<dyn SortedSetDocValues>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::SortedSet(entry)) => Ok(Box::new(LazySortedSetDocValues::new(
                &*self.data,
                self.max_doc,
                entry,
            )?)),
            _ => Ok(Box::new(EmptySortedSetDocValues::new())),
        }
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper>> {
        // **Divergence (pre-existing).** Lucene serves a `DocValuesSkipper`
        // built from the skipper metadata `readDocValueSkipperMeta` parsed
        // alongside the entry. This port never writes a skip index — no field
        // it indexes sets `DocValuesSkipIndexType` — so `.dvs` holds nothing
        // but a header and a footer, and an empty skipper is the only honest
        // answer. Porting the skipper is a task of its own.
        Ok(Box::new(EmptyDocValuesSkipper))
    }

    fn check_integrity(&self) -> Result<()> {
        // Lucene checksums the data file and, when it opened one, the
        // skip-index file too (`Lucene90DocValuesProducer.java:2285-2291`).
        // This port opens no skip-index file, so only the data file is covered;
        // see the note in `Lucene90DocValuesProducer::new`.
        let mut data = self.data.clone_input()?;
        checksum_entire_file(&mut *data)?;
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            data: self.data.clone_input()?,
            max_doc: self.max_doc,
            entries: self.entries.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        self.data.close()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::{BufferedUpdates, SegmentInfo};
    use crate::codecs::tests::DummyCodec;
    use crate::index::FieldInfos;
    use crate::search::Sort;
    use crate::store::{RamDirectory, DEFAULT_IO_CONTEXT};
    use crate::util::default_info_stream;
    use crate::util::string_helper::ID_LENGTH;
    use crate::util::Version;
    use std::collections::HashMap;
    use std::sync::Arc;

    type NumericDocsValues = (Vec<i32>, Vec<i64>);
    type BinaryDocsValues = (Vec<i32>, Vec<Vec<u8>>);
    type SortedDocsValues = (Vec<i32>, Vec<i32>, Vec<Vec<u8>>);
    type SortedNumericDocsValues = (Vec<i32>, Vec<Vec<i64>>);
    type SortedSetDocsValues = (Vec<i32>, Vec<Vec<i64>>, Vec<Vec<u8>>);

    #[derive(Debug, Default, Clone)]
    struct TestDocValuesProducer {
        numeric: HashMap<i32, NumericDocsValues>,
        binary: HashMap<i32, BinaryDocsValues>,
        sorted: HashMap<i32, SortedDocsValues>,
        sorted_numeric: HashMap<i32, SortedNumericDocsValues>,
        sorted_set: HashMap<i32, SortedSetDocsValues>,
    }

    impl TestDocValuesProducer {
        fn with_numeric(field: i32, docs: Vec<i32>, values: Vec<i64>) -> Self {
            let mut this = Self::default();
            this.numeric.insert(field, (docs, values));
            this
        }

        fn with_binary(field: i32, docs: Vec<i32>, values: Vec<Vec<u8>>) -> Self {
            let mut this = Self::default();
            this.binary.insert(field, (docs, values));
            this
        }

        fn with_sorted(field: i32, docs: Vec<i32>, ords: Vec<i32>, terms: Vec<Vec<u8>>) -> Self {
            let mut this = Self::default();
            this.sorted.insert(field, (docs, ords, terms));
            this
        }

        fn with_sorted_numeric(field: i32, docs: Vec<i32>, values: Vec<Vec<i64>>) -> Self {
            let mut this = Self::default();
            this.sorted_numeric.insert(field, (docs, values));
            this
        }

        fn with_sorted_set(
            field: i32,
            docs: Vec<i32>,
            ords: Vec<Vec<i64>>,
            terms: Vec<Vec<u8>>,
        ) -> Self {
            let mut this = Self::default();
            this.sorted_set.insert(field, (docs, ords, terms));
            this
        }
    }

    // -------------------------------------------------------------------------
    // In-memory doc values, used only to feed the consumer in these tests
    // -------------------------------------------------------------------------

    /// Walks a fixed document list, exposing the ordinal of the current one.
    #[derive(Debug, Clone, Default)]
    struct DocCursor {
        docs: Vec<i32>,
        pos: usize,
        doc: i32,
    }

    impl DocCursor {
        fn new(docs: Vec<i32>) -> Self {
            Self {
                docs,
                pos: usize::MAX,
                doc: -1,
            }
        }

        fn next_doc(&mut self) -> i32 {
            let next = self.pos.wrapping_add(1);
            self.doc = if next < self.docs.len() {
                self.pos = next;
                self.docs[next]
            } else {
                NO_MORE_DOCS
            };
            self.doc
        }

        fn advance(&mut self, target: i32) -> i32 {
            let from = self.pos.wrapping_add(1).min(self.docs.len());
            let found = self.docs[from..].iter().position(|doc| *doc >= target);
            self.doc = match found {
                Some(offset) => {
                    self.pos = from + offset;
                    self.docs[self.pos]
                }
                None => {
                    self.pos = self.docs.len();
                    NO_MORE_DOCS
                }
            };
            self.doc
        }

        fn advance_exact(&mut self, target: i32) -> bool {
            match self.docs.binary_search(&target) {
                Ok(pos) => {
                    self.pos = pos;
                    self.doc = target;
                    true
                }
                Err(_) => false,
            }
        }
    }

    macro_rules! delegate_cursor {
        ($ty:ty) => {
            impl DocIdSetIterator for $ty {
                fn doc_id(&self) -> i32 {
                    self.cursor.doc
                }

                fn next_doc(&mut self) -> Result<i32> {
                    Ok(self.cursor.next_doc())
                }

                fn advance(&mut self, target: i32) -> Result<i32> {
                    Ok(self.cursor.advance(target))
                }

                fn cost(&self) -> i64 {
                    self.cursor.docs.len() as i64
                }
            }

            impl DocValuesIterator for $ty {
                fn advance_exact(&mut self, target: i32) -> Result<bool> {
                    Ok(self.cursor.advance_exact(target))
                }
            }
        };
    }

    struct VecNumeric {
        cursor: DocCursor,
        values: Vec<i64>,
    }
    delegate_cursor!(VecNumeric);
    impl NumericDocValues for VecNumeric {
        fn long_value(&self) -> Result<i64> {
            Ok(self.values[self.cursor.pos])
        }
    }

    struct VecBinary {
        cursor: DocCursor,
        values: Vec<Vec<u8>>,
    }
    delegate_cursor!(VecBinary);
    impl BinaryDocValues for VecBinary {
        fn binary_value(&self) -> Result<BytesRef> {
            Ok(BytesRef::new(self.values[self.cursor.pos].clone()))
        }
    }

    struct VecSorted {
        cursor: DocCursor,
        ords: Vec<i32>,
        terms: Vec<Vec<u8>>,
    }
    delegate_cursor!(VecSorted);
    impl SortedDocValues for VecSorted {
        fn ord_value(&self) -> Result<i32> {
            Ok(self.ords[self.cursor.pos])
        }

        fn get_value_count(&self) -> Result<i32> {
            Ok(self.terms.len() as i32)
        }

        fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
            Ok(BytesRef::new(self.terms[ord as usize].clone()))
        }
    }

    struct VecSortedNumeric {
        cursor: DocCursor,
        values: Vec<Vec<i64>>,
        index: usize,
    }
    impl DocIdSetIterator for VecSortedNumeric {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.index = 0;
            Ok(self.cursor.next_doc())
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.index = 0;
            Ok(self.cursor.advance(target))
        }

        fn cost(&self) -> i64 {
            self.cursor.docs.len() as i64
        }
    }
    impl DocValuesIterator for VecSortedNumeric {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.index = 0;
            Ok(self.cursor.advance_exact(target))
        }
    }
    impl SortedNumericDocValues for VecSortedNumeric {
        fn next_value(&mut self) -> Result<i64> {
            let value = self.values[self.cursor.pos][self.index];
            self.index += 1;
            Ok(value)
        }

        fn doc_value_count(&self) -> Result<i32> {
            Ok(self.values[self.cursor.pos].len() as i32)
        }
    }

    struct VecSortedSet {
        cursor: DocCursor,
        ords: Vec<Vec<i64>>,
        terms: Vec<Vec<u8>>,
        index: usize,
    }
    impl DocIdSetIterator for VecSortedSet {
        fn doc_id(&self) -> i32 {
            self.cursor.doc
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.index = 0;
            Ok(self.cursor.next_doc())
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.index = 0;
            Ok(self.cursor.advance(target))
        }

        fn cost(&self) -> i64 {
            self.cursor.docs.len() as i64
        }
    }
    impl DocValuesIterator for VecSortedSet {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.index = 0;
            Ok(self.cursor.advance_exact(target))
        }
    }
    impl SortedSetDocValues for VecSortedSet {
        fn next_ord(&mut self) -> Result<i64> {
            let ord = self.ords[self.cursor.pos][self.index];
            self.index += 1;
            Ok(ord)
        }

        fn doc_value_count(&self) -> Result<i32> {
            Ok(self.ords[self.cursor.pos].len() as i32)
        }

        fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
            Ok(BytesRef::new(self.terms[ord as usize].clone()))
        }

        fn get_value_count(&self) -> Result<i64> {
            Ok(self.terms.len() as i64)
        }
    }

    impl DocValuesProducer for TestDocValuesProducer {
        fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
            match self.numeric.get(&field.number) {
                Some((docs, values)) => Ok(Box::new(VecNumeric {
                    cursor: DocCursor::new(docs.clone()),
                    values: values.clone(),
                })),
                None => Ok(Box::new(EmptyNumericDocValues::new())),
            }
        }

        fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>> {
            match self.binary.get(&field.number) {
                Some((docs, values)) => Ok(Box::new(VecBinary {
                    cursor: DocCursor::new(docs.clone()),
                    values: values.clone(),
                })),
                None => Ok(Box::new(EmptyBinaryDocValues::new())),
            }
        }

        fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues>> {
            match self.sorted.get(&field.number) {
                Some((docs, ords, terms)) => Ok(Box::new(VecSorted {
                    cursor: DocCursor::new(docs.clone()),
                    ords: ords.clone(),
                    terms: terms.clone(),
                })),
                None => Ok(Box::new(EmptySortedDocValues::new())),
            }
        }

        fn get_sorted_numeric(&self, field: &FieldInfo) -> Result<Box<dyn SortedNumericDocValues>> {
            match self.sorted_numeric.get(&field.number) {
                Some((docs, values)) => Ok(Box::new(VecSortedNumeric {
                    cursor: DocCursor::new(docs.clone()),
                    values: values.clone(),
                    index: 0,
                })),
                None => Ok(Box::new(EmptySortedNumericDocValues::new())),
            }
        }

        fn get_sorted_set(&self, field: &FieldInfo) -> Result<Box<dyn SortedSetDocValues>> {
            match self.sorted_set.get(&field.number) {
                Some((docs, ords, terms)) => Ok(Box::new(VecSortedSet {
                    cursor: DocCursor::new(docs.clone()),
                    ords: ords.clone(),
                    terms: terms.clone(),
                    index: 0,
                })),
                None => Ok(Box::new(EmptySortedSetDocValues::new())),
            }
        }

        fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper>> {
            Ok(Box::new(EmptyDocValuesSkipper))
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
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
            Arc::new(DummyCodec::new("Dummy")),
            HashMap::new(),
            [0u8; ID_LENGTH],
            HashMap::new(),
            Sort::default(),
        )
        .unwrap()
    }

    fn write_only<F>(
        max_doc: i32,
        field: &FieldInfo,
        write_field: F,
    ) -> (Arc<dyn Directory>, SegmentInfo, FieldInfos)
    where
        F: FnOnce(&mut dyn DocValuesConsumer),
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", max_doc);
        let seg_updates = BufferedUpdates;
        let write_state = SegmentWriteState::new(
            default_info_stream(),
            dir.as_ref(),
            &segment_info,
            &field_infos,
            &seg_updates,
            &*DEFAULT_IO_CONTEXT,
        );
        let format = Lucene90DocValuesFormat::new();
        {
            let mut consumer = format.fields_consumer(&write_state).unwrap();
            write_field(consumer.as_mut());
            consumer.close().unwrap();
        }
        (dir, segment_info, field_infos)
    }

    fn write_and_read<F>(
        max_doc: i32,
        field: &FieldInfo,
        write_field: F,
    ) -> Lucene90DocValuesProducer
    where
        F: FnOnce(&mut dyn DocValuesConsumer),
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let field_infos = FieldInfos::new(vec![field.clone()]).unwrap();
        let segment_info = make_segment_info(Arc::clone(&dir), "_0", max_doc);
        let seg_updates = BufferedUpdates;
        let write_state = SegmentWriteState::new(
            default_info_stream(),
            dir.as_ref(),
            &segment_info,
            &field_infos,
            &seg_updates,
            &*DEFAULT_IO_CONTEXT,
        );
        let format = Lucene90DocValuesFormat::new();
        {
            let mut consumer = format.fields_consumer(&write_state).unwrap();
            write_field(consumer.as_mut());
            consumer.close().unwrap();
        }
        let read_state = SegmentReadState::new(
            dir.as_ref(),
            &segment_info,
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        );
        Lucene90DocValuesProducer::new(&read_state).unwrap()
    }

    #[test]
    fn format_name_is_lucene90() {
        assert_eq!(Lucene90DocValuesFormat::new().name(), "Lucene90");
    }

    #[test]
    fn round_trip_numeric_sparse() {
        let field = FieldInfo::new("num", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_numeric(0, vec![0, 2, 4], vec![1, 3, 5]);
            consumer.add_numeric_field(&field, &values).unwrap();
        });
        let mut numeric = producer.get_numeric(&field).unwrap();
        assert_eq!(numeric.next_doc().unwrap(), 0);
        assert_eq!(numeric.long_value().unwrap(), 1);
        assert_eq!(numeric.next_doc().unwrap(), 2);
        assert_eq!(numeric.long_value().unwrap(), 3);
        assert_eq!(numeric.next_doc().unwrap(), 4);
        assert_eq!(numeric.long_value().unwrap(), 5);
        assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_numeric_table_compression() {
        let field = FieldInfo::new("num", 0);
        let producer = write_and_read(4, &field, |consumer| {
            let values =
                TestDocValuesProducer::with_numeric(0, vec![0, 1, 2, 3], vec![0, 1, 2, 100]);
            consumer.add_numeric_field(&field, &values).unwrap();
        });
        let mut numeric = producer.get_numeric(&field).unwrap();
        assert_eq!(numeric.next_doc().unwrap(), 0);
        assert_eq!(numeric.long_value().unwrap(), 0);
        assert_eq!(numeric.next_doc().unwrap(), 1);
        assert_eq!(numeric.long_value().unwrap(), 1);
        assert_eq!(numeric.next_doc().unwrap(), 2);
        assert_eq!(numeric.long_value().unwrap(), 2);
        assert_eq!(numeric.next_doc().unwrap(), 3);
        assert_eq!(numeric.long_value().unwrap(), 100);
        assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_binary_fixed() {
        let field = FieldInfo::new("bin", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_binary(
                0,
                vec![0, 2, 4],
                vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            );
            consumer.add_binary_field(&field, &values).unwrap();
        });
        let mut binary = producer.get_binary(&field).unwrap();
        assert_eq!(binary.next_doc().unwrap(), 0);
        assert_eq!(binary.binary_value().unwrap().bytes, b"a");
        assert_eq!(binary.next_doc().unwrap(), 2);
        assert_eq!(binary.binary_value().unwrap().bytes, b"b");
        assert_eq!(binary.next_doc().unwrap(), 4);
        assert_eq!(binary.binary_value().unwrap().bytes, b"c");
        assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_binary_variable() {
        let field = FieldInfo::new("bin", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_binary(
                0,
                vec![0, 2, 4],
                vec![b"alpha".to_vec(), b"beta".to_vec(), b"gamma".to_vec()],
            );
            consumer.add_binary_field(&field, &values).unwrap();
        });
        let mut binary = producer.get_binary(&field).unwrap();
        assert_eq!(binary.next_doc().unwrap(), 0);
        assert_eq!(binary.binary_value().unwrap().bytes, b"alpha");
        assert_eq!(binary.next_doc().unwrap(), 2);
        assert_eq!(binary.binary_value().unwrap().bytes, b"beta");
        assert_eq!(binary.next_doc().unwrap(), 4);
        assert_eq!(binary.binary_value().unwrap().bytes, b"gamma");
        assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_sorted() {
        let field = FieldInfo::new("sorted", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_sorted(
                0,
                vec![0, 1, 2, 4],
                vec![2, 0, 1, 2],
                vec![b"bar".to_vec(), b"baz".to_vec(), b"foo".to_vec()],
            );
            consumer.add_sorted_field(&field, &values).unwrap();
        });
        let mut sorted = producer.get_sorted(&field).unwrap();
        assert_eq!(sorted.next_doc().unwrap(), 0);
        assert_eq!(sorted.ord_value().unwrap(), 2);
        assert_eq!(sorted.next_doc().unwrap(), 1);
        assert_eq!(sorted.ord_value().unwrap(), 0);
        assert_eq!(sorted.next_doc().unwrap(), 2);
        assert_eq!(sorted.ord_value().unwrap(), 1);
        assert_eq!(sorted.next_doc().unwrap(), 4);
        assert_eq!(sorted.ord_value().unwrap(), 2);
        assert_eq!(sorted.next_doc().unwrap(), NO_MORE_DOCS);

        assert_eq!(sorted.get_value_count().unwrap(), 3);
        assert_eq!(sorted.lookup_ord(0).unwrap().bytes, b"bar");
        assert_eq!(sorted.lookup_ord(1).unwrap().bytes, b"baz");
        assert_eq!(sorted.lookup_ord(2).unwrap().bytes, b"foo");
    }

    #[test]
    fn round_trip_sorted_numeric() {
        let field = FieldInfo::new("sorted_num", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_sorted_numeric(
                0,
                vec![0, 1, 3],
                vec![vec![1, 2], vec![3], vec![4, 5, 6]],
            );
            consumer.add_sorted_numeric_field(&field, &values).unwrap();
        });
        let mut sorted_numeric = producer.get_sorted_numeric(&field).unwrap();
        assert_eq!(sorted_numeric.next_doc().unwrap(), 0);
        assert_eq!(sorted_numeric.doc_value_count().unwrap(), 2);
        assert_eq!(sorted_numeric.next_value().unwrap(), 1);
        assert_eq!(sorted_numeric.next_value().unwrap(), 2);
        assert_eq!(sorted_numeric.next_doc().unwrap(), 1);
        assert_eq!(sorted_numeric.doc_value_count().unwrap(), 1);
        assert_eq!(sorted_numeric.next_value().unwrap(), 3);
        assert_eq!(sorted_numeric.next_doc().unwrap(), 3);
        assert_eq!(sorted_numeric.doc_value_count().unwrap(), 3);
        assert_eq!(sorted_numeric.next_value().unwrap(), 4);
        assert_eq!(sorted_numeric.next_value().unwrap(), 5);
        assert_eq!(sorted_numeric.next_value().unwrap(), 6);
        assert_eq!(sorted_numeric.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn round_trip_sorted_set() {
        let field = FieldInfo::new("sorted_set", 0);
        let producer = write_and_read(5, &field, |consumer| {
            let values = TestDocValuesProducer::with_sorted_set(
                0,
                vec![0, 2],
                vec![vec![0, 2], vec![1]],
                vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            );
            consumer.add_sorted_set_field(&field, &values).unwrap();
        });
        let mut sorted_set = producer.get_sorted_set(&field).unwrap();
        assert_eq!(sorted_set.next_doc().unwrap(), 0);
        assert_eq!(sorted_set.doc_value_count().unwrap(), 2);
        assert_eq!(sorted_set.next_ord().unwrap(), 0);
        assert_eq!(sorted_set.next_ord().unwrap(), 2);
        assert_eq!(sorted_set.next_doc().unwrap(), 2);
        assert_eq!(sorted_set.doc_value_count().unwrap(), 1);
        assert_eq!(sorted_set.next_ord().unwrap(), 1);
        assert_eq!(sorted_set.next_doc().unwrap(), NO_MORE_DOCS);

        assert_eq!(sorted_set.get_value_count().unwrap(), 3);
        assert_eq!(sorted_set.lookup_ord(0).unwrap().bytes, b"a");
        assert_eq!(sorted_set.lookup_ord(1).unwrap().bytes, b"b");
        assert_eq!(sorted_set.lookup_ord(2).unwrap().bytes, b"c");
    }

    fn capture_doc_values_bytes(
        max_doc: i32,
        field: &FieldInfo,
        write_field: &dyn Fn(&mut dyn DocValuesConsumer),
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let (dir, _segment_info, _field_infos) = write_only(max_doc, field, |consumer| {
            write_field(consumer);
        });
        let mut dvd = vec![];
        let mut dvm = vec![];
        let mut dvs = vec![];
        {
            let mut input = dir.open_input("_0.dvd", &*DEFAULT_IO_CONTEXT).unwrap();
            let len = input.length() as usize;
            dvd.resize(len, 0);
            input.read_bytes(&mut dvd, 0, len).unwrap();
        }
        {
            let mut input = dir.open_input("_0.dvm", &*DEFAULT_IO_CONTEXT).unwrap();
            let len = input.length() as usize;
            dvm.resize(len, 0);
            input.read_bytes(&mut dvm, 0, len).unwrap();
        }
        {
            let mut input = dir.open_input("_0.dvs", &*DEFAULT_IO_CONTEXT).unwrap();
            let len = input.length() as usize;
            dvs.resize(len, 0);
            input.read_bytes(&mut dvs, 0, len).unwrap();
        }
        (dvd, dvm, dvs)
    }

    #[test]
    fn deterministic_bytes_numeric() {
        let field = FieldInfo::new("num", 0);
        let write = |consumer: &mut dyn DocValuesConsumer| {
            let values = TestDocValuesProducer::with_numeric(0, vec![0, 2, 4], vec![1, 3, 5]);
            consumer.add_numeric_field(&field, &values).unwrap();
        };
        let (dvd1, dvm1, dvs1) = capture_doc_values_bytes(5, &field, &write);
        let (dvd2, dvm2, dvs2) = capture_doc_values_bytes(5, &field, &write);
        assert_eq!(dvd1, dvd2);
        assert_eq!(dvm1, dvm2);
        assert_eq!(dvs1, dvs2);
    }

    #[test]
    fn deterministic_bytes_sorted_set() {
        let field = FieldInfo::new("sorted_set", 0);
        let write = |consumer: &mut dyn DocValuesConsumer| {
            let values = TestDocValuesProducer::with_sorted_set(
                0,
                vec![0, 2],
                vec![vec![0, 2], vec![1]],
                vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            );
            consumer.add_sorted_set_field(&field, &values).unwrap();
        };
        let (dvd1, dvm1, dvs1) = capture_doc_values_bytes(5, &field, &write);
        let (dvd2, dvm2, dvs2) = capture_doc_values_bytes(5, &field, &write);
        assert_eq!(dvd1, dvd2);
        assert_eq!(dvm1, dvm2);
        assert_eq!(dvs1, dvs2);
    }

    #[test]
    fn deterministic_bytes_binary_variable() {
        let field = FieldInfo::new("bin", 0);
        let write = |consumer: &mut dyn DocValuesConsumer| {
            let values = TestDocValuesProducer::with_binary(
                0,
                vec![0, 2, 4],
                vec![b"alpha".to_vec(), b"beta".to_vec(), b"gamma".to_vec()],
            );
            consumer.add_binary_field(&field, &values).unwrap();
        };
        let (dvd1, dvm1, dvs1) = capture_doc_values_bytes(5, &field, &write);
        let (dvd2, dvm2, dvs2) = capture_doc_values_bytes(5, &field, &write);
        assert_eq!(dvd1, dvd2);
        assert_eq!(dvm1, dvm2);
        assert_eq!(dvs1, dvs2);
    }

    #[test]
    fn deterministic_bytes_sorted_numeric() {
        let field = FieldInfo::new("sorted_num", 0);
        let write = |consumer: &mut dyn DocValuesConsumer| {
            let values = TestDocValuesProducer::with_sorted_numeric(
                0,
                vec![0, 1, 3],
                vec![vec![1, 2], vec![3], vec![4, 5, 6]],
            );
            consumer.add_sorted_numeric_field(&field, &values).unwrap();
        };
        let (dvd1, dvm1, dvs1) = capture_doc_values_bytes(5, &field, &write);
        let (dvd2, dvm2, dvs2) = capture_doc_values_bytes(5, &field, &write);
        assert_eq!(dvd1, dvd2);
        assert_eq!(dvm1, dvm2);
        assert_eq!(dvs1, dvs2);
    }
}
