//! Indexed DISI (Document-ID Set Iterator) writer and reader.
//!
//! Equivalent to `org.apache.lucene.codecs.lucene90.IndexedDISI`.
//!
//! This module provides a compressed bit-set representation of a sparse or
//! dense set of document IDs and a matching iterator. It is used by both the
//! Lucene 9.0 doc-values format and the Lucene 9.9+ flat/HNSW vector formats.

#![deny(unsafe_code)]

use std::cmp;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
/// The largest number of documents a block stores as a sparse array of
/// shorts; a block with more is dense or all.
///
/// Equivalent to the package-private `IndexedDISI.MAX_ARRAY_LENGTH`; `pub`
/// here, as tests outside the crate read it the way Java's same-package
/// tests do.
pub const MAX_ARRAY_LENGTH: usize = (1 << 12) - 1;

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
            for word in buffer.get_bits() {
                out.write_long(word.load(std::sync::atomic::Ordering::Relaxed) as i64)?;
            }
        }
    } else {
        let bits = buffer.get_bits();
        for word_idx in 0..DENSE_BLOCK_LONGS {
            let word = bits[word_idx].load(std::sync::atomic::Ordering::Relaxed);
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
        bit_count += bits[word]
            .load(std::sync::atomic::Ordering::Relaxed)
            .count_ones() as usize;
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
        // `IndexedDISI.java:195-203`; `(int) Math.pow(2, denseRankPower)`
        // truncates and saturates as `as i32` does.
        return Err(LuceneError::illegal_argument(format!(
            "Acceptable values for denseRankPower are 7-15 (every 128-32768 docIDs). \
             The provided power was {dense_rank_power} (every {} docIDs)",
            2f64.powi(i32::from(dense_rank_power)) as i32
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
            out.file_pointer() - origo,
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
        out.file_pointer() - origo,
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

fn add_jumps(jumps: &mut Vec<i32>, offset: i64, index: i32, start_block: usize, end_block: usize) {
    debug_assert!(
        offset < i64::from(i32::MAX),
        "Logically the offset should not exceed 2^30 but was >= Integer.MAX_VALUE"
    );
    let offset = offset as i32;
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

/// Opens the data-blocks slice of an `IndexedDISI` stream, i.e. everything at
/// `offset` except the jump table stored after it.
///
/// # Errors
///
/// Propagates the error `slice` raises for a range the input cannot serve,
/// which is how a corrupt `length` or `jump_table_entry_count` is refused.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene90.IndexedDISI#createBlockSlice`.
pub fn create_block_slice(
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

/// Opens the jump table of an `IndexedDISI` stream, which sits in the last
/// `8 * jump_table_entry_count` bytes of the range, or `None` when the stream
/// was written without one.
///
/// # Errors
///
/// Propagates the error `random_access_slice` raises for a range the input
/// cannot serve.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene90.IndexedDISI#createJumpTable`.
pub fn create_jump_table(
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

/// How a block of an [`IndexedDISI`] stores its documents.
///
/// Equivalent to the package-private `IndexedDISI.Method`; `pub` here, as
/// tests outside the crate read it the way Java's same-package tests do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// At most [`MAX_ARRAY_LENGTH`] documents, as an array of shorts.
    Sparse,
    /// More than [`MAX_ARRAY_LENGTH`] documents, as a bit set.
    Dense,
    /// Every document of the block.
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
    /// The storage of the current block.
    ///
    /// Equivalent to the package-private `IndexedDISI.method` field; `pub`
    /// here, as tests outside the crate read it the way Java's same-package
    /// tests do.
    pub method: Method,

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
    /// The words of the current dense block that `into_bit_set` has read,
    /// allocated on its first use (`IndexedDISI.bitSet`).
    bit_set: Option<FixedBitSet>,
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
            // `IndexedDISI.java:332-340`, whose message ends in `". "`.
            return Err(LuceneError::illegal_argument(format!(
                "Acceptable values for denseRankPower are 7-15 (every 128-32768 docIDs). \
                 The provided power was {dense_rank_power} (every {} docIDs). ",
                2f64.powi(i32::from(dense_rank_power)) as i32
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
            bit_set: None,
        })
    }

    /// Returns the ordinal (index) of the current document.
    pub fn index(&self) -> i32 {
        self.index
    }

    /// Returns an iterator that delegates to this `IndexedDISI`. Advancing the
    /// returned iterator advances the underlying `IndexedDISI`, and vice-versa.
    ///
    /// Lucene wraps `disi` in an anonymous `KnnVectorValues.DocIndexIterator`
    /// that forwards `docID`, `index`, `nextDoc`, `advance` and `cost` to it
    /// (`IndexedDISI.java:428-456`). The sparse vector values keep one `disi`
    /// and hand out such a view on every `iterator()` call, so successive
    /// iterators continue where the previous one stopped
    /// (`OffHeapByteVectorValues.java:258-260`). The disi is therefore shared
    /// behind `Arc<Mutex<_>>`: the view locks it for each forwarded call.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene90.IndexedDISI#asDocIndexIterator`.
    pub fn as_doc_index_iterator(disi: Arc<Mutex<IndexedDISI>>) -> Box<dyn DocIndexIterator> {
        Box::new(SharedDocIndexIterator(disi))
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
        // `Short.toUnsignedInt(slice.readShort()) << 16`, an `int` shift that
        // wraps for a corrupt short with its top bit set, which Java only
        // asserts against (`IndexedDISI.java:520-521`).
        self.block = (block_short as i32) << 16;
        debug_assert!(self.block >= 0);
        let num_values = 1 + (self.slice.read_short()? as u16 as i32);
        self.index = self.next_block_index;
        // Java adds these as `int`s and wraps; a jump-table entry read off a
        // corrupt file can make `index` any value at all.
        self.next_block_index = self.index.wrapping_add(num_values);
        if num_values <= MAX_ARRAY_LENGTH as i32 {
            self.method = Method::Sparse;
            self.block_end = self.slice.file_pointer()? + (num_values as i64 * 2);
            self.next_exist_doc_in_block = -1;
        } else if num_values == BLOCK_SIZE as i32 {
            self.method = Method::All;
            self.block_end = self.slice.file_pointer()?;
            let gap = self.block.wrapping_sub(self.index).wrapping_sub(1);
            self.doc = -1; // will be set by advance
            self.dense_origo_index = gap; // repurpose for ALL gap
        } else {
            self.method = Method::Dense;
            self.dense_bitmap_offset = self.slice.file_pointer()?
                + if self.dense_rank_power == -1 {
                    0
                } else {
                    self.dense_rank_table.len() as i64
                };
            self.block_end = self.dense_bitmap_offset + (1i64 << 13);
            if self.dense_rank_power != -1 {
                let len = self.dense_rank_table.len();
                self.slice
                    .read_bytes(Some(&mut self.dense_rank_table), 0, len)?;
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
            debug_assert!(!self.exists);
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
                    self.slice.seek(self.slice.file_pointer()? - 2)?;
                    break;
                }
                self.exists = true;
                return Ok(true);
            }
        }
        self.exists = false;
        Ok(false)
    }

    /// Sets the bits of the documents of the current block up to `up_to`,
    /// shifted down by `offset`; returns whether the iterator stopped inside
    /// the block, on the first document at or after `up_to`.
    ///
    /// Equivalent to `Method.intoBitSetWithinBlock(IndexedDISI, int,
    /// FixedBitSet, int)` of each block method (`IndexedDISI.java`).
    fn into_bit_set_within_block(
        &mut self,
        up_to: i32,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<bool> {
        match self.method {
            Method::Sparse => {
                bit_set.set(self.doc.wrapping_sub(offset) as usize);
                while self.index < self.next_block_index {
                    let doc_in_block = self.slice.read_short()? as u16 as i32;
                    let doc = self.block | doc_in_block;
                    self.index = self.index.wrapping_add(1);
                    if doc >= up_to {
                        self.doc = doc;
                        self.exists = true;
                        self.next_exist_doc_in_block = doc_in_block;
                        return Ok(true);
                    }
                    bit_set.set(doc.wrapping_sub(offset) as usize);
                }
                Ok(false)
            }
            Method::Dense => {
                let mut source = self
                    .bit_set
                    .take()
                    .unwrap_or_else(|| FixedBitSet::new(BLOCK_SIZE));
                let result = self.dense_into_bit_set(&mut source, up_to, bit_set, offset);
                self.bit_set = Some(source);
                result
            }
            Method::All => {
                let block_end = self.block | 0xFFFF;
                if up_to <= block_end {
                    bit_set.set_range(
                        self.doc.wrapping_sub(offset) as usize,
                        up_to.wrapping_sub(offset) as usize,
                    );
                    self.advance_within_block(up_to)?;
                    Ok(true)
                } else {
                    bit_set.set_range(
                        self.doc.wrapping_sub(offset) as usize,
                        (block_end - offset + 1) as usize,
                    );
                    Ok(false)
                }
            }
        }
    }

    /// `DENSE.intoBitSetWithinBlock`, over the block words `source`
    /// (`disi.bitSet`).
    fn dense_into_bit_set(
        &mut self,
        source: &mut FixedBitSet,
        up_to: i32,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<bool> {
        let dest_from = self.doc.wrapping_sub(offset);
        let dest_to = crate::util::math_util::MathUtil::unsigned_min(
            up_to,
            offset.wrapping_add(bit_set.length() as i32),
        );
        let source_from = self.doc & 0xFFFF;
        let source_to = cmp::min(dest_to.wrapping_sub(self.block), BLOCK_SIZE as i32);

        let fp = self.slice.file_pointer()?;
        // seek back a long to include current word (disi.word).
        self.slice.seek(fp - 8)?;
        let word_index = self.word_index as usize;
        let num_words = FixedBitSet::bits2words(source_to as usize) - word_index;
        let mut words = vec![0i64; num_words];
        self.slice.read_longs(Some(&mut words), 0, num_words)?;
        let bits = source.get_bits();
        for (i, word) in words.into_iter().enumerate() {
            bits[word_index + i].store(word as u64, std::sync::atomic::Ordering::Relaxed);
        }
        FixedBitSet::or_range(
            source,
            source_from as usize,
            bit_set,
            dest_from as usize,
            (source_to - source_from) as usize,
        );

        let block_end = self.block | 0xFFFF;
        if dest_to > block_end {
            self.slice.seek(self.block_end)?;
            self.index = self.index.wrapping_add(
                source.cardinality_range(source_from as usize, source_to as usize) as i32,
            );
            Ok(false)
        } else {
            self.slice.seek(fp)?;
            let found = self.dense_advance(dest_to)?;
            if found && self.doc < up_to {
                return Err(LuceneError::illegal_state(format!(
                    "There are bits set in the source bitset that are not accounted for. \
                     doc={} upTo={} disi.block={}",
                    self.doc, up_to, self.block
                )));
            }
            Ok(found)
        }
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
    /// `IndexedDISI` declares no `toString()`, so `java.lang.Object.toString()` applies
    /// ([`java_object_to_string_stand_in`](crate::index::index_reader::java_object_to_string_stand_in)).
    fn java_to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string_stand_in(
            "org.apache.lucene.codecs.lucene90.IndexedDISI",
        )
    }
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
        // document (`IndexedDISI.java:470-471`), which holds for every block it
        // writes; without assertions a corrupt block leaves a stale `doc`.
        let found = self.advance_within_block(self.block)?;
        debug_assert!(found);
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.cost
    }

    /// Equivalent to `IndexedDISI.intoBitSet(int, FixedBitSet, int)`.
    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        debug_assert!(self.doc >= offset, "offset={offset} doc={}", self.doc);
        while self.doc < up_to && !self.into_bit_set_within_block(up_to, bit_set, offset)? {
            self.read_block_header()?;
            // Java asserts the freshly read block yields its own first
            // document, as `advance` does (`IndexedDISI.java:480-481`).
            let found = self.advance_within_block(self.block)?;
            debug_assert!(found);
        }
        Ok(())
    }

    /// Equivalent to `IndexedDISI.docIDRunEnd()`, `Method.docIDRunEnd` of
    /// the current block's method.
    ///
    /// Java's `int` additions wrap (`IndexedDISI.java:645-647, 757-762,
    /// 792-794`): on an exhausted iterator (`doc == NO_MORE_DOCS`) the run end
    /// is `Integer.MIN_VALUE`.
    fn doc_id_run_end(&mut self) -> Result<i32> {
        Ok(match self.method {
            Method::Sparse => self.doc.wrapping_add(1),
            Method::Dense => {
                if self.word == u64::MAX {
                    (self.doc | 0x3F).wrapping_add(1)
                } else {
                    self.doc.wrapping_add(1)
                }
            }
            Method::All => (self.doc | 0xFFFF).wrapping_add(1),
        })
    }
}

impl DocIndexIterator for IndexedDISI {
    fn index(&self) -> i32 {
        self.index
    }
}

/// The anonymous `DocIndexIterator` of `IndexedDISI.asDocIndexIterator`
/// (`IndexedDISI.java:428-456`): every call is forwarded to the shared disi.
struct SharedDocIndexIterator(Arc<Mutex<IndexedDISI>>);

impl SharedDocIndexIterator {
    fn disi(&self) -> MutexGuard<'_, IndexedDISI> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl DocIdSetIterator for SharedDocIndexIterator {
    /// `IndexedDISI$1` declares no `toString()`, so `java.lang.Object.toString()` applies
    /// ([`java_object_to_string_stand_in`](crate::index::index_reader::java_object_to_string_stand_in)).
    fn java_to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string_stand_in(
            "org.apache.lucene.codecs.lucene90.IndexedDISI$1",
        )
    }
    fn doc_id(&self) -> i32 {
        self.disi().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.disi().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        DocIdSetIterator::advance(&mut *self.disi(), target)
    }

    fn cost(&self) -> i64 {
        self.disi().cost()
    }
}

impl DocIndexIterator for SharedDocIndexIterator {
    fn index(&self) -> i32 {
        self.disi().index
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
