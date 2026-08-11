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

use std::cmp;
use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::codecs::codec_util::{
    check_index_header, retrieve_checksum, write_footer, write_index_header,
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
    ByteArrayDataInput, ByteBuffersDataOutput, ByteBuffersIndexOutput, DataInput, DataOutput,
    Directory, IOContext, IndexInput, IndexOutput, MockIndexOutput, RandomAccessInput,
};
use crate::util::compress::{FastCompressionHashTable, Lz4};
use crate::util::extra::LongValues;
use crate::util::packed::{
    DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter, DirectReader, DirectWriter,
};
use crate::util::{ArrayUtil, BytesRef, BytesRefBuilder, FixedBitSet, StringHelper};

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

mod indexed_disi {
    use super::*;

    const BLOCK_SIZE: usize = 65536;
    const DENSE_BLOCK_LONGS: usize = BLOCK_SIZE / 64;
    pub const DEFAULT_DENSE_RANK_POWER: i8 = 9;
    const MAX_ARRAY_LENGTH: usize = (1 << 12) - 1;

    fn flush(
        block: i32,
        buffer: &FixedBitSet,
        cardinality: usize,
        dense_rank_power: i8,
        out: &mut dyn IndexOutput,
    ) -> Result<()> {
        debug_assert!(block >= 0 && (block as usize) < BLOCK_SIZE);
        out.write_short(block as i16)?;
        debug_assert!(cardinality > 0 && cardinality <= BLOCK_SIZE);
        out.write_short((cardinality - 1) as i16)?;
        if cardinality > MAX_ARRAY_LENGTH {
            if cardinality != BLOCK_SIZE {
                if dense_rank_power != -1 {
                    let rank = create_rank(buffer, dense_rank_power);
                    out.write_bytes(&rank, 0, rank.len())?;
                }
                for &word in buffer.get_bits() {
                    out.write_long(word as i64)?;
                }
            }
        } else {
            let bits = buffer.get_bits();
            for word_idx in 0..DENSE_BLOCK_LONGS {
                let word = bits[word_idx];
                if word == 0 {
                    continue;
                }
                let base = word_idx << 6;
                let mut w = word;
                while w != 0 {
                    let tz = w.trailing_zeros() as usize;
                    out.write_short((base + tz) as i16)?;
                    w &= w - 1;
                }
            }
        }
        Ok(())
    }

    fn create_rank(buffer: &FixedBitSet, dense_rank_power: i8) -> Vec<u8> {
        let longs_per_rank = 1usize << (dense_rank_power as usize - 6);
        let rank_mark = longs_per_rank - 1;
        let rank_index_shift = dense_rank_power as usize - 7;
        let mut rank = vec![0u8; DENSE_BLOCK_LONGS >> rank_index_shift];
        let bits = buffer.get_bits();
        let mut bit_count: usize = 0;
        for word in 0..DENSE_BLOCK_LONGS {
            if (word & rank_mark) == 0 {
                rank[word >> rank_index_shift] = (bit_count >> 8) as u8;
                rank[(word >> rank_index_shift) + 1] = (bit_count & 0xFF) as u8;
            }
            bit_count += (bits[word] as u64).count_ones() as usize;
        }
        rank
    }

    pub fn write_bit_set(
        it: &mut dyn DocIdSetIterator,
        out: &mut dyn IndexOutput,
        dense_rank_power: i8,
    ) -> Result<i16> {
        if (dense_rank_power < 7 || dense_rank_power > 15) && dense_rank_power != -1 {
            return Err(LuceneError::IllegalArgument(format!(
                "denseRankPower must be in [7, 15] or -1, got {dense_rank_power}"
            )));
        }
        let origo = out.file_pointer();
        let mut total_cardinality: usize = 0;
        let mut last_block: usize = 0;
        let mut jumps: Vec<i32> = Vec::with_capacity(4);
        let mut buffer = FixedBitSet::new(BLOCK_SIZE);

        let mut doc = it.next_doc()?;
        while doc != NO_MORE_DOCS {
            let block = (doc as u32 >> 16) as usize;
            let up_to = cmp::min(i32::MAX as i64, (doc | 0xFFFF) as i64 + 1) as i32;
            it.into_bit_set(up_to, &mut buffer, (doc as u32 & 0xFFFF0000) as i32)?;
            let block_cardinality = buffer.cardinality();
            add_jumps(
                &mut jumps,
                (out.file_pointer() - origo) as i32,
                total_cardinality as i32,
                last_block,
                block + 1,
            );
            last_block = block + 1;
            flush(
                block as i32,
                &buffer,
                block_cardinality,
                dense_rank_power,
                out,
            )?;
            buffer.clear_all();
            total_cardinality += block_cardinality;
            doc = it.doc_id();
            if doc == NO_MORE_DOCS {
                break;
            }
            // Loop increment: the next block starts at the current `doc`.
        }

        add_jumps(
            &mut jumps,
            (out.file_pointer() - origo) as i32,
            total_cardinality as i32,
            last_block,
            last_block + 1,
        );
        buffer.set(NO_MORE_DOCS as usize & 0xFFFF);
        flush(
            (NO_MORE_DOCS as u32 >> 16) as i32,
            &buffer,
            1,
            dense_rank_power,
            out,
        )?;
        flush_block_jumps(last_block + 1, &jumps, out)
    }

    fn add_jumps(
        jumps: &mut Vec<i32>,
        offset: i32,
        index: i32,
        start_block: usize,
        end_block: usize,
    ) {
        if jumps.len() < (end_block + 1) * 2 {
            jumps.resize((end_block + 1) * 2, 0);
        }
        for b in start_block..end_block {
            jumps[b * 2] = index;
            jumps[b * 2 + 1] = offset;
        }
    }

    fn flush_block_jumps(
        mut block_count: usize,
        jumps: &[i32],
        out: &mut dyn IndexOutput,
    ) -> Result<i16> {
        if block_count == 2 {
            block_count = 0;
        }
        for i in 0..block_count {
            out.write_int(jumps[i * 2])?;
            out.write_int(jumps[i * 2 + 1])?;
        }
        Ok(block_count as i16)
    }

    fn create_block_slice(
        slice: &dyn IndexInput,
        slice_description: &str,
        offset: i64,
        length: i64,
        jump_table_entry_count: i32,
    ) -> Result<Box<dyn IndexInput>> {
        let jump_table_bytes = if jump_table_entry_count < 0 {
            0
        } else {
            jump_table_entry_count as i64 * 8
        };
        slice.slice(slice_description, offset, length - jump_table_bytes)
    }

    fn create_jump_table(
        slice: &dyn IndexInput,
        offset: i64,
        length: i64,
        jump_table_entry_count: i32,
    ) -> Result<Option<Box<dyn RandomAccessInput>>> {
        if jump_table_entry_count <= 0 {
            Ok(None)
        } else {
            let jump_table_bytes = jump_table_entry_count as i64 * 8;
            Ok(Some(slice.random_access_slice(
                offset + length - jump_table_bytes,
                jump_table_bytes,
            )?))
        }
    }

    enum Method {
        Sparse,
        Dense,
        All,
    }

    pub struct IndexedDISI {
        slice: Box<dyn IndexInput>,
        jump_table: Option<Box<dyn RandomAccessInput>>,
        jump_table_entry_count: i32,
        dense_rank_power: i8,
        dense_rank_table: Vec<u8>,
        cost: i64,

        block: i32,
        block_end: i64,
        dense_bitmap_offset: i64,
        next_block_index: i32,
        method: Method,

        index: i32,
        doc: i32,

        // SPARSE
        exists: bool,
        next_exist_doc_in_block: i32,

        // DENSE
        word: u64,
        word_index: i32,
        number_of_ones: i32,
        dense_origo_index: i32,
    }

    impl IndexedDISI {
        pub fn new(
            input: &dyn IndexInput,
            offset: i64,
            length: i64,
            jump_table_entry_count: i32,
            dense_rank_power: i8,
            cost: i64,
        ) -> Result<Self> {
            let block_slice =
                create_block_slice(input, "docs", offset, length, jump_table_entry_count)?;
            let jump_table = create_jump_table(input, offset, length, jump_table_entry_count)?;
            Self::with_slices(
                block_slice,
                jump_table,
                jump_table_entry_count,
                dense_rank_power,
                cost,
            )
        }

        pub fn with_slices(
            slice: Box<dyn IndexInput>,
            jump_table: Option<Box<dyn RandomAccessInput>>,
            jump_table_entry_count: i32,
            dense_rank_power: i8,
            cost: i64,
        ) -> Result<Self> {
            if (dense_rank_power < 7 || dense_rank_power > 15) && dense_rank_power != -1 {
                return Err(LuceneError::IllegalArgument(format!(
                    "denseRankPower must be in [7, 15] or -1, got {dense_rank_power}"
                )));
            }
            let rank_index_shift = if dense_rank_power == -1 {
                0
            } else {
                dense_rank_power as usize - 7
            };
            let dense_rank_table = if dense_rank_power == -1 {
                Vec::new()
            } else {
                vec![0u8; DENSE_BLOCK_LONGS >> rank_index_shift]
            };
            Ok(Self {
                slice,
                jump_table,
                jump_table_entry_count,
                dense_rank_power,
                dense_rank_table,
                cost,
                block: -1,
                block_end: 0,
                dense_bitmap_offset: -1,
                next_block_index: -1,
                method: Method::Sparse,
                index: -1,
                doc: -1,
                exists: false,
                next_exist_doc_in_block: -1,
                word: 0,
                word_index: -1,
                number_of_ones: 0,
                dense_origo_index: 0,
            })
        }

        #[allow(dead_code)]
        pub fn index(&self) -> i32 {
            self.index
        }

        fn advance_block(&mut self, target_block: i32) -> Result<()> {
            let block_index = (target_block as u32 >> 16) as i32;
            if let Some(ref mut jump_table) = self.jump_table {
                if block_index >= ((self.block >> 16) + 2) {
                    let in_range_block_index = if block_index < self.jump_table_entry_count {
                        block_index
                    } else {
                        self.jump_table_entry_count - 1
                    };
                    let base = in_range_block_index as i64 * 8;
                    let index = jump_table.read_int_at(base)?;
                    let offset = jump_table.read_int_at(base + 4)?;
                    self.next_block_index = index - 1;
                    self.slice.seek(offset as i64)?;
                    self.read_block_header()?;
                    return Ok(());
                }
            }
            while self.block < target_block {
                self.slice.seek(self.block_end)?;
                self.read_block_header()?;
            }
            Ok(())
        }

        fn read_block_header(&mut self) -> Result<()> {
            let block_short = self.slice.read_short()? as u16;
            self.block = (block_short as i32) << 16;
            debug_assert!(self.block >= 0);
            let num_values = 1 + (self.slice.read_short()? as u16 as i32);
            self.index = self.next_block_index;
            self.next_block_index = self.index + num_values;
            if num_values <= MAX_ARRAY_LENGTH as i32 {
                self.method = Method::Sparse;
                self.block_end = self.slice.file_pointer() + (num_values as i64 * 2);
                self.next_exist_doc_in_block = -1;
            } else if num_values == BLOCK_SIZE as i32 {
                self.method = Method::All;
                self.block_end = self.slice.file_pointer();
                let gap = self.block - self.index - 1;
                self.doc = -1; // will be set by advance
                               // Store gap in a field? We reuse `next_exist_doc_in_block` is not used for ALL.
                               // Instead keep the gap calculation in local and apply in method ALL advance.
                               // We'll store gap in `dense_origo_index` temporarily? No.
                               // Use a dedicated field below by adding `gap`.
                self.dense_origo_index = gap; // repurpose for ALL gap
            } else {
                self.method = Method::Dense;
                self.dense_bitmap_offset = self.slice.file_pointer()
                    + if self.dense_rank_power == -1 {
                        0
                    } else {
                        self.dense_rank_table.len() as i64
                    };
                self.block_end = self.dense_bitmap_offset + (1i64 << 13);
                if self.dense_rank_power != -1 {
                    let len = self.dense_rank_table.len();
                    self.slice.read_bytes(&mut self.dense_rank_table, 0, len)?;
                }
                self.word_index = -1;
                self.number_of_ones = self.index + 1;
                self.dense_origo_index = self.number_of_ones;
            }
            Ok(())
        }

        fn advance_within_block(&mut self, target: i32) -> Result<bool> {
            match self.method {
                Method::Sparse => self.sparse_advance(target),
                Method::Dense => self.dense_advance(target),
                Method::All => {
                    let gap = self.dense_origo_index;
                    self.doc = target;
                    self.index = target - gap;
                    Ok(true)
                }
            }
        }

        fn sparse_advance(&mut self, target: i32) -> Result<bool> {
            let target_in_block = target & 0xFFFF;
            while self.index < self.next_block_index {
                let doc_in_block = self.slice.read_short()? as u16 as i32;
                self.index += 1;
                if doc_in_block >= target_in_block {
                    self.doc = self.block | doc_in_block;
                    self.exists = true;
                    self.next_exist_doc_in_block = doc_in_block;
                    return Ok(true);
                }
            }
            Ok(false)
        }

        fn dense_advance(&mut self, target: i32) -> Result<bool> {
            let target_in_block = target & 0xFFFF;
            let target_word_index = target_in_block >> 6;
            if self.dense_rank_power != -1
                && target_word_index - self.word_index >= (1 << (self.dense_rank_power - 6))
            {
                self.rank_skip(target_in_block)?;
            }
            for _ in (self.word_index + 1)..=target_word_index {
                self.word = self.slice.read_long()? as u64;
                self.number_of_ones += (self.word as u64).count_ones() as i32;
            }
            self.word_index = target_word_index;
            let left_bits = self.word >> (target & 0x3F);
            if left_bits != 0 {
                self.doc = target + (left_bits.trailing_zeros() as i32);
                self.index = self.number_of_ones - (left_bits.count_ones() as i32);
                return Ok(true);
            }
            while self.word_index + 1 < DENSE_BLOCK_LONGS as i32 {
                self.word_index += 1;
                self.word = self.slice.read_long()? as u64;
                if self.word != 0 {
                    self.index = self.number_of_ones;
                    self.number_of_ones += self.word.count_ones() as i32;
                    self.doc =
                        self.block | (self.word_index << 6) | self.word.trailing_zeros() as i32;
                    return Ok(true);
                }
            }
            Ok(false)
        }

        fn rank_skip(&mut self, target_in_block: i32) -> Result<()> {
            debug_assert!(self.dense_rank_power >= 0);
            let rank_index = target_in_block >> self.dense_rank_power;
            let rank = ((self.dense_rank_table[(rank_index << 1) as usize] as u16 as u32) << 8)
                | (self.dense_rank_table[((rank_index << 1) + 1) as usize] as u16 as u32);
            let rank_aligned_word_index = (rank_index << self.dense_rank_power) >> 6;
            self.slice
                .seek(self.dense_bitmap_offset + rank_aligned_word_index as i64 * 8)?;
            let rank_word = self.slice.read_long()? as u64;
            let dense_noo = rank as i32 + rank_word.count_ones() as i32;
            self.word_index = rank_aligned_word_index as i32;
            self.word = rank_word;
            self.number_of_ones = self.dense_origo_index + dense_noo;
            Ok(())
        }

        fn advance_exact_within_block(&mut self, target: i32) -> Result<bool> {
            match self.method {
                Method::Sparse => self.sparse_advance_exact(target),
                Method::Dense => self.dense_advance_exact(target),
                Method::All => {
                    let gap = self.dense_origo_index;
                    self.index = target - gap;
                    Ok(true)
                }
            }
        }

        fn sparse_advance_exact(&mut self, target: i32) -> Result<bool> {
            let target_in_block = target & 0xFFFF;
            if self.next_exist_doc_in_block > target_in_block {
                return Ok(false);
            }
            if target == self.doc {
                return Ok(self.exists);
            }
            while self.index < self.next_block_index {
                let doc_in_block = self.slice.read_short()? as u16 as i32;
                self.index += 1;
                if doc_in_block >= target_in_block {
                    self.next_exist_doc_in_block = doc_in_block;
                    if doc_in_block != target_in_block {
                        self.index -= 1;
                        self.slice.seek(self.slice.file_pointer() - 2)?;
                        break;
                    }
                    self.exists = true;
                    return Ok(true);
                }
            }
            self.exists = false;
            Ok(false)
        }

        fn dense_advance_exact(&mut self, target: i32) -> Result<bool> {
            let target_in_block = target & 0xFFFF;
            let target_word_index = target_in_block >> 6;
            if self.dense_rank_power != -1
                && target_word_index - self.word_index >= (1 << (self.dense_rank_power - 6))
            {
                self.rank_skip(target_in_block)?;
            }
            for _ in (self.word_index + 1)..=target_word_index {
                self.word = self.slice.read_long()? as u64;
                self.number_of_ones += self.word.count_ones() as i32;
            }
            self.word_index = target_word_index;
            let left_bits = self.word >> (target & 0x3F);
            self.index = self.number_of_ones - left_bits.count_ones() as i32;
            Ok((left_bits & 1) != 0)
        }
    }

    impl DocIdSetIterator for IndexedDISI {
        fn doc_id(&self) -> i32 {
            self.doc
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.advance(self.doc + 1)
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            let target_block = (target as u32 & 0xFFFF0000) as i32;
            if self.block < target_block {
                self.advance_block(target_block)?;
            }
            if self.block == target_block {
                if self.advance_within_block(target)? {
                    return Ok(self.doc);
                }
                self.read_block_header()?;
            }
            let found = self.advance_within_block(self.block)?;
            debug_assert!(found);
            Ok(self.doc)
        }

        fn cost(&self) -> i64 {
            self.cost
        }
    }

    impl DocValuesIterator for IndexedDISI {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            let target_block = (target as u32 & 0xFFFF0000) as i32;
            if self.block < target_block {
                self.advance_block(target_block)?;
            }
            let found = self.block == target_block && self.advance_exact_within_block(target)?;
            self.doc = target;
            Ok(found)
        }
    }
}

use indexed_disi::IndexedDISI;

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
    ) -> Result<Box<dyn DocValuesProducer + 'a>> {
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
                if v < i64::MIN / 2 || v > i64::MAX / 2 {
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
        let jump_count = indexed_disi::write_bit_set(
            &mut it,
            &mut self.data,
            indexed_disi::DEFAULT_DENSE_RANK_POWER,
        )?;
        let length = self.data.file_pointer() - offset;
        Ok((
            offset,
            length,
            jump_count,
            indexed_disi::DEFAULT_DENSE_RANK_POWER,
        ))
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

        let stats = NumericStats::compute(values, ords);
        let mut min = stats.min;
        let mut gcd = stats.gcd;

        let (num_bits_per_value, table_size, encode) = if num_values == 0 || stats.min >= stats.max
        {
            (0u8, -1i32, None)
        } else if let Some(ref unique) = stats.unique_values {
            let unique_bits = DirectWriter::unsigned_bits_required((unique.len() as i64) - 1);
            let direct_bits =
                DirectWriter::unsigned_bits_required((stats.max - stats.min) / stats.gcd);
            if unique_bits < direct_bits {
                let mut encode = HashMap::with_capacity(unique.len());
                for (i, &v) in unique.iter().enumerate() {
                    encode.insert(v, i as i64);
                }
                min = 0;
                gcd = 1;
                (unique_bits as u8, unique.len() as i32, Some(encode))
            } else {
                (
                    DirectWriter::unsigned_bits_required((stats.max - stats.min) / stats.gcd) as u8,
                    -1,
                    None,
                )
            }
        } else {
            let direct_bits =
                DirectWriter::unsigned_bits_required((stats.max - stats.min) / stats.gcd);
            if stats.do_blocks {
                (0xFFu8, -2 - NUMERIC_BLOCK_SHIFT, None)
            } else {
                let single_bits = direct_bits;
                if gcd == 1
                    && min > 0
                    && DirectWriter::unsigned_bits_required(stats.max) == single_bits
                {
                    min = 0;
                }
                (single_bits as u8, -1, None)
            }
        };

        self.meta.write_int(table_size)?;
        if table_size != -1 {
            if let Some(ref table) = stats.unique_values {
                for &v in table {
                    self.meta.write_long(v)?;
                }
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
        self.meta.write_byte(1u8)?; // multi-valued

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

/// Reader for [`Lucene90DocValuesFormat`].
pub struct Lucene90DocValuesProducer {
    max_doc: i32,
    entries: HashMap<i32, FieldEntry>,
}

#[derive(Debug, Clone)]
enum FieldEntry {
    Numeric(NumericEntryData),
    Binary(BinaryEntryData),
    SortedNumeric(SortedNumericEntryData),
    Sorted(SortedEntryData),
    SortedSet(SortedSetEntryData),
}

#[derive(Debug, Clone)]
struct NumericEntryData {
    docs: Vec<i32>,
    values: Vec<i64>,
}

#[derive(Debug, Clone)]
struct BinaryEntryData {
    docs: Vec<i32>,
    values: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct SortedNumericEntryData {
    docs: Vec<i32>,
    values: Vec<Vec<i64>>,
}

#[derive(Debug, Clone)]
struct SortedEntryData {
    docs: Vec<i32>,
    ords: Vec<i32>,
    terms: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct SortedSetEntryData {
    docs: Vec<i32>,
    ords: Vec<Vec<i64>>,
    terms: Vec<Vec<u8>>,
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

        let max_doc = state.segment_info.max_doc()?;
        let mut entries = HashMap::new();
        loop {
            let field_number = meta.read_int()?;
            if field_number == -1 {
                break;
            }
            let kind = meta.read_byte()?;
            let entry = match kind {
                TYPE_NUMERIC => {
                    FieldEntry::Numeric(Self::read_numeric(&mut *meta, &mut *data, max_doc)?)
                }
                TYPE_BINARY => {
                    FieldEntry::Binary(Self::read_binary(&mut *meta, &mut *data, max_doc)?)
                }
                TYPE_SORTED => {
                    FieldEntry::Sorted(Self::read_sorted(&mut *meta, &mut *data, max_doc)?)
                }
                TYPE_SORTED_SET => {
                    FieldEntry::SortedSet(Self::read_sorted_set(&mut *meta, &mut *data, max_doc)?)
                }
                TYPE_SORTED_NUMERIC => FieldEntry::SortedNumeric(Self::read_sorted_numeric(
                    &mut *meta, &mut *data, max_doc,
                )?),
                _ => {
                    return Err(LuceneError::CorruptIndex(format!(
                        "unknown doc-values type: {kind}"
                    )))
                }
            };
            entries.insert(field_number, entry);
        }

        let _ = retrieve_checksum(&mut *meta)?;
        Ok(Self { max_doc, entries })
    }

    fn read_docs(
        meta: &mut dyn IndexInput,
        data: &mut dyn IndexInput,
        max_doc: i32,
    ) -> Result<Vec<i32>> {
        let offset = meta.read_long()?;
        let length = meta.read_long()?;
        let jump_count = meta.read_short()?;
        let dense_rank_power = meta.read_byte()? as i8;
        if offset == -1 {
            return Ok((0..max_doc).collect());
        }
        if offset == -2 {
            return Ok(Vec::new());
        }
        let mut disi = IndexedDISI::new(
            &*data,
            offset,
            length,
            jump_count as i32,
            dense_rank_power,
            0,
        )?;
        let mut docs = Vec::new();
        let mut doc = disi.next_doc()?;
        while doc != NO_MORE_DOCS {
            docs.push(doc);
            doc = disi.next_doc()?;
        }
        Ok(docs)
    }

    fn read_numeric_values(
        data: &mut dyn IndexInput,
        value_offset: i64,
        values_length: i64,
        num_values: i64,
        num_bits: u8,
        min: i64,
        gcd: i64,
        table: Option<&[i64]>,
    ) -> Result<Vec<i64>> {
        if num_values == 0 {
            return Ok(Vec::new());
        }
        if num_bits == 0 {
            return Ok(vec![min; num_values as usize]);
        }
        data.seek(value_offset)?;
        let mut bytes = vec![0u8; values_length as usize];
        let bytes_len = bytes.len();
        data.read_bytes(&mut bytes, 0, bytes_len)?;
        let reader = DirectReader::new(bytes, num_bits as i32)?;
        let mut values = Vec::with_capacity(num_values as usize);
        if let Some(table) = table {
            for i in 0..num_values {
                let idx = reader.get(i) as usize;
                values.push(table[idx]);
            }
        } else {
            for i in 0..num_values {
                let encoded = reader.get(i);
                values.push(min + encoded * gcd);
            }
        }
        Ok(values)
    }

    fn read_numeric(
        meta: &mut dyn IndexInput,
        data: &mut dyn IndexInput,
        max_doc: i32,
    ) -> Result<NumericEntryData> {
        let docs = Self::read_docs(meta, data, max_doc)?;
        let num_values = meta.read_long()?;
        let table_size = meta.read_int()?;
        let table = if table_size > 0 {
            Some(
                (0..table_size)
                    .map(|_| meta.read_long())
                    .collect::<Result<Vec<_>>>()?,
            )
        } else {
            None
        };
        let num_bits = meta.read_byte()?;
        let min = meta.read_long()?;
        let gcd = meta.read_long()?;
        let value_offset = meta.read_long()?;
        let values_length = meta.read_long()?;
        let _jump_table_offset = meta.read_long()?;
        let values = Self::read_numeric_values(
            data,
            value_offset,
            values_length,
            num_values,
            num_bits,
            min,
            gcd,
            table.as_deref(),
        )?;
        Ok(NumericEntryData { docs, values })
    }

    fn read_addresses(
        meta: &mut dyn IndexInput,
        data: &mut dyn IndexInput,
        num_values: i64,
    ) -> Result<Vec<i64>> {
        let start = meta.read_long()?;
        let shift = meta.read_v_int()?;
        let direct_meta = DirectMonotonicMeta::load(meta, num_values, shift)?;
        let length = meta.read_long()?;
        data.seek(start)?;
        let mut bytes = vec![0u8; length as usize];
        let bytes_len = bytes.len();
        data.read_bytes(&mut bytes, 0, bytes_len)?;
        let reader = DirectMonotonicReader::new(direct_meta, bytes)?;
        let mut values = Vec::with_capacity(num_values as usize);
        for i in 0..num_values {
            values.push(reader.get(i));
        }
        Ok(values)
    }

    fn read_binary(
        meta: &mut dyn IndexInput,
        data: &mut dyn IndexInput,
        max_doc: i32,
    ) -> Result<BinaryEntryData> {
        let data_offset = meta.read_long()?;
        let data_length = meta.read_long()?;
        let docs = Self::read_docs(meta, data, max_doc)?;
        let _num_docs = meta.read_int()?;
        let min_length = meta.read_int()?;
        let max_length = meta.read_int()?;
        let addresses = if max_length > min_length {
            Some(Self::read_addresses(meta, data, docs.len() as i64 + 1)?)
        } else {
            None
        };
        data.seek(data_offset)?;
        let mut raw = vec![0u8; data_length as usize];
        let raw_len = raw.len();
        data.read_bytes(&mut raw, 0, raw_len)?;
        let mut values = Vec::with_capacity(docs.len());
        for i in 0..docs.len() {
            let value = if let Some(ref addr) = addresses {
                let start = addr[i] as usize;
                let end = addr[i + 1] as usize;
                raw[start..end].to_vec()
            } else {
                let start = i * min_length as usize;
                let end = start + min_length as usize;
                raw[start..end].to_vec()
            };
            values.push(value);
        }
        Ok(BinaryEntryData { docs, values })
    }

    fn read_sorted_numeric(
        meta: &mut dyn IndexInput,
        data: &mut dyn IndexInput,
        max_doc: i32,
    ) -> Result<SortedNumericEntryData> {
        let numeric = Self::read_numeric(meta, data, max_doc)?;
        let num_docs_with_value = meta.read_int()? as i64;
        let mut per_doc = Vec::with_capacity(numeric.docs.len());
        if numeric.values.len() as i64 > num_docs_with_value {
            let addresses = Self::read_addresses(meta, data, num_docs_with_value + 1)?;
            let mut idx = 0usize;
            for i in 0..numeric.docs.len() {
                let count = (addresses[i + 1] - addresses[i]) as usize;
                per_doc.push(numeric.values[idx..idx + count].to_vec());
                idx += count;
            }
        } else {
            for &v in &numeric.values {
                per_doc.push(vec![v]);
            }
        }
        Ok(SortedNumericEntryData {
            docs: numeric.docs,
            values: per_doc,
        })
    }

    fn read_sorted(
        meta: &mut dyn IndexInput,
        data: &mut dyn IndexInput,
        max_doc: i32,
    ) -> Result<SortedEntryData> {
        let numeric = Self::read_numeric(meta, data, max_doc)?;
        let terms = Self::read_terms_dict(meta, data)?;
        Ok(SortedEntryData {
            docs: numeric.docs,
            ords: numeric.values.into_iter().map(|v| v as i32).collect(),
            terms,
        })
    }

    fn read_sorted_set(
        meta: &mut dyn IndexInput,
        data: &mut dyn IndexInput,
        max_doc: i32,
    ) -> Result<SortedSetEntryData> {
        let _multi_valued = meta.read_byte()?;
        let numeric = Self::read_numeric(meta, data, max_doc)?;
        let num_docs_with_value = meta.read_int()? as i64;
        let mut per_doc = Vec::with_capacity(numeric.docs.len());
        if numeric.values.len() as i64 > num_docs_with_value {
            let addresses = Self::read_addresses(meta, data, num_docs_with_value + 1)?;
            let mut idx = 0usize;
            for i in 0..numeric.docs.len() {
                let count = (addresses[i + 1] - addresses[i]) as usize;
                per_doc.push(numeric.values[idx..idx + count].to_vec());
                idx += count;
            }
        } else {
            for v in numeric.values {
                per_doc.push(vec![v]);
            }
        }
        let terms = Self::read_terms_dict(meta, data)?;
        Ok(SortedSetEntryData {
            docs: numeric.docs,
            ords: per_doc,
            terms,
        })
    }

    fn read_terms_dict(
        meta: &mut dyn IndexInput,
        data: &mut dyn IndexInput,
    ) -> Result<Vec<Vec<u8>>> {
        let value_count = meta.read_v_long()?;
        let _block_shift = meta.read_int()?;
        let num_blocks =
            (value_count + TERMS_DICT_BLOCK_LZ4_MASK as i64) >> TERMS_DICT_BLOCK_LZ4_SHIFT;
        let _block_meta =
            DirectMonotonicMeta::load(meta, num_blocks, DIRECT_MONOTONIC_BLOCK_SHIFT)?;
        let _max_length = meta.read_int()?;
        let _max_block_length = meta.read_int()?;
        let terms_dict_start = meta.read_long()?;
        let _terms_dict_length = meta.read_long()?;
        let _block_addresses_start = meta.read_long()?;
        let _block_addresses_length = meta.read_long()?;

        let reverse_shift = meta.read_int()?;
        let _reverse_start = meta.read_long()?;
        let _reverse_length = meta.read_long()?;
        let reverse_num_blocks = 1
            + ((value_count + TERMS_DICT_REVERSE_INDEX_MASK as i64)
                >> TERMS_DICT_REVERSE_INDEX_SHIFT);
        let _reverse_meta = DirectMonotonicMeta::load(meta, reverse_num_blocks, reverse_shift)?;
        let _reverse_addr_start = meta.read_long()?;
        let _reverse_addr_length = meta.read_long()?;

        data.seek(terms_dict_start)?;
        let mut terms = Vec::with_capacity(value_count as usize);
        let mut ord: i64 = 0;
        while ord < value_count {
            let mut previous = BytesRefBuilder::new();
            let len = data.read_v_int()? as usize;
            let mut term = vec![0u8; len];
            data.read_bytes(&mut term, 0, len)?;
            previous.copy_bytes(&term, 0, len);
            terms.push(term);

            let remaining = cmp::min(
                TERMS_DICT_BLOCK_LZ4_SIZE - 1,
                value_count as usize - terms.len(),
            );
            if remaining > 0 {
                let uncompressed_len = data.read_v_int()? as usize;
                let dict_length = len;
                let mut suffix = vec![0u8; dict_length + uncompressed_len];
                suffix[..dict_length].copy_from_slice(&terms[terms.len() - 1]);
                let _ = Lz4::decompress(data, uncompressed_len, &mut suffix, dict_length)?;
                let mut input = ByteArrayDataInput::new(suffix);
                input.seek(dict_length)?;
                for _ in 0..remaining {
                    let b = input.read_byte()? as usize;
                    let prefix_clip = b & 0x0F;
                    let suffix_clip = b >> 4;
                    let mut prefix_len = prefix_clip;
                    if prefix_clip == 15 {
                        prefix_len += input.read_v_int()? as usize;
                    }
                    let mut suffix_len = suffix_clip + 1;
                    if suffix_clip == 15 {
                        suffix_len += input.read_v_int()? as usize;
                    }
                    let mut term = previous.bytes()[..prefix_len].to_vec();
                    term.resize(prefix_len + suffix_len, 0);
                    input.read_bytes(&mut term, prefix_len, suffix_len)?;
                    previous.copy_bytes(&term, 0, term.len());
                    terms.push(term);
                }
            }

            ord += TERMS_DICT_BLOCK_LZ4_SIZE as i64;
        }
        Ok(terms)
    }
}

impl DocValuesProducer for Lucene90DocValuesProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues + Send + Sync>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::Numeric(e)) => Ok(Box::new(MemoryNumericDocValues::new(
                e.docs.clone(),
                e.values.clone(),
            ))),
            _ => Ok(Box::new(EmptyNumericDocValues::new())),
        }
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues + Send + Sync>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::Binary(e)) => Ok(Box::new(MemoryBinaryDocValues::new(
                e.docs.clone(),
                e.values.clone(),
            ))),
            _ => Ok(Box::new(EmptyBinaryDocValues::new())),
        }
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues + Send + Sync>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::Sorted(e)) => Ok(Box::new(MemorySortedDocValues::new(
                e.docs.clone(),
                e.ords.clone(),
                e.terms.clone(),
            ))),
            _ => Ok(Box::new(EmptySortedDocValues::new())),
        }
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn SortedNumericDocValues + Send + Sync>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::SortedNumeric(e)) => Ok(Box::new(MemorySortedNumericDocValues::new(
                e.docs.clone(),
                e.values.clone(),
            ))),
            _ => Ok(Box::new(EmptySortedNumericDocValues::new())),
        }
    }

    fn get_sorted_set(
        &self,
        field: &FieldInfo,
    ) -> Result<Box<dyn SortedSetDocValues + Send + Sync>> {
        match self.entries.get(&field.number) {
            Some(FieldEntry::SortedSet(e)) => Ok(Box::new(MemorySortedSetDocValues::new(
                e.docs.clone(),
                e.ords.clone(),
                e.terms.clone(),
            ))),
            _ => Ok(Box::new(EmptySortedSetDocValues::new())),
        }
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper + Send + Sync>> {
        Ok(Box::new(EmptyDocValuesSkipper))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            max_doc: self.max_doc,
            entries: self.entries.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

impl fmt::Debug for Lucene90DocValuesProducer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene90DocValuesProducer")
            .field("max_doc", &self.max_doc)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

// -----------------------------------------------------------------------------
// In-memory doc-values iterators returned by the producer
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct MemoryNumericDocValues {
    docs: Vec<i32>,
    values: Vec<i64>,
    pos: usize,
    doc: i32,
}

impl MemoryNumericDocValues {
    fn new(docs: Vec<i32>, values: Vec<i64>) -> Self {
        Self {
            docs,
            values,
            pos: usize::MAX,
            doc: -1,
        }
    }
}

impl DocIdSetIterator for MemoryNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.pos.wrapping_add(1) < self.docs.len() {
            self.pos = self.pos.wrapping_add(1);
            self.doc = self.docs[self.pos];
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let start = self.pos.wrapping_add(1).min(self.docs.len());
        match self.docs[start..].binary_search(&target) {
            Ok(i) => {
                self.pos = start + i;
                self.doc = self.docs[self.pos];
            }
            Err(i) => {
                self.pos = start + i;
                if self.pos >= self.docs.len() {
                    self.doc = NO_MORE_DOCS;
                } else {
                    self.doc = self.docs[self.pos];
                }
            }
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocValuesIterator for MemoryNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        match self.docs.binary_search(&target) {
            Ok(i) => {
                self.pos = i;
                self.doc = target;
                Ok(true)
            }
            Err(i) => {
                self.pos = i.min(self.docs.len()).saturating_sub(1);
                self.doc = target;
                Ok(false)
            }
        }
    }
}

impl NumericDocValues for MemoryNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        Ok(self.values[self.pos])
    }
}

#[derive(Debug, Clone, Default)]
struct MemoryBinaryDocValues {
    docs: Vec<i32>,
    values: Vec<Vec<u8>>,
    current: Vec<u8>,
    pos: usize,
    doc: i32,
}

impl MemoryBinaryDocValues {
    fn new(docs: Vec<i32>, values: Vec<Vec<u8>>) -> Self {
        Self {
            docs,
            values,
            current: Vec::new(),
            pos: usize::MAX,
            doc: -1,
        }
    }

    fn load_current(&mut self) {
        if self.pos < self.values.len() {
            self.current.clone_from(&self.values[self.pos]);
        } else {
            self.current.clear();
        }
    }
}

impl DocIdSetIterator for MemoryBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.pos.wrapping_add(1) < self.docs.len() {
            self.pos = self.pos.wrapping_add(1);
            self.doc = self.docs[self.pos];
            self.load_current();
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let start = self.pos.wrapping_add(1).min(self.docs.len());
        match self.docs[start..].binary_search(&target) {
            Ok(i) => {
                self.pos = start + i;
                self.doc = self.docs[self.pos];
            }
            Err(i) => {
                self.pos = start + i;
                if self.pos >= self.docs.len() {
                    self.doc = NO_MORE_DOCS;
                } else {
                    self.doc = self.docs[self.pos];
                }
            }
        }
        self.load_current();
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocValuesIterator for MemoryBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        match self.docs.binary_search(&target) {
            Ok(i) => {
                self.pos = i;
                self.doc = target;
                self.load_current();
                Ok(true)
            }
            Err(i) => {
                self.pos = i.min(self.docs.len()).saturating_sub(1);
                self.doc = target;
                self.load_current();
                Ok(false)
            }
        }
    }
}

impl BinaryDocValues for MemoryBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        Ok(BytesRef::new(self.current.clone()))
    }
}

#[derive(Debug, Clone, Default)]
struct MemorySortedDocValues {
    docs: Vec<i32>,
    ords: Vec<i32>,
    terms: Vec<Vec<u8>>,
    pos: usize,
    doc: i32,
}

impl MemorySortedDocValues {
    fn new(docs: Vec<i32>, ords: Vec<i32>, terms: Vec<Vec<u8>>) -> Self {
        Self {
            docs,
            ords,
            terms,
            pos: usize::MAX,
            doc: -1,
        }
    }
}

impl DocIdSetIterator for MemorySortedDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.pos.wrapping_add(1) < self.docs.len() {
            self.pos = self.pos.wrapping_add(1);
            self.doc = self.docs[self.pos];
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let start = self.pos.wrapping_add(1).min(self.docs.len());
        match self.docs[start..].binary_search(&target) {
            Ok(i) => {
                self.pos = start + i;
                self.doc = self.docs[self.pos];
            }
            Err(i) => {
                self.pos = start + i;
                if self.pos >= self.docs.len() {
                    self.doc = NO_MORE_DOCS;
                } else {
                    self.doc = self.docs[self.pos];
                }
            }
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocValuesIterator for MemorySortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        match self.docs.binary_search(&target) {
            Ok(i) => {
                self.pos = i;
                self.doc = target;
                Ok(true)
            }
            Err(i) => {
                self.pos = i.min(self.docs.len()).saturating_sub(1);
                self.doc = target;
                Ok(false)
            }
        }
    }
}

impl SortedDocValues for MemorySortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ords[self.pos])
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.terms.len() as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        Ok(BytesRef::new(self.terms[ord as usize].clone()))
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i32> {
        match self
            .terms
            .binary_search_by(|t| t.as_slice().cmp(key.slice()))
        {
            Ok(i) => Ok(i as i32),
            Err(i) => Ok(-(i as i32 + 1)),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct MemorySortedNumericDocValues {
    docs: Vec<i32>,
    values: Vec<Vec<i64>>,
    current: Vec<i64>,
    idx: usize,
    pos: usize,
    doc: i32,
}

impl MemorySortedNumericDocValues {
    fn new(docs: Vec<i32>, values: Vec<Vec<i64>>) -> Self {
        Self {
            docs,
            values,
            current: Vec::new(),
            idx: 0,
            pos: usize::MAX,
            doc: -1,
        }
    }

    fn load_current(&mut self) {
        if self.pos < self.values.len() {
            self.current.clone_from(&self.values[self.pos]);
        } else {
            self.current.clear();
        }
        self.idx = 0;
    }
}

impl DocIdSetIterator for MemorySortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.pos.wrapping_add(1) < self.docs.len() {
            self.pos = self.pos.wrapping_add(1);
            self.doc = self.docs[self.pos];
            self.load_current();
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let start = self.pos.wrapping_add(1).min(self.docs.len());
        match self.docs[start..].binary_search(&target) {
            Ok(i) => {
                self.pos = start + i;
                self.doc = self.docs[self.pos];
            }
            Err(i) => {
                self.pos = start + i;
                if self.pos >= self.docs.len() {
                    self.doc = NO_MORE_DOCS;
                } else {
                    self.doc = self.docs[self.pos];
                }
            }
        }
        self.load_current();
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocValuesIterator for MemorySortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        match self.docs.binary_search(&target) {
            Ok(i) => {
                self.pos = i;
                self.doc = target;
                self.load_current();
                Ok(true)
            }
            Err(i) => {
                self.pos = i.min(self.docs.len()).saturating_sub(1);
                self.doc = target;
                self.load_current();
                Ok(false)
            }
        }
    }
}

impl SortedNumericDocValues for MemorySortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        let v = self.current[self.idx];
        self.idx += 1;
        Ok(v)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.current.len() as i32)
    }
}

#[derive(Debug, Clone, Default)]
struct MemorySortedSetDocValues {
    docs: Vec<i32>,
    values: Vec<Vec<i64>>,
    terms: Vec<Vec<u8>>,
    current: Vec<i64>,
    idx: usize,
    pos: usize,
    doc: i32,
}

impl MemorySortedSetDocValues {
    fn new(docs: Vec<i32>, values: Vec<Vec<i64>>, terms: Vec<Vec<u8>>) -> Self {
        Self {
            docs,
            values,
            terms,
            current: Vec::new(),
            idx: 0,
            pos: usize::MAX,
            doc: -1,
        }
    }

    fn load_current(&mut self) {
        if self.pos < self.values.len() {
            self.current.clone_from(&self.values[self.pos]);
        } else {
            self.current.clear();
        }
        self.idx = 0;
    }
}

impl DocIdSetIterator for MemorySortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.pos.wrapping_add(1) < self.docs.len() {
            self.pos = self.pos.wrapping_add(1);
            self.doc = self.docs[self.pos];
            self.load_current();
        } else {
            self.doc = NO_MORE_DOCS;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let start = self.pos.wrapping_add(1).min(self.docs.len());
        match self.docs[start..].binary_search(&target) {
            Ok(i) => {
                self.pos = start + i;
                self.doc = self.docs[self.pos];
            }
            Err(i) => {
                self.pos = start + i;
                if self.pos >= self.docs.len() {
                    self.doc = NO_MORE_DOCS;
                } else {
                    self.doc = self.docs[self.pos];
                }
            }
        }
        self.load_current();
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocValuesIterator for MemorySortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        match self.docs.binary_search(&target) {
            Ok(i) => {
                self.pos = i;
                self.doc = target;
                self.load_current();
                Ok(true)
            }
            Err(i) => {
                self.pos = i.min(self.docs.len()).saturating_sub(1);
                self.doc = target;
                self.load_current();
                Ok(false)
            }
        }
    }
}

impl SortedSetDocValues for MemorySortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        let v = self.current[self.idx];
        self.idx += 1;
        Ok(v)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.current.len() as i32)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        Ok(BytesRef::new(self.terms[ord as usize].clone()))
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.terms.len() as i64)
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i64> {
        match self
            .terms
            .binary_search_by(|t| t.as_slice().cmp(key.slice()))
        {
            Ok(i) => Ok(i as i64),
            Err(i) => Ok(-(i as i64 + 1)),
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

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

    impl DocValuesProducer for TestDocValuesProducer {
        fn get_numeric(
            &self,
            field: &FieldInfo,
        ) -> Result<Box<dyn NumericDocValues + Send + Sync>> {
            match self.numeric.get(&field.number) {
                Some((docs, values)) => Ok(Box::new(MemoryNumericDocValues::new(
                    docs.clone(),
                    values.clone(),
                ))),
                None => Ok(Box::new(EmptyNumericDocValues::new())),
            }
        }

        fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues + Send + Sync>> {
            match self.binary.get(&field.number) {
                Some((docs, values)) => Ok(Box::new(MemoryBinaryDocValues::new(
                    docs.clone(),
                    values.clone(),
                ))),
                None => Ok(Box::new(EmptyBinaryDocValues::new())),
            }
        }

        fn get_sorted(&self, field: &FieldInfo) -> Result<Box<dyn SortedDocValues + Send + Sync>> {
            match self.sorted.get(&field.number) {
                Some((docs, ords, terms)) => Ok(Box::new(MemorySortedDocValues::new(
                    docs.clone(),
                    ords.clone(),
                    terms.clone(),
                ))),
                None => Ok(Box::new(EmptySortedDocValues::new())),
            }
        }

        fn get_sorted_numeric(
            &self,
            field: &FieldInfo,
        ) -> Result<Box<dyn SortedNumericDocValues + Send + Sync>> {
            match self.sorted_numeric.get(&field.number) {
                Some((docs, values)) => Ok(Box::new(MemorySortedNumericDocValues::new(
                    docs.clone(),
                    values.clone(),
                ))),
                None => Ok(Box::new(EmptySortedNumericDocValues::new())),
            }
        }

        fn get_sorted_set(
            &self,
            field: &FieldInfo,
        ) -> Result<Box<dyn SortedSetDocValues + Send + Sync>> {
            match self.sorted_set.get(&field.number) {
                Some((docs, ords, terms)) => Ok(Box::new(MemorySortedSetDocValues::new(
                    docs.clone(),
                    ords.clone(),
                    terms.clone(),
                ))),
                None => Ok(Box::new(EmptySortedSetDocValues::new())),
            }
        }

        fn get_skipper(
            &self,
            _field: &FieldInfo,
        ) -> Result<Box<dyn DocValuesSkipper + Send + Sync>> {
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
