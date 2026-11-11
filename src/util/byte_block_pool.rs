//! Block-based byte arena ported from `org.apache.lucene.util.ByteBlockPool`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`ByteBlockPool`] | `ByteBlockPool` |
//! | [`ByteBlockPool::add_bytes_ref`] | `BytesRefBlockPool.addBytesRef` |
//! | [`ByteBlockPool::term_bytes`] | `BytesRefBlockPool.fillBytesRef` |
//!
//! The pool is a growable list of fixed-size blocks of
//! [`BYTE_BLOCK_SIZE`] bytes. Callers append to the head block and, when it is
//! exhausted, advance to a fresh one with [`ByteBlockPool::next_buffer`]. A
//! *global offset* addresses any byte in the pool: the block index is
//! `offset >> BYTE_BLOCK_SHIFT` and the position inside that block is
//! `offset & BYTE_BLOCK_MASK`.
//!
//! Blocks are always zero-filled on allocation. The slice allocator built on
//! top of this pool (see `crate::index::freq_prox_terms_writer`) relies on that
//! invariant to detect the end of a slice, so blocks must never be handed out
//! with residual data.
//!
//! # Java to Rust adaptations
//!
//! * Lucene exposes `public byte[] buffer` — a direct reference to the head
//!   block — so that callers can read and write it without going through the
//!   pool. Rust cannot hand out a long-lived `&mut [u8]` into a `Vec<Vec<u8>>`
//!   while the pool itself stays usable, so this port addresses blocks by index
//!   ([`ByteBlockPool::buffer`], [`ByteBlockPool::buffer_mut`]). The generated
//!   code is equivalent: one bounds-checked index instead of a field load.
//! * Lucene's `Allocator` hierarchy hands out and takes back the pool's
//!   blocks. The pool holds one [`Allocator`]; [`DirectTrackingAllocator`] is
//!   the one Lucene's users install, charging every block to a counter and
//!   dropping the blocks it takes back. The indexing chain charges the shared
//!   `AtomicI64` that already accounts its RAM, where Java charges a
//!   `Counter`; [`DirectTrackingAllocator::from_atomic`] is that counter.
//! * Lucene's `buffers` is a `byte[][]` whose slots are `null` until a block is
//!   allocated into them, and which is grown — never shrunk — by
//!   `ArrayUtil.oversize`. A `Vec<Vec<u8>>` has no empty slots, so the length of
//!   that reference array is carried separately in `buffers_length` and grown by
//!   the same rule. It is not bookkeeping for its own sake:
//!   `ByteBlockPool.ramBytesUsed()` charges the *whole* reference array, empty
//!   slots included, so without it the estimate cannot reproduce Lucene's.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::util::concurrent::Counter;
use crate::util::{Accountable, ArrayUtil, BytesRefBuilder, RamUsageEstimator, StringHelper};

/// Hands out and takes back the byte blocks of a [`ByteBlockPool`].
///
/// Equivalent to the abstract class `ByteBlockPool.Allocator`
/// (`ByteBlockPool.java:57-71`). Java keeps `blockSize` as a protected field;
/// [`block_size`](Self::block_size) reads it here.
pub trait Allocator: std::fmt::Debug + Send + Sync {
    /// The size of the blocks this allocator hands out.
    ///
    /// Equivalent to the field `Allocator.blockSize`.
    fn block_size(&self) -> usize;

    /// Takes back the blocks in `blocks[start..end]`.
    ///
    /// Equivalent to `Allocator.recycleByteBlocks(byte[][], int, int)`. The
    /// pool drops the slots afterwards, as Java nulls them.
    fn recycle_byte_blocks(&self, blocks: &mut [Vec<u8>], start: usize, end: usize);

    /// Returns a new, zero-filled block.
    ///
    /// Equivalent to `Allocator.getByteBlock()`, `new byte[blockSize]`.
    fn get_byte_block(&self) -> Vec<u8> {
        vec![0u8; self.block_size()]
    }
}

/// An [`Allocator`] of [`BYTE_BLOCK_SIZE`] blocks that never recycles.
///
/// Equivalent to `ByteBlockPool.DirectAllocator` (`ByteBlockPool.java:74-83`).
#[derive(Debug, Default)]
pub struct DirectAllocator;

impl DirectAllocator {
    /// Equivalent to `new DirectAllocator()`, `super(BYTE_BLOCK_SIZE)`.
    pub fn new() -> Self {
        DirectAllocator
    }
}

impl Allocator for DirectAllocator {
    fn block_size(&self) -> usize {
        BYTE_BLOCK_SIZE
    }

    /// Equivalent to `DirectAllocator.recycleByteBlocks`, which does
    /// nothing.
    fn recycle_byte_blocks(&self, _blocks: &mut [Vec<u8>], _start: usize, _end: usize) {}
}

/// The counter a [`DirectTrackingAllocator`] charges.
#[derive(Debug)]
enum TrackingCounter {
    /// Java's `Counter`.
    Counter(Arc<Counter>),
    /// The `AtomicI64` the port's indexing chain accounts its RAM with, in
    /// place of Java's `Counter`.
    Atomic(Arc<AtomicI64>),
}

impl TrackingCounter {
    fn add_and_get(&self, delta: i64) -> i64 {
        match self {
            Self::Counter(counter) => counter.add_and_get(delta),
            Self::Atomic(atomic) => atomic.fetch_add(delta, Ordering::AcqRel) + delta,
        }
    }
}

/// An [`Allocator`] that never recycles, but tracks how much RAM is in use.
///
/// Equivalent to `ByteBlockPool.DirectTrackingAllocator`
/// (`ByteBlockPool.java:84-104`): it charges every block it hands out to its
/// counter and gives back the bytes of every block it takes back.
#[derive(Debug)]
pub struct DirectTrackingAllocator {
    block_size: usize,
    bytes_used: TrackingCounter,
}

impl DirectTrackingAllocator {
    /// Creates an allocator of [`BYTE_BLOCK_SIZE`] blocks charging
    /// `bytes_used`.
    ///
    /// Equivalent to `new DirectTrackingAllocator(Counter)`.
    pub fn new(bytes_used: Arc<Counter>) -> Self {
        Self {
            block_size: BYTE_BLOCK_SIZE,
            bytes_used: TrackingCounter::Counter(bytes_used),
        }
    }

    /// Creates an allocator of [`BYTE_BLOCK_SIZE`] blocks charging the
    /// `AtomicI64` the indexing chain accounts its RAM with.
    ///
    /// Equivalent to `new DirectTrackingAllocator(Counter)` over the chain's
    /// `bytesUsed` counter, which this port keeps as an `AtomicI64`.
    pub fn from_atomic(bytes_used: Arc<AtomicI64>) -> Self {
        Self {
            block_size: BYTE_BLOCK_SIZE,
            bytes_used: TrackingCounter::Atomic(bytes_used),
        }
    }
}

impl Allocator for DirectTrackingAllocator {
    fn block_size(&self) -> usize {
        self.block_size
    }

    /// Equivalent to `DirectTrackingAllocator.recycleByteBlocks`: gives back
    /// `(end - start) * blockSize` bytes and drops the blocks.
    fn recycle_byte_blocks(&self, blocks: &mut [Vec<u8>], start: usize, end: usize) {
        self.bytes_used
            .add_and_get(-(((end - start) * self.block_size) as i64));
        for block in &mut blocks[start..end] {
            *block = Vec::new();
        }
    }

    /// Equivalent to `DirectTrackingAllocator.getByteBlock()`: charges the
    /// block, then allocates it.
    fn get_byte_block(&self) -> Vec<u8> {
        self.bytes_used.add_and_get(self.block_size as i64);
        vec![0u8; self.block_size]
    }
}

/// Number of bits used to address a byte inside one block.
///
/// Equivalent to `ByteBlockPool.BYTE_BLOCK_SHIFT`.
pub const BYTE_BLOCK_SHIFT: u32 = 15;

/// Size in bytes of every block in the pool.
///
/// Equivalent to `ByteBlockPool.BYTE_BLOCK_SIZE`.
pub const BYTE_BLOCK_SIZE: usize = 1 << BYTE_BLOCK_SHIFT;

/// Number of block slots the pool starts with.
///
/// Equivalent to the initial length of `ByteBlockPool.buffers`, which Java
/// declares as `new byte[10][]`.
const INITIAL_BUFFERS_LENGTH: usize = 10;

/// Mask that extracts the position of a global offset inside its block.
///
/// Equivalent to `ByteBlockPool.BYTE_BLOCK_MASK`.
pub const BYTE_BLOCK_MASK: usize = BYTE_BLOCK_SIZE - 1;

/// Longest byte sequence [`ByteBlockPool::add_bytes_ref`] accepts.
///
/// Equivalent to `IndexWriter.MAX_TERM_LENGTH`: a term plus its 2-byte length
/// prefix must fit in a single block.
pub const MAX_TERM_LENGTH: usize = BYTE_BLOCK_SIZE - 2;

/// Seed used when hashing term bytes.
///
/// Lucene uses `StringHelper.GOOD_FAST_HASH_SEED`, which is randomised per JVM
/// (from `System.nanoTime()`) precisely to prove that nothing in the index
/// format depends on it: the seed only decides bucket placement inside the
/// in-memory hash table and never reaches a file. This port therefore uses a
/// fixed seed, which keeps the in-memory structure deterministic across runs
/// without changing a single output byte.
pub const TERM_HASH_SEED: i32 = 0;

/// A growable arena of fixed-size byte blocks.
///
/// Equivalent to `org.apache.lucene.util.ByteBlockPool`.
#[derive(Debug)]
pub struct ByteBlockPool {
    buffers: Vec<Vec<u8>>,
    /// Length of the `byte[][]` Lucene would be holding.
    ///
    /// Equivalent to `ByteBlockPool.buffers.length`. See the module-level
    /// adaptation notes: this port stores only the blocks it allocated, so the
    /// length of Lucene's reference array — which
    /// [`ram_bytes_used`](Accountable::ram_bytes_used) charges in full — is
    /// tracked here instead of being read off `buffers`.
    buffers_length: usize,
    /// Index of the head block, or `-1` before the first allocation.
    buffer_upto: i32,
    /// Next free position inside the head block.
    byte_upto: usize,
    /// Global offset of the first byte of the head block.
    byte_offset: i32,
    /// Equivalent to `ByteBlockPool.allocator`.
    allocator: Box<dyn Allocator>,
}

impl ByteBlockPool {
    /// Creates an empty pool that charges every allocated block to `bytes_used`.
    ///
    /// The pool starts with no block; the first write triggers
    /// [`Self::next_buffer`]. Equivalent to
    /// `new ByteBlockPool(new DirectTrackingAllocator(bytesUsed))` over the
    /// indexing chain's counter; see [`DirectTrackingAllocator::from_atomic`].
    pub fn new(bytes_used: Arc<AtomicI64>) -> Self {
        Self::with_allocator(Box::new(DirectTrackingAllocator::from_atomic(bytes_used)))
    }

    /// Creates an empty pool over `allocator`.
    ///
    /// Equivalent to `ByteBlockPool(Allocator)` (`ByteBlockPool.java:124-126`).
    pub fn with_allocator(allocator: Box<dyn Allocator>) -> Self {
        Self {
            buffers: Vec::new(),
            buffers_length: INITIAL_BUFFERS_LENGTH,
            buffer_upto: -1,
            byte_upto: BYTE_BLOCK_SIZE,
            byte_offset: -(BYTE_BLOCK_SIZE as i32),
            allocator,
        }
    }

    /// Returns the next free position inside the head block.
    ///
    /// Equivalent to the public field `ByteBlockPool.byteUpto`.
    pub fn byte_upto(&self) -> usize {
        self.byte_upto
    }

    /// Returns the global offset of the first byte of the head block.
    ///
    /// Equivalent to the public field `ByteBlockPool.byteOffset`.
    pub fn byte_offset(&self) -> i32 {
        self.byte_offset
    }

    /// Returns the length of the `byte[][] buffers` Lucene would be holding.
    ///
    /// Reads the private field `ByteBlockPool.buffers`' `length`, which
    /// `RamUsageTester` reaches by reflection.
    pub fn buffers_length(&self) -> usize {
        self.buffers_length
    }

    /// Returns the index of the head block, or `-1` if no block was allocated.
    pub fn buffer_upto(&self) -> i32 {
        self.buffer_upto
    }

    /// Advances `byte_upto` by `len` bytes inside the head block.
    ///
    /// # Panics
    ///
    /// Panics if the head block does not have `len` free bytes; callers must
    /// call [`Self::next_buffer`] first.
    pub fn advance(&mut self, len: usize) {
        debug_assert!(self.byte_upto + len <= BYTE_BLOCK_SIZE);
        self.byte_upto += len;
    }

    /// Returns the block with the given index.
    ///
    /// # Panics
    ///
    /// Panics if `index` addresses a block that was never allocated.
    pub fn buffer(&self, index: usize) -> &[u8] {
        &self.buffers[index]
    }

    /// Returns the block with the given index for mutation.
    ///
    /// # Panics
    ///
    /// Panics if `index` addresses a block that was never allocated.
    pub fn buffer_mut(&mut self, index: usize) -> &mut [u8] {
        &mut self.buffers[index]
    }

    /// Returns the byte at the given global offset.
    ///
    /// Equivalent to `org.apache.lucene.util.ByteBlockPool#readByte(long)`
    /// (`ByteBlockPool.java:335-339`), which `ByteBlockPoolReverseBytesReader`
    /// calls ([`ByteBlockPoolReverseBytesReader`](crate::util::fst::ByteBlockPoolReverseBytesReader)
    /// does). Java casts `offset >> BYTE_BLOCK_SHIFT` to an `int` unchecked;
    /// so does this.
    ///
    /// # Panics
    ///
    /// Panics if the offset addresses a block that was never allocated, where
    /// Java throws `ArrayIndexOutOfBoundsException`.
    pub fn read_byte(&self, offset: i64) -> u8 {
        let buffer_index = (offset >> BYTE_BLOCK_SHIFT) as i32;
        let pos = (offset & BYTE_BLOCK_MASK as i64) as usize;
        self.buffers[buffer_index as usize][pos]
    }

    /// Allocates a fresh zero-filled block and makes it the head block.
    ///
    /// Equivalent to `ByteBlockPool.nextBuffer()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::Arithmetic`](crate::error::LuceneErrorKind::Arithmetic) (`integer overflow`) when the pool
    /// would exceed the `i32` global-offset space: Java's
    /// `byteOffset = Math.addExact(byteOffset, BYTE_BLOCK_SIZE)`
    /// (`ByteBlockPool.java:187-200`) throws `ArithmeticException`. As there,
    /// the block is allocated and charged and the pool advanced to it before
    /// the offset overflows, so only `byteOffset` keeps its old value.
    pub fn next_buffer(&mut self) -> Result<()> {
        if 1 + self.buffer_upto == self.buffers_length as i32 {
            // The reference array is full - expand it, as `nextBuffer` does.
            self.buffers_length = ArrayUtil::oversize(
                self.buffers_length + 1,
                RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize,
            );
        }
        // `allocator.getByteBlock()`: the tracking allocator charges the block
        // as it hands it out.
        let block = self.allocator.get_byte_block();
        self.buffers.push(block);
        self.buffer_upto += 1;
        self.byte_upto = 0;
        self.byte_offset = self
            .byte_offset
            .checked_add(BYTE_BLOCK_SIZE as i32)
            .ok_or_else(LuceneError::integer_overflow)?;
        Ok(())
    }

    /// Ensures the head block has room for `size` more bytes, advancing to a
    /// new block when it does not.
    ///
    /// # Errors
    ///
    /// Propagates the overflow error of [`Self::next_buffer`].
    pub fn ensure_capacity(&mut self, size: usize) -> Result<()> {
        debug_assert!(size <= BYTE_BLOCK_SIZE);
        if self.byte_upto > BYTE_BLOCK_SIZE - size {
            self.next_buffer()?;
        }
        Ok(())
    }

    /// The bytes of the allocated blocks, which the allocator charged to the
    /// counter and [`reset`](Self::reset) gives back.
    pub fn charged_block_bytes(&self) -> i64 {
        (self.buffer_upto as i64 + 1) * BYTE_BLOCK_SIZE as i64
    }

    /// Resets the pool to its initial state, optionally reusing the first
    /// block.
    ///
    /// Equivalent to `ByteBlockPool.reset(boolean, boolean)`
    /// (`ByteBlockPool.java:147-185`). When `zero_fill_buffers` is set, the
    /// blocks that were used are zero-filled first — the head block only up
    /// to `byte_upto`. Every block that is not reused is handed back to the
    /// allocator with `recycleByteBlocks` and its slot dropped; with
    /// `reuse_first` the first block stays as the head, at offset 0, so no
    /// [`next_buffer`](Self::next_buffer) is needed before the next write.
    /// A pool that never allocated a block is left alone.
    pub fn reset(&mut self, zero_fill_buffers: bool, reuse_first: bool) {
        if self.buffer_upto == -1 {
            return;
        }
        // We allocated at least one buffer.
        let buffer_upto = self.buffer_upto as usize;
        if zero_fill_buffers {
            // Fully zero-fill the buffers that were fully used.
            for buffer in &mut self.buffers[..buffer_upto] {
                buffer.fill(0);
            }
            // Partially zero-fill the final buffer.
            self.buffers[buffer_upto][..self.byte_upto].fill(0);
        }
        if buffer_upto > 0 || !reuse_first {
            let offset = usize::from(reuse_first);
            // Recycle all but the first buffer.
            self.allocator
                .recycle_byte_blocks(&mut self.buffers, offset, 1 + buffer_upto);
            // `Arrays.fill(buffers, offset, 1 + bufferUpto, null)`.
            // `buffers_length` is left alone: Java nulls the slots but never
            // shrinks the reference array, so a reset pool still reports the
            // shallow size it grew to.
            self.buffers.truncate(offset);
        }
        if reuse_first {
            // Re-use the first buffer.
            self.buffer_upto = 0;
            self.byte_upto = 0;
            self.byte_offset = 0;
        } else {
            self.buffer_upto = -1;
            self.byte_upto = BYTE_BLOCK_SIZE;
            self.byte_offset = -(BYTE_BLOCK_SIZE as i32);
        }
    }

    /// Appends `bytes` prefixed by its length and returns the global offset of
    /// the prefix.
    ///
    /// Equivalent to `BytesRefBlockPool.addBytesRef(BytesRef)`. The length is
    /// stored in one byte when it is below 128 and otherwise in two big-endian
    /// bytes with the high bit of the first byte set. The value never straddles
    /// two blocks, so [`Self::term_bytes`] can return a borrowed slice.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::MaxBytesLengthExceeded`](crate::error::LuceneErrorKind::MaxBytesLengthExceeded),
    /// Lucene's `BytesRefHash.MaxBytesLengthExceededException`, when `bytes`
    /// is longer than [`MAX_TERM_LENGTH`].
    pub fn add_bytes_ref(&mut self, bytes: &[u8]) -> Result<i32> {
        let length = bytes.len();
        let prefix_len = if length < 128 { 1 } else { 2 };
        let total = length + prefix_len;
        if length > MAX_TERM_LENGTH {
            return Err(LuceneError::from(
                crate::error::LuceneErrorKind::MaxBytesLengthExceeded(format!(
                    "bytes can be at most {MAX_TERM_LENGTH} in length; got {length}"
                )),
            ));
        }
        // Lucene compares against `2 + length` for both branches, so a 127-byte
        // value that would fit with a 1-byte prefix still rolls over to a new
        // block when only 128 bytes remain. Keeping that comparison identical
        // keeps the two ports' block layouts identical.
        if length + 2 + self.byte_upto > BYTE_BLOCK_SIZE {
            self.next_buffer()?;
        }
        let buffer_index = self.buffer_upto as usize;
        let buffer_upto = self.byte_upto;
        let text_start = buffer_upto as i32 + self.byte_offset;
        let buffer = &mut self.buffers[buffer_index];
        if prefix_len == 1 {
            buffer[buffer_upto] = length as u8;
        } else {
            let encoded = (length as u16) | 0x8000;
            buffer[buffer_upto] = (encoded >> 8) as u8;
            buffer[buffer_upto + 1] = (encoded & 0xFF) as u8;
        }
        buffer[buffer_upto + prefix_len..buffer_upto + total].copy_from_slice(bytes);
        self.byte_upto += total;
        Ok(text_start)
    }

    /// Appends `bytes` at the current position.
    ///
    /// Equivalent to `ByteBlockPool.append(byte[], int, int)`, which fills the
    /// head block, moves to the next one, and repeats until nothing is left.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`next_buffer`](Self::next_buffer) raises.
    pub fn append(&mut self, bytes: &[u8]) -> Result<()> {
        let mut bytes_left = bytes.len();
        let mut offset = 0usize;
        while bytes_left > 0 {
            let buffer_left = BYTE_BLOCK_SIZE - self.byte_upto;
            if bytes_left < buffer_left {
                // Fits within the current buffer.
                let buffer = &mut self.buffers[self.buffer_upto as usize];
                buffer[self.byte_upto..self.byte_upto + bytes_left]
                    .copy_from_slice(&bytes[offset..offset + bytes_left]);
                self.byte_upto += bytes_left;
                break;
            }
            // Fill up this buffer and move to the next one.
            if buffer_left > 0 {
                let buffer = &mut self.buffers[self.buffer_upto as usize];
                buffer[self.byte_upto..self.byte_upto + buffer_left]
                    .copy_from_slice(&bytes[offset..offset + buffer_left]);
                bytes_left -= buffer_left;
                offset += buffer_left;
            }
            self.next_buffer()?;
        }
        Ok(())
    }

    /// Appends `length` bytes taken from `src_pool` at `src_offset`.
    ///
    /// Equivalent to `ByteBlockPool.append(ByteBlockPool, long, int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`next_buffer`](Self::next_buffer) raises.
    pub fn append_pool(
        &mut self,
        src_pool: &ByteBlockPool,
        mut src_offset: i64,
        length: usize,
    ) -> Result<()> {
        let mut bytes_left = length;
        while bytes_left > 0 {
            let buffer_left = BYTE_BLOCK_SIZE - self.byte_upto;
            if bytes_left < buffer_left {
                // Fits within the current buffer.
                self.append_bytes_single_buffer(src_pool, src_offset, bytes_left);
                break;
            }
            // Fill up this buffer and move to the next one.
            if buffer_left > 0 {
                self.append_bytes_single_buffer(src_pool, src_offset, buffer_left);
                bytes_left -= buffer_left;
                src_offset += buffer_left as i64;
            }
            self.next_buffer()?;
        }
        Ok(())
    }

    /// Copies `length` bytes from `src_pool` into the head block.
    ///
    /// Equivalent to the private
    /// `ByteBlockPool.appendBytesSingleBuffer(ByteBlockPool, long, int)`. The
    /// bytes copied must fit in what is left of the head block, but they may
    /// span several blocks of the source, which is why this loops.
    fn append_bytes_single_buffer(
        &mut self,
        src_pool: &ByteBlockPool,
        mut src_offset: i64,
        mut length: usize,
    ) {
        debug_assert!(length <= BYTE_BLOCK_SIZE - self.byte_upto);
        while length > 0 {
            let src_block = (src_offset >> BYTE_BLOCK_SHIFT) as usize;
            let src_pos = (src_offset as usize) & BYTE_BLOCK_MASK;
            let bytes_to_copy = length.min(BYTE_BLOCK_SIZE - src_pos);
            let buffer = &mut self.buffers[self.buffer_upto as usize];
            buffer[self.byte_upto..self.byte_upto + bytes_to_copy]
                .copy_from_slice(&src_pool.buffers[src_block][src_pos..src_pos + bytes_to_copy]);
            length -= bytes_to_copy;
            src_offset += bytes_to_copy as i64;
            self.byte_upto += bytes_to_copy;
        }
    }

    /// Reads `bytes_length` bytes out of the pool starting at the global
    /// `offset` into `bytes` at `bytes_offset`.
    ///
    /// Note: this method allows to copy across block boundaries.
    ///
    /// Equivalent to `org.apache.lucene.util.ByteBlockPool#readBytes`.
    ///
    /// # Panics
    ///
    /// Panics if the range addresses a block that was never allocated, or if
    /// `bytes` is too short to receive `bytes_length` bytes at `bytes_offset`.
    pub fn read_bytes(
        &self,
        offset: i64,
        bytes: &mut [u8],
        mut bytes_offset: usize,
        bytes_length: usize,
    ) {
        let mut bytes_left = bytes_length;
        let mut buffer_index = (offset >> BYTE_BLOCK_SHIFT) as usize;
        let mut pos = (offset as usize) & BYTE_BLOCK_MASK;
        while bytes_left > 0 {
            let buffer = &self.buffers[buffer_index];
            buffer_index += 1;
            let chunk = bytes_left.min(BYTE_BLOCK_SIZE - pos);
            bytes[bytes_offset..bytes_offset + chunk].copy_from_slice(&buffer[pos..pos + chunk]);
            bytes_offset += chunk;
            bytes_left -= chunk;
            pos = 0;
        }
    }

    /// Returns the `length` bytes stored at the global `offset`, without
    /// copying them when they live in a single block; otherwise it copies them
    /// into `builder`, grown as needed, and returns those.
    ///
    /// Equivalent to `ByteBlockPool#setBytesRef`
    /// (`ByteBlockPool.java:207-224`). Like Java, the common case references
    /// the block directly and the uncommon one — a slice spanning at least two
    /// blocks — goes through `builder` with `growNoCopy` and `readBytes`,
    /// leaving the builder's own length untouched.
    ///
    /// **Divergence from Lucene 10.5.0.** Java fills a caller-supplied
    /// `BytesRef result`, pointing its `bytes`/`offset`/`length` at the block
    /// or at the builder's array. Rucene's [`BytesRef`](crate::util::BytesRef)
    /// owns its buffer and cannot point into the pool, so the same view is
    /// returned as a slice of exactly `length` bytes, borrowed from whichever
    /// of the two Java would have pointed at.
    ///
    /// # Panics
    ///
    /// Panics if `offset >> BYTE_BLOCK_SHIFT` does not fit an `int`, which is
    /// Java's `ArithmeticException` from `Math.toIntExact`, or if the range
    /// addresses a block that was never allocated.
    pub fn set_bytes_ref<'a>(
        &'a self,
        builder: &'a mut BytesRefBuilder,
        offset: i64,
        length: usize,
    ) -> &'a [u8] {
        let buffer_index = i32::try_from(offset >> BYTE_BLOCK_SHIFT)
            .expect("integer overflow: Math.toIntExact(offset >> BYTE_BLOCK_SHIFT)");
        let buffer = &self.buffers[buffer_index as usize];
        let pos = (offset as usize) & BYTE_BLOCK_MASK;
        if pos + length <= BYTE_BLOCK_SIZE {
            // Common case: the slice lives in a single block. Reference the
            // buffer directly.
            &buffer[pos..pos + length]
        } else {
            // Uncommon case: the slice spans at least two blocks, so the bytes
            // must be copied.
            builder.grow_no_copy(length);
            self.read_bytes(offset, &mut builder.inner.bytes, 0, length);
            &builder.inner.bytes[..length]
        }
    }

    /// Returns the bytes previously stored at `start` by [`Self::add_bytes_ref`].
    ///
    /// Equivalent to `BytesRefBlockPool.fillBytesRef(BytesRef, int)`.
    ///
    /// # Panics
    ///
    /// Panics if `start` was not produced by [`Self::add_bytes_ref`] on this
    /// pool.
    pub fn term_bytes(&self, start: i32) -> &[u8] {
        let start = start as usize;
        let buffer = &self.buffers[start >> BYTE_BLOCK_SHIFT];
        let pos = start & BYTE_BLOCK_MASK;
        if buffer[pos] & 0x80 == 0 {
            let len = buffer[pos] as usize;
            &buffer[pos + 1..pos + 1 + len]
        } else {
            let len = ((((buffer[pos] as u16) << 8) | buffer[pos + 1] as u16) & 0x7FFF) as usize;
            &buffer[pos + 2..pos + 2 + len]
        }
    }

    /// Returns the hash of the bytes stored at `start`.
    ///
    /// Equivalent to `BytesRefBlockPool.hash(int)`.
    ///
    /// # Panics
    ///
    /// Panics if `start` was not produced by [`Self::add_bytes_ref`].
    pub fn hash_at(&self, start: i32) -> i32 {
        hash_bytes(self.term_bytes(start))
    }

    /// Returns the current position (in absolute value) of this byte pool.
    ///
    /// Equivalent to `org.apache.lucene.util.ByteBlockPool#getPosition`.
    pub fn position(&self) -> i64 {
        self.buffer_upto as i64 * BYTE_BLOCK_SIZE as i64 + self.byte_upto as i64
    }
}

impl ByteBlockPool {
    /// Shallow size of the Java class, the fixed part of
    /// [`ByteBlockPool::ram_bytes_used`](Accountable::ram_bytes_used).
    ///
    /// Equivalent to the private `ByteBlockPool.BASE_RAM_BYTES`, which Java
    /// computes as
    /// `RamUsageEstimator.shallowSizeOfInstance(ByteBlockPool.class)`: an object
    /// header, the three references `buffers`, `buffer` and `allocator`, and the
    /// three `int` fields `bufferUpto`, `byteUpto` and `byteOffset`. Measured
    /// against `lucene-core-10.5.0.jar` on a JVM with compressed references,
    /// that is 40 bytes.
    pub const BASE_RAM_BYTES: i64 = RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + 3 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + 3 * 4,
    );
}

impl Accountable for ByteBlockPool {
    /// Returns an estimate of the heap this pool occupies.
    ///
    /// Equivalent to `org.apache.lucene.util.ByteBlockPool#ramBytesUsed`
    /// (`ByteBlockPool.java:341-348`), which is `BASE_RAM_BYTES` plus
    /// `RamUsageEstimator.shallowSizeOf(buffers)` plus
    /// `RamUsageEstimator.sizeOfObject(buf)` for every block.
    ///
    /// Three parts of that are easy to lose, and each one changes the answer:
    /// the shallow size of the pool object itself; the *whole* reference array,
    /// including the slots that hold no block and the ones a
    /// [`reset`](Self::reset) emptied; and the 16-byte array header every block
    /// carries, which is why the estimate is never a whole number of blocks.
    /// A pool holding a single block reports 32880, not 32768.
    ///
    /// This is an *estimate of heap*, not the allocator's charge: what the pool
    /// adds to the `Counter` it shares with the rest of the indexing chain is
    /// exactly [`BYTE_BLOCK_SIZE`] per block. The two numbers are different and
    /// are not interchangeable.
    fn ram_bytes_used(&self) -> i64 {
        let mut size = Self::BASE_RAM_BYTES;
        // `RamUsageEstimator.shallowSizeOf(byte[][])` for the reference array.
        size += RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_ARRAY_HEADER
                + RamUsageEstimator::NUM_BYTES_OBJECT_REF * self.buffers_length as i64,
        );
        for buffer in &self.buffers {
            size += RamUsageEstimator::size_of(buffer);
        }
        size
    }
}

/// Hashes a byte sequence the way `BytesRefHash.doHash` does.
///
/// Equivalent to
/// `StringHelper.murmurhash3_x86_32(bytes, offset, length, GOOD_FAST_HASH_SEED)`;
/// see [`TERM_HASH_SEED`] for why the seed differs.
pub fn hash_bytes(bytes: &[u8]) -> i32 {
    StringHelper::murmurhash3_x86_32(bytes, 0, bytes.len() as i32, TERM_HASH_SEED)
}

#[cfg(test)]
mod next_buffer_overflow_tests {
    use super::*;
    use crate::error::LuceneErrorKind;

    /// `nextBuffer` allocates and charges the block and advances `bufferUpto`
    /// and `byteUpto` before `byteOffset = Math.addExact(byteOffset,
    /// BYTE_BLOCK_SIZE)` overflows (`ByteBlockPool.java:187-200`). Measured
    /// against `lucene-core-10.5.0.jar` (`oracle/fix/MiscFix.java`, item 9):
    /// `ArithmeticException: integer overflow`, then `byteOffset=2147450880
    /// byteUpto=0 charged=32768`.
    #[test]
    fn an_overflowing_next_buffer_is_an_arithmetic_error_after_the_allocation() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        pool.next_buffer().unwrap();
        pool.byte_upto = 7;
        pool.byte_offset = i32::MAX - BYTE_BLOCK_SIZE as i32 + 1;
        let charged_before = bytes_used.load(Ordering::Acquire);
        let error = pool.next_buffer().unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Arithmetic(message), .. } if message == "integer overflow"),
            "{error:?}"
        );
        assert_eq!(
            (
                pool.byte_offset,
                pool.byte_upto,
                pool.buffer_upto,
                bytes_used.load(Ordering::Acquire) - charged_before
            ),
            (2_147_450_880, 0, 1, BYTE_BLOCK_SIZE as i64)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::LuceneErrorKind;

    fn pool() -> ByteBlockPool {
        ByteBlockPool::new(Arc::new(AtomicI64::new(0)))
    }

    #[test]
    fn new_pool_starts_with_no_block() {
        let pool = pool();
        assert_eq!(pool.buffer_upto(), -1);
        assert_eq!(pool.byte_upto(), BYTE_BLOCK_SIZE);
        assert_eq!(pool.byte_offset(), -(BYTE_BLOCK_SIZE as i32));
    }

    #[test]
    fn next_buffer_allocates_zero_filled_block_and_accounts_ram() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        pool.next_buffer().expect("first block");
        assert_eq!(pool.buffer_upto(), 0);
        assert_eq!(pool.byte_upto(), 0);
        assert_eq!(pool.byte_offset(), 0);
        assert_eq!(bytes_used.load(Ordering::Acquire), BYTE_BLOCK_SIZE as i64);
        assert!(pool.buffer(0).iter().all(|b| *b == 0));

        pool.next_buffer().expect("second block");
        assert_eq!(pool.buffer_upto(), 1);
        assert_eq!(pool.byte_offset(), BYTE_BLOCK_SIZE as i32);
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            2 * BYTE_BLOCK_SIZE as i64
        );
    }

    #[test]
    fn reset_frees_every_block_and_uncharges_ram() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        pool.next_buffer().expect("block");
        pool.next_buffer().expect("block");
        pool.reset(false, false);
        assert_eq!(pool.buffer_upto(), -1);
        assert_eq!(bytes_used.load(Ordering::Acquire), 0);
        // Resetting an untouched pool is a no-op.
        pool.reset(false, false);
        assert_eq!(bytes_used.load(Ordering::Acquire), 0);
    }

    #[test]
    fn add_bytes_ref_round_trips_short_and_long_values() {
        let mut pool = pool();
        let short = b"lucene".to_vec();
        let long = vec![7u8; 300];
        let short_start = pool.add_bytes_ref(&short).expect("short");
        let long_start = pool.add_bytes_ref(&long).expect("long");
        assert_eq!(pool.term_bytes(short_start), &short[..]);
        assert_eq!(pool.term_bytes(long_start), &long[..]);
        // A 1-byte length prefix for < 128 and a 2-byte prefix otherwise.
        assert_eq!(long_start - short_start, short.len() as i32 + 1);
    }

    #[test]
    fn add_bytes_ref_stores_boundary_lengths_verbatim() {
        let mut pool = pool();
        for len in [0usize, 1, 126, 127, 128, 129, 1000] {
            let value = vec![(len % 251) as u8; len];
            let start = pool.add_bytes_ref(&value).expect("value");
            assert_eq!(pool.term_bytes(start), &value[..], "length {len}");
        }
    }

    #[test]
    fn add_bytes_ref_never_straddles_two_blocks() {
        let mut pool = pool();
        let value = vec![3u8; 1000];
        let mut starts = Vec::new();
        for _ in 0..100 {
            starts.push(pool.add_bytes_ref(&value).expect("value"));
        }
        assert!(pool.buffer_upto() > 0, "test must span several blocks");
        for start in starts {
            assert_eq!(pool.term_bytes(start), &value[..]);
        }
    }

    #[test]
    fn add_bytes_ref_rejects_values_longer_than_max_term_length() {
        let mut pool = pool();
        let too_long = vec![0u8; MAX_TERM_LENGTH + 1];
        let error = pool.add_bytes_ref(&too_long).expect_err("must be rejected");
        assert!(matches!(
            error,
            LuceneError {
                kind: LuceneErrorKind::MaxBytesLengthExceeded(_),
                ..
            }
        ));
        // The longest accepted value still round-trips.
        let longest = vec![1u8; MAX_TERM_LENGTH];
        let start = pool.add_bytes_ref(&longest).expect("longest");
        assert_eq!(pool.term_bytes(start), &longest[..]);
    }

    #[test]
    fn hash_at_matches_hashing_the_bytes_directly() {
        let mut pool = pool();
        let value = b"quick brown fox".to_vec();
        let start = pool.add_bytes_ref(&value).expect("value");
        assert_eq!(pool.hash_at(start), hash_bytes(&value));
    }

    #[test]
    fn byte_at_reads_across_block_boundaries() {
        let mut pool = pool();
        pool.next_buffer().expect("block");
        pool.buffer_mut(0)[BYTE_BLOCK_SIZE - 1] = 42;
        pool.next_buffer().expect("block");
        pool.buffer_mut(1)[0] = 43;
        assert_eq!(pool.read_byte(BYTE_BLOCK_SIZE as i64 - 1), 42);
        assert_eq!(pool.read_byte(BYTE_BLOCK_SIZE as i64), 43);
    }

    /// `readByte(long)` reads one byte at a global offset, across block
    /// boundaries, and agrees with `readBytes` over the same range.
    #[test]
    fn read_byte_reads_one_byte_at_any_global_offset() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(bytes_used);

        // Fill two whole blocks plus a little, with a position-dependent value
        // so a misread offset cannot pass.
        let total = BYTE_BLOCK_SIZE * 2 + 37;
        let payload: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
        pool.append(&payload).expect("append");

        for offset in 0..total {
            assert_eq!(
                pool.read_byte(offset as i64),
                payload[offset],
                "offset={offset}"
            );
        }

        // The same bytes through the bulk reader.
        let mut buf = vec![0u8; total];
        pool.read_bytes(0, &mut buf, 0, total);
        assert_eq!(buf, payload);

        // The block boundaries specifically.
        assert_eq!(
            pool.read_byte(BYTE_BLOCK_SIZE as i64 - 1),
            payload[BYTE_BLOCK_SIZE - 1]
        );
        assert_eq!(
            pool.read_byte(BYTE_BLOCK_SIZE as i64),
            payload[BYTE_BLOCK_SIZE]
        );
    }
}

#[cfg(test)]
mod reset_tests {
    use super::*;

    /// rmp #299: `reset(zeroFillBuffers, reuseFirst)` (`ByteBlockPool.java:147-185`)
    /// keeps the first block as the head at offset 0 when `reuseFirst`, hands
    /// every other block back to the allocator — which gives back their bytes —
    /// and zero-fills the used part of the blocks when asked to.
    #[test]
    fn reset_reuses_the_first_block_and_recycles_the_rest() {
        let counter = Arc::new(Counter::new_counter());
        let mut pool = ByteBlockPool::with_allocator(Box::new(DirectTrackingAllocator::new(
            Arc::clone(&counter),
        )));
        pool.next_buffer().unwrap();
        pool.append(&vec![9u8; BYTE_BLOCK_SIZE + 10]).unwrap();
        assert_eq!(counter.get(), 2 * BYTE_BLOCK_SIZE as i64);
        pool.reset(true, true);
        assert_eq!(
            (pool.buffer_upto(), pool.byte_upto(), pool.byte_offset()),
            (0, 0, 0)
        );
        assert_eq!(counter.get(), BYTE_BLOCK_SIZE as i64);
        assert!(pool.buffer(0).iter().all(|&b| b == 0));
        // Without zero-filling, the reused block keeps what it held.
        pool.append(&[5, 6, 7]).unwrap();
        pool.reset(false, true);
        assert_eq!(&pool.buffer(0)[..3], &[5, 6, 7]);
        // Without reuse, every block goes back.
        pool.reset(false, false);
        assert_eq!(
            (pool.buffer_upto(), pool.byte_upto(), pool.byte_offset()),
            (-1, BYTE_BLOCK_SIZE, -(BYTE_BLOCK_SIZE as i32))
        );
        assert_eq!(counter.get(), 0);
    }
}
