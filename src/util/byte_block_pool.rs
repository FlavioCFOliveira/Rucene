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

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::util::{Accountable, StringHelper};

/// Number of bits used to address a byte inside one block.
///
/// Equivalent to `ByteBlockPool.BYTE_BLOCK_SHIFT`.
pub const BYTE_BLOCK_SHIFT: u32 = 15;

/// Size in bytes of every block in the pool.
///
/// Equivalent to `ByteBlockPool.BYTE_BLOCK_SIZE`.
pub const BYTE_BLOCK_SIZE: usize = 1 << BYTE_BLOCK_SHIFT;

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
    /// # Panics
    ///
    /// Panics if the offset addresses a block that was never allocated.
    pub fn byte_at(&self, offset: i32) -> u8 {
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
}

impl Accountable for ByteBlockPool {
    fn ram_bytes_used(&self) -> i64 {
        self.buffers.len() as i64 * BYTE_BLOCK_SIZE as i64
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
        assert_eq!(pool.byte_at(BYTE_BLOCK_SIZE as i32 - 1), 42);
        assert_eq!(pool.byte_at(BYTE_BLOCK_SIZE as i32), 43);
    }
}
