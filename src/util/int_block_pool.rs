//! Block pools and recycling allocators ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`IntBlockPool`] | `IntBlockPool` |
//! | [`IntBlockAllocator`] | `IntBlockPool.Allocator` |
//! | [`DirectIntAllocator`] | `IntBlockPool.DirectAllocator` |
//! | [`RecyclingIntBlockAllocator`] | `RecyclingIntBlockAllocator` |
//! | [`ByteBlockAllocator`] | `ByteBlockPool.Allocator` |
//! | [`RecyclingByteBlockAllocator`] | `RecyclingByteBlockAllocator` |
//!
//! **Divergence from Lucene 10.5.0.** [`crate::util::byte_block_pool`] folded
//! `ByteBlockPool.Allocator` into the pool itself, charging every block to a
//! shared `AtomicI64` and dropping released blocks rather than recycling them.
//! `RecyclingByteBlockAllocator` is a public Lucene class in its own right, so
//! it is ported here in full, together with the [`ByteBlockAllocator`] trait it
//! implements; nothing in the crate wires it into that pool yet.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::util::concurrent::Counter;
use crate::util::{ArrayUtil, RamUsageEstimator, BYTE_BLOCK_SIZE};

// ---------------------------------------------------------------------------
// IntBlockPool
// ---------------------------------------------------------------------------

/// Number of bits used to address an `i32` inside one block.
/// `IntBlockPool.INT_BLOCK_SHIFT`.
pub const INT_BLOCK_SHIFT: u32 = 13;
/// Number of `i32`s in each block. `IntBlockPool.INT_BLOCK_SIZE`.
pub const INT_BLOCK_SIZE: usize = 1 << INT_BLOCK_SHIFT;
/// Mask extracting the position of a global offset inside its block.
/// `IntBlockPool.INT_BLOCK_MASK`.
pub const INT_BLOCK_MASK: usize = INT_BLOCK_SIZE - 1;

/// Allocates and frees `i32` blocks.
///
/// Port of the abstract class `IntBlockPool.Allocator`.
pub trait IntBlockAllocator {
    /// The size in `i32`s of every block this allocator hands out.
    ///
    /// Equivalent to the protected field `blockSize`.
    fn block_size(&self) -> usize;

    /// Returns `blocks[start..end]` to the allocator, clearing those slots.
    fn recycle_int_blocks(&mut self, blocks: &mut [Option<Vec<i32>>], start: usize, end: usize);

    /// Returns a fresh block.
    fn get_int_block(&mut self) -> Vec<i32> {
        vec![0; self.block_size()]
    }
}

/// An [`IntBlockAllocator`] that never recycles.
///
/// Port of `IntBlockPool.DirectAllocator`.
#[derive(Debug, Clone, Copy)]
pub struct DirectIntAllocator {
    block_size: usize,
}

impl Default for DirectIntAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectIntAllocator {
    /// Creates an allocator handing out [`INT_BLOCK_SIZE`]-sized blocks.
    pub fn new() -> Self {
        Self {
            block_size: INT_BLOCK_SIZE,
        }
    }
}

impl IntBlockAllocator for DirectIntAllocator {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn recycle_int_blocks(&mut self, _blocks: &mut [Option<Vec<i32>>], _start: usize, _end: usize) {
    }
}

/// A pool of `i32` blocks, the sibling of
/// [`crate::util::byte_block_pool::ByteBlockPool`].
///
/// Port of `org.apache.lucene.util.IntBlockPool`.
///
/// **Divergence from Lucene 10.5.0.** Java exposes `public int[] buffer`, a
/// direct handle on the head block. Rust cannot hand out a long-lived
/// `&mut [i32]` into a `Vec<Vec<i32>>` and keep the pool usable, so blocks are
/// addressed by index through [`IntBlockPool::buffer`] and
/// [`IntBlockPool::buffer_mut`] — the same choice
/// [`crate::util::byte_block_pool`] already made.
pub struct IntBlockPool {
    /// Blocks currently used by the pool, allocated on demand.
    buffers: Vec<Option<Vec<i32>>>,
    /// Index of the head block, or `-1` before the first allocation.
    buffer_upto: i32,
    /// Next free position inside the head block.
    int_upto: usize,
    /// Global offset of the first `i32` of the head block.
    int_offset: i32,
    allocator: Box<dyn IntBlockAllocator>,
}

impl std::fmt::Debug for IntBlockPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntBlockPool")
            .field("buffer_upto", &self.buffer_upto)
            .field("int_upto", &self.int_upto)
            .field("int_offset", &self.int_offset)
            .finish()
    }
}

impl Default for IntBlockPool {
    fn default() -> Self {
        Self::new()
    }
}

impl IntBlockPool {
    /// Creates a pool with a [`DirectIntAllocator`].
    pub fn new() -> Self {
        Self::with_allocator(Box::new(DirectIntAllocator::new()))
    }

    /// Creates a pool with the given allocator.
    pub fn with_allocator(allocator: Box<dyn IntBlockAllocator>) -> Self {
        Self {
            // Java starts with `new int[10][]`.
            buffers: (0..10).map(|_| None).collect(),
            buffer_upto: -1,
            int_upto: INT_BLOCK_SIZE,
            int_offset: -(INT_BLOCK_SIZE as i32),
            allocator,
        }
    }

    /// Returns the next free position inside the head block.
    ///
    /// Equivalent to the public field `IntBlockPool.intUpto`.
    pub fn int_upto(&self) -> usize {
        self.int_upto
    }

    /// Sets the next free position inside the head block.
    pub fn set_int_upto(&mut self, int_upto: usize) {
        self.int_upto = int_upto;
    }

    /// Returns the global offset of the first `i32` of the head block.
    ///
    /// Equivalent to the public field `IntBlockPool.intOffset`.
    pub fn int_offset(&self) -> i32 {
        self.int_offset
    }

    /// Returns the index of the head block, or `-1` if no block was allocated.
    pub fn buffer_upto(&self) -> i32 {
        self.buffer_upto
    }

    /// Returns the number of block slots currently held.
    pub fn num_buffers(&self) -> usize {
        self.buffers.len()
    }

    /// Returns the block at `index`.
    ///
    /// # Panics
    ///
    /// Panics when the block was never allocated.
    pub fn buffer(&self, index: usize) -> &[i32] {
        self.buffers[index]
            .as_deref()
            .expect("INVARIANT: block index refers to an allocated block")
    }

    /// Returns the block at `index` mutably.
    ///
    /// # Panics
    ///
    /// Panics when the block was never allocated.
    pub fn buffer_mut(&mut self, index: usize) -> &mut [i32] {
        self.buffers[index]
            .as_deref_mut()
            .expect("INVARIANT: block index refers to an allocated block")
    }

    /// Resets the pool to its initial state, optionally reusing the first
    /// block.
    ///
    /// Blocks that are not reused go back to the allocator. When
    /// `zero_fill_buffers` is set, they are zeroed first, which a slice pool
    /// layered on top of this one relies on to find the non-zero end of a
    /// slice. When `reuse_first` is set, the first block is kept and
    /// [`IntBlockPool::next_buffer`] need not be called again.
    pub fn reset(&mut self, zero_fill_buffers: bool, reuse_first: bool) {
        if self.buffer_upto != -1 {
            // At least one block was allocated.
            let buffer_upto = self.buffer_upto as usize;

            if zero_fill_buffers {
                for i in 0..buffer_upto {
                    // Fully zero-fill the blocks that were fully used.
                    if let Some(buf) = self.buffers[i].as_mut() {
                        buf.iter_mut().for_each(|v| *v = 0);
                    }
                }
                // Partially zero-fill the final block.
                if let Some(buf) = self.buffers[buffer_upto].as_mut() {
                    buf[..self.int_upto].iter_mut().for_each(|v| *v = 0);
                }
            }

            if buffer_upto > 0 || !reuse_first {
                let offset = usize::from(reuse_first);
                // Recycle all but the first block.
                self.allocator
                    .recycle_int_blocks(&mut self.buffers, offset, 1 + buffer_upto);
                for slot in self.buffers[offset..=buffer_upto].iter_mut() {
                    *slot = None;
                }
            }
            if reuse_first {
                // Re-use the first block.
                self.buffer_upto = 0;
                self.int_upto = 0;
                self.int_offset = 0;
            } else {
                self.buffer_upto = -1;
                self.int_upto = INT_BLOCK_SIZE;
                self.int_offset = -(INT_BLOCK_SIZE as i32);
            }
        }
    }

    /// Advances the pool to its next block.
    ///
    /// Must be called once after construction to initialise the pool; after a
    /// [`IntBlockPool::reset`] with `reuse_first`, the pool is already on its
    /// first block.
    ///
    /// # Panics
    ///
    /// Panics when the global offset would overflow, which is Java's
    /// `Math.addExact`.
    pub fn next_buffer(&mut self) {
        if 1 + self.buffer_upto == self.buffers.len() as i32 {
            let new_len = (self.buffers.len() as f64 * 1.5) as usize;
            self.buffers
                .resize_with(new_len.max(self.buffers.len() + 1), || None);
        }
        let block = self.allocator.get_int_block();
        let index = (1 + self.buffer_upto) as usize;
        self.buffers[index] = Some(block);
        self.buffer_upto += 1;

        self.int_upto = 0;
        self.int_offset = self
            .int_offset
            .checked_add(INT_BLOCK_SIZE as i32)
            .expect("INVARIANT: IntBlockPool offset overflows, as Math.addExact would");
    }
}

// ---------------------------------------------------------------------------
// RecyclingIntBlockAllocator
// ---------------------------------------------------------------------------

/// Default number of blocks buffered by the recycling allocators.
///
/// `RecyclingIntBlockAllocator.DEFAULT_BUFFERED_BLOCKS` and
/// `RecyclingByteBlockAllocator.DEFAULT_BUFFERED_BLOCKS`.
pub const DEFAULT_BUFFERED_BLOCKS: usize = 64;

/// An [`IntBlockAllocator`] that recycles unused blocks.
///
/// Port of `org.apache.lucene.util.RecyclingIntBlockAllocator`. Not thread
/// safe, exactly as Lucene warns.
pub struct RecyclingIntBlockAllocator {
    block_size: usize,
    free_byte_blocks: Vec<Option<Vec<i32>>>,
    max_buffered_blocks: usize,
    free_blocks: usize,
    bytes_used: Arc<Counter>,
}

impl std::fmt::Debug for RecyclingIntBlockAllocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecyclingIntBlockAllocator")
            .field("block_size", &self.block_size)
            .field("max_buffered_blocks", &self.max_buffered_blocks)
            .field("free_blocks", &self.free_blocks)
            .finish()
    }
}

impl Default for RecyclingIntBlockAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl RecyclingIntBlockAllocator {
    /// Creates an allocator handing out `block_size`-sized blocks, buffering at
    /// most `max_buffered_blocks` of them and charging allocations to
    /// `bytes_used`.
    pub fn with_counter(
        block_size: usize,
        max_buffered_blocks: usize,
        bytes_used: Arc<Counter>,
    ) -> Self {
        Self {
            block_size,
            free_byte_blocks: (0..max_buffered_blocks).map(|_| None).collect(),
            max_buffered_blocks,
            free_blocks: 0,
            bytes_used,
        }
    }

    /// Creates an allocator with its own counter.
    pub fn with_block_size(block_size: usize, max_buffered_blocks: usize) -> Self {
        Self::with_counter(
            block_size,
            max_buffered_blocks,
            Arc::new(Counter::new_counter()),
        )
    }

    /// Creates an allocator with [`INT_BLOCK_SIZE`]-sized blocks and
    /// [`DEFAULT_BUFFERED_BLOCKS`] buffered blocks.
    pub fn new() -> Self {
        Self::with_counter(
            INT_BLOCK_SIZE,
            DEFAULT_BUFFERED_BLOCKS,
            Arc::new(Counter::new_counter()),
        )
    }

    /// Returns the number of currently buffered blocks.
    pub fn num_buffered_blocks(&self) -> usize {
        self.free_blocks
    }

    /// Returns the number of bytes currently allocated by this allocator.
    pub fn bytes_used(&self) -> i64 {
        self.bytes_used.get()
    }

    /// Returns the maximum number of buffered blocks.
    pub fn max_buffered_blocks(&self) -> usize {
        self.max_buffered_blocks
    }

    /// Removes up to `num` blocks from the buffer, returning how many were
    /// actually removed.
    pub fn free_blocks(&mut self, num: usize) -> usize {
        let (stop, count) = if num > self.free_blocks {
            (0, self.free_blocks)
        } else {
            (self.free_blocks - num, num)
        };
        while self.free_blocks > stop {
            self.free_blocks -= 1;
            self.free_byte_blocks[self.free_blocks] = None;
        }
        self.bytes_used
            .add_and_get(-((count as i64) * self.block_size as i64 * 4));
        debug_assert!(self.bytes_used.get() >= 0);
        count
    }
}

impl IntBlockAllocator for RecyclingIntBlockAllocator {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn get_int_block(&mut self) -> Vec<i32> {
        if self.free_blocks == 0 {
            self.bytes_used.add_and_get(self.block_size as i64 * 4);
            return vec![0; self.block_size];
        }
        self.free_blocks -= 1;
        self.free_byte_blocks[self.free_blocks]
            .take()
            .expect("INVARIANT: slots below free_blocks always hold a block")
    }

    fn recycle_int_blocks(&mut self, blocks: &mut [Option<Vec<i32>>], start: usize, end: usize) {
        let num_blocks = (self.max_buffered_blocks - self.free_blocks).min(end - start);
        let size = self.free_blocks + num_blocks;
        if size >= self.free_byte_blocks.len() {
            let target =
                ArrayUtil::oversize(size, RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize)
                    .max(size + 1);
            self.free_byte_blocks.resize_with(target, || None);
        }
        let stop = start + num_blocks;
        for block in blocks.iter_mut().take(stop).skip(start) {
            self.free_byte_blocks[self.free_blocks] = block.take();
            self.free_blocks += 1;
        }
        for block in blocks.iter_mut().take(end).skip(stop) {
            *block = None;
        }
        self.bytes_used
            .add_and_get(-((end - stop) as i64) * (self.block_size as i64 * 4));
        debug_assert!(self.bytes_used.get() >= 0);
    }
}

// ---------------------------------------------------------------------------
// RecyclingByteBlockAllocator
// ---------------------------------------------------------------------------

/// Allocates and frees `u8` blocks.
///
/// Port of the abstract class `ByteBlockPool.Allocator`. See the module
/// documentation for why it lives here rather than beside
/// [`crate::util::byte_block_pool::ByteBlockPool`].
pub trait ByteBlockAllocator {
    /// The size in bytes of every block this allocator hands out.
    fn block_size(&self) -> usize;

    /// Returns `blocks[start..end]` to the allocator, clearing those slots.
    fn recycle_byte_blocks(&mut self, blocks: &mut [Option<Vec<u8>>], start: usize, end: usize);

    /// Returns a fresh block.
    fn get_byte_block(&mut self) -> Vec<u8> {
        vec![0; self.block_size()]
    }
}

/// A [`ByteBlockAllocator`] that recycles unused byte blocks.
///
/// Port of `org.apache.lucene.util.RecyclingByteBlockAllocator`. Not thread
/// safe, exactly as Lucene warns.
pub struct RecyclingByteBlockAllocator {
    block_size: usize,
    free_byte_blocks: Vec<Option<Vec<u8>>>,
    max_buffered_blocks: usize,
    free_blocks: usize,
    bytes_used: Arc<Counter>,
}

impl std::fmt::Debug for RecyclingByteBlockAllocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecyclingByteBlockAllocator")
            .field("block_size", &self.block_size)
            .field("max_buffered_blocks", &self.max_buffered_blocks)
            .field("free_blocks", &self.free_blocks)
            .finish()
    }
}

impl Default for RecyclingByteBlockAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl RecyclingByteBlockAllocator {
    /// Creates an allocator buffering at most `max_buffered_blocks` blocks of
    /// [`BYTE_BLOCK_SIZE`] bytes, charging allocations to `bytes_used`.
    pub fn with_counter(max_buffered_blocks: usize, bytes_used: Arc<Counter>) -> Self {
        Self {
            block_size: BYTE_BLOCK_SIZE,
            free_byte_blocks: (0..max_buffered_blocks).map(|_| None).collect(),
            max_buffered_blocks,
            free_blocks: 0,
            bytes_used,
        }
    }

    /// Creates an allocator with its own counter.
    pub fn with_max_buffered_blocks(max_buffered_blocks: usize) -> Self {
        Self::with_counter(max_buffered_blocks, Arc::new(Counter::new_counter()))
    }

    /// Creates an allocator with [`DEFAULT_BUFFERED_BLOCKS`] buffered blocks.
    pub fn new() -> Self {
        Self::with_counter(DEFAULT_BUFFERED_BLOCKS, Arc::new(Counter::new_counter()))
    }

    /// Returns the number of currently buffered blocks.
    pub fn num_buffered_blocks(&self) -> usize {
        self.free_blocks
    }

    /// Returns the number of bytes currently allocated by this allocator.
    pub fn bytes_used(&self) -> i64 {
        self.bytes_used.get()
    }

    /// Returns the maximum number of buffered blocks.
    pub fn max_buffered_blocks(&self) -> usize {
        self.max_buffered_blocks
    }

    /// Removes up to `num` blocks from the buffer, returning how many were
    /// actually removed.
    pub fn free_blocks(&mut self, num: usize) -> usize {
        let (stop, count) = if num > self.free_blocks {
            (0, self.free_blocks)
        } else {
            (self.free_blocks - num, num)
        };
        while self.free_blocks > stop {
            self.free_blocks -= 1;
            self.free_byte_blocks[self.free_blocks] = None;
        }
        self.bytes_used
            .add_and_get(-((count as i64) * self.block_size as i64));
        debug_assert!(self.bytes_used.get() >= 0);
        count
    }
}

impl ByteBlockAllocator for RecyclingByteBlockAllocator {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn get_byte_block(&mut self) -> Vec<u8> {
        if self.free_blocks == 0 {
            self.bytes_used.add_and_get(self.block_size as i64);
            return vec![0; self.block_size];
        }
        self.free_blocks -= 1;
        self.free_byte_blocks[self.free_blocks]
            .take()
            .expect("INVARIANT: slots below free_blocks always hold a block")
    }

    fn recycle_byte_blocks(&mut self, blocks: &mut [Option<Vec<u8>>], start: usize, end: usize) {
        let num_blocks = (self.max_buffered_blocks - self.free_blocks).min(end - start);
        let size = self.free_blocks + num_blocks;
        if size >= self.free_byte_blocks.len() {
            let target =
                ArrayUtil::oversize(size, RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize)
                    .max(size + 1);
            self.free_byte_blocks.resize_with(target, || None);
        }
        let stop = start + num_blocks;
        for block in blocks.iter_mut().take(stop).skip(start) {
            self.free_byte_blocks[self.free_blocks] = block.take();
            self.free_blocks += 1;
        }
        for block in blocks.iter_mut().take(end).skip(stop) {
            *block = None;
        }
        self.bytes_used
            .add_and_get(-((end - stop) as i64) * self.block_size as i64);
        debug_assert!(self.bytes_used.get() >= 0);
    }
}
