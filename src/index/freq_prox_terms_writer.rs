//! In-memory inverted index ported from `org.apache.lucene.index`.
//!
//! This module buffers postings for one segment in RAM and, at flush time,
//! streams them through the codec's postings format.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`ByteSlicePool`] | `ByteSlicePool` |
//! | [`ByteSliceReader`] | `ByteSliceReader` |
//! | [`TermsHash`] | `TermsHash` |
//! | [`TermsHashPerField`] | `TermsHashPerField` |
//! | [`FreqProxPosting`] | `ParallelPostingsArray` + `FreqProxPostingsArray` |
//! | [`FreqProxTermsWriterPerField`] | `FreqProxTermsWriterPerField` |
//! | [`FreqProxFields`] | `FreqProxFields` |
//! | [`FreqProxTermsWriter`] | `FreqProxTermsWriter` |
//!
//! # How the postings are buffered
//!
//! Every term of every field owns one or two *streams* of bytes:
//!
//! * stream `0` carries the doc/freq list: a vInt `docDelta << 1 | 1` when the
//!   term occurs once in the document, otherwise `docDelta << 1` followed by a
//!   vInt frequency. Fields indexed with `IndexOptions::DOCS` write the raw
//!   `docDelta` instead, with no frequency at all.
//! * stream `1` carries proximity data — a vInt `positionDelta << 1`, with the
//!   low bit set when a payload follows, then the payload length and bytes, then
//!   the two offset vInts when offsets are indexed.
//!
//! Streams are interleaved inside a single [`ByteBlockPool`] using
//! [`ByteSlicePool`] slices of growing size, so a term with three postings costs
//! five bytes rather than a heap allocation. The final byte of every slice holds
//! `16 | level`, which is never zero: writers detect the end of a slice by
//! hitting a non-zero byte, and [`ByteSliceReader`] follows the four-byte
//! forwarding address that replaces it when the slice is chained.
//!
//! # Java to Rust adaptations
//!
//! * **`ParallelPostingsArray` becomes an array of structs.** Lucene keeps six
//!   parallel `int[]` arrays indexed by term id, and grows them together
//!   through the `BytesStartArray` callback. That layout exists because the JVM
//!   has no value types: an array of small objects would cost a header and a
//!   pointer chase per term, and would flood the GC. Rust has value types, and
//!   every one of `newTerm`/`addTerm`'s accesses touches several fields of the
//!   *same* term id, so a `Vec<FreqProxPosting>` puts all of them on one cache
//!   line instead of spreading them across six arrays. It also removes the
//!   lockstep-growth machinery entirely: term ids are dense and increasing, so
//!   a new term is one `Vec::push`.
//! * **`IntBlockPool` is gone.** Lucene stores the per-term stream addresses in
//!   a second pool because `ParallelPostingsArray` cannot hold a variable-length
//!   `int[]` per term without allocating. A term has at most two streams, fixed
//!   by its index options, so this port inlines them as
//!   [`FreqProxPosting::stream_address`]. One whole pool, its buffer bookkeeping
//!   and one level of indirection per written byte disappear.
//! * **The template method becomes a returned enum.** `TermsHashPerField.add`
//!   calls back into the abstract `newTerm`/`addTerm` of its subclass. This port
//!   has [`TermsHashPerField::intern`] return a [`TermSlot`] and lets the caller
//!   dispatch, which is the same control flow without the inheritance.
//! * **Flush hands out `Arc` snapshots.** `Fields::terms` returns
//!   `Box<dyn Terms>`, which is `'static`, so the read-back view cannot borrow
//!   the writer. At flush the buffers are moved — not copied — behind an
//!   [`Arc`], which every terms view, terms enum and postings enum then
//!   shares.
//!
//! # Scope
//!
//! Lucene chains a second `TermsHash` — the
//! [`TermVectorsConsumer`](crate::index::term_vectors_consumer::TermVectorsConsumer)
//! — behind the postings one, and shares the *term* byte pool between them:
//! the token text is interned once, here, and the second hash keys on the pool
//! offset it was interned at. This port keeps the same arrangement without the
//! inheritance. [`TermsHash`] still owns exactly one pool, the term pool; the
//! term-vectors consumer owns a second [`TermsHash`] for its own position and
//! offset streams, and reaches into this one through
//! [`FreqProxTermsWriter::pool`]. [`FreqProxTermsWriterPerField::add`] returns
//! the pool offset of the token it just interned, which is exactly the
//! `textStart` Lucene forwards to `nextPerField.add(int, int)`, and
//! [`TermsHashPerField::new_chained`] plus
//! [`TermsHashPerField::intern_by_text_start`] are that secondary entry point.
//!
//! Index sorting (`Sorter.DocMap`, `FreqProxTermsWriter.SortingTerms`) is not
//! ported.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::codecs::postings::NormsProducer;
use crate::codecs::state::SegmentWriteState;
use crate::error::{LuceneError, Result};
use crate::index::documents_writer::TermDelete;
use crate::index::indexing_chain::FieldInvertState;
use crate::index::{
    FieldInfo, Fields, ImpactsEnum, IndexOptions, PostingsEnum, SeekStatus, Terms, TermsEnum,
    POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_POSITIONS,
};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::DataInput;
use crate::util::byte_block_pool::{ByteBlockPool, BYTE_BLOCK_MASK, BYTE_BLOCK_SHIFT};
use crate::util::bytes_ref_hash::BytesRefHash;
use crate::util::{compare_utf16, AttributeSource, BytesRef, FixedBitSet};

// ---------------------------------------------------------------------------
// ByteSlicePool
// ---------------------------------------------------------------------------

/// Size in bytes of each slice level.
///
/// Equivalent to `ByteSlicePool.LEVEL_SIZE_ARRAY`.
pub const LEVEL_SIZE_ARRAY: [usize; 10] = [5, 14, 20, 30, 40, 40, 80, 80, 120, 200];

/// Level that follows each level; the last level repeats itself.
///
/// Equivalent to `ByteSlicePool.NEXT_LEVEL_ARRAY`. Every value must be below
/// 16 because the level is encoded in the low four bits of a slice's last byte.
pub const NEXT_LEVEL_ARRAY: [usize; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 9];

/// Size of a brand-new slice.
///
/// Equivalent to `ByteSlicePool.FIRST_LEVEL_SIZE`.
pub const FIRST_LEVEL_SIZE: usize = LEVEL_SIZE_ARRAY[0];

/// Allocator of chained, growing byte slices inside a [`ByteBlockPool`].
///
/// Equivalent to `org.apache.lucene.index.ByteSlicePool`. Lucene's class wraps
/// the pool; here the operations are associated functions that take the pool,
/// because the pool is shared with the term hash of every field and Rust cannot
/// express that with two owning handles.
pub struct ByteSlicePool;

impl ByteSlicePool {
    /// Allocates a level-0 slice of `size` bytes and returns its position
    /// inside the pool's head block.
    ///
    /// Equivalent to `ByteSlicePool.newSlice(int)`.
    ///
    /// # Errors
    ///
    /// Propagates the pool-overflow error of [`ByteBlockPool::next_buffer`].
    ///
    /// # Panics
    ///
    /// Panics if `size` exceeds the pool's block size.
    pub fn new_slice(pool: &mut ByteBlockPool, size: usize) -> Result<usize> {
        pool.ensure_capacity(size)?;
        let upto = pool.byte_upto();
        pool.advance(size);
        let block = pool.buffer_upto() as usize;
        let last = pool.byte_upto() - 1;
        // 16 codifies level 0: the marker is never zero, which is how writers
        // recognise the end of a slice.
        pool.buffer_mut(block)[last] = 16;
        Ok(upto)
    }

    /// Chains a new slice after the one whose last byte is at `upto` of block
    /// `block`, and returns the position of its first writable byte.
    ///
    /// Equivalent to `ByteSlicePool.allocSlice(byte[], int)`.
    ///
    /// # Errors
    ///
    /// Propagates the pool-overflow error of [`ByteBlockPool::next_buffer`].
    pub fn alloc_slice(pool: &mut ByteBlockPool, block: usize, upto: usize) -> Result<usize> {
        Ok(Self::alloc_known_size_slice(pool, block, upto)?.0)
    }

    /// Chains a new slice and returns its first writable position together with
    /// the number of bytes writable there.
    ///
    /// Equivalent to `ByteSlicePool.allocKnownSizeSlice(byte[], int)`, which
    /// packs the same pair into one `int`. Returning a tuple removes the
    /// packing and the 24-bit ceiling it implies.
    ///
    /// # Errors
    ///
    /// Propagates the pool-overflow error of [`ByteBlockPool::next_buffer`].
    pub fn alloc_known_size_slice(
        pool: &mut ByteBlockPool,
        block: usize,
        upto: usize,
    ) -> Result<(usize, usize)> {
        let level = (pool.buffer(block)[upto] & 15) as usize;
        let new_level = NEXT_LEVEL_ARRAY[level];
        let new_size = LEVEL_SIZE_ARRAY[new_level];

        pool.ensure_capacity(new_size)?;

        let new_upto = pool.byte_upto();
        let forwarding_address = new_upto as i32 + pool.byte_offset();
        pool.advance(new_size);
        let new_block = pool.buffer_upto() as usize;

        // The three bytes before the level marker already carry stream data;
        // the forwarding address is four bytes wide and will overwrite them, so
        // move them to the front of the new slice first.
        let carried = {
            let buffer = pool.buffer(block);
            [buffer[upto - 3], buffer[upto - 2], buffer[upto - 1]]
        };
        pool.buffer_mut(new_block)[new_upto..new_upto + 3].copy_from_slice(&carried);

        // Write the forwarding address over the carried bytes and the marker.
        // The new slice starts after `upto`, so this never overlaps the copy
        // above even when both slices live in the same block.
        pool.buffer_mut(block)[upto - 3..upto + 1]
            .copy_from_slice(&forwarding_address.to_le_bytes());

        let last = new_upto + new_size - 1;
        pool.buffer_mut(new_block)[last] = (16 | new_level) as u8;

        Ok((new_upto + 3, new_size - 3))
    }
}

// ---------------------------------------------------------------------------
// ByteSliceReader
// ---------------------------------------------------------------------------

/// Sequential reader over the chained slices written by [`TermsHashPerField`].
///
/// Equivalent to `org.apache.lucene.index.ByteSliceReader`. Lucene's class
/// extends `DataInput`; this port keeps the pool out of the struct — every
/// method takes it — so that the reader stays a plain cursor that several
/// callers can create over one shared [`Arc<ByteBlockPool>`].
#[derive(Debug, Default, Clone)]
pub struct ByteSliceReader {
    block: usize,
    upto: usize,
    limit: usize,
    level: usize,
    block_offset: i32,
    end_index: i32,
}

impl ByteSliceReader {
    /// Creates a reader positioned before the first byte of the stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Positions the reader over the slice chain spanning
    /// `[start_index, end_index)`.
    ///
    /// Equivalent to `ByteSliceReader.init(ByteBlockPool, int, int)`.
    pub fn init(&mut self, start_index: i32, end_index: i32) {
        debug_assert!(start_index >= 0);
        debug_assert!(end_index >= start_index);
        self.end_index = end_index;
        self.level = 0;
        self.block = (start_index as usize) >> BYTE_BLOCK_SHIFT;
        self.block_offset = (self.block as i32) << BYTE_BLOCK_SHIFT;
        self.upto = (start_index as usize) & BYTE_BLOCK_MASK;
        self.limit = if start_index + FIRST_LEVEL_SIZE as i32 >= end_index {
            // The whole stream fits in the first slice.
            (end_index as usize) & BYTE_BLOCK_MASK
        } else {
            self.upto + FIRST_LEVEL_SIZE - 4
        };
    }

    /// Returns `true` when every byte of the stream was consumed.
    pub fn eof(&self) -> bool {
        self.remaining() == 0
    }

    /// Returns how many bytes of the stream are still unread.
    ///
    /// Java has no equivalent: `ByteSliceReader` extends `DataInput` and simply
    /// trusts the caller not to over-read. This port exposes the count so that
    /// [`PooledSliceReader`] can turn an over-read into an error rather than an
    /// out-of-bounds panic.
    pub fn remaining(&self) -> i32 {
        self.end_index - (self.upto as i32 + self.block_offset)
    }

    /// Reads one byte.
    ///
    /// # Panics
    ///
    /// Panics if called at end of stream.
    pub fn read_byte(&mut self, pool: &ByteBlockPool) -> u8 {
        debug_assert!(!self.eof());
        if self.upto == self.limit {
            self.next_slice(pool);
        }
        let value = pool.buffer(self.block)[self.upto];
        self.upto += 1;
        value
    }

    /// Reads `out.len()` bytes.
    ///
    /// # Panics
    ///
    /// Panics if the stream holds fewer bytes than requested.
    pub fn read_bytes(&mut self, pool: &ByteBlockPool, out: &mut [u8]) {
        let mut written = 0usize;
        while written < out.len() {
            let remaining = out.len() - written;
            let available = self.limit - self.upto;
            if available < remaining {
                out[written..written + available]
                    .copy_from_slice(&pool.buffer(self.block)[self.upto..self.limit]);
                written += available;
                self.next_slice(pool);
            } else {
                out[written..]
                    .copy_from_slice(&pool.buffer(self.block)[self.upto..self.upto + remaining]);
                self.upto += remaining;
                break;
            }
        }
    }

    /// Reads a variable-length signed 32-bit integer.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] if the encoding uses more than
    /// five bytes, which cannot happen for buffers this module wrote.
    pub fn read_v_int(&mut self, pool: &ByteBlockPool) -> Result<i32> {
        let mut byte = self.read_byte(pool);
        let mut value = (byte & 0x7F) as i32;
        let mut shift = 7;
        while byte & 0x80 != 0 {
            if shift > 28 {
                return Err(LuceneError::CorruptIndex(
                    "invalid vInt in buffered postings".to_string(),
                ));
            }
            byte = self.read_byte(pool);
            value |= ((byte & 0x7F) as i32) << shift;
            shift += 7;
        }
        Ok(value)
    }

    /// Advances to the next slice of the chain.
    fn next_slice(&mut self, pool: &ByteBlockPool) {
        let buffer = pool.buffer(self.block);
        let next_index = i32::from_le_bytes([
            buffer[self.limit],
            buffer[self.limit + 1],
            buffer[self.limit + 2],
            buffer[self.limit + 3],
        ]);

        self.level = NEXT_LEVEL_ARRAY[self.level];
        let new_size = LEVEL_SIZE_ARRAY[self.level];

        self.block = (next_index as usize) >> BYTE_BLOCK_SHIFT;
        self.block_offset = (self.block as i32) << BYTE_BLOCK_SHIFT;
        self.upto = (next_index as usize) & BYTE_BLOCK_MASK;

        self.limit = if next_index + new_size as i32 >= self.end_index {
            (self.end_index - self.block_offset) as usize
        } else {
            self.upto + new_size - 4
        };
    }
}

/// A [`ByteSliceReader`] bound to the pool it reads from, so that it can be
/// consumed as a [`DataInput`].
///
/// Lucene's `ByteSliceReader` *is* a `DataInput`, because it owns a reference
/// to its `ByteBlockPool`. This port keeps the pool out of the reader — several
/// readers share one pool — so the two are paired here instead, for the one
/// caller that needs the `DataInput` shape:
/// [`TermVectorsWriter::add_prox`](crate::codecs::term_vectors::TermVectorsWriter::add_prox).
///
/// Unlike the bare reader, this one reports an over-read as
/// [`LuceneError::CorruptIndex`] instead of panicking, so that a codec reading
/// more than the indexer wrote fails cleanly.
#[derive(Debug)]
pub struct PooledSliceReader<'a> {
    reader: ByteSliceReader,
    pool: &'a ByteBlockPool,
}

impl<'a> PooledSliceReader<'a> {
    /// Pairs `reader` with the `pool` its slices live in.
    pub fn new(reader: ByteSliceReader, pool: &'a ByteBlockPool) -> Self {
        Self { reader, pool }
    }

    /// Returns `true` when every byte of the stream was consumed.
    pub fn eof(&self) -> bool {
        self.reader.eof()
    }

    fn ensure(&self, len: usize) -> Result<()> {
        let remaining = self.reader.remaining();
        if (remaining as i64) < len as i64 {
            return Err(LuceneError::CorruptIndex(format!(
                "read past the end of a buffered term stream: {len} bytes requested, \
                 {remaining} available"
            )));
        }
        Ok(())
    }
}

impl DataInput for PooledSliceReader<'_> {
    fn read_byte(&mut self) -> Result<u8> {
        self.ensure(1)?;
        Ok(self.reader.read_byte(self.pool))
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.ensure(len)?;
        self.reader
            .read_bytes(self.pool, &mut b[offset..offset + len]);
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let len = usize::try_from(num_bytes)
            .map_err(|_| LuceneError::IllegalArgument(format!("cannot skip {num_bytes} bytes")))?;
        self.ensure(len)?;
        let mut scratch = [0u8; 64];
        let mut left = len;
        while left > 0 {
            let chunk = std::cmp::min(left, scratch.len());
            self.reader.read_bytes(self.pool, &mut scratch[..chunk]);
            left -= chunk;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Postings slots
// ---------------------------------------------------------------------------

/// Per-term state kept while a segment is being buffered.
///
/// Equivalent to the columns of `ParallelPostingsArray` and
/// `FreqProxTermsWriterPerField.FreqProxPostingsArray` for one term id; see the
/// module-level notes for why this port uses an array of structs.
///
/// One [`TermsHashPerField`] backs both consumers of the terms hash, so this
/// struct is the union of the two Java posting arrays. The term-vectors
/// consumer uses exactly the columns of `TermVectorsPostingsArray` —
/// [`stream_address`](Self::stream_address), [`byte_start`](Self::byte_start),
/// [`term_freq`](Self::term_freq) (`freqs`),
/// [`last_position`](Self::last_position) (`lastPositions`) and
/// [`last_offset`](Self::last_offset) (`lastOffsets`) — and leaves
/// [`last_doc_id`](Self::last_doc_id) and
/// [`last_doc_code`](Self::last_doc_code) untouched, because a term vector is
/// built one document at a time and has no doc list. Note that the two
/// consumers give `last_offset` different meanings: the postings writer stores
/// the previous *start* offset there, the term-vectors writer the previous
/// *end* offset (`FreqProxTermsWriterPerField.java:109` versus
/// `TermVectorsConsumerPerField.java:242`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FreqProxPosting {
    /// Next free global pool offset of each of the term's streams. Replaces
    /// Lucene's `addressOffset` indirection through the `IntBlockPool`.
    pub stream_address: [i32; 2],
    /// Global pool offset of the term's first stream slice.
    ///
    /// Equivalent to `ParallelPostingsArray.byteStarts`.
    pub byte_start: i32,
    /// Last document in which the term was seen.
    pub last_doc_id: i32,
    /// Pending doc code of the previous document.
    pub last_doc_code: i32,
    /// Occurrences of the term in the current document.
    pub term_freq: i32,
    /// Last position at which the term was seen.
    pub last_position: i32,
    /// Last start offset at which the term was seen.
    pub last_offset: i32,
}

/// Heap cost of one buffered term, used for the RAM-driven flush triggers.
///
/// Mirrors `FreqProxPostingsArray.bytesPerPosting()`: the struct itself plus the
/// `bytesStart` entry that the term hash keeps for the same term id.
const BYTES_PER_POSTING: i64 = std::mem::size_of::<FreqProxPosting>() as i64 + 4;

/// Outcome of interning a term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSlot {
    /// Dense id of the term inside its field.
    pub term_id: i32,
    /// `true` when the term had not been seen since the last flush.
    pub is_new: bool,
}

// ---------------------------------------------------------------------------
// TermsHash / TermsHashPerField
// ---------------------------------------------------------------------------

/// Buffers shared by every per-field writer of one segment.
///
/// Equivalent to `org.apache.lucene.index.TermsHash`. Each instance owns one
/// [`ByteBlockPool`]: for the postings hash that pool is the *term* pool, where
/// every token's text is interned and where the doc/freq/prox streams live; for
/// the second hash of the chain — the
/// [`TermVectorsConsumer`](crate::index::TermVectorsConsumer), which owns a
/// `TermsHash` of its own — it holds only that consumer's per-document position
/// and offset streams, because its term table keys on offsets into the postings
/// pool instead of interning the bytes a second time
/// (`TermsHash.java:52-56`).
#[derive(Debug)]
pub struct TermsHash {
    pool: ByteBlockPool,
    bytes_used: Arc<AtomicI64>,
}

impl TermsHash {
    /// Creates the shared buffers, charging their blocks to `bytes_used`.
    pub fn new(bytes_used: Arc<AtomicI64>) -> Self {
        Self {
            pool: ByteBlockPool::new(Arc::clone(&bytes_used)),
            bytes_used,
        }
    }

    /// Returns the shared byte pool.
    pub fn pool(&self) -> &ByteBlockPool {
        &self.pool
    }

    /// Returns the shared byte pool for mutation.
    pub fn pool_mut(&mut self) -> &mut ByteBlockPool {
        &mut self.pool
    }

    /// Returns the shared RAM counter.
    pub fn bytes_used(&self) -> &Arc<AtomicI64> {
        &self.bytes_used
    }

    /// Discards every buffered byte.
    ///
    /// Equivalent to `TermsHash.reset()`.
    pub fn reset(&mut self) {
        self.pool.reset();
    }

    /// Moves the pool out, leaving an empty one behind.
    ///
    /// Used at flush time to hand the buffers to the read-back view without
    /// copying them.
    fn take_pool(&mut self) -> ByteBlockPool {
        std::mem::replace(
            &mut self.pool,
            ByteBlockPool::new(Arc::clone(&self.bytes_used)),
        )
    }
}

/// Per-field term table and stream writer.
///
/// Equivalent to `org.apache.lucene.index.TermsHashPerField`.
#[derive(Debug)]
pub struct TermsHashPerField {
    stream_count: usize,
    field_name: String,
    index_options: IndexOptions,
    bytes_hash: BytesRefHash,
    postings: Vec<FreqProxPosting>,
    bytes_used: Arc<AtomicI64>,
    /// Highest document id passed to [`Self::intern`], checked in debug builds
    /// the way Lucene's `assertDocId` does.
    last_doc_id: i32,
}

impl TermsHashPerField {
    /// Creates a table for `field_name` writing `stream_count` streams per term.
    ///
    /// # Panics
    ///
    /// Panics if `index_options` is [`IndexOptions::NONE`] or if `stream_count`
    /// is not 1 or 2.
    pub fn new(
        stream_count: usize,
        field_name: String,
        index_options: IndexOptions,
        bytes_used: Arc<AtomicI64>,
    ) -> Self {
        assert_ne!(index_options, IndexOptions::NONE);
        assert!(
            stream_count == 1 || stream_count == 2,
            "a term has one or two streams, got {stream_count}"
        );
        Self {
            stream_count,
            field_name,
            index_options,
            bytes_hash: BytesRefHash::new(Arc::clone(&bytes_used)),
            postings: Vec::new(),
            bytes_used,
            last_doc_id: 0,
        }
    }

    /// Creates a table for the *second* term hash of the chain.
    ///
    /// Equivalent to constructing a `TermsHashPerField` whose `termBytePool`
    /// is the primary hash's byte pool: the term text is already interned
    /// there, so this table keys on the pool offset instead of on the bytes and
    /// only [`Self::intern_by_text_start`] may feed it. Its own streams still
    /// live in whichever pool the caller passes to the write methods, which for
    /// term vectors is the second hash's pool, not the term pool.
    ///
    /// # Panics
    ///
    /// Panics if `index_options` is [`IndexOptions::NONE`] or if `stream_count`
    /// is not 1 or 2.
    pub fn new_chained(
        stream_count: usize,
        field_name: String,
        index_options: IndexOptions,
        bytes_used: Arc<AtomicI64>,
    ) -> Self {
        let mut field = Self::new(
            stream_count,
            field_name,
            index_options,
            Arc::clone(&bytes_used),
        );
        field.bytes_hash.release_accounting();
        field.bytes_hash = BytesRefHash::new_by_pool_offset(bytes_used);
        field
    }

    /// Returns the field this table belongs to.
    pub fn field_name(&self) -> &str {
        &self.field_name
    }

    /// Returns the index options the field was inverted with.
    pub fn index_options(&self) -> IndexOptions {
        self.index_options
    }

    /// Returns the number of distinct terms buffered for this field.
    ///
    /// Equivalent to `TermsHashPerField.getNumTerms()`.
    pub fn num_terms(&self) -> i32 {
        self.bytes_hash.size()
    }

    /// Returns the per-term state of `term_id`.
    pub fn posting(&self, term_id: i32) -> &FreqProxPosting {
        &self.postings[term_id as usize]
    }

    /// Returns the per-term state of `term_id` for mutation.
    ///
    /// The consumer driving this table owns the columns it uses; see
    /// [`FreqProxPosting`] for which consumer owns which.
    ///
    /// # Panics
    ///
    /// Panics if `term_id` was never returned by [`Self::intern`] or
    /// [`Self::intern_by_text_start`].
    pub fn posting_mut(&mut self, term_id: i32) -> &mut FreqProxPosting {
        &mut self.postings[term_id as usize]
    }

    /// Returns the offset, in the term pool, of the bytes of `term_id`.
    ///
    /// Equivalent to reading `ParallelPostingsArray.textStarts[termID]`, which
    /// is what Lucene forwards to the next hash of the chain.
    ///
    /// # Panics
    ///
    /// Panics if `term_id` was never returned by [`Self::intern`] or
    /// [`Self::intern_by_text_start`].
    pub fn text_start(&self, term_id: i32) -> i32 {
        self.bytes_hash.byte_start(term_id)
    }

    /// Returns every buffered term id ordered by its term bytes, which live in
    /// `term_pool`.
    ///
    /// Equivalent to `TermsHashPerField.sortTerms()` followed by
    /// `getSortedTermIDs()`. Lucene splits the two because its `BytesRefHash`
    /// sort is destructive; this one is not, so a single call does both and may
    /// be repeated.
    pub fn sorted_term_ids(&self, term_pool: &ByteBlockPool) -> Vec<i32> {
        self.bytes_hash.sort(term_pool)
    }

    /// Positions `reader` over `stream` of `term_id`.
    ///
    /// Equivalent to `TermsHashPerField.initReader(ByteSliceReader, int, int)`.
    /// The reader then reads from the pool the streams were written to.
    ///
    /// # Panics
    ///
    /// Panics if `stream` is not below this table's stream count, or if
    /// `term_id` was never interned.
    pub fn init_reader(&self, reader: &mut ByteSliceReader, term_id: i32, stream: usize) {
        assert!(stream < self.stream_count);
        let slot = &self.postings[term_id as usize];
        reader.init(
            slot.byte_start + (stream * FIRST_LEVEL_SIZE) as i32,
            slot.stream_address[stream],
        );
    }

    /// Hands every byte this table charged back to the shared counter.
    ///
    /// The table keeps its contents but must not be charged again; callers
    /// either discard it or re-create it right after.
    pub fn release_accounting(&mut self) {
        self.bytes_used.fetch_sub(
            self.postings.len() as i64 * BYTES_PER_POSTING,
            Ordering::AcqRel,
        );
        self.postings = Vec::new();
        self.bytes_hash.release_accounting();
    }

    /// Discards every buffered term.
    ///
    /// Equivalent to `TermsHashPerField.reset()`. The byte pool is owned by
    /// [`TermsHash`] and is reset separately.
    pub fn reset(&mut self) {
        let keyed_by_pool_offset = self.bytes_hash.is_keyed_by_pool_offset();
        self.release_accounting();
        self.bytes_hash = if keyed_by_pool_offset {
            BytesRefHash::new_by_pool_offset(Arc::clone(&self.bytes_used))
        } else {
            BytesRefHash::new(Arc::clone(&self.bytes_used))
        };
        self.last_doc_id = 0;
    }

    /// Interns `term_bytes` and, for a new term, allocates its stream slices.
    ///
    /// Equivalent to the body of `TermsHashPerField.add(BytesRef, int)` up to
    /// the `newTerm`/`addTerm` dispatch, which the caller performs on the
    /// returned [`TermSlot`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the term exceeds
    /// [`crate::util::MAX_TERM_LENGTH`], and propagates pool-overflow errors.
    pub fn intern(
        &mut self,
        pool: &mut ByteBlockPool,
        term_bytes: &[u8],
        doc_id: i32,
    ) -> Result<TermSlot> {
        debug_assert!(
            doc_id >= self.last_doc_id,
            "docID must be >= {} but was {doc_id}",
            self.last_doc_id
        );
        self.last_doc_id = doc_id;

        let raw = self.bytes_hash.add(pool, term_bytes)?;
        if raw >= 0 {
            self.init_stream_slices(pool, raw)?;
            Ok(TermSlot {
                term_id: raw,
                is_new: true,
            })
        } else {
            Ok(TermSlot {
                term_id: -raw - 1,
                is_new: false,
            })
        }
    }

    /// Records an occurrence of the term already interned at `text_start` and,
    /// for a term this table has not seen yet, allocates its stream slices.
    ///
    /// Equivalent to the private `TermsHashPerField.add(int textStart, int
    /// docID)` — Lucene's *secondary* entry point, taken by every hash but the
    /// first of the chain — up to the `newTerm`/`addTerm` dispatch, which the
    /// caller performs on the returned [`TermSlot`].
    ///
    /// `streams` is the pool this table writes its streams to, which is *not*
    /// the pool `text_start` points into.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if the table was not created with
    /// [`Self::new_chained`].
    ///
    /// # Errors
    ///
    /// Propagates pool-overflow errors raised while reserving the slices.
    pub fn intern_by_text_start(
        &mut self,
        streams: &mut ByteBlockPool,
        text_start: i32,
    ) -> Result<TermSlot> {
        let raw = self.bytes_hash.add_by_pool_offset(text_start);
        if raw >= 0 {
            self.init_stream_slices(streams, raw)?;
            Ok(TermSlot {
                term_id: raw,
                is_new: true,
            })
        } else {
            Ok(TermSlot {
                term_id: -raw - 1,
                is_new: false,
            })
        }
    }

    /// Reserves one slice per stream for a freshly interned term.
    ///
    /// Equivalent to `TermsHashPerField.initStreamSlices(int, int)` minus the
    /// `IntBlockPool` reservation, which this port inlines into the slot.
    fn init_stream_slices(&mut self, pool: &mut ByteBlockPool, term_id: i32) -> Result<()> {
        debug_assert_eq!(term_id as usize, self.postings.len());
        // Make sure at least one byte per stream fits in the current block, so
        // that the streams of a term stay close together.
        pool.ensure_capacity(2 * self.stream_count * FIRST_LEVEL_SIZE)?;

        let mut slot = FreqProxPosting::default();
        for stream in 0..self.stream_count {
            let upto = ByteSlicePool::new_slice(pool, FIRST_LEVEL_SIZE)?;
            slot.stream_address[stream] = upto as i32 + pool.byte_offset();
        }
        slot.byte_start = slot.stream_address[0];
        self.postings.push(slot);
        self.bytes_used
            .fetch_add(BYTES_PER_POSTING, Ordering::AcqRel);
        Ok(())
    }

    /// Appends one byte to `stream` of `term_id`.
    ///
    /// Equivalent to `TermsHashPerField.writeByte(int, byte)`.
    ///
    /// # Errors
    ///
    /// Propagates pool-overflow errors raised while chaining a new slice.
    pub fn write_byte(
        &mut self,
        pool: &mut ByteBlockPool,
        term_id: i32,
        stream: usize,
        value: u8,
    ) -> Result<()> {
        debug_assert!(stream < self.stream_count);
        let slot = &mut self.postings[term_id as usize];
        let address = slot.stream_address[stream] as usize;
        let block = address >> BYTE_BLOCK_SHIFT;
        let offset = address & BYTE_BLOCK_MASK;

        if pool.buffer(block)[offset] != 0 {
            // End of slice: chain a new one and restart there.
            let new_offset = ByteSlicePool::alloc_slice(pool, block, offset)?;
            let new_block = pool.buffer_upto() as usize;
            pool.buffer_mut(new_block)[new_offset] = value;
            self.postings[term_id as usize].stream_address[stream] =
                new_offset as i32 + pool.byte_offset() + 1;
        } else {
            pool.buffer_mut(block)[offset] = value;
            slot.stream_address[stream] += 1;
        }
        Ok(())
    }

    /// Appends `value` to `stream` of `term_id`.
    ///
    /// Equivalent to `TermsHashPerField.writeBytes(int, byte[], int, int)`.
    ///
    /// # Errors
    ///
    /// Propagates pool-overflow errors raised while chaining new slices.
    pub fn write_bytes(
        &mut self,
        pool: &mut ByteBlockPool,
        term_id: i32,
        stream: usize,
        value: &[u8],
    ) -> Result<()> {
        debug_assert!(stream < self.stream_count);
        let address = self.postings[term_id as usize].stream_address[stream] as usize;
        let mut block = address >> BYTE_BLOCK_SHIFT;
        let mut offset = address & BYTE_BLOCK_MASK;
        let mut written = 0usize;

        // Fill the remainder of the current slice, one byte at a time, until we
        // hit its non-zero end marker.
        while written < value.len() && pool.buffer(block)[offset] == 0 {
            pool.buffer_mut(block)[offset] = value[written];
            offset += 1;
            written += 1;
            self.postings[term_id as usize].stream_address[stream] += 1;
        }

        while written < value.len() {
            let (new_offset, slice_len) =
                ByteSlicePool::alloc_known_size_slice(pool, block, offset)?;
            block = pool.buffer_upto() as usize;
            offset = new_offset;
            // The last byte of the new slice must stay free for its end marker.
            let chunk = std::cmp::min(slice_len - 1, value.len() - written);
            pool.buffer_mut(block)[offset..offset + chunk]
                .copy_from_slice(&value[written..written + chunk]);
            offset += chunk;
            written += chunk;
            self.postings[term_id as usize].stream_address[stream] =
                offset as i32 + pool.byte_offset();
        }
        Ok(())
    }

    /// Appends `value` to `stream` of `term_id` as a variable-length integer.
    ///
    /// Equivalent to `TermsHashPerField.writeVInt(int, int)`.
    ///
    /// # Errors
    ///
    /// Propagates pool-overflow errors raised while chaining new slices.
    pub fn write_v_int(
        &mut self,
        pool: &mut ByteBlockPool,
        term_id: i32,
        stream: usize,
        value: i32,
    ) -> Result<()> {
        let mut remaining = value as u32;
        while remaining & !0x7F != 0 {
            self.write_byte(pool, term_id, stream, ((remaining & 0x7F) | 0x80) as u8)?;
            remaining >>= 7;
        }
        self.write_byte(pool, term_id, stream, remaining as u8)
    }
}

// ---------------------------------------------------------------------------
// FreqProxTermsWriterPerField
// ---------------------------------------------------------------------------

/// One inverted token, as read from the token stream's attributes.
///
/// Lucene reads these straight off the `AttributeSource` cached in
/// `FieldInvertState`; grouping them keeps the per-field writer independent of
/// the analysis package.
#[derive(Debug, Default, Clone, Copy)]
pub struct InvertedToken<'a> {
    /// Start offset reported by the token stream, before the field's offset gap.
    pub start_offset: i32,
    /// End offset reported by the token stream, before the field's offset gap.
    pub end_offset: i32,
    /// Payload attached to the token, if any.
    pub payload: Option<&'a [u8]>,
    /// Custom term frequency; `1` unless the stream set one.
    pub term_freq: i32,
    /// `true` when the token stream carries a `TermFrequencyAttribute` at all,
    /// which Lucene distinguishes from a frequency that happens to be 1.
    pub has_term_freq_attribute: bool,
}

/// Buffers the postings of one field.
///
/// Equivalent to `org.apache.lucene.index.FreqProxTermsWriterPerField`.
#[derive(Debug)]
pub struct FreqProxTermsWriterPerField {
    base: TermsHashPerField,
    field_info: FieldInfo,
    has_freq: bool,
    has_prox: bool,
    has_offsets: bool,
    is_term_doc: bool,
    saw_payloads: bool,
}

impl FreqProxTermsWriterPerField {
    /// Creates a writer for `field_info`.
    ///
    /// # Panics
    ///
    /// Panics if the field's index options are [`IndexOptions::NONE`].
    pub fn new(field_info: FieldInfo, bytes_used: Arc<AtomicI64>) -> Self {
        let index_options = field_info.index_options;
        let has_freq = index_options.subsumes(IndexOptions::DOCS_AND_FREQS);
        let has_prox = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        let has_offsets =
            index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        let stream_count = if has_prox { 2 } else { 1 };
        Self {
            base: TermsHashPerField::new(
                stream_count,
                field_info.name.clone(),
                index_options,
                bytes_used,
            ),
            is_term_doc: field_info.is_term_doc_field(),
            field_info,
            has_freq,
            has_prox,
            has_offsets,
            saw_payloads: false,
        }
    }

    /// Returns the field's metadata as it was when inversion started.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Returns the field name.
    pub fn field_name(&self) -> &str {
        self.base.field_name()
    }

    /// Returns the number of distinct terms buffered.
    pub fn num_terms(&self) -> i32 {
        self.base.num_terms()
    }

    /// Returns `true` when any token of this field carried a payload.
    ///
    /// Equivalent to `FreqProxTermsWriterPerField.sawPayloads`, which drives
    /// `FieldInfo.setStorePayloads()`.
    pub fn saw_payloads(&self) -> bool {
        self.saw_payloads
    }

    /// Discards every buffered term, keeping the writer usable.
    pub fn reset(&mut self) {
        self.base.reset();
        self.saw_payloads = false;
    }

    /// Hands every accounted byte back to the shared counter.
    ///
    /// Used when the writer is about to be dropped, where re-allocating the
    /// term table that [`Self::reset`] leaves behind would be wasted work.
    pub fn release_accounting(&mut self) {
        self.base.release_accounting();
        self.saw_payloads = false;
    }

    /// Records one occurrence of `term_bytes` in `doc_id` and returns the
    /// offset `term_bytes` was interned at in `pool`.
    ///
    /// Equivalent to `TermsHashPerField.add(BytesRef, int)` followed by
    /// `FreqProxTermsWriterPerField.newTerm` or `.addTerm`. The returned offset
    /// is `postingsArray.textStarts[termID]`, the value Lucene hands to
    /// `nextPerField.add(int, int)` so that the next hash of the chain — the
    /// term-vectors consumer — can key on the already-interned text instead of
    /// interning it a second time.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an over-long term or a
    /// duplicate term in a term-doc field, and [`LuceneError::IllegalState`]
    /// when a custom term frequency is combined with positions or with a field
    /// that does not index frequencies.
    pub fn add(
        &mut self,
        pool: &mut ByteBlockPool,
        field_state: &mut FieldInvertState,
        term_bytes: &[u8],
        doc_id: i32,
        token: &InvertedToken<'_>,
    ) -> Result<i32> {
        let slot = self.base.intern(pool, term_bytes, doc_id)?;
        if slot.is_new {
            self.new_term(pool, field_state, slot.term_id, doc_id, token)?;
        } else {
            self.add_term(pool, field_state, slot.term_id, doc_id, token)?;
        }
        Ok(self.base.text_start(slot.term_id))
    }

    /// Returns the frequency to record for the current token.
    ///
    /// Equivalent to `FreqProxTermsWriterPerField.getTermFreq()`.
    fn term_freq(&self, token: &InvertedToken<'_>) -> Result<i32> {
        let freq = if token.has_term_freq_attribute {
            token.term_freq
        } else {
            1
        };
        if freq != 1 && self.has_prox {
            return Err(LuceneError::IllegalState(format!(
                "field \"{}\": cannot index positions while using custom TermFrequencyAttribute",
                self.field_name()
            )));
        }
        Ok(freq)
    }

    /// Writes the proximity entry of the current token.
    ///
    /// Equivalent to `FreqProxTermsWriterPerField.writeProx(int, int)`.
    fn write_prox(
        &mut self,
        pool: &mut ByteBlockPool,
        field_state: &FieldInvertState,
        term_id: i32,
        prox_code: i32,
        token: &InvertedToken<'_>,
    ) -> Result<()> {
        match token.payload {
            Some(payload) if !payload.is_empty() => {
                self.base
                    .write_v_int(pool, term_id, 1, (prox_code << 1) | 1)?;
                self.base
                    .write_v_int(pool, term_id, 1, payload.len() as i32)?;
                self.base.write_bytes(pool, term_id, 1, payload)?;
                self.saw_payloads = true;
            }
            _ => {
                self.base.write_v_int(pool, term_id, 1, prox_code << 1)?;
            }
        }
        self.base.postings[term_id as usize].last_position = field_state.position();
        Ok(())
    }

    /// Writes the offset entry of the current token.
    ///
    /// Equivalent to `FreqProxTermsWriterPerField.writeOffsets(int, int)`.
    fn write_offsets(
        &mut self,
        pool: &mut ByteBlockPool,
        term_id: i32,
        offset_accum: i32,
        token: &InvertedToken<'_>,
    ) -> Result<()> {
        let start_offset = offset_accum + token.start_offset;
        let end_offset = offset_accum + token.end_offset;
        let last_offset = self.base.postings[term_id as usize].last_offset;
        debug_assert!(start_offset - last_offset >= 0);
        self.base
            .write_v_int(pool, term_id, 1, start_offset - last_offset)?;
        self.base
            .write_v_int(pool, term_id, 1, end_offset - start_offset)?;
        self.base.postings[term_id as usize].last_offset = start_offset;
        Ok(())
    }

    /// Records the first occurrence of a term since the last flush.
    ///
    /// Equivalent to `FreqProxTermsWriterPerField.newTerm(int, int)`.
    fn new_term(
        &mut self,
        pool: &mut ByteBlockPool,
        field_state: &mut FieldInvertState,
        term_id: i32,
        doc_id: i32,
        token: &InvertedToken<'_>,
    ) -> Result<()> {
        self.base.postings[term_id as usize].last_doc_id = doc_id;
        if !self.has_freq {
            self.base.postings[term_id as usize].last_doc_code = doc_id;
            field_state.set_max_term_frequency(std::cmp::max(1, field_state.max_term_frequency()));
        } else {
            let freq = self.term_freq(token)?;
            self.base.postings[term_id as usize].last_doc_code = doc_id << 1;
            self.base.postings[term_id as usize].term_freq = freq;
            if self.has_prox {
                let position = field_state.position();
                self.write_prox(pool, field_state, term_id, position, token)?;
                if self.has_offsets {
                    self.write_offsets(pool, term_id, field_state.offset(), token)?;
                }
            }
            field_state
                .set_max_term_frequency(std::cmp::max(freq, field_state.max_term_frequency()));
        }
        field_state.increment_unique_term_count();
        Ok(())
    }

    /// Records a repeat occurrence of an already-seen term.
    ///
    /// Equivalent to `FreqProxTermsWriterPerField.addTerm(int, int)`.
    fn add_term(
        &mut self,
        pool: &mut ByteBlockPool,
        field_state: &mut FieldInvertState,
        term_id: i32,
        doc_id: i32,
        token: &InvertedToken<'_>,
    ) -> Result<()> {
        let index = term_id as usize;
        if !self.has_freq {
            if token.has_term_freq_attribute && token.term_freq != 1 {
                return Err(LuceneError::IllegalState(format!(
                    "field \"{}\": must index term freq while using custom TermFrequencyAttribute",
                    self.field_name()
                )));
            }
            if doc_id != self.base.postings[index].last_doc_id {
                debug_assert!(doc_id > self.base.postings[index].last_doc_id);
                // The previous document is complete; flush its doc code.
                let code = self.base.postings[index].last_doc_code;
                self.base.write_v_int(pool, term_id, 0, code)?;
                self.base.postings[index].last_doc_code =
                    doc_id - self.base.postings[index].last_doc_id;
                self.base.postings[index].last_doc_id = doc_id;
                field_state.increment_unique_term_count();
            }
            return Ok(());
        }

        if doc_id != self.base.postings[index].last_doc_id {
            debug_assert!(
                doc_id > self.base.postings[index].last_doc_id,
                "id: {doc_id} postings ID: {} termID: {term_id}",
                self.base.postings[index].last_doc_id
            );
            // Now that the previous document's frequency is final, write it.
            if self.base.postings[index].term_freq == 1 {
                let code = self.base.postings[index].last_doc_code | 1;
                self.base.write_v_int(pool, term_id, 0, code)?;
            } else {
                let code = self.base.postings[index].last_doc_code;
                let freq = self.base.postings[index].term_freq;
                self.base.write_v_int(pool, term_id, 0, code)?;
                self.base.write_v_int(pool, term_id, 0, freq)?;
            }

            let freq = self.term_freq(token)?;
            self.base.postings[index].term_freq = freq;
            field_state
                .set_max_term_frequency(std::cmp::max(freq, field_state.max_term_frequency()));
            self.base.postings[index].last_doc_code =
                (doc_id - self.base.postings[index].last_doc_id) << 1;
            self.base.postings[index].last_doc_id = doc_id;
            if self.has_prox {
                let position = field_state.position();
                self.write_prox(pool, field_state, term_id, position, token)?;
                if self.has_offsets {
                    self.base.postings[index].last_offset = 0;
                    self.write_offsets(pool, term_id, field_state.offset(), token)?;
                }
            }
            field_state.increment_unique_term_count();
        } else {
            if self.is_term_doc {
                return Err(LuceneError::IllegalArgument(format!(
                    "field '{}' has duplicate term",
                    self.field_name()
                )));
            }
            let freq = self.term_freq(token)?;
            self.base.postings[index].term_freq = self.base.postings[index]
                .term_freq
                .checked_add(freq)
                .ok_or_else(|| {
                    LuceneError::IllegalArgument(format!(
                        "too many tokens for field \"{}\"",
                        self.field_name()
                    ))
                })?;
            field_state.set_max_term_frequency(std::cmp::max(
                field_state.max_term_frequency(),
                self.base.postings[index].term_freq,
            ));
            if self.has_prox {
                let delta = field_state.position() - self.base.postings[index].last_position;
                self.write_prox(pool, field_state, term_id, delta, token)?;
                if self.has_offsets {
                    self.write_offsets(pool, term_id, field_state.offset(), token)?;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FreqProxFields: read-back view over the buffered postings
// ---------------------------------------------------------------------------

/// Frozen buffers of one field, shared by the read-back iterators.
#[derive(Debug)]
struct FieldPostings {
    field_name: String,
    index_options: IndexOptions,
    has_freq: bool,
    has_prox: bool,
    has_offsets: bool,
    saw_payloads: bool,
    /// Term ids ordered by their term bytes.
    sorted_term_ids: Vec<i32>,
    /// Maps a term id to the pool offset of its bytes.
    bytes_start: Vec<i32>,
    postings: Vec<FreqProxPosting>,
    pool: Arc<ByteBlockPool>,
}

impl FieldPostings {
    fn term_bytes(&self, ord: usize) -> &[u8] {
        let term_id = self.sorted_term_ids[ord];
        self.pool.term_bytes(self.bytes_start[term_id as usize])
    }

    /// Positions `reader` over `stream` of `term_id`.
    ///
    /// Equivalent to `TermsHashPerField.initReader(ByteSliceReader, int, int)`.
    fn init_reader(&self, reader: &mut ByteSliceReader, term_id: i32, stream: usize) {
        let slot = &self.postings[term_id as usize];
        reader.init(
            slot.byte_start + (stream * FIRST_LEVEL_SIZE) as i32,
            slot.stream_address[stream],
        );
    }
}

/// Read-back view over the postings buffered for a segment.
///
/// Equivalent to `org.apache.lucene.index.FreqProxFields`. Only iteration is
/// supported: the buffers hold no per-term statistics, exactly as in Lucene.
#[derive(Debug)]
pub struct FreqProxFields {
    /// Fields in ascending name order, matching Lucene's sorted field list.
    fields: Vec<Arc<FieldPostings>>,
    by_name: HashMap<String, usize>,
}

impl FreqProxFields {
    fn new(fields: Vec<Arc<FieldPostings>>) -> Self {
        let by_name = fields
            .iter()
            .enumerate()
            .map(|(index, field)| (field.field_name.clone(), index))
            .collect();
        Self { fields, by_name }
    }
}

impl Fields for FreqProxFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.fields.iter().map(|field| field.field_name.clone()))
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(self.by_name.get(field).map(|index| {
            Box::new(FreqProxTerms {
                field: Arc::clone(&self.fields[*index]),
            }) as Box<dyn Terms>
        }))
    }

    fn size(&self) -> i32 {
        self.fields.len() as i32
    }
}

/// Terms of one buffered field.
///
/// Equivalent to `FreqProxFields.FreqProxTerms`.
#[derive(Debug)]
struct FreqProxTerms {
    field: Arc<FieldPostings>,
}

impl Terms for FreqProxTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(FreqProxTermsEnum::new(Arc::clone(&self.field))))
    }

    /// Returns `-1`: the buffers keep no per-field statistics.
    ///
    /// Lucene throws `UnsupportedOperationException` here; the trait cannot
    /// fail, so this reports the "unknown" sentinel the trait documents. The
    /// terms dictionary derives its statistics from the postings writer, so it
    /// never consults these.
    fn size(&self) -> i64 {
        -1
    }

    /// Returns `-1`; see [`Terms::size`] on this type.
    fn sum_total_term_freq(&self) -> i64 {
        -1
    }

    /// Returns `-1`; see [`Terms::size`] on this type.
    fn sum_doc_freq(&self) -> i64 {
        -1
    }

    /// Returns `-1`; see [`Terms::size`] on this type.
    fn doc_count(&self) -> i32 {
        -1
    }

    fn has_freqs(&self) -> bool {
        self.field
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    /// Note: the buffer may hold offsets because that is what the field infos
    /// said when indexing started, but the options may have been downgraded
    /// since; the field's options are therefore the authority, exactly as in
    /// `FreqProxFields.FreqProxTerms`.
    fn has_offsets(&self) -> bool {
        self.field
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    /// See [`Terms::has_offsets`] on this type.
    fn has_positions(&self) -> bool {
        self.field
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.field.saw_payloads
    }
}

/// Iterates the sorted terms of one buffered field.
///
/// Equivalent to `FreqProxFields.FreqProxTermsEnum`.
#[derive(Debug)]
struct FreqProxTermsEnum {
    field: Arc<FieldPostings>,
    num_terms: usize,
    /// Current position, or `-1` before the first [`TermsEnum::next`].
    ord: i64,
    attributes: AttributeSource,
}

impl FreqProxTermsEnum {
    fn new(field: Arc<FieldPostings>) -> Self {
        let num_terms = field.sorted_term_ids.len();
        Self {
            field,
            num_terms,
            ord: -1,
            attributes: AttributeSource::new(),
        }
    }

    fn current_term(&self) -> Result<&[u8]> {
        if self.ord < 0 || self.ord as usize >= self.num_terms {
            return Err(LuceneError::IllegalState(
                "terms enum is not positioned on a term".to_string(),
            ));
        }
        Ok(self.field.term_bytes(self.ord as usize))
    }
}

impl TermsEnum for FreqProxTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        let target = text.slice();
        let mut low = 0i64;
        let mut high = self.num_terms as i64 - 1;
        while high >= low {
            let mid = (low + high) >> 1;
            match self.field.term_bytes(mid as usize).cmp(target) {
                std::cmp::Ordering::Less => low = mid + 1,
                std::cmp::Ordering::Greater => high = mid - 1,
                std::cmp::Ordering::Equal => {
                    self.ord = mid;
                    return Ok(SeekStatus::FOUND);
                }
            }
        }
        self.ord = low;
        if self.ord >= self.num_terms as i64 {
            Ok(SeekStatus::END)
        } else {
            Ok(SeekStatus::NOT_FOUND)
        }
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        if ord < 0 || ord >= self.num_terms as i64 {
            return Err(LuceneError::IllegalArgument(format!(
                "ord {ord} is out of range [0, {})",
                self.num_terms
            )));
        }
        self.ord = ord;
        Ok(())
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(BytesRef::new(self.current_term()?.to_vec()))
    }

    fn ord(&self) -> Result<i64> {
        Ok(self.ord)
    }

    /// Always fails: the buffers do not record a document frequency.
    ///
    /// Equivalent to `FreqProxTermsEnum.docFreq()`, which throws
    /// `UnsupportedOperationException` for the same reason — computing it would
    /// need an extra pass over the postings.
    fn doc_freq(&self) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "buffered postings do not record docFreq".to_string(),
        ))
    }

    /// Always fails; see [`TermsEnum::doc_freq`] on this type.
    fn total_term_freq(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "buffered postings do not record totalTermFreq".to_string(),
        ))
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        if self.ord < 0 || self.ord as usize >= self.num_terms {
            return Err(LuceneError::IllegalState(
                "terms enum is not positioned on a term".to_string(),
            ));
        }
        let term_id = self.field.sorted_term_ids[self.ord as usize];

        if crate::index::feature_requested(flags, POSTINGS_ENUM_POSITIONS) {
            if !self.field.has_prox {
                return Err(LuceneError::IllegalArgument(
                    "did not index positions".to_string(),
                ));
            }
            if !self.field.has_offsets
                && crate::index::feature_requested(flags, POSTINGS_ENUM_OFFSETS)
            {
                return Err(LuceneError::IllegalArgument(
                    "did not index offsets".to_string(),
                ));
            }
            return Ok(Box::new(FreqProxPostingsEnum::new(
                Arc::clone(&self.field),
                term_id,
            )));
        }

        Ok(Box::new(FreqProxDocsEnum::new(
            Arc::clone(&self.field),
            term_id,
        )))
    }

    /// Always fails: impacts require the on-disk postings format.
    ///
    /// Equivalent to `FreqProxTermsEnum.impacts(int)`.
    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "buffered postings do not expose impacts".to_string(),
        ))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.ord += 1;
        if self.ord as usize >= self.num_terms {
            Ok(None)
        } else {
            Ok(Some(BytesRef::new(
                self.field.term_bytes(self.ord as usize).to_vec(),
            )))
        }
    }
}

/// Doc/freq iterator over one buffered term.
///
/// Equivalent to `FreqProxFields.FreqProxDocsEnum`.
#[derive(Debug)]
struct FreqProxDocsEnum {
    field: Arc<FieldPostings>,
    reader: ByteSliceReader,
    read_term_freq: bool,
    doc_id: i32,
    freq: i32,
    ended: bool,
    term_id: i32,
}

impl FreqProxDocsEnum {
    fn new(field: Arc<FieldPostings>, term_id: i32) -> Self {
        let read_term_freq = field.has_freq;
        let mut reader = ByteSliceReader::new();
        field.init_reader(&mut reader, term_id, 0);
        Self {
            field,
            reader,
            read_term_freq,
            doc_id: -1,
            freq: 0,
            ended: false,
            term_id,
        }
    }
}

impl DocIdSetIterator for FreqProxDocsEnum {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.doc_id == -1 {
            self.doc_id = 0;
        }
        if self.reader.eof() {
            if self.ended {
                return Ok(NO_MORE_DOCS);
            }
            self.ended = true;
            let slot = &self.field.postings[self.term_id as usize];
            self.doc_id = slot.last_doc_id;
            if self.read_term_freq {
                self.freq = slot.term_freq;
            }
        } else {
            let code = self.reader.read_v_int(&self.field.pool)?;
            if self.read_term_freq {
                self.doc_id += ((code as u32) >> 1) as i32;
                if code & 1 != 0 {
                    self.freq = 1;
                } else {
                    self.freq = self.reader.read_v_int(&self.field.pool)?;
                }
            } else {
                self.doc_id += code;
            }
        }
        Ok(self.doc_id)
    }

    /// Always fails: the buffered postings are read strictly sequentially.
    ///
    /// Equivalent to `FreqProxDocsEnum.advance(int)`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "buffered postings cannot be advanced".to_string(),
        ))
    }

    /// Returns [`i64::MAX`]: the cost is unknown without a full pass.
    ///
    /// Lucene throws `UnsupportedOperationException`; the trait cannot fail, so
    /// this reports the most pessimistic estimate instead.
    fn cost(&self) -> i64 {
        i64::MAX
    }
}

impl PostingsEnum for FreqProxDocsEnum {
    fn freq(&self) -> Result<i32> {
        if self.read_term_freq {
            Ok(self.freq)
        } else {
            Err(LuceneError::IllegalState(
                "freq was not indexed".to_string(),
            ))
        }
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(-1)
    }

    fn start_offset(&self) -> i32 {
        -1
    }

    fn end_offset(&self) -> i32 {
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(None)
    }
}

/// Doc/freq/position iterator over one buffered term.
///
/// Equivalent to `FreqProxFields.FreqProxPostingsEnum`.
#[derive(Debug)]
struct FreqProxPostingsEnum {
    field: Arc<FieldPostings>,
    reader: ByteSliceReader,
    pos_reader: ByteSliceReader,
    read_offsets: bool,
    doc_id: i32,
    freq: i32,
    position: i32,
    start_offset: i32,
    end_offset: i32,
    positions_left: i32,
    term_id: i32,
    ended: bool,
    payload: Vec<u8>,
    has_payload: bool,
}

impl FreqProxPostingsEnum {
    fn new(field: Arc<FieldPostings>, term_id: i32) -> Self {
        debug_assert!(field.has_prox && field.has_freq);
        let read_offsets = field.has_offsets;
        let mut reader = ByteSliceReader::new();
        let mut pos_reader = ByteSliceReader::new();
        field.init_reader(&mut reader, term_id, 0);
        field.init_reader(&mut pos_reader, term_id, 1);
        Self {
            field,
            reader,
            pos_reader,
            read_offsets,
            doc_id: -1,
            freq: 0,
            position: 0,
            start_offset: 0,
            end_offset: 0,
            positions_left: 0,
            term_id,
            ended: false,
            payload: Vec::new(),
            has_payload: false,
        }
    }
}

impl DocIdSetIterator for FreqProxPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.doc_id == -1 {
            self.doc_id = 0;
        }
        // The proximity stream must be drained before moving on, otherwise the
        // next document would read this one's leftover positions.
        while self.positions_left != 0 {
            self.next_position()?;
        }

        if self.reader.eof() {
            if self.ended {
                return Ok(NO_MORE_DOCS);
            }
            self.ended = true;
            let slot = &self.field.postings[self.term_id as usize];
            self.doc_id = slot.last_doc_id;
            self.freq = slot.term_freq;
        } else {
            let code = self.reader.read_v_int(&self.field.pool)?;
            self.doc_id += ((code as u32) >> 1) as i32;
            if code & 1 != 0 {
                self.freq = 1;
            } else {
                self.freq = self.reader.read_v_int(&self.field.pool)?;
            }
        }

        self.positions_left = self.freq;
        self.position = 0;
        self.start_offset = 0;
        Ok(self.doc_id)
    }

    /// Always fails; see [`DocIdSetIterator::advance`] on [`FreqProxDocsEnum`].
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "buffered postings cannot be advanced".to_string(),
        ))
    }

    /// Returns [`i64::MAX`]; see [`DocIdSetIterator::cost`] on
    /// [`FreqProxDocsEnum`].
    fn cost(&self) -> i64 {
        i64::MAX
    }
}

impl PostingsEnum for FreqProxPostingsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.freq)
    }

    fn next_position(&mut self) -> Result<i32> {
        debug_assert!(self.positions_left > 0);
        self.positions_left -= 1;
        let code = self.pos_reader.read_v_int(&self.field.pool)?;
        self.position += ((code as u32) >> 1) as i32;
        if code & 1 != 0 {
            self.has_payload = true;
            let length = self.pos_reader.read_v_int(&self.field.pool)? as usize;
            self.payload.clear();
            self.payload.resize(length, 0);
            let pool = Arc::clone(&self.field.pool);
            self.pos_reader.read_bytes(&pool, &mut self.payload);
        } else {
            self.has_payload = false;
        }

        if self.read_offsets {
            self.start_offset += self.pos_reader.read_v_int(&self.field.pool)?;
            self.end_offset = self.start_offset + self.pos_reader.read_v_int(&self.field.pool)?;
        }

        Ok(self.position)
    }

    fn start_offset(&self) -> i32 {
        if self.read_offsets {
            self.start_offset
        } else {
            -1
        }
    }

    fn end_offset(&self) -> i32 {
        if self.read_offsets {
            self.end_offset
        } else {
            -1
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        if self.has_payload {
            Ok(Some(&self.payload))
        } else {
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// FreqProxTermsWriter
// ---------------------------------------------------------------------------

/// Buffers the postings of every field of a segment and flushes them.
///
/// Equivalent to `org.apache.lucene.index.FreqProxTermsWriter`.
#[derive(Debug)]
pub struct FreqProxTermsWriter {
    terms_hash: TermsHash,
    /// Per-field writers keyed by field name, in first-seen order.
    fields: Vec<FreqProxTermsWriterPerField>,
    field_index: HashMap<String, usize>,
}

impl FreqProxTermsWriter {
    /// Creates an empty writer charging its buffers to `bytes_used`.
    pub fn new(bytes_used: Arc<AtomicI64>) -> Self {
        Self {
            terms_hash: TermsHash::new(bytes_used),
            fields: Vec::new(),
            field_index: HashMap::new(),
        }
    }

    /// Returns the per-field writer for `field_info`, creating it on first use.
    ///
    /// Equivalent to `FreqProxTermsWriter.addField(FieldInvertState, FieldInfo)`
    /// combined with the `PerField` cache of `IndexingChain`.
    pub fn add_field(&mut self, field_info: &FieldInfo) -> usize {
        if let Some(index) = self.field_index.get(&field_info.name) {
            return *index;
        }
        let index = self.fields.len();
        self.fields.push(FreqProxTermsWriterPerField::new(
            field_info.clone(),
            Arc::clone(self.terms_hash.bytes_used()),
        ));
        self.field_index.insert(field_info.name.clone(), index);
        index
    }

    /// Returns the *term* byte pool, where every token's text is interned.
    ///
    /// Equivalent to `TermsHash.termBytePool`, which Lucene shares with the
    /// term-vectors hash: that hash keys on offsets into this pool and reads
    /// the term bytes back from it at flush time.
    pub fn pool(&self) -> &ByteBlockPool {
        self.terms_hash.pool()
    }

    /// Returns the byte pool and the writer of `index` at the same time.
    ///
    /// Splitting the borrow this way is what lets one shared pool serve every
    /// per-field writer, which is exactly Lucene's arrangement.
    pub fn pool_and_field(
        &mut self,
        index: usize,
    ) -> (&mut ByteBlockPool, &mut FreqProxTermsWriterPerField) {
        (self.terms_hash.pool_mut(), &mut self.fields[index])
    }

    /// Returns the per-field writer of `index`.
    pub fn field(&self, index: usize) -> &FreqProxTermsWriterPerField {
        &self.fields[index]
    }

    /// Returns the per-field writer for `field_name`, if the field was seen.
    pub fn field_by_name(&self, field_name: &str) -> Option<&FreqProxTermsWriterPerField> {
        self.field_index
            .get(field_name)
            .map(|index| &self.fields[*index])
    }

    /// Discards every buffered posting.
    ///
    /// Equivalent to `TermsHash.abort()`.
    pub fn abort(&mut self) {
        for field in &mut self.fields {
            field.release_accounting();
        }
        self.fields.clear();
        self.field_index.clear();
        self.terms_hash.reset();
    }

    /// Freezes the buffers into the read-back view the codec consumes.
    ///
    /// Equivalent to the `FreqProxFields` construction inside
    /// `FreqProxTermsWriter.flush`: only fields that saw at least one term are
    /// included, and they are sorted by name.
    fn freeze(&mut self) -> FreqProxFields {
        let pool = Arc::new(self.terms_hash.take_pool());
        let mut frozen: Vec<Arc<FieldPostings>> = Vec::new();
        for mut field in std::mem::take(&mut self.fields) {
            if field.num_terms() == 0 {
                field.base.release_accounting();
                continue;
            }
            let sorted_term_ids = field.base.bytes_hash.sort(&pool);
            let bytes_start: Vec<i32> = (0..field.base.bytes_hash.size())
                .map(|term_id| field.base.bytes_hash.byte_start(term_id))
                .collect();
            // The buffers move into the frozen view; the counter must stop
            // charging them here, because nothing will reset this table again.
            let postings = std::mem::take(&mut field.base.postings);
            field.base.release_accounting();
            frozen.push(Arc::new(FieldPostings {
                field_name: field.base.field_name.clone(),
                index_options: field.base.index_options,
                has_freq: field.has_freq,
                has_prox: field.has_prox,
                has_offsets: field.has_offsets,
                saw_payloads: field.saw_payloads,
                sorted_term_ids,
                bytes_start,
                postings,
                pool: Arc::clone(&pool),
            }));
        }
        self.field_index.clear();
        // `CollectionUtil.introSort(allFields)` orders by
        // `TermsHashPerField.compareTo`, which is `String.compareTo`: UTF-16
        // code-unit order, not UTF-8 byte order. The two differ above
        // `U+E000`, and this order is written into the `.tim` file, so the
        // Java comparator is the one that has to be reproduced.
        frozen.sort_by(|left, right| compare_utf16(&left.field_name, &right.field_name));
        FreqProxFields::new(frozen)
    }

    /// Writes the buffered postings into the segment described by `state`.
    ///
    /// Equivalent to `FreqProxTermsWriter.flush`. Segment-private delete terms
    /// are applied first, so `state.live_docs` and `state.del_count_on_flush`
    /// may be updated before the codec runs.
    ///
    /// # Errors
    ///
    /// Propagates any I/O or consistency error raised while writing.
    pub fn flush(
        &mut self,
        state: &mut SegmentWriteState<'_>,
        delete_terms: &[TermDelete],
        norms: &dyn NormsProducer,
    ) -> Result<()> {
        let fields = self.freeze();
        self.terms_hash.reset();

        if !state.field_infos.has_postings() {
            debug_assert_eq!(fields.size(), 0);
            return Ok(());
        }

        apply_deletes(state, &fields, delete_terms)?;

        let codec = state.segment_info.codec().ok_or_else(|| {
            LuceneError::IllegalState(format!(
                "segment {} has no codec; cannot write postings",
                state.segment_info.name
            ))
        })?;
        let mut consumer = codec.postings_format().fields_consumer(state)?;
        let outcome = consumer.write(&fields, norms);
        let closed = consumer.close();
        outcome?;
        closed
    }
}

/// Applies this segment's buffered delete-by-term to the flushing segment.
///
/// Equivalent to `FreqProxTermsWriter.applyDeletes`. Each delete term removes
/// every document strictly below its `docIDUpto`, which is the document count
/// at the moment the delete was buffered.
///
/// # Errors
///
/// Propagates errors raised while reading the buffered postings.
fn apply_deletes(
    state: &mut SegmentWriteState<'_>,
    fields: &FreqProxFields,
    delete_terms: &[TermDelete],
) -> Result<()> {
    let max_doc = state.segment_info.max_doc()?;
    apply_deletes_to(
        fields,
        delete_terms,
        max_doc,
        &mut state.live_docs,
        &mut state.del_count_on_flush,
    )
}

/// Core of [`apply_deletes`], split out so it can be driven without a
/// `SegmentWriteState`.
///
/// # Errors
///
/// Propagates errors raised while reading the buffered postings.
fn apply_deletes_to(
    fields: &FreqProxFields,
    delete_terms: &[TermDelete],
    max_doc: i32,
    live_docs: &mut Option<FixedBitSet>,
    del_count_on_flush: &mut i32,
) -> Result<()> {
    if delete_terms.is_empty() || max_doc <= 0 {
        return Ok(());
    }

    // Lucene iterates the delete terms grouped by field and sorted inside each
    // field so that one terms enum can be reused; the resulting deletions are
    // the same in any order.
    let mut ordered: Vec<&TermDelete> = delete_terms.iter().collect();
    ordered.sort_by(|left, right| {
        left.term
            .field()
            .cmp(right.term.field())
            .then_with(|| left.term.bytes().slice().cmp(right.term.bytes().slice()))
    });

    let mut current_field: Option<String> = None;
    let mut terms_enum: Option<Box<dyn TermsEnum>> = None;
    for delete in ordered {
        if current_field.as_deref() != Some(delete.term.field()) {
            current_field = Some(delete.term.field().to_string());
            terms_enum = match fields.terms(delete.term.field())? {
                Some(terms) => Some(terms.iterator()?),
                None => None,
            };
        }
        let Some(enumerator) = terms_enum.as_mut() else {
            continue;
        };
        if !enumerator.seek_exact(delete.term.bytes())? {
            continue;
        }
        let mut postings = enumerator.postings(None, 0)?;
        loop {
            let doc = postings.next_doc()?;
            if doc >= delete.doc_id_upto || doc == NO_MORE_DOCS {
                break;
            }
            let bits = live_docs.get_or_insert_with(|| {
                let mut bits = FixedBitSet::new(max_doc as usize);
                for index in 0..max_doc as usize {
                    bits.set(index);
                }
                bits
            });
            if bits.get(doc as usize) {
                bits.clear(doc as usize);
                *del_count_on_flush += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::terms::Term;
    use crate::index::{
        DocValuesSkipIndexType, DocValuesType, VectorEncoding, VectorSimilarityFunction,
    };
    use crate::util::BytesRef;

    /// Builds a minimal `FieldInfo` for `name` with the given index options.
    fn field_info(name: &str, options: IndexOptions, store_payloads: bool) -> FieldInfo {
        FieldInfo::new_full(
            name,
            0,
            false,
            true,
            store_payloads,
            options,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            -1,
            HashMap::new(),
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )
        .expect("field info")
    }

    /// One position as read back from the buffered postings.
    #[derive(Debug, PartialEq, Eq)]
    struct ReadPosition {
        position: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<Vec<u8>>,
    }

    /// One document's postings as read back.
    #[derive(Debug, PartialEq, Eq)]
    struct ReadDoc {
        doc: i32,
        freq: i32,
        positions: Vec<ReadPosition>,
    }

    /// A scripted token, mirroring what the analysis chain would produce.
    #[derive(Debug, Clone)]
    struct Token {
        term: &'static str,
        position_increment: i32,
        start_offset: i32,
        end_offset: i32,
        payload: Option<Vec<u8>>,
        term_freq: i32,
        has_term_freq_attribute: bool,
    }

    impl Token {
        fn new(term: &'static str, position_increment: i32) -> Self {
            Self {
                term,
                position_increment,
                start_offset: 0,
                end_offset: term.len() as i32,
                payload: None,
                term_freq: 1,
                has_term_freq_attribute: false,
            }
        }

        fn at(term: &'static str, position_increment: i32, start: i32, end: i32) -> Self {
            Self {
                start_offset: start,
                end_offset: end,
                ..Self::new(term, position_increment)
            }
        }

        fn with_payload(mut self, payload: &[u8]) -> Self {
            self.payload = Some(payload.to_vec());
            self
        }

        fn with_term_freq(mut self, term_freq: i32) -> Self {
            self.term_freq = term_freq;
            self.has_term_freq_attribute = true;
            self
        }
    }

    /// Drives one field of one segment: feeds documents and freezes the result.
    struct Harness {
        writer: FreqProxTermsWriter,
        field_state: FieldInvertState,
        index: usize,
    }

    impl Harness {
        fn new(options: IndexOptions) -> Self {
            Self::with_field(field_info("body", options, false))
        }

        fn with_field(info: FieldInfo) -> Self {
            let bytes_used = Arc::new(AtomicI64::new(0));
            let mut writer = FreqProxTermsWriter::new(bytes_used);
            let index = writer.add_field(&info);
            let field_state = FieldInvertState::new(10, info.name.clone(), info.index_options);
            Self {
                writer,
                field_state,
                index,
            }
        }

        /// Indexes one document as a single field value.
        fn add_doc(&mut self, doc_id: i32, tokens: &[Token]) -> Result<()> {
            self.field_state.reset();
            self.add_value(doc_id, tokens)
        }

        /// Indexes one more value of the same field in the same document,
        /// applying the position and offset gaps the way the chain does.
        fn add_value(&mut self, doc_id: i32, tokens: &[Token]) -> Result<()> {
            for token in tokens {
                let next = self.field_state.position() + token.position_increment;
                self.field_state.set_position(next);
                let inverted = InvertedToken {
                    start_offset: token.start_offset,
                    end_offset: token.end_offset,
                    payload: token.payload.as_deref(),
                    term_freq: token.term_freq,
                    has_term_freq_attribute: token.has_term_freq_attribute,
                };
                let (pool, field) = self.writer.pool_and_field(self.index);
                field.add(
                    pool,
                    &mut self.field_state,
                    token.term.as_bytes(),
                    doc_id,
                    &inverted,
                )?;
            }
            Ok(())
        }

        /// Applies the gaps a multi-valued field pays between two values.
        fn apply_gaps(&mut self, position_gap: i32, offset_gap: i32, last_end_offset: i32) {
            let offset = self.field_state.offset() + last_end_offset + offset_gap;
            self.field_state.set_offset(offset);
            let position = self.field_state.position() + position_gap;
            self.field_state.set_position(position);
        }

        fn freeze(&mut self) -> FreqProxFields {
            self.writer.freeze()
        }
    }

    /// Reads every term of `field` back out of the frozen buffers.
    #[allow(clippy::type_complexity)]
    fn read_back(fields: &FreqProxFields, field: &str) -> Vec<(String, Vec<ReadDoc>)> {
        let terms = fields
            .terms(field)
            .expect("terms")
            .expect("field is present");
        let has_positions = terms.has_positions();
        let has_offsets = terms.has_offsets();
        let has_payloads = terms.has_payloads();
        let has_freqs = terms.has_freqs();
        let mut enumerator = terms.iterator().expect("iterator");
        let mut out = Vec::new();
        while let Some(term) = enumerator.next().expect("next term") {
            let flags = if has_positions {
                if has_offsets && has_payloads {
                    crate::index::POSTINGS_ENUM_ALL
                } else if has_offsets {
                    POSTINGS_ENUM_OFFSETS
                } else if has_payloads {
                    crate::index::POSTINGS_ENUM_PAYLOADS
                } else {
                    POSTINGS_ENUM_POSITIONS
                }
            } else if has_freqs {
                crate::index::POSTINGS_ENUM_FREQS
            } else {
                crate::index::POSTINGS_ENUM_NONE
            };
            let mut postings = enumerator.postings(None, flags).expect("postings");
            let mut docs = Vec::new();
            loop {
                let doc = postings.next_doc().expect("next doc");
                if doc == NO_MORE_DOCS {
                    break;
                }
                let freq = if has_freqs {
                    postings.freq().expect("freq")
                } else {
                    1
                };
                let mut positions = Vec::new();
                if has_positions {
                    for _ in 0..freq {
                        let position = postings.next_position().expect("position");
                        positions.push(ReadPosition {
                            position,
                            start_offset: postings.start_offset(),
                            end_offset: postings.end_offset(),
                            payload: postings.get_payload().expect("payload").map(<[u8]>::to_vec),
                        });
                    }
                }
                docs.push(ReadDoc {
                    doc,
                    freq,
                    positions,
                });
            }
            out.push((
                String::from_utf8(term.slice().to_vec()).expect("utf-8 term"),
                docs,
            ));
        }
        out
    }

    // -- ByteSlicePool / ByteSliceReader -----------------------------------

    #[test]
    fn new_slice_marks_the_end_of_the_slice_with_level_zero() {
        let mut pool = ByteBlockPool::new(Arc::new(AtomicI64::new(0)));
        let start = ByteSlicePool::new_slice(&mut pool, FIRST_LEVEL_SIZE).expect("slice");
        let block = pool.buffer_upto() as usize;
        assert_eq!(pool.buffer(block)[start + FIRST_LEVEL_SIZE - 1], 16);
        assert!(pool.buffer(block)[start..start + FIRST_LEVEL_SIZE - 1]
            .iter()
            .all(|byte| *byte == 0));
    }

    #[test]
    fn a_stream_longer_than_one_slice_round_trips_through_the_reader() {
        // 4 000 bytes forces the allocator through every level of
        // LEVEL_SIZE_ARRAY and several block boundaries.
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut field = TermsHashPerField::new(
            1,
            "body".to_string(),
            IndexOptions::DOCS,
            Arc::clone(&bytes_used),
        );
        let slot = field.intern(&mut pool, b"term", 0).expect("intern");
        assert!(slot.is_new);

        let expected: Vec<u8> = (0..4000u32).map(|i| (i % 251) as u8).collect();
        for byte in &expected {
            field
                .write_byte(&mut pool, slot.term_id, 0, *byte)
                .expect("write byte");
        }

        let mut reader = ByteSliceReader::new();
        let posting = *field.posting(slot.term_id);
        reader.init(posting.byte_start, posting.stream_address[0]);
        let mut actual = Vec::with_capacity(expected.len());
        while !reader.eof() {
            actual.push(reader.read_byte(&pool));
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn write_bytes_spans_several_slices_and_reads_back_identically() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut field = TermsHashPerField::new(
            1,
            "body".to_string(),
            IndexOptions::DOCS,
            Arc::clone(&bytes_used),
        );
        let slot = field.intern(&mut pool, b"term", 0).expect("intern");

        // Each chunk is far longer than the first slice level, so `write_bytes`
        // must chain slices in its second loop.
        let chunks: Vec<Vec<u8>> = (1..40usize)
            .map(|n| (0..n * 13).map(|i| (i % 253) as u8).collect())
            .collect();
        for chunk in &chunks {
            field
                .write_bytes(&mut pool, slot.term_id, 0, chunk)
                .expect("write bytes");
        }

        let mut reader = ByteSliceReader::new();
        let posting = *field.posting(slot.term_id);
        reader.init(posting.byte_start, posting.stream_address[0]);
        for chunk in &chunks {
            let mut actual = vec![0u8; chunk.len()];
            reader.read_bytes(&pool, &mut actual);
            assert_eq!(&actual, chunk);
        }
        assert!(reader.eof());
    }

    #[test]
    fn v_ints_round_trip_across_slice_boundaries() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut field = TermsHashPerField::new(
            2,
            "body".to_string(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            Arc::clone(&bytes_used),
        );
        let slot = field.intern(&mut pool, b"term", 0).expect("intern");
        let values: Vec<i32> = vec![0, 1, 127, 128, 300, 16_383, 16_384, 1 << 20, i32::MAX];
        // Interleave the two streams to prove they stay independent.
        for value in &values {
            field
                .write_v_int(&mut pool, slot.term_id, 0, *value)
                .expect("stream 0");
            field
                .write_v_int(&mut pool, slot.term_id, 1, *value / 2)
                .expect("stream 1");
        }

        let posting = *field.posting(slot.term_id);
        for stream in 0..2 {
            let mut reader = ByteSliceReader::new();
            reader.init(
                posting.byte_start + (stream * FIRST_LEVEL_SIZE) as i32,
                posting.stream_address[stream],
            );
            for value in &values {
                let expected = if stream == 0 { *value } else { *value / 2 };
                assert_eq!(reader.read_v_int(&pool).expect("v int"), expected);
            }
            assert!(reader.eof(), "stream {stream} must be fully consumed");
        }
    }

    // -- Term interning ----------------------------------------------------

    #[test]
    fn interning_returns_the_same_id_for_a_repeated_term() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut field = TermsHashPerField::new(
            1,
            "body".to_string(),
            IndexOptions::DOCS,
            Arc::clone(&bytes_used),
        );
        let first = field.intern(&mut pool, b"alpha", 0).expect("alpha");
        let second = field.intern(&mut pool, b"beta", 0).expect("beta");
        let again = field.intern(&mut pool, b"alpha", 1).expect("alpha again");
        assert_eq!((first.term_id, first.is_new), (0, true));
        assert_eq!((second.term_id, second.is_new), (1, true));
        assert_eq!((again.term_id, again.is_new), (0, false));
        assert_eq!(field.num_terms(), 2);
    }

    #[test]
    fn ram_accounting_grows_with_buffered_terms_and_returns_on_reset() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut writer = FreqProxTermsWriter::new(Arc::clone(&bytes_used));
        let info = field_info("body", IndexOptions::DOCS_AND_FREQS, false);
        let index = writer.add_field(&info);
        let mut state = FieldInvertState::new(10, "body".to_string(), info.index_options);
        state.reset();
        for i in 0..500u32 {
            let term = format!("term{i}");
            state.set_position(state.position() + 1);
            let (pool, field) = writer.pool_and_field(index);
            field
                .add(
                    pool,
                    &mut state,
                    term.as_bytes(),
                    0,
                    &InvertedToken {
                        start_offset: 0,
                        end_offset: 1,
                        payload: None,
                        term_freq: 1,
                        has_term_freq_attribute: false,
                    },
                )
                .expect("add");
        }
        let after = bytes_used.load(Ordering::Acquire);
        assert!(after > 0, "buffered terms must be accounted");
        writer.abort();
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            0,
            "aborting must return every accounted byte"
        );
    }

    // -- Postings encoding, one test per IndexOptions level ----------------

    #[test]
    fn docs_only_records_documents_without_frequencies() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        harness
            .add_doc(0, &[Token::new("alpha", 1), Token::new("alpha", 1)])
            .expect("doc 0");
        harness.add_doc(1, &[Token::new("beta", 1)]).expect("doc 1");
        harness
            .add_doc(2, &[Token::new("alpha", 1), Token::new("beta", 1)])
            .expect("doc 2");
        let fields = harness.freeze();
        let terms = fields.terms("body").expect("terms").expect("present");
        assert!(!terms.has_freqs());
        assert!(!terms.has_positions());

        let read = read_back(&fields, "body");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].0, "alpha");
        assert_eq!(
            read[0].1.iter().map(|doc| doc.doc).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(read[1].0, "beta");
        assert_eq!(
            read[1].1.iter().map(|doc| doc.doc).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn docs_and_freqs_records_the_occurrence_count_per_document() {
        let mut harness = Harness::new(IndexOptions::DOCS_AND_FREQS);
        harness
            .add_doc(
                0,
                &[
                    Token::new("alpha", 1),
                    Token::new("beta", 1),
                    Token::new("alpha", 1),
                    Token::new("alpha", 1),
                ],
            )
            .expect("doc 0");
        harness
            .add_doc(3, &[Token::new("alpha", 1)])
            .expect("doc 3");
        let fields = harness.freeze();

        let read = read_back(&fields, "body");
        assert_eq!(read[0].0, "alpha");
        assert_eq!(
            read[0]
                .1
                .iter()
                .map(|doc| (doc.doc, doc.freq))
                .collect::<Vec<_>>(),
            vec![(0, 3), (3, 1)]
        );
        assert_eq!(read[1].0, "beta");
        assert_eq!(
            read[1]
                .1
                .iter()
                .map(|doc| (doc.doc, doc.freq))
                .collect::<Vec<_>>(),
            vec![(0, 1)]
        );
    }

    #[test]
    fn positions_are_recorded_with_their_increments() {
        let mut harness = Harness::new(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        harness
            .add_doc(
                0,
                &[
                    Token::new("alpha", 1),
                    Token::new("beta", 3),
                    Token::new("alpha", 1),
                    // A zero increment stacks a synonym on the previous position.
                    Token::new("gamma", 0),
                ],
            )
            .expect("doc 0");
        let fields = harness.freeze();
        let read = read_back(&fields, "body");

        let positions = |name: &str| -> Vec<i32> {
            read.iter().find(|(term, _)| term == name).expect("term").1[0]
                .positions
                .iter()
                .map(|position| position.position)
                .collect()
        };
        assert_eq!(positions("alpha"), vec![0, 4]);
        assert_eq!(positions("beta"), vec![3]);
        assert_eq!(positions("gamma"), vec![4]);
    }

    #[test]
    fn offsets_are_recorded_as_deltas_and_read_back_absolute() {
        let mut harness = Harness::new(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        harness
            .add_doc(
                0,
                &[
                    Token::at("alpha", 1, 0, 5),
                    Token::at("beta", 1, 6, 10),
                    Token::at("alpha", 1, 11, 16),
                ],
            )
            .expect("doc 0");
        let fields = harness.freeze();
        let read = read_back(&fields, "body");

        let alpha = &read
            .iter()
            .find(|(term, _)| term == "alpha")
            .expect("alpha")
            .1[0];
        assert_eq!(
            alpha
                .positions
                .iter()
                .map(|position| (position.start_offset, position.end_offset))
                .collect::<Vec<_>>(),
            vec![(0, 5), (11, 16)]
        );
        let beta = &read
            .iter()
            .find(|(term, _)| term == "beta")
            .expect("beta")
            .1[0];
        assert_eq!(
            beta.positions
                .iter()
                .map(|position| (position.start_offset, position.end_offset))
                .collect::<Vec<_>>(),
            vec![(6, 10)]
        );
    }

    #[test]
    fn payloads_are_recorded_per_position_and_only_where_present() {
        let mut harness = Harness::new(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        harness
            .add_doc(
                0,
                &[
                    Token::new("alpha", 1).with_payload(&[1, 2, 3]),
                    Token::new("alpha", 1),
                    // An empty payload must be treated as no payload at all.
                    Token::new("alpha", 1).with_payload(&[]),
                    Token::new("alpha", 1).with_payload(&vec![9u8; 300]),
                ],
            )
            .expect("doc 0");
        assert!(
            harness.writer.field(harness.index).saw_payloads(),
            "the field must remember that it saw a payload"
        );
        let fields = harness.freeze();
        let read = read_back(&fields, "body");

        let payloads: Vec<Option<Vec<u8>>> = read[0].1[0]
            .positions
            .iter()
            .map(|position| position.payload.clone())
            .collect();
        assert_eq!(
            payloads,
            vec![Some(vec![1, 2, 3]), None, None, Some(vec![9u8; 300]),]
        );
    }

    #[test]
    fn a_multi_valued_field_applies_the_position_and_offset_gaps() {
        let mut harness = Harness::new(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        harness
            .add_doc(0, &[Token::at("alpha", 1, 0, 5)])
            .expect("first value");
        // The chain adds the analyzer's gaps between two values of one field.
        harness.apply_gaps(100, 1, 5);
        harness
            .add_value(0, &[Token::at("alpha", 1, 0, 5)])
            .expect("second value");
        let fields = harness.freeze();
        let read = read_back(&fields, "body");

        assert_eq!(
            read[0].1[0]
                .positions
                .iter()
                .map(|position| (
                    position.position,
                    position.start_offset,
                    position.end_offset
                ))
                .collect::<Vec<_>>(),
            vec![(0, 0, 5), (101, 6, 11)],
            "the second value must start after the position and offset gaps"
        );
        assert_eq!(read[0].1[0].freq, 2);
    }

    #[test]
    fn terms_are_frozen_in_ascending_byte_order() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        harness
            .add_doc(
                0,
                &[
                    Token::new("zeta", 1),
                    Token::new("alpha", 1),
                    Token::new("Alpha", 1),
                    Token::new("beta", 1),
                ],
            )
            .expect("doc 0");
        let fields = harness.freeze();
        let read = read_back(&fields, "body");
        assert_eq!(
            read.iter()
                .map(|(term, _)| term.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "alpha", "beta", "zeta"]
        );
    }

    #[test]
    fn a_field_without_tokens_produces_no_terms_and_is_dropped_at_freeze() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        harness.add_doc(0, &[]).expect("empty doc");
        assert_eq!(harness.writer.field(harness.index).num_terms(), 0);
        let fields = harness.freeze();
        assert_eq!(fields.size(), 0);
        assert!(fields.terms("body").expect("terms").is_none());
    }

    // -- Error paths -------------------------------------------------------

    #[test]
    fn a_custom_term_frequency_is_rejected_when_positions_are_indexed() {
        let mut harness = Harness::new(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        let error = harness
            .add_doc(0, &[Token::new("alpha", 1).with_term_freq(4)])
            .expect_err("must be rejected");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn a_custom_term_frequency_is_rejected_when_frequencies_are_not_indexed() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        harness
            .add_doc(0, &[Token::new("alpha", 1)])
            .expect("first occurrence");
        let error = harness
            .add_doc(1, &[Token::new("alpha", 1).with_term_freq(4)])
            .expect_err("must be rejected");
        assert!(matches!(error, LuceneError::IllegalState(_)), "{error:?}");
    }

    #[test]
    fn a_custom_term_frequency_is_accumulated_when_only_frequencies_are_indexed() {
        let mut harness = Harness::new(IndexOptions::DOCS_AND_FREQS);
        harness
            .add_doc(
                0,
                &[
                    Token::new("alpha", 1).with_term_freq(3),
                    Token::new("alpha", 1).with_term_freq(4),
                ],
            )
            .expect("doc 0");
        let fields = harness.freeze();
        let read = read_back(&fields, "body");
        assert_eq!(read[0].1[0].freq, 7);
    }

    #[test]
    fn a_duplicate_term_is_rejected_in_a_term_doc_field() {
        // `FieldInfo.isTermDocField()` is exactly `DOCS_AND_CUSTOM_FREQS`.
        let info = field_info("body", IndexOptions::DOCS_AND_CUSTOM_FREQS, false);
        assert!(info.is_term_doc_field());
        let mut harness = Harness::with_field(info);
        let error = harness
            .add_doc(0, &[Token::new("alpha", 1), Token::new("alpha", 1)])
            .expect_err("must be rejected");
        assert!(
            matches!(error, LuceneError::IllegalArgument(_)),
            "{error:?}"
        );
    }

    #[test]
    fn a_term_longer_than_the_block_size_is_rejected() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut field = TermsHashPerField::new(
            1,
            "body".to_string(),
            IndexOptions::DOCS,
            Arc::clone(&bytes_used),
        );
        let huge = vec![b'a'; crate::util::MAX_TERM_LENGTH + 1];
        let error = field.intern(&mut pool, &huge, 0).expect_err("too long");
        assert!(
            matches!(error, LuceneError::IllegalArgument(_)),
            "{error:?}"
        );
        assert_eq!(field.num_terms(), 0);
    }

    // -- Terms enum contract ----------------------------------------------

    #[test]
    fn seek_ceil_finds_terms_and_reports_the_next_one_otherwise() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        harness
            .add_doc(
                0,
                &[
                    Token::new("alpha", 1),
                    Token::new("delta", 1),
                    Token::new("gamma", 1),
                ],
            )
            .expect("doc 0");
        let fields = harness.freeze();
        let terms = fields.terms("body").expect("terms").expect("present");
        let mut enumerator = terms.iterator().expect("iterator");

        assert_eq!(
            enumerator
                .seek_ceil(&BytesRef::new(b"delta".to_vec()))
                .expect("seek"),
            SeekStatus::FOUND
        );
        assert_eq!(enumerator.term().expect("term").slice(), b"delta");
        assert_eq!(enumerator.ord().expect("ord"), 1);

        assert_eq!(
            enumerator
                .seek_ceil(&BytesRef::new(b"beta".to_vec()))
                .expect("seek"),
            SeekStatus::NOT_FOUND
        );
        assert_eq!(enumerator.term().expect("term").slice(), b"delta");

        assert_eq!(
            enumerator
                .seek_ceil(&BytesRef::new(b"zulu".to_vec()))
                .expect("seek"),
            SeekStatus::END
        );
    }

    #[test]
    fn doc_freq_and_impacts_are_reported_as_unsupported() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        harness
            .add_doc(0, &[Token::new("alpha", 1)])
            .expect("doc 0");
        let fields = harness.freeze();
        let terms = fields.terms("body").expect("terms").expect("present");
        let mut enumerator = terms.iterator().expect("iterator");
        enumerator.next().expect("first term");
        assert!(matches!(
            enumerator.doc_freq(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            enumerator.total_term_freq(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            enumerator.impacts(0),
            Err(LuceneError::UnsupportedOperation(_))
        ));
    }

    #[test]
    fn requesting_positions_from_a_docs_only_field_is_refused() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        harness
            .add_doc(0, &[Token::new("alpha", 1)])
            .expect("doc 0");
        let fields = harness.freeze();
        let terms = fields.terms("body").expect("terms").expect("present");
        let mut enumerator = terms.iterator().expect("iterator");
        enumerator.next().expect("first term");
        assert!(matches!(
            enumerator.postings(None, POSTINGS_ENUM_POSITIONS),
            Err(LuceneError::IllegalArgument(_))
        ));
    }

    #[test]
    fn requesting_offsets_from_a_positions_only_field_is_refused() {
        let mut harness = Harness::new(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        harness
            .add_doc(0, &[Token::new("alpha", 1)])
            .expect("doc 0");
        let fields = harness.freeze();
        let terms = fields.terms("body").expect("terms").expect("present");
        let mut enumerator = terms.iterator().expect("iterator");
        enumerator.next().expect("first term");
        assert!(matches!(
            enumerator.postings(None, POSTINGS_ENUM_OFFSETS),
            Err(LuceneError::IllegalArgument(_))
        ));
    }

    // -- applyDeletes ------------------------------------------------------

    #[test]
    fn apply_deletes_clears_documents_below_the_doc_id_upto() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        for doc in 0..5 {
            harness
                .add_doc(doc, &[Token::new("alpha", 1)])
                .expect("doc");
        }
        harness.add_doc(5, &[Token::new("beta", 1)]).expect("doc 5");
        let fields = harness.freeze();

        let mut live_docs = None;
        let mut del_count = 0;
        let deletes = vec![TermDelete {
            term: Term::new("body", BytesRef::new(b"alpha".to_vec())),
            doc_id_upto: 3,
        }];
        apply_deletes_to(&fields, &deletes, 6, &mut live_docs, &mut del_count).expect("deletes");

        assert_eq!(del_count, 3, "docs 0, 1 and 2 carry the deleted term");
        let bits = live_docs.expect("live docs");
        assert!(!bits.get(0) && !bits.get(1) && !bits.get(2));
        assert!(bits.get(3) && bits.get(4) && bits.get(5));
    }

    #[test]
    fn apply_deletes_ignores_terms_and_fields_that_are_absent() {
        let mut harness = Harness::new(IndexOptions::DOCS);
        harness
            .add_doc(0, &[Token::new("alpha", 1)])
            .expect("doc 0");
        let fields = harness.freeze();

        let mut live_docs = None;
        let mut del_count = 0;
        let deletes = vec![
            TermDelete {
                term: Term::new("body", BytesRef::new(b"missing".to_vec())),
                doc_id_upto: 10,
            },
            TermDelete {
                term: Term::new("other", BytesRef::new(b"alpha".to_vec())),
                doc_id_upto: 10,
            },
        ];
        apply_deletes_to(&fields, &deletes, 1, &mut live_docs, &mut del_count).expect("deletes");
        assert_eq!(del_count, 0);
        assert!(
            live_docs.is_none(),
            "no bitset is allocated when nothing is deleted"
        );
    }

    // -- Chained term hash --------------------------------------------------

    #[test]
    fn a_chained_table_keys_on_the_offset_the_primary_interned_the_term_at() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut term_pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut streams = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut primary = TermsHashPerField::new(
            2,
            "body".to_string(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            Arc::clone(&bytes_used),
        );
        let mut chained = TermsHashPerField::new_chained(
            2,
            "body".to_string(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            Arc::clone(&bytes_used),
        );

        let mut text_starts = Vec::new();
        for term in ["gamma", "alpha", "gamma", "beta"] {
            let slot = primary
                .intern(&mut term_pool, term.as_bytes(), 0)
                .expect("intern");
            text_starts.push(primary.text_start(slot.term_id));
        }
        // "gamma" was interned once, so it has one offset and one chained id.
        assert_eq!(text_starts[0], text_starts[2]);

        let ids: Vec<TermSlot> = text_starts
            .iter()
            .map(|start| {
                chained
                    .intern_by_text_start(&mut streams, *start)
                    .expect("chained intern")
            })
            .collect();
        assert_eq!(
            ids[0],
            TermSlot {
                term_id: 0,
                is_new: true
            }
        );
        assert_eq!(
            ids[1],
            TermSlot {
                term_id: 1,
                is_new: true
            }
        );
        assert_eq!(
            ids[2],
            TermSlot {
                term_id: 0,
                is_new: false
            }
        );
        assert_eq!(
            ids[3],
            TermSlot {
                term_id: 2,
                is_new: true
            }
        );
        assert_eq!(chained.num_terms(), 3);

        let sorted: Vec<&[u8]> = chained
            .sorted_term_ids(&term_pool)
            .into_iter()
            .map(|id| term_pool.term_bytes(chained.text_start(id)))
            .collect();
        assert_eq!(sorted, vec![&b"alpha"[..], &b"beta"[..], &b"gamma"[..]]);
    }

    #[test]
    fn a_chained_table_writes_its_streams_into_its_own_pool() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut term_pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut streams = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut primary = TermsHashPerField::new(
            1,
            "body".to_string(),
            IndexOptions::DOCS,
            Arc::clone(&bytes_used),
        );
        let mut chained = TermsHashPerField::new_chained(
            2,
            "body".to_string(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            Arc::clone(&bytes_used),
        );
        let slot = primary.intern(&mut term_pool, b"alpha", 0).expect("intern");
        let text_start = primary.text_start(slot.term_id);
        let term_pool_len_before = term_pool.byte_upto();

        let chained_slot = chained
            .intern_by_text_start(&mut streams, text_start)
            .expect("chained intern");
        for stream in 0..2 {
            for value in [0, 1, 300, 70_000] {
                chained
                    .write_v_int(&mut streams, chained_slot.term_id, stream, value)
                    .expect("write");
            }
        }
        assert_eq!(
            term_pool.byte_upto(),
            term_pool_len_before,
            "the chained table must not touch the term pool"
        );
        for stream in 0..2 {
            let mut reader = ByteSliceReader::new();
            chained.init_reader(&mut reader, chained_slot.term_id, stream);
            for value in [0, 1, 300, 70_000] {
                assert_eq!(reader.read_v_int(&streams).expect("read"), value);
            }
            assert!(reader.eof());
        }
    }

    #[test]
    fn resetting_a_chained_table_keeps_it_keyed_on_pool_offsets() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut term_pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut streams = ByteBlockPool::new(Arc::clone(&bytes_used));
        let text_start = term_pool.add_bytes_ref(b"alpha").expect("intern");
        let mut chained = TermsHashPerField::new_chained(
            2,
            "body".to_string(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            bytes_used,
        );
        chained
            .intern_by_text_start(&mut streams, text_start)
            .expect("first document");
        chained.reset();
        assert_eq!(chained.num_terms(), 0);
        let slot = chained
            .intern_by_text_start(&mut streams, text_start)
            .expect("second document");
        assert!(slot.is_new, "the reset dropped the previous document's ids");
    }

    // -- PooledSliceReader --------------------------------------------------

    #[test]
    fn a_pooled_slice_reader_reads_the_stream_as_a_data_input() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut field = TermsHashPerField::new(
            1,
            "body".to_string(),
            IndexOptions::DOCS,
            Arc::clone(&bytes_used),
        );
        let slot = field.intern(&mut pool, b"term", 0).expect("intern");
        let payload: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8).collect();
        field
            .write_v_int(&mut pool, slot.term_id, 0, 12_345)
            .expect("v int");
        field
            .write_bytes(&mut pool, slot.term_id, 0, &payload)
            .expect("bytes");

        let mut reader = ByteSliceReader::new();
        field.init_reader(&mut reader, slot.term_id, 0);
        let mut input = PooledSliceReader::new(reader, &pool);
        assert_eq!(input.read_v_int().expect("v int"), 12_345);
        let mut read_back = vec![0u8; payload.len()];
        input
            .read_bytes(&mut read_back, 0, payload.len())
            .expect("bytes");
        assert_eq!(read_back, payload);
        assert!(input.eof());
    }

    #[test]
    fn a_pooled_slice_reader_reports_an_over_read_instead_of_panicking() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut field = TermsHashPerField::new(
            1,
            "body".to_string(),
            IndexOptions::DOCS,
            Arc::clone(&bytes_used),
        );
        let slot = field.intern(&mut pool, b"term", 0).expect("intern");
        field
            .write_byte(&mut pool, slot.term_id, 0, 7)
            .expect("byte");

        let mut reader = ByteSliceReader::new();
        field.init_reader(&mut reader, slot.term_id, 0);
        let mut input = PooledSliceReader::new(reader, &pool);
        assert_eq!(input.read_byte().expect("byte"), 7);
        let error = input.read_byte().expect_err("the stream held one byte");
        assert!(matches!(error, LuceneError::CorruptIndex(_)), "{error:?}");

        let mut reader = ByteSliceReader::new();
        field.init_reader(&mut reader, slot.term_id, 0);
        let mut input = PooledSliceReader::new(reader, &pool);
        let mut buffer = [0u8; 4];
        let error = input
            .read_bytes(&mut buffer, 0, 4)
            .expect_err("the stream held one byte");
        assert!(matches!(error, LuceneError::CorruptIndex(_)), "{error:?}");

        let mut reader = ByteSliceReader::new();
        field.init_reader(&mut reader, slot.term_id, 0);
        let mut input = PooledSliceReader::new(reader, &pool);
        let error = input.skip_bytes(-1).expect_err("negative skip");
        assert!(
            matches!(error, LuceneError::IllegalArgument(_)),
            "{error:?}"
        );
        input.skip_bytes(1).expect("one byte to skip");
        assert!(input.eof());
    }

    // -- Field ordering ------------------------------------------------------

    #[test]
    fn buffered_fields_are_frozen_in_utf16_name_order() {
        // `U+10000` sorts before `U+FFFF` in Java's `String.compareTo` and
        // after it in Rust's `str` ordering; the order reaches the `.tim` file.
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut writer = FreqProxTermsWriter::new(Arc::clone(&bytes_used));
        let names = ["\u{FFFF}", "\u{10000}", "a"];
        for (number, name) in names.iter().enumerate() {
            let mut info = FieldInfo::new(*name, number as i32);
            info.index_options = IndexOptions::DOCS;
            let index = writer.add_field(&info);
            let mut state = FieldInvertState::new(10, (*name).to_string(), IndexOptions::DOCS);
            let (pool, field) = writer.pool_and_field(index);
            field
                .add(
                    pool,
                    &mut state,
                    b"token",
                    0,
                    &InvertedToken {
                        start_offset: 0,
                        end_offset: 5,
                        payload: None,
                        term_freq: 1,
                        has_term_freq_attribute: false,
                    },
                )
                .expect("add");
        }
        let fields = writer.freeze();
        let frozen: Vec<String> = fields.iterator().collect();
        assert_eq!(frozen, vec!["a", "\u{10000}", "\u{FFFF}"]);
    }
}
