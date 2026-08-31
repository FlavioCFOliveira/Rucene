//! Indexed DISI (Document-ID Set Iterator) writer and reader.
//!
//! Equivalent to `org.apache.lucene.codecs.lucene90.IndexedDISI`.
//!
//! This module provides a compressed bit-set representation of a sparse or
//! dense set of document IDs and a matching iterator. It is used by both the
//! Lucene 9.0 doc-values format and the Lucene 9.9+ flat/HNSW vector formats.

#![deny(unsafe_code)]

use std::cmp;

use crate::error::{LuceneError, Result};
use crate::index::doc_values::DocValuesIterator;
use crate::index::vector_values::DocIndexIterator;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{IndexInput, IndexOutput, RandomAccessInput};
use crate::util::FixedBitSet;

const BLOCK_SIZE: usize = 65536;
const DENSE_BLOCK_LONGS: usize = BLOCK_SIZE / 64;
/// Default exponent used for dense-block rank tables.
///
/// A value of `9` means one rank entry is stored for every 512 bits (2^9 bits).
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

/// Writes a bit set representation of the documents returned by `it` to `out`.
///
/// Equivalent to `IndexedDISI.writeBitSet`.
///
/// Returns the number of entries in the block jump table, which must be stored
/// in the format metadata to allow the reader to locate the jump table.
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

fn add_jumps(jumps: &mut Vec<i32>, offset: i32, index: i32, start_block: usize, end_block: usize) {
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
        i64::from(jump_table_entry_count) * 8
    };
    // Both the length and the entry count come off disk. Java subtracts them as
    // `long`s and lets the result wrap, then leaves it to `slice` to refuse a
    // length it cannot serve; the wrap has to be explicit here, or a corrupt
    // length near `Long.MIN_VALUE` aborts instead of being refused.
    slice.slice(
        slice_description,
        offset,
        length.wrapping_sub(jump_table_bytes),
    )
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
        let jump_table_bytes = i64::from(jump_table_entry_count) * 8;
        Ok(Some(slice.random_access_slice(
            offset.wrapping_add(length).wrapping_sub(jump_table_bytes),
            jump_table_bytes,
        )?))
    }
}

enum Method {
    Sparse,
    Dense,
    All,
}

/// An indexed, random-accessible DISI.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.IndexedDISI`.
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
    /// Creates a new indexed DISI reading from `input` at the given offset.
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

    /// Creates a new indexed DISI from pre-built slice and jump table.
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

    /// Returns the ordinal (index) of the current document.
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
                self.next_block_index = index.wrapping_sub(1);
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
        // Java writes `doc >>> 16` as a short and asserts `block >= 0` when it
        // reads it back (`IndexedDISI.java:520-521`). A document id is a
        // non-negative `int`, so a legal block index never exceeds 32767 and
        // the top bit of this short is only ever set by corruption. Java's
        // assertion is disabled at runtime and it carries on with a negative
        // block; a `debug_assert!` here would instead abort the process on a
        // corrupt file, so the invariant is enforced as an error.
        if block_short > i16::MAX as u16 {
            return Err(LuceneError::CorruptIndex(format!(
                "invalid IndexedDISI block index: {block_short}"
            )));
        }
        self.block = (block_short as i32) << 16;
        let num_values = 1 + (self.slice.read_short()? as u16 as i32);
        self.index = self.next_block_index;
        // Java adds these as `int`s and wraps; a jump-table entry read off a
        // corrupt file can make `index` any value at all.
        self.next_block_index = self.index.wrapping_add(num_values);
        if num_values <= MAX_ARRAY_LENGTH as i32 {
            self.method = Method::Sparse;
            self.block_end = self.slice.file_pointer() + (num_values as i64 * 2);
            self.next_exist_doc_in_block = -1;
        } else if num_values == BLOCK_SIZE as i32 {
            self.method = Method::All;
            self.block_end = self.slice.file_pointer();
            let gap = self.block.wrapping_sub(self.index).wrapping_sub(1);
            self.doc = -1; // will be set by advance
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
            self.number_of_ones = self.index.wrapping_add(1);
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
                self.index = target.wrapping_sub(gap);
                Ok(true)
            }
        }
    }

    fn sparse_advance(&mut self, target: i32) -> Result<bool> {
        let target_in_block = target & 0xFFFF;
        while self.index < self.next_block_index {
            let doc_in_block = self.slice.read_short()? as u16 as i32;
            self.index = self.index.wrapping_add(1);
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
            && target_word_index.wrapping_sub(self.word_index) >= (1 << (self.dense_rank_power - 6))
        {
            self.rank_skip(target_in_block)?;
        }
        for _ in self.word_index.wrapping_add(1)..=target_word_index {
            self.word = self.slice.read_long()? as u64;
            self.number_of_ones = self
                .number_of_ones
                .wrapping_add(self.word.count_ones() as i32);
        }
        self.word_index = target_word_index;
        let left_bits = self.word >> (target & 0x3F);
        if left_bits != 0 {
            self.doc = target.wrapping_add(left_bits.trailing_zeros() as i32);
            self.index = self
                .number_of_ones
                .wrapping_sub(left_bits.count_ones() as i32);
            return Ok(true);
        }
        while self.word_index + 1 < DENSE_BLOCK_LONGS as i32 {
            self.word_index += 1;
            self.word = self.slice.read_long()? as u64;
            if self.word != 0 {
                self.index = self.number_of_ones;
                self.number_of_ones = self
                    .number_of_ones
                    .wrapping_add(self.word.count_ones() as i32);
                self.doc = self.block | (self.word_index << 6) | self.word.trailing_zeros() as i32;
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
        let dense_noo = (rank as i32).wrapping_add(rank_word.count_ones() as i32);
        self.word_index = rank_aligned_word_index;
        self.word = rank_word;
        self.number_of_ones = self.dense_origo_index.wrapping_add(dense_noo);
        Ok(())
    }

    fn advance_exact_within_block(&mut self, target: i32) -> Result<bool> {
        match self.method {
            Method::Sparse => self.sparse_advance_exact(target),
            Method::Dense => self.dense_advance_exact(target),
            Method::All => {
                let gap = self.dense_origo_index;
                self.index = target.wrapping_sub(gap);
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
            self.index = self.index.wrapping_add(1);
            if doc_in_block >= target_in_block {
                self.next_exist_doc_in_block = doc_in_block;
                if doc_in_block != target_in_block {
                    self.index = self.index.wrapping_sub(1);
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
            && target_word_index.wrapping_sub(self.word_index) >= (1 << (self.dense_rank_power - 6))
        {
            self.rank_skip(target_in_block)?;
        }
        for _ in self.word_index.wrapping_add(1)..=target_word_index {
            self.word = self.slice.read_long()? as u64;
            self.number_of_ones = self
                .number_of_ones
                .wrapping_add(self.word.count_ones() as i32);
        }
        self.word_index = target_word_index;
        let left_bits = self.word >> (target & 0x3F);
        self.index = self
            .number_of_ones
            .wrapping_sub(left_bits.count_ones() as i32);
        Ok((left_bits & 1) != 0)
    }
}

impl DocIdSetIterator for IndexedDISI {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc.wrapping_add(1))
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
        // Java asserts here that the freshly read block yields its own first
        // document (`IndexedDISI.java:582`), which holds for every block it
        // writes. Assertions are off at runtime, so a corrupt block leaves Java
        // with a stale `doc`; a `debug_assert!` would instead abort this
        // process, so the broken invariant is reported as corruption.
        if !self.advance_within_block(self.block)? {
            return Err(LuceneError::CorruptIndex(format!(
                "IndexedDISI block {} names no document of its own",
                self.block
            )));
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl DocIndexIterator for IndexedDISI {
    fn index(&self) -> i32 {
        self.index
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocIdSetIterator;
    use crate::store::MockIndexOutput;

    /// A simple iterator over a sorted list of doc IDs.
    struct VecDocIter {
        docs: Vec<i32>,
        idx: usize,
    }

    impl VecDocIter {
        fn new(docs: Vec<i32>) -> Self {
            Self { docs, idx: 0 }
        }
    }

    impl DocIdSetIterator for VecDocIter {
        fn doc_id(&self) -> i32 {
            if self.idx == 0 {
                -1
            } else if self.idx > self.docs.len() {
                NO_MORE_DOCS
            } else {
                self.docs[self.idx - 1]
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            if self.idx < self.docs.len() {
                self.idx += 1;
                Ok(self.docs[self.idx - 1])
            } else {
                self.idx = self.docs.len() + 1;
                Ok(NO_MORE_DOCS)
            }
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            while self.idx < self.docs.len() && self.docs[self.idx] < target {
                self.idx += 1;
            }
            self.next_doc()
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    #[test]
    fn write_and_read_empty_bit_set() {
        let mut out = MockIndexOutput::new("test", "test");
        let mut it = VecDocIter::new(vec![]);
        let jump_count = write_bit_set(&mut it, &mut out, DEFAULT_DENSE_RANK_POWER).unwrap();
        assert_eq!(jump_count, 1);

        let bytes = out.into_inner();
        let input = crate::store::MockIndexInput::new(bytes, "test");
        let mut disi =
            IndexedDISI::new(&input, 0, input.length(), 0, DEFAULT_DENSE_RANK_POWER, 0).unwrap();
        assert_eq!(disi.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn write_and_read_dense_bit_set() {
        let mut out = MockIndexOutput::new("test", "test");
        let docs: Vec<i32> = (0..10).collect();
        let mut it = VecDocIter::new(docs.clone());
        let jump_count = write_bit_set(&mut it, &mut out, DEFAULT_DENSE_RANK_POWER).unwrap();

        let bytes = out.into_inner();
        let input = crate::store::MockIndexInput::new(bytes, "test");
        let mut disi = IndexedDISI::new(
            &input,
            0,
            input.length() as i64,
            jump_count as i32,
            DEFAULT_DENSE_RANK_POWER,
            10,
        )
        .unwrap();
        for expected in docs {
            assert_eq!(disi.next_doc().unwrap(), expected);
            assert_eq!(disi.index(), expected);
        }
        assert_eq!(disi.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn write_and_read_sparse_bit_set() {
        let mut out = MockIndexOutput::new("test", "test");
        let docs = vec![0, 5, 100, 200, 500];
        let mut it = VecDocIter::new(docs.clone());
        let jump_count = write_bit_set(&mut it, &mut out, DEFAULT_DENSE_RANK_POWER).unwrap();

        let bytes = out.into_inner();
        let input = crate::store::MockIndexInput::new(bytes, "test");
        let mut disi = IndexedDISI::new(
            &input,
            0,
            input.length() as i64,
            jump_count as i32,
            DEFAULT_DENSE_RANK_POWER,
            docs.len() as i64,
        )
        .unwrap();
        let mut seen = Vec::new();
        loop {
            let doc = disi.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            seen.push(doc);
            assert_eq!(disi.index(), seen.len() as i32 - 1);
        }
        assert_eq!(seen, docs);
    }
}
