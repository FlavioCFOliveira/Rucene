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
//! * Lucene's `Allocator` hierarchy exists to recycle `byte[]` blocks across
//!   `DocumentsWriterPerThread` instances and to charge them to a `Counter`.
//!   Only `DirectTrackingAllocator` is used by the indexing chain, so this port
//!   folds it into the pool: allocation is charged to the shared
//!   `AtomicI64` that already accounts indexing RAM, and released blocks are
//!   dropped rather than recycled, which in Rust costs one `free` instead of
//!   keeping a global block cache alive.
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
use crate::util::{Accountable, ArrayUtil, BytesRefBuilder, RamUsageEstimator, StringHelper};

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
    bytes_used: Arc<AtomicI64>,
}

impl ByteBlockPool {
    /// Creates an empty pool that charges every allocated block to `bytes_used`.
    ///
    /// The pool starts with no block; the first write triggers
    /// [`Self::next_buffer`].
    pub fn new(bytes_used: Arc<AtomicI64>) -> Self {
        Self {
            buffers: Vec::new(),
            buffers_length: INITIAL_BUFFERS_LENGTH,
            buffer_upto: -1,
            byte_upto: BYTE_BLOCK_SIZE,
            byte_offset: -(BYTE_BLOCK_SIZE as i32),
            bytes_used,
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
    /// (`ByteBlockPool.java:335-339`).
    ///
    /// Java's parameter is a `long`; this port takes an `i32` because its
    /// global offsets are bounded by `i32`
    /// ([`next_buffer`](Self::next_buffer) refuses to grow past that), so a
    /// wider parameter could only carry an offset the pool cannot hold.
    ///
    /// # Panics
    ///
    /// Panics if the offset addresses a block that was never allocated.
    pub fn read_byte(&self, offset: i32) -> u8 {
        let offset = offset as usize;
        self.buffers[offset >> BYTE_BLOCK_SHIFT][offset & BYTE_BLOCK_MASK]
    }

    /// Allocates a fresh zero-filled block and makes it the head block.
    ///
    /// Equivalent to `ByteBlockPool.nextBuffer()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::ResourceLimit`] when the pool would exceed the
    /// `i32` global-offset space, which is Lucene's `Math.addExact` overflow.
    pub fn next_buffer(&mut self) -> Result<()> {
        let next_offset = self
            .byte_offset
            .checked_add(BYTE_BLOCK_SIZE as i32)
            .ok_or_else(|| {
                LuceneError::ResourceLimit(
                    "ByteBlockPool exceeded the addressable 2 GB of buffered postings".to_string(),
                )
            })?;
        if 1 + self.buffer_upto == self.buffers_length as i32 {
            // The reference array is full - expand it, as `nextBuffer` does.
            self.buffers_length = ArrayUtil::oversize(
                self.buffers_length + 1,
                RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize,
            );
        }
        self.buffers.push(vec![0u8; BYTE_BLOCK_SIZE]);
        self.buffer_upto += 1;
        self.byte_upto = 0;
        self.byte_offset = next_offset;
        self.bytes_used
            .fetch_add(BYTE_BLOCK_SIZE as i64, Ordering::AcqRel);
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

    /// Drops every block and returns the pool to its initial state.
    ///
    /// Equivalent to `ByteBlockPool.reset(false, false)`. Blocks are always
    /// freed rather than recycled; see the module-level adaptation notes.
    pub fn reset(&mut self) {
        if self.buffer_upto == -1 {
            return;
        }
        let released = (self.buffer_upto as i64 + 1) * BYTE_BLOCK_SIZE as i64;
        self.bytes_used.fetch_sub(released, Ordering::AcqRel);
        // `buffers_length` is deliberately left alone: Java nulls the slots and
        // recycles the blocks but never shrinks the reference array, so a reset
        // pool still reports the larger shallow size it grew to.
        self.buffers.clear();
        self.buffer_upto = -1;
        self.byte_upto = BYTE_BLOCK_SIZE;
        self.byte_offset = -(BYTE_BLOCK_SIZE as i32);
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
    /// Returns [`LuceneError::IllegalArgument`] when `bytes` is longer than
    /// [`MAX_TERM_LENGTH`], which is Lucene's
    /// `BytesRefHash.MaxBytesLengthExceededException`.
    pub fn add_bytes_ref(&mut self, bytes: &[u8]) -> Result<i32> {
        let length = bytes.len();
        let prefix_len = if length < 128 { 1 } else { 2 };
        let total = length + prefix_len;
        if length > MAX_TERM_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "bytes can be at most {MAX_TERM_LENGTH} in length; got {length}"
            )));
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
mod tests {
    use super::*;

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
        pool.reset();
        assert_eq!(pool.buffer_upto(), -1);
        assert_eq!(bytes_used.load(Ordering::Acquire), 0);
        // Resetting an untouched pool is a no-op.
        pool.reset();
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
        assert!(matches!(error, LuceneError::IllegalArgument(_)));
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
        assert_eq!(pool.read_byte(BYTE_BLOCK_SIZE as i32 - 1), 42);
        assert_eq!(pool.read_byte(BYTE_BLOCK_SIZE as i32), 43);
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
                pool.read_byte(offset as i32),
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
            pool.read_byte(BYTE_BLOCK_SIZE as i32 - 1),
            payload[BYTE_BLOCK_SIZE - 1]
        );
        assert_eq!(
            pool.read_byte(BYTE_BLOCK_SIZE as i32),
            payload[BYTE_BLOCK_SIZE]
        );
    }
}
