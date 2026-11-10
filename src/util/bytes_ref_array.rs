//! Append-only `BytesRef` collections ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`BytesRefIterator`] | `BytesRefIterator` |
//! | [`EmptyBytesRefIterator`] | `BytesRefIterator.EMPTY` |
//! | [`SortableBytesRefArray`] | `SortableBytesRefArray` |
//! | [`BytesRefArray`] | `BytesRefArray` |
//! | [`FixedLengthBytesRefArray`] | `FixedLengthBytesRefArray` |
//! | [`BytesRefBlockPool`] | `BytesRefBlockPool` |

#![deny(unsafe_code)]

use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::util::byte_block_pool::{ByteBlockPool, DirectTrackingAllocator};
use crate::util::concurrent::Counter;
use crate::util::sorter::{
    StableStringSorter, StableStringSorterOps, StringSorter, StringSorterComparator,
    StringSorterOps,
};
use crate::util::{Accountable, ArrayUtil, BytesRef, BytesRefBuilder, RamUsageEstimator};

// ---------------------------------------------------------------------------
// BytesRefIterator
// ---------------------------------------------------------------------------

/// A simple iterator over [`BytesRef`] values.
///
/// Port of `org.apache.lucene.util.BytesRefIterator`.
///
/// **Divergence from Lucene 10.5.0.** Java returns `null` at the end and may
/// reuse the returned `BytesRef` across calls; Rucene's [`BytesRef`] owns its
/// buffer, so each call yields an independent value and the end of iteration is
/// `Ok(None)`.
pub trait BytesRefIterator {
    /// Advances to the next value, returning `None` at the end of the
    /// iteration.
    ///
    /// # Errors
    ///
    /// Propagates low-level I/O errors, as Java's `IOException` does.
    fn next(&mut self) -> Result<Option<BytesRef>>;
}

/// A [`BytesRefIterator`] over zero values.
///
/// Port of the singleton `BytesRefIterator.EMPTY`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyBytesRefIterator;

impl BytesRefIterator for EmptyBytesRefIterator {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        Ok(None)
    }
}

/// A [`BytesRefIterator`] that also reports the ordinal of the value it just
/// returned.
///
/// Port of the nested interface `BytesRefArray.IndexedBytesRefIterator`.
pub trait IndexedBytesRefIterator: BytesRefIterator {
    /// Returns the ordinal of the value returned by the last
    /// [`BytesRefIterator::next`].
    fn ord(&self) -> usize;
}

// ---------------------------------------------------------------------------
// SortableBytesRefArray
// ---------------------------------------------------------------------------

/// An append-only collection of [`BytesRef`] values that can be iterated in
/// sorted order.
///
/// Port of the package-private interface
/// `org.apache.lucene.util.SortableBytesRefArray`, public here because Rust has
/// no package visibility between sibling modules.
pub trait SortableBytesRefArray {
    /// Appends a value and returns its ordinal.
    ///
    /// # Errors
    ///
    /// Returns an error when the value does not fit the array's constraints.
    fn append(&mut self, bytes: &BytesRef) -> Result<usize>;

    /// Clears every previously stored value.
    fn clear(&mut self);

    /// Returns the number of values appended so far.
    fn size(&self) -> usize;

    /// Sorts every value by `comp` and returns an iterator over the result.
    ///
    /// # Errors
    ///
    /// Propagates errors raised while materialising the iterator.
    fn sorted_iterator<'a>(
        &'a mut self,
        comp: StringSorterComparator<'a>,
    ) -> Result<Box<dyn BytesRefIterator + 'a>>;
}

// ---------------------------------------------------------------------------
// BytesRefArray
// ---------------------------------------------------------------------------

/// A simple append-only array of [`BytesRef`] values, with ordinal access and
/// sorted iteration.
///
/// Port of `org.apache.lucene.util.BytesRefArray`.
#[derive(Debug)]
pub struct BytesRefArray {
    /// Equivalent to `BytesRefArray.pool`, a `ByteBlockPool` over a
    /// `DirectTrackingAllocator` charging `bytes_used`.
    pool: ByteBlockPool,
    offsets: Vec<i32>,
    last_element: usize,
    current_offset: usize,
    bytes_used: Arc<Counter>,
}

impl BytesRefArray {
    /// Creates an array charging its allocations to `bytes_used`.
    ///
    /// Equivalent to `BytesRefArray(Counter)` (`BytesRefArray.java:40-45`):
    /// a `ByteBlockPool` over a `DirectTrackingAllocator(bytesUsed)`, advanced
    /// to its first block, plus `NUM_BYTES_ARRAY_HEADER * Integer.BYTES`
    /// charged for the offsets.
    pub fn new(bytes_used: Arc<Counter>) -> Self {
        let mut pool = ByteBlockPool::with_allocator(Box::new(DirectTrackingAllocator::new(
            Arc::clone(&bytes_used),
        )));
        pool.next_buffer()
            .expect("INVARIANT: the first block of an empty pool starts at offset 0");
        bytes_used.add_and_get(RamUsageEstimator::NUM_BYTES_ARRAY_HEADER * 4);
        Self {
            pool,
            offsets: vec![0; 1],
            last_element: 0,
            current_offset: 0,
            bytes_used,
        }
    }

    /// Returns the value at `index`.
    ///
    /// Equivalent to `BytesRefArray.get(BytesRefBuilder, int)`
    /// (`BytesRefArray.java:87-95`), which copies the value out of the pool
    /// with `readBytes`; Rucene's owning [`BytesRef`] makes the scratch builder
    /// unnecessary.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `index` is out of bounds,
    /// which is Java's `IndexOutOfBoundsException` from `Objects.checkIndex`.
    pub fn get(&self, index: usize) -> Result<BytesRef> {
        if index >= self.last_element {
            return Err(LuceneError::IllegalArgument(format!(
                "Index {index} out of bounds for length {}",
                self.last_element
            )));
        }
        let (offset, length) = self.offset_and_length(index);
        let mut bytes = vec![0u8; length];
        self.pool.read_bytes(offset as i64, &mut bytes, 0, length);
        Ok(BytesRef::new(bytes))
    }

    /// The offset and length of the value at `index`.
    fn offset_and_length(&self, index: usize) -> (usize, usize) {
        let offset = self.offsets[index] as usize;
        let length = if index == self.last_element - 1 {
            self.current_offset - offset
        } else {
            self.offsets[index + 1] as usize - offset
        };
        (offset, length)
    }

    /// Returns the value at `index`, which the caller has bounds-checked.
    ///
    /// Equivalent to the private `BytesRefArray.setBytesRef(BytesRefBuilder,
    /// BytesRef, int)` (`BytesRefArray.java:101-111`), which the sort and the
    /// iterator use: `pool.setBytesRef` references the block directly when the
    /// value lies in one block, and copies it through the builder otherwise.
    /// The owning [`BytesRef`] then takes a copy of the slice it returns.
    fn set_bytes_ref(&self, index: usize) -> BytesRef {
        let (offset, length) = self.offset_and_length(index);
        let mut builder = BytesRefBuilder::new();
        BytesRef::new(
            self.pool
                .set_bytes_ref(&mut builder, offset as i64, length)
                .to_vec(),
        )
    }

    /// Sorts the values and returns the resulting order.
    ///
    /// Equivalent to `BytesRefArray.sort(Comparator, boolean)`.
    pub fn sort(&self, comp: StringSorterComparator<'_>, stable: bool) -> SortState {
        let size = self.size();
        let ordered_entries: Vec<usize> = (0..size).collect();
        if stable {
            let mut ops = StableSortAdapter {
                array: self,
                ordered_entries,
                tmp: vec![0; size],
            };
            StableStringSorter::new(comp).sort(&mut ops, 0, size);
            SortState {
                indices: ops.ordered_entries,
            }
        } else {
            let mut ops = SortAdapter {
                array: self,
                ordered_entries,
            };
            StringSorter::new(comp).sort(&mut ops, 0, size);
            SortState {
                indices: ops.ordered_entries,
            }
        }
    }

    /// Returns an iterator over the values in insertion order.
    ///
    /// Equivalent to `BytesRefArray.iterator()`.
    pub fn iterator(&self) -> BytesRefArrayIterator<'_> {
        self.iterator_with_sort_state(None)
    }

    /// Returns an iterator following `sort_state`, or insertion order when it
    /// is `None`.
    ///
    /// Equivalent to `BytesRefArray.iterator(SortState)`.
    pub fn iterator_with_sort_state(
        &self,
        sort_state: Option<SortState>,
    ) -> BytesRefArrayIterator<'_> {
        let size = self.size();
        let indices = sort_state.map(|s| s.indices);
        debug_assert!(
            indices.as_ref().is_none_or(|i| i.len() == size),
            "{} != {size}",
            indices.as_ref().map_or(0, |i| i.len())
        );
        BytesRefArrayIterator {
            array: self,
            indices,
            size,
            pos: 0,
            ord: 0,
        }
    }
}

impl SortableBytesRefArray for BytesRefArray {
    fn append(&mut self, bytes: &BytesRef) -> Result<usize> {
        if self.last_element >= self.offsets.len() {
            let old_len = self.offsets.len();
            let target = ArrayUtil::oversize(old_len + 1, 4).max(old_len + 1);
            self.offsets.resize(target, 0);
            self.bytes_used
                .add_and_get((self.offsets.len() - old_len) as i64 * 4);
        }
        self.pool.append(bytes.slice())?;
        self.offsets[self.last_element] = self.current_offset as i32;
        self.last_element += 1;
        self.current_offset += bytes.length;
        Ok(self.last_element - 1)
    }

    fn clear(&mut self) {
        self.last_element = 0;
        self.current_offset = 0;
        self.offsets.iter_mut().for_each(|o| *o = 0);
        // `pool.reset(false, true)`: no need to zero-fill the buffers, the
        // allocator is ours.
        self.pool.reset(false, true);
    }

    fn size(&self) -> usize {
        self.last_element
    }

    fn sorted_iterator<'a>(
        &'a mut self,
        comp: StringSorterComparator<'a>,
    ) -> Result<Box<dyn BytesRefIterator + 'a>> {
        let state = self.sort(comp, false);
        Ok(Box::new(self.iterator_with_sort_state(Some(state))))
    }
}

/// The order [`BytesRefArray::sort`] produced.
///
/// Port of the nested class `BytesRefArray.SortState`.
#[derive(Debug, Clone)]
pub struct SortState {
    indices: Vec<usize>,
}

impl SortState {
    /// Returns the ordinals in sorted order.
    pub fn indices(&self) -> &[usize] {
        &self.indices
    }
}

impl Accountable for SortState {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + self.indices.len() as i64 * 4
    }
}

/// The iterator [`BytesRefArray::iterator_with_sort_state`] returns.
pub struct BytesRefArrayIterator<'a> {
    array: &'a BytesRefArray,
    indices: Option<Vec<usize>>,
    size: usize,
    pos: usize,
    ord: usize,
}

impl BytesRefIterator for BytesRefArrayIterator<'_> {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.pos < self.size {
            self.ord = match &self.indices {
                None => self.pos,
                Some(indices) => indices[self.pos],
            };
            self.pos += 1;
            Ok(Some(self.array.set_bytes_ref(self.ord)))
        } else {
            Ok(None)
        }
    }
}

impl IndexedBytesRefIterator for BytesRefArrayIterator<'_> {
    fn ord(&self) -> usize {
        self.ord
    }
}

/// The anonymous `StringSorter` of `BytesRefArray.sort`.
struct SortAdapter<'a> {
    array: &'a BytesRefArray,
    ordered_entries: Vec<usize>,
}

impl StringSorterOps for SortAdapter<'_> {
    fn get(&self, i: usize) -> BytesRef {
        self.array.set_bytes_ref(self.ordered_entries[i])
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.ordered_entries.swap(i, j);
    }
}

/// The anonymous `StableStringSorter` of `BytesRefArray.sort`.
struct StableSortAdapter<'a> {
    array: &'a BytesRefArray,
    ordered_entries: Vec<usize>,
    tmp: Vec<usize>,
}

impl StringSorterOps for StableSortAdapter<'_> {
    fn get(&self, i: usize) -> BytesRef {
        self.array.set_bytes_ref(self.ordered_entries[i])
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.ordered_entries.swap(i, j);
    }
}

impl StableStringSorterOps for StableSortAdapter<'_> {
    fn save(&mut self, i: usize, j: usize) {
        self.tmp[j] = self.ordered_entries[i];
    }

    fn restore(&mut self, i: usize, j: usize) {
        self.ordered_entries[i..j].copy_from_slice(&self.tmp[i..j]);
    }
}

// ---------------------------------------------------------------------------
// FixedLengthBytesRefArray
// ---------------------------------------------------------------------------

/// An append-only array of fixed-length [`BytesRef`] values.
///
/// Port of `org.apache.lucene.util.FixedLengthBytesRefArray`.
#[derive(Debug)]
pub struct FixedLengthBytesRefArray {
    value_length: usize,
    values_per_block: usize,
    size: usize,
    /// Index of the block being filled, or `-1` before the first allocation.
    current_block: i32,
    next_entry: usize,
    blocks: Vec<Vec<u8>>,
}

impl FixedLengthBytesRefArray {
    /// Creates an array of values that are always `value_length` bytes long.
    ///
    /// # Panics
    ///
    /// Panics when `value_length` is zero, which would make the block layout
    /// undefined; Java would divide by zero.
    pub fn new(value_length: usize) -> Self {
        assert!(value_length > 0, "value length must be > 0");
        // ~32K per page, unless each value is larger than 32K.
        let values_per_block = (32768 / value_length).max(1);
        Self {
            value_length,
            values_per_block,
            size: 0,
            current_block: -1,
            next_entry: values_per_block,
            blocks: Vec::new(),
        }
    }

    /// Returns the value at `index`.
    ///
    /// Equivalent to `FixedLengthBytesRefArray.get(BytesRef, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `index` is out of bounds.
    pub fn get(&self, index: usize) -> Result<BytesRef> {
        if index >= self.size {
            return Err(LuceneError::IllegalArgument(format!(
                "Index {index} out of bounds for length {}",
                self.size
            )));
        }
        Ok(self.value_at(index))
    }

    fn value_at(&self, index: usize) -> BytesRef {
        let block = index / self.values_per_block;
        let pos = (index % self.values_per_block) * self.value_length;
        BytesRef::new(self.blocks[block][pos..pos + self.value_length].to_vec())
    }

    /// Sorts the values and returns the resulting order.
    ///
    /// Equivalent to the private `FixedLengthBytesRefArray.sort`.
    fn sort_order(&self, comp: StringSorterComparator<'_>) -> Vec<usize> {
        let size = self.size();
        let mut ops = FixedSortAdapter {
            array: self,
            ordered_entries: (0..size).collect(),
        };
        StringSorter::new(comp).sort(&mut ops, 0, size);
        ops.ordered_entries
    }

    /// Returns an iterator over the values in the order `comp` defines.
    ///
    /// Equivalent to `FixedLengthBytesRefArray.iterator(Comparator)`.
    pub fn iterator(
        &self,
        comp: StringSorterComparator<'_>,
    ) -> FixedLengthBytesRefArrayIterator<'_> {
        FixedLengthBytesRefArrayIterator {
            array: self,
            indices: self.sort_order(comp),
            pos: 0,
        }
    }
}

impl SortableBytesRefArray for FixedLengthBytesRefArray {
    fn append(&mut self, bytes: &BytesRef) -> Result<usize> {
        if bytes.length != self.value_length {
            return Err(LuceneError::IllegalArgument(format!(
                "value length is {} but is supposed to always be {}",
                bytes.length, self.value_length
            )));
        }
        if self.next_entry == self.values_per_block {
            self.current_block += 1;
            if self.current_block as usize == self.blocks.len() {
                self.blocks
                    .push(vec![0u8; self.values_per_block * self.value_length]);
            } else {
                let block = self.current_block as usize;
                self.blocks[block] = vec![0u8; self.values_per_block * self.value_length];
            }
            self.next_entry = 0;
        }

        let block = self.current_block as usize;
        let pos = self.next_entry * self.value_length;
        self.blocks[block][pos..pos + self.value_length].copy_from_slice(bytes.slice());
        self.next_entry += 1;

        self.size += 1;
        Ok(self.size - 1)
    }

    fn clear(&mut self) {
        self.size = 0;
        self.blocks = Vec::new();
        self.current_block = -1;
        self.next_entry = self.values_per_block;
    }

    fn size(&self) -> usize {
        self.size
    }

    fn sorted_iterator<'a>(
        &'a mut self,
        comp: StringSorterComparator<'a>,
    ) -> Result<Box<dyn BytesRefIterator + 'a>> {
        Ok(Box::new(FixedLengthBytesRefArrayIterator {
            array: &*self,
            indices: self.sort_order(comp),
            pos: 0,
        }))
    }
}

/// The iterator [`FixedLengthBytesRefArray::iterator`] returns.
pub struct FixedLengthBytesRefArrayIterator<'a> {
    array: &'a FixedLengthBytesRefArray,
    indices: Vec<usize>,
    pos: usize,
}

impl BytesRefIterator for FixedLengthBytesRefArrayIterator<'_> {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.pos < self.indices.len() {
            let index = self.indices[self.pos];
            self.pos += 1;
            Ok(Some(self.array.value_at(index)))
        } else {
            Ok(None)
        }
    }
}

/// The anonymous `StringSorter` of `FixedLengthBytesRefArray.sort`.
struct FixedSortAdapter<'a> {
    array: &'a FixedLengthBytesRefArray,
    ordered_entries: Vec<usize>,
}

impl StringSorterOps for FixedSortAdapter<'_> {
    fn get(&self, i: usize) -> BytesRef {
        self.array.value_at(self.ordered_entries[i])
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.ordered_entries.swap(i, j);
    }
}

// ---------------------------------------------------------------------------
// BytesRefBlockPool
// ---------------------------------------------------------------------------

/// Stores length-prefixed [`BytesRef`] values in a
/// [`ByteBlockPool`], addressing each by the offset it was written at.
///
/// Port of `org.apache.lucene.util.BytesRefBlockPool`. The length prefix is one
/// byte below 128 and two big-endian bytes with the high bit set otherwise,
/// exactly as Lucene encodes it.
///
/// Rucene's [`ByteBlockPool`] already carries these operations — its module
/// documentation maps `add_bytes_ref` to `BytesRefBlockPool.addBytesRef` and
/// `term_bytes` to `BytesRefBlockPool.fillBytesRef` — so this type is the thin
/// wrapper Lucene declares, delegating to them rather than duplicating the
/// encoding.
#[derive(Debug)]
pub struct BytesRefBlockPool {
    byte_block_pool: ByteBlockPool,
}

impl Default for BytesRefBlockPool {
    fn default() -> Self {
        Self::new()
    }
}

impl BytesRefBlockPool {
    /// Creates a pool with its own byte arena.
    pub fn new() -> Self {
        Self::with_pool(ByteBlockPool::new(Arc::new(AtomicI64::new(0))))
    }

    /// Creates a pool over an existing byte arena.
    pub fn with_pool(byte_block_pool: ByteBlockPool) -> Self {
        Self { byte_block_pool }
    }

    /// Returns the underlying arena.
    pub fn byte_block_pool(&self) -> &ByteBlockPool {
        &self.byte_block_pool
    }

    /// Empties the pool.
    ///
    /// Equivalent to the package-private `BytesRefBlockPool.reset()`.
    pub fn reset(&mut self) {
        self.byte_block_pool.reset(false, false);
    }

    /// Returns the value written at `start`.
    ///
    /// Equivalent to `BytesRefBlockPool.fillBytesRef(BytesRef, int)`; Rucene's
    /// owning [`BytesRef`] makes the output parameter unnecessary.
    pub fn fill_bytes_ref(&self, start: i32) -> BytesRef {
        BytesRef::new(self.byte_block_pool.term_bytes(start).to_vec())
    }

    /// Writes `bytes` and returns the offset it was written at.
    ///
    /// # Errors
    ///
    /// Returns an error when the value plus its prefix does not fit in one
    /// block, which is Java's `BytesRefHash.MaxBytesLengthExceededException`.
    pub fn add_bytes_ref(&mut self, bytes: &BytesRef) -> Result<i32> {
        self.byte_block_pool.add_bytes_ref(bytes.slice())
    }

    /// Returns the hash of the value written at `start`.
    ///
    /// Equivalent to the package-private `BytesRefBlockPool.hash(int)`.
    pub fn hash(&self, start: i32) -> i32 {
        self.byte_block_pool.hash_at(start)
    }

    /// Returns whether the value written at `start` equals `b`.
    ///
    /// Equivalent to the package-private `BytesRefBlockPool.equals(int, BytesRef)`.
    pub fn equals(&self, start: i32, b: &BytesRef) -> bool {
        self.byte_block_pool.term_bytes(start) == b.slice()
    }
}

impl BytesRefBlockPool {
    /// Shallow size of the Java class: an object header and the
    /// `byteBlockPool` reference.
    ///
    /// Equivalent to the private `BytesRefBlockPool.BASE_RAM_BYTES`,
    /// `RamUsageEstimator.shallowSizeOfInstance(BytesRefBlockPool.class)`,
    /// which is 16 on a JVM with compressed references.
    pub const BASE_RAM_BYTES: i64 = RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + RamUsageEstimator::NUM_BYTES_OBJECT_REF,
    );
}

impl Accountable for BytesRefBlockPool {
    fn ram_bytes_used(&self) -> i64 {
        Self::BASE_RAM_BYTES + self.byte_block_pool.ram_bytes_used()
    }
}

#[cfg(test)]
mod byte_block_pool_tests {
    use super::*;

    /// rmp #299: `BytesRefArray` stores into a `ByteBlockPool` over a
    /// `DirectTrackingAllocator(bytesUsed)` and clears it with
    /// `pool.reset(false, true)` (`BytesRefArray.java:40-56`). Measured against
    /// `lucene-core-10.5.0.jar` (`oracle/passD/Oracle.java`, `BRA`): the
    /// counter reads 32832 new and after one short value, 65620 after a 40000
    /// byte value, 65684 after 22 values, 32916 once cleared, and 98452 after
    /// two 40000-byte values, the second of which reads back whole.
    #[test]
    fn the_counter_follows_javas_pool_and_allocator() {
        let counter = Arc::new(Counter::new_counter());
        let mut array = BytesRefArray::new(Arc::clone(&counter));
        assert_eq!(counter.get(), 32832);
        array.append(&BytesRef::new(b"a".to_vec())).unwrap();
        assert_eq!(counter.get(), 32832);
        let big = BytesRef::new(vec![7u8; 40000]);
        array.append(&big).unwrap();
        assert_eq!(counter.get(), 65620);
        for i in 0..20 {
            array
                .append(&BytesRef::new(format!("x{i}").into_bytes()))
                .unwrap();
        }
        assert_eq!((counter.get(), array.size()), (65684, 22));
        array.clear();
        assert_eq!(counter.get(), 32916);
        array.append(&big).unwrap();
        array.append(&big).unwrap();
        assert_eq!(counter.get(), 98452);
        assert_eq!(array.get(1).unwrap(), big);
        // The sort reads values through `setBytesRef`, across block boundaries.
        let compare = |a: &BytesRef, b: &BytesRef| a.cmp(b) as i32;
        let state = array.sort(StringSorterComparator::Generic(&compare), false);
        let mut it = array.iterator_with_sort_state(Some(state));
        assert_eq!(it.next().unwrap().unwrap(), big);
    }
}
