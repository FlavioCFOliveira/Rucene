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
use crate::store::{IndexInput, IndexOutput, MemorySegment, RandomAccessInput};
use crate::util::atomic_ref_cell::AtomicRefCell;
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
        for (word_idx, bit_word) in bits[..DENSE_BLOCK_LONGS].iter().enumerate() {
            let word = bit_word.load(std::sync::atomic::Ordering::Relaxed);
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
    if !(7..=15).contains(&dense_rank_power) && dense_rank_power != -1 {
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

/// The jump table an [`IndexedDISI`] reads.
///
/// Java's `final RandomAccessInput jumpTable` (`IndexedDISI.java:276`) is a
/// reference: a norms merge instance points every iterator of a field at the
/// one jump table it caches (`Lucene90NormsProducer.java:358-375`). A Rust
/// input reads through `&mut self`, so that shared table is held through
/// a shared cell, and an iterator's own table is boxed.
pub(crate) enum JumpTable {
    /// A jump table of the iterator's own.
    Owned(Box<dyn RandomAccessInput>),
    /// A jump table a merge instance shares between the iterators it hands
    /// out.
    Shared(Arc<AtomicRefCell<Box<dyn RandomAccessInput>>>),
}

impl JumpTable {
    /// `jumpTable.readInt(pos)`.
    fn read_int_at(&mut self, pos: i64) -> Result<i32> {
        match self {
            Self::Owned(jump_table) => jump_table.read_int_at(pos),
            Self::Shared(jump_table) => jump_table.borrow_mut().read_int_at(pos),
        }
    }
}

/// The bytes of `slice`, positioned as its file pointer, when it is a
/// memory-mapped input whose whole range lies in one mapped segment; `None`
/// otherwise, or when the input refuses the request (closed).
///
/// Reached through Lucene's own `MemorySegmentAccessInput.segmentSliceOrNull`
/// ([`IndexInput::as_memory_segment_access_input`]); see
/// [`IndexedDISI`]'s `block_bytes` for how the scan uses it.
pub(crate) fn block_segment_of(slice: &dyn IndexInput) -> Option<MemorySegment> {
    slice
        .as_memory_segment_access_input()?
        .segment_slice_or_null(0, slice.length())
        .ok()
        .flatten()
}

/// What [`scan_block_bytes`] found.
struct BlockScan {
    /// The number of shorts read before the one that stopped the scan (all
    /// the shorts read when nothing stopped it).
    skipped: usize,
    /// The doc id in block that stopped the scan: the first one at or after
    /// the target.
    found: Option<i32>,
    /// Whether the scan stopped because `bytes` ran out before `remaining`
    /// shorts were read: the next short must then be read through the slice.
    unservable: bool,
}

/// The index of the first little-endian unsigned short of `window` that is
/// `>= target_in_block`, or `None`.
///
/// The same answer as reading the shorts one by one and stopping at the
/// first match; the shorts are compared sixteen at a time, in a fixed-size
/// loop the compiler turns into vector compares, the doc ids of a SPARSE
/// block being scanned linearly by Java too (`// TODO: binary search`).
#[inline]
fn first_at_or_after(window: &[u8], target_in_block: i32) -> Option<usize> {
    const LANES: usize = 16;
    // `target & 0xFFFF` at every caller, so the comparison fits 16 bits.
    debug_assert!((0..=0xFFFF).contains(&target_in_block));
    let target = target_in_block as u16;
    let at_least = |pair: &[u8; 2]| u16::from_le_bytes(*pair) >= target;
    let (pairs, _) = window.as_chunks::<2>();
    let (blocks, rest) = pairs.as_chunks::<LANES>();
    for (block, lanes) in blocks.iter().enumerate() {
        let mut mask = 0u32;
        for (lane, pair) in lanes.iter().enumerate() {
            mask |= u32::from(at_least(pair)) << lane;
        }
        if mask != 0 {
            return Some(block * LANES + mask.trailing_zeros() as usize);
        }
    }
    rest.iter()
        .position(at_least)
        .map(|at| blocks.len() * LANES + at)
}

/// The SPARSE scan of `IndexedDISI.java:583-593` and `:607-621` over the
/// in-memory bytes: reads at most `remaining` unsigned shorts from `bytes`
/// starting at `fp`, until the first one `>= target_in_block`.
///
/// Every short read is the one `slice.readShort()` would return at the same
/// position; the scan stops early (`unservable`) at the first short that is
/// not all inside `bytes`.
#[inline]
fn scan_block_bytes(bytes: &[u8], fp: i64, remaining: i64, target_in_block: i32) -> BlockScan {
    let available = usize::try_from(fp)
        .ok()
        .and_then(|fp| bytes.get(fp..))
        .unwrap_or(&[]);
    let wanted = usize::try_from(remaining.max(0)).unwrap_or(usize::MAX);
    let servable = cmp::min(wanted, available.len() / 2);
    let window = &available[..servable * 2];
    match first_at_or_after(window, target_in_block) {
        Some(at) => BlockScan {
            skipped: at,
            found: Some(i32::from(u16::from_le_bytes([
                window[2 * at],
                window[2 * at + 1],
            ]))),
            unservable: false,
        },
        None => BlockScan {
            skipped: servable,
            found: None,
            unservable: servable < wanted,
        },
    }
}

/// An indexed, random-accessible DISI.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.IndexedDISI`.
pub struct IndexedDISI {
    slice: Box<dyn IndexInput>,
    /// The bytes of `slice` in memory ([`block_segment_of`]), or of the shared
    /// block slice a norms merge instance wraps
    /// ([`set_block_segment`](Self::set_block_segment)).
    ///
    /// **Internal deviation from Lucene 10.5.0, no observable difference.**
    /// Java's SPARSE scan reads each doc id with `slice.readShort()`
    /// (`IndexedDISI.java:597-623`, `:648-660`), which the JIT inlines into a
    /// plain load. A Rust `Box<dyn IndexInput>` pays a dynamic call per short
    /// (plus, on a merge instance, the shared-input borrow and seek of
    /// `getDisiInput`'s wrapper), which made the scan ~4x Lucene's cost (rmp
    /// #675). The scan therefore reads the shorts straight from these bytes
    /// — the same bytes, at the same positions, that `readShort` would return —
    /// and keeps the position it reaches in `scan_fp`, so that `slice` is
    /// where Java's would be whenever it is read again. Any read the bytes
    /// cannot serve (a position past the end, a closed input) goes through
    /// `slice` itself, which raises exactly what Java raises; without bytes
    /// the scan is Java's.
    block_bytes: Option<MemorySegment>,
    /// The file pointer the SPARSE scan over `block_bytes` reached and has
    /// not yet written back to `slice`, or `None` when `slice`'s own file
    /// pointer is current.
    ///
    /// **Internal deviation from Lucene 10.5.0, no observable difference.**
    /// Java's scan moves `slice`'s position with every `readShort()`. Taking
    /// the file pointer and seeking `slice` on every `advanceExact` costs two
    /// dynamic calls per scored document here (rmp #676), so the position is
    /// written back lazily, by [`sync_slice`](Self::sync_slice), before any
    /// other use of `slice`. `slice` is private to this iterator (Java's
    /// `IndexedDISI.slice` field; every caller hands over a slice of its own,
    /// or the merge instance's wrapper with its own offset), so nothing else
    /// can observe its position in between.
    scan_fp: Option<i64>,
    jump_table: Option<JumpTable>,
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
        Self::with_jump_table(
            slice,
            jump_table.map(JumpTable::Owned),
            jump_table_entry_count,
            dense_rank_power,
            cost,
        )
    }

    /// [`with_slices`](Self::with_slices), with a jump table that may be
    /// shared: Java's `IndexedDISI(IndexInput, RandomAccessInput, int, byte,
    /// long)` (`IndexedDISI.java:318-345`) takes the table by reference.
    pub(crate) fn with_jump_table(
        slice: Box<dyn IndexInput>,
        jump_table: Option<JumpTable>,
        jump_table_entry_count: i32,
        dense_rank_power: i8,
        cost: i64,
    ) -> Result<Self> {
        if !(7..=15).contains(&dense_rank_power) && dense_rank_power != -1 {
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
        let block_bytes = block_segment_of(slice.as_ref());
        Ok(Self {
            slice,
            block_bytes,
            scan_fp: None,
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

    /// Sets the in-memory bytes of `slice` when `slice` cannot give them
    /// itself: the wrapper a norms merge instance hands out over its shared
    /// block slice (`Lucene90NormsProducer.getDisiInput`), whose file pointer
    /// is a position in that slice.
    pub(crate) fn set_block_segment(&mut self, block_bytes: Option<MemorySegment>) {
        self.block_bytes = block_bytes;
    }

    /// Writes the position the scan over `block_bytes` reached back to
    /// `slice` (see `scan_fp`); a no-op when `slice` is current.
    #[inline]
    fn sync_slice(&mut self) -> Result<()> {
        if let Some(fp) = self.scan_fp {
            self.slice.seek(fp)?;
            self.scan_fp = None;
        }
        Ok(())
    }

    /// `slice.getFilePointer()`, the pending scan position included.
    #[inline]
    fn slice_file_pointer(&self) -> Result<i64> {
        match self.scan_fp {
            Some(fp) => Ok(fp),
            None => self.slice.file_pointer(),
        }
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
                self.sync_slice()?;
                self.slice.seek(offset as i64)?;
                self.read_block_header()?;
                return Ok(());
            }
        }
        while self.block < target_block {
            self.sync_slice()?;
            self.slice.seek(self.block_end)?;
            self.read_block_header()?;
        }
        Ok(())
    }

    fn read_block_header(&mut self) -> Result<()> {
        self.sync_slice()?;
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
        // PERF: see `block_bytes` and `scan_fp`.
        if let Some(segment) = self.block_bytes.as_ref().filter(|s| s.is_alive()) {
            let target_in_block = target & 0xFFFF;
            let start = self.slice_file_pointer()?;
            let remaining = i64::from(self.next_block_index) - i64::from(self.index);
            let scan = scan_block_bytes(segment.bytes(), start, remaining, target_in_block);
            // `disi.index++` once per short read, as `int` (which wraps).
            self.index = self.index.wrapping_add(scan.skipped as i32);
            let mut fp = start + 2 * scan.skipped as i64;
            if let Some(doc_in_block) = scan.found {
                self.index = self.index.wrapping_add(1);
                fp += 2;
                self.doc = self.block | doc_in_block;
                self.exists = true;
                self.next_exist_doc_in_block = doc_in_block;
                self.scan_fp = Some(fp);
                return Ok(true);
            }
            if scan.unservable {
                // Not in the bytes: `slice` reads (and fails) as Java does.
                self.slice.seek(fp)?;
                self.scan_fp = None;
                return self.sparse_advance_scan(target);
            }
            self.scan_fp = Some(fp);
            return Ok(false);
        }
        self.sparse_advance_scan(target)
    }

    /// The SPARSE `advanceWithinBlock` loop, reading through `slice`.
    fn sparse_advance_scan(&mut self, target: i32) -> Result<bool> {
        self.sync_slice()?;
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
        self.sync_slice()?;
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
        self.sync_slice()?;
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
        // PERF: see `block_bytes` and `scan_fp`.
        if let Some(segment) = self.block_bytes.as_ref().filter(|s| s.is_alive()) {
            let start = self.slice_file_pointer()?;
            let remaining = i64::from(self.next_block_index) - i64::from(self.index);
            let scan = scan_block_bytes(segment.bytes(), start, remaining, target_in_block);
            // `disi.index++` once per short read, as `int`; a short past the
            // target is read and then given back (`disi.index--` and the seek
            // back of `IndexedDISI.java:612-616`), so neither moves for it.
            self.index = self.index.wrapping_add(scan.skipped as i32);
            let fp = start + 2 * scan.skipped as i64;
            if let Some(doc_in_block) = scan.found {
                self.next_exist_doc_in_block = doc_in_block;
                if doc_in_block == target_in_block {
                    self.index = self.index.wrapping_add(1);
                    self.exists = true;
                    self.scan_fp = Some(fp + 2);
                    return Ok(true);
                }
            } else if scan.unservable {
                // Not in the bytes: `slice` reads (and fails) as Java does.
                self.slice.seek(fp)?;
                self.scan_fp = None;
                return self.sparse_advance_exact_scan(target);
            }
            self.scan_fp = Some(fp);
            self.exists = false;
            return Ok(false);
        }
        self.sparse_advance_exact_scan(target)
    }

    /// The SPARSE `advanceExactWithinBlock` loop, reading through `slice`.
    fn sparse_advance_exact_scan(&mut self, target: i32) -> Result<bool> {
        self.sync_slice()?;
        let target_in_block = target & 0xFFFF;
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
    // Lucene: `Method.intoBitSetWithinBlock` advances the iterator, so it takes `&mut self`.
    #[allow(clippy::wrong_self_convention)]
    fn into_bit_set_within_block(
        &mut self,
        up_to: i32,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<bool> {
        self.sync_slice()?;
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

        self.sync_slice()?;
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
        let first_word = self.word_index.wrapping_add(1);
        if first_word <= target_word_index {
            // PERF (rmp #681): the words `slice.readLong()` returns
            // (`IndexedDISI.java:668-672`), read from the mapped block bytes
            // at the same positions when they are all there; the position
            // reached is written back lazily, as the SPARSE scan does (see
            // `block_bytes` and `scan_fp`). Otherwise through `slice`.
            if !self.dense_words_from_bytes(first_word, target_word_index)? {
                self.sync_slice()?;
                for _ in first_word..=target_word_index {
                    self.word = self.slice.read_long()? as u64;
                    self.number_of_ones = self
                        .number_of_ones
                        .wrapping_add(self.word.count_ones() as i32);
                }
            }
        }
        self.word_index = target_word_index;
        let left_bits = self.word >> (target & 0x3F);
        self.index = self
            .number_of_ones
            .wrapping_sub(left_bits.count_ones() as i32);
        Ok((left_bits & 1) != 0)
    }

    /// Reads the DENSE words `first..=last` from `block_bytes`, accumulating
    /// `word` and `number_of_ones` exactly as the `slice.readLong()` loop
    /// does, and records the position reached in `scan_fp`. Returns `false`,
    /// having changed nothing, when the bytes are absent, closed or do not
    /// hold every word: the caller then reads through `slice`.
    #[inline]
    fn dense_words_from_bytes(&mut self, first: i32, last: i32) -> Result<bool> {
        let Some(segment) = self.block_bytes.as_ref().filter(|s| s.is_alive()) else {
            return Ok(false);
        };
        let start = self.slice_file_pointer()?;
        let count = (last - first + 1) as usize;
        let Some(window) = usize::try_from(start).ok().and_then(|at| {
            segment
                .bytes()
                .get(at..at.checked_add(count.checked_mul(8)?)?)
        }) else {
            return Ok(false);
        };
        let (words, _) = window.as_chunks::<8>();
        let mut number_of_ones = self.number_of_ones;
        let mut word = self.word;
        for bytes in words {
            word = u64::from_le_bytes(*bytes);
            number_of_ones = number_of_ones.wrapping_add(word.count_ones() as i32);
        }
        self.word = word;
        self.number_of_ones = number_of_ones;
        self.scan_fp = Some(start + 8 * count as i64);
        Ok(true)
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
            input.length(),
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
            input.length(),
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

    /// rmp #676: the scan over the mapped bytes (`block_bytes`, with the
    /// lazily written-back `scan_fp` and the sixteen-lane compare) answers
    /// every `advance_exact`, `advance` and `next_doc` exactly as the scan
    /// through `slice` does, across SPARSE, DENSE and ALL blocks and the
    /// moves between them; and a SPARSE block whose doc count runs past the
    /// end of the slice fails identically on both paths.
    #[test]
    fn mapped_sparse_scan_matches_the_slice_scan() {
        use crate::store::mmap::MMapDirectory;
        use crate::store::Directory as _;

        // Block 0 SPARSE, block 1 DENSE, block 2 ALL, block 3 SPARSE (short),
        // block 5 SPARSE (block 4 empty).
        let mut docs: Vec<i32> = (0..3000).map(|i| i * 21 + (i % 7)).collect();
        docs.extend((0..20_000).map(|i| 65_536 + i * 3));
        docs.extend(2 * 65_536..3 * 65_536);
        docs.extend((0..40).map(|i| 3 * 65_536 + i * 977));
        docs.extend((0..4095).map(|i| 5 * 65_536 + i * 16));
        let mut out = MockIndexOutput::new("test", "test");
        let jump_count = write_bit_set(
            &mut VecDocIter::new(docs.clone()),
            &mut out,
            DEFAULT_DENSE_RANK_POWER,
        )
        .unwrap();
        let mut bytes = vec![0xA5u8; 3];
        bytes.extend(out.into_inner());

        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().canonicalize().unwrap();
        std::fs::write(path.join("d.bin"), &bytes).unwrap();
        let dir = MMapDirectory::open(path).unwrap();
        let mapped_input = dir
            .open_input("d.bin", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let plain_input = crate::store::MockIndexInput::new(bytes.clone(), "test");
        let length = bytes.len() as i64 - 3;
        let open = |input: &dyn IndexInput| {
            IndexedDISI::new(
                input,
                3,
                length,
                jump_count as i32,
                DEFAULT_DENSE_RANK_POWER,
                docs.len() as i64,
            )
            .unwrap()
        };

        let max = 6 * 65_536;
        for seed in 1..=40u64 {
            let mut mapped = open(mapped_input.as_ref());
            let mut plain = open(&plain_input);
            // Positive control: one takes the mapped scan, the other cannot.
            assert!(mapped.block_bytes.is_some());
            assert!(plain.block_bytes.is_none());
            let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };
            let mut target = 0i32;
            let mut exact_mode = seed % 2 == 0;
            loop {
                let r = next();
                if r % 50 == 0 {
                    exact_mode = !exact_mode;
                }
                // Steps of 1 to ~200 docs, now and then a long jump.
                let step = if r % 23 == 0 {
                    (r >> 8) % 70_000
                } else {
                    1 + (r >> 8) % 200
                };
                target = target.saturating_add(step as i32);
                if target >= max {
                    break;
                }
                if exact_mode {
                    let m = mapped.advance_exact(target).unwrap();
                    let p = plain.advance_exact(target).unwrap();
                    assert_eq!(m, p, "advance_exact({target}) seed {seed}");
                    if m {
                        assert_eq!(mapped.index(), plain.index(), "index at {target}");
                    }
                } else if r % 3 == 0 {
                    let m = mapped.next_doc().unwrap();
                    let p = plain.next_doc().unwrap();
                    assert_eq!(m, p, "next_doc after {target} seed {seed}");
                    assert_eq!(mapped.index(), plain.index());
                    if m == NO_MORE_DOCS {
                        break;
                    }
                    target = m;
                } else {
                    if target <= mapped.doc_id() {
                        continue;
                    }
                    let m = mapped.advance(target).unwrap();
                    let p = plain.advance(target).unwrap();
                    assert_eq!(m, p, "advance({target}) seed {seed}");
                    assert_eq!(mapped.index(), plain.index());
                    if m == NO_MORE_DOCS {
                        break;
                    }
                    target = m;
                }
                assert_eq!(mapped.doc_id(), plain.doc_id());
            }
        }

        // A SPARSE block claiming 100 docs with only 3 in the slice: both
        // scans read past the end and fail with the same error.
        let mut corrupt = vec![0xA5u8; 3];
        corrupt.extend_from_slice(&0u16.to_le_bytes()); // block 0
        corrupt.extend_from_slice(&99u16.to_le_bytes()); // 100 docs
        for doc in [4u16, 9, 30] {
            corrupt.extend_from_slice(&doc.to_le_bytes());
        }
        std::fs::write(temp.path().join("c.bin"), &corrupt).unwrap();
        let mapped_input = dir
            .open_input("c.bin", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let plain_input = crate::store::MockIndexInput::new(corrupt.clone(), "c.bin");
        let corrupt_length = corrupt.len() as i64 - 3;
        let open = |input: &dyn IndexInput| {
            IndexedDISI::with_slices(
                input.slice("docs", 3, corrupt_length).unwrap(),
                None,
                -1,
                DEFAULT_DENSE_RANK_POWER,
                100,
            )
            .unwrap()
        };
        let mut mapped = open(mapped_input.as_ref());
        assert!(mapped.block_bytes.is_some());
        assert!(mapped.advance_exact(9).unwrap());
        let mapped_err = mapped.advance_exact(500).unwrap_err();
        let mut mapped_again = open(mapped_input.as_ref());
        let mapped_advance_err = mapped_again.advance(500).unwrap_err();
        // The slice scan of a mapped input is the reference here: the error
        // names the mapped input, so compare against it with no bytes.
        let mut reference = open(mapped_input.as_ref());
        reference.set_block_segment(None);
        assert!(reference.advance_exact(9).unwrap());
        let reference_err = reference.advance_exact(500).unwrap_err();
        assert_eq!(format!("{mapped_err:?}"), format!("{reference_err:?}"));
        let mut reference = open(mapped_input.as_ref());
        reference.set_block_segment(None);
        assert_eq!(
            format!("{mapped_advance_err:?}"),
            format!("{:?}", reference.advance(500).unwrap_err())
        );
        // And the plain input fails at the same point.
        let mut plain = open(&plain_input);
        assert!(plain.advance_exact(9).unwrap());
        assert!(plain.advance_exact(500).is_err());
    }
}
