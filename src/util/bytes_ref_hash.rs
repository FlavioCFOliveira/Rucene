//! Term deduplication table ported from `org.apache.lucene.util.BytesRefHash`.
//!
//! The table maps a byte sequence to a dense, zero-based *term id*. Term ids
//! are handed out in first-seen order, which lets callers keep parallel
//! per-term state in a plain vector indexed by term id. The bytes themselves
//! live in a [`ByteBlockPool`] and are addressed by the global offset returned
//! by [`ByteBlockPool::add_bytes_ref`].
//!
//! # Java to Rust adaptations
//!
//! * **The pool is a parameter, not a field.** Lucene's `BytesRefHash` owns a
//!   reference to the `ByteBlockPool` that stores the term bytes, and the
//!   indexing chain deliberately shares one pool between the per-field hashes
//!   *and* the per-term posting slices. Rust cannot express that aliasing with
//!   owning handles, so every method that touches term bytes takes the pool as
//!   an argument. Ownership therefore lives one level up, in the structure that
//!   owns both the pool and the per-field hashes.
//! * **`BytesStartArray` is gone.** Lucene needs that callback so a subclass can
//!   grow its parallel `int[]` arrays in lockstep with `bytesStart`. Term ids
//!   are dense and monotonically increasing, so this port simply pushes onto a
//!   `Vec`, and callers push onto their own `Vec` in the same step. No callback,
//!   no `System.arraycopy`, no possibility of the two arrays disagreeing.
//! * **The two hash keys are chosen at construction, not per call.** Lucene's
//!   `BytesRefHash` exposes both `add(BytesRef)`, which hashes the *bytes*, and
//!   `addByPoolOffset(int)`, which hashes the *pool offset* of bytes some other
//!   hash already interned (the term-vectors hash uses the latter). The two
//!   store different things in a slot — the first packs the high bits of the
//!   byte hash next to the term id, the second stores the bare term id — so a
//!   table that mixed them would corrupt itself. Java relies on each caller
//!   picking one and never straying; this port makes the choice explicit in the
//!   constructor ([`BytesRefHash::new`] versus
//!   [`BytesRefHash::new_by_pool_offset`]) and asserts it on every entry point.
//! * **[`BytesRefHash::sort`] does not consume the table.** Lucene's `sort()`
//!   reorders the `ids` array in place and returns it, so `clear()` must be
//!   called before the hash is reused. This port sorts a fresh array of the
//!   ids, with the same `StringSorter`/`MSBStringRadixSorter` Lucene uses, and
//!   returns only the `count` sorted ids; the order is the same.

use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::error::Result;
use crate::util::byte_block_pool::{hash_bytes, ByteBlockPool};
use crate::util::selector::{intro_sort, PivotOps};
use crate::util::sorter::{
    MSBRadixSorter, MSBRadixSorterOps, MSB_HISTOGRAM_SIZE, MSB_LENGTH_THRESHOLD,
    MSB_LEVEL_THRESHOLD,
};
use crate::util::RamUsageEstimator;

/// The anonymous `MSBStringRadixSorter` subclass of `BytesRefHash.sort()`
/// (`BytesRefHash.java:205-252`), together with the `StringSorter` it lives in:
/// bytes are read as `BytesRefComparator.NATURAL` reads them, and each entry's
/// bucket is cached at `compact[tmp_offset + i]` while a level is reordered.
///
/// Java keeps the bucket cache in the second half of the `compact` array and
/// writes it from `buildHistogram`; [`MSBRadixSorterOps::build_histogram`]
/// takes `&self`, so the cache is a separate array of cells here. The values
/// and the moments they are written and read are Java's.
struct HashRadixOps<'a> {
    compact: &'a mut [i32],
    /// Equivalent to `compact[tmpOffset ..]`.
    bucket_cache: Vec<Cell<i32>>,
    pool: &'a ByteBlockPool,
    bytes_start: &'a [i32],
    /// The byte the current histogram was built for; Java asserts it matches
    /// the one `reorder` is called with.
    k: Cell<usize>,
}

impl HashRadixOps<'_> {
    /// The term bytes of the entry at `i`.
    ///
    /// Equivalent to the overridden `StringSorter.get(BytesRefBuilder,
    /// BytesRef, int)`: `pool.fillBytesRef(result, bytesStart[compact[i]])`.
    fn term(&self, i: usize) -> &[u8] {
        self.pool
            .term_bytes(self.bytes_start[self.compact[i] as usize])
    }

    /// `swapBucketCache(int, int)`.
    fn swap_bucket_cache(&mut self, i: usize, j: usize) {
        self.compact.swap(i, j);
        self.bucket_cache.swap(i, j);
    }
}

impl MSBRadixSorterOps for HashRadixOps<'_> {
    /// `MSBStringRadixSorter.byteAt`, over `BytesRefComparator.NATURAL.byteAt`.
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        let term = self.term(i);
        if term.len() <= k {
            -1
        } else {
            i32::from(term[k])
        }
    }

    /// The overridden `StringSorter.swap(int, int)`.
    fn swap(&mut self, i: usize, j: usize) {
        self.compact.swap(i, j);
    }

    fn build_histogram(
        &self,
        prefix_common_bucket: usize,
        prefix_common_len: i32,
        from: usize,
        to: usize,
        k: usize,
        histogram: &mut [i32],
    ) {
        self.k.set(k);
        histogram[prefix_common_bucket] = prefix_common_len;
        let fill_from = from - prefix_common_len as usize;
        for cell in &self.bucket_cache[fill_from..from] {
            cell.set(prefix_common_bucket as i32);
        }
        for i in from..to {
            let b = self.get_bucket(i, k);
            self.bucket_cache[i].set(b as i32);
            histogram[b] += 1;
        }
    }

    fn should_fallback(&self, from: usize, to: usize, l: usize) -> bool {
        // We lower the fallback threshold because the bucket cache speeds up
        // the reorder.
        to - from <= MSB_LENGTH_THRESHOLD / 2 || l >= MSB_LEVEL_THRESHOLD
    }

    fn reorder(
        &mut self,
        from: usize,
        _to: usize,
        start_offsets: &mut [i32],
        end_offsets: &[i32],
        k: usize,
    ) {
        debug_assert_eq!(self.k.get(), k);
        for i in 0..MSB_HISTOGRAM_SIZE {
            let limit = end_offsets[i];
            loop {
                let h1 = start_offsets[i];
                if h1 >= limit {
                    break;
                }
                let b = self.bucket_cache[from + h1 as usize].get() as usize;
                let h2 = start_offsets[b];
                start_offsets[b] += 1;
                self.swap_bucket_cache(from + h1 as usize, from + h2 as usize);
            }
        }
    }

    /// `MSBStringRadixSorter.getFallbackSorter(k)`: the `StringSorter`
    /// intro sort under `(o1, o2) -> NATURAL.compare(o1, o2, k)`.
    fn fallback_sort(&mut self, from: usize, to: usize, k: usize, _max_length: usize) {
        let mut fallback = HashFallbackSorter {
            ops: self,
            k,
            pivot: Vec::new(),
        };
        intro_sort(&mut fallback, from, to);
    }
}

/// The intro sort of `StringSorter.fallbackSorter(Comparator)` over the
/// hash's entries, comparing from byte `k` on, as
/// `BytesRefComparator.NATURAL.compare(o1, o2, k)` does.
struct HashFallbackSorter<'a, 'b> {
    ops: &'a mut HashRadixOps<'b>,
    k: usize,
    pivot: Vec<u8>,
}

impl PivotOps for HashFallbackSorter<'_, '_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.ops.compact.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot.clear();
        self.pivot.extend_from_slice(self.ops.term(i));
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        compare_unsigned_from(&self.pivot, self.ops.term(j), self.k)
    }

    fn compare(&mut self, i: usize, j: usize) -> i32 {
        compare_unsigned_from(self.ops.term(i), self.ops.term(j), self.k)
    }
}

/// `Arrays.compareUnsigned` over both arrays from offset `k` to their ends,
/// which is what `BytesRefComparator.NATURAL.compare(o1, o2, k)` computes.
fn compare_unsigned_from(a: &[u8], b: &[u8], k: usize) -> i32 {
    let a = a.get(k..).unwrap_or(&[]);
    let b = b.get(k..).unwrap_or(&[]);
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return i32::from(a[i]) - i32::from(b[i]);
        }
    }
    a.len() as i32 - b.len() as i32
}

/// Initial number of slots in the hash table.
///
/// Equivalent to the `HASH_INIT_SIZE` used by `TermsHashPerField`.
pub const HASH_INIT_SIZE: usize = 4;

/// Maps byte sequences to dense term ids.
///
/// Equivalent to `org.apache.lucene.util.BytesRefHash`.
#[derive(Debug)]
pub struct BytesRefHash {
    /// Maps a term id to the global pool offset of its bytes.
    bytes_start: Vec<i32>,
    /// Open-addressed slots. `-1` marks an empty slot; otherwise the low bits
    /// hold the term id and the high bits hold part of the hash code, so a
    /// probe can reject a mismatch without touching the pool.
    ids: Vec<i32>,
    hash_half_size: usize,
    hash_mask: i32,
    high_mask: i32,
    count: i32,
    /// The count the previous [`clear`](BytesRefHash::clear) saw, which is the
    /// size the table shrinks towards on the next one. `-1` before the first.
    ///
    /// Equivalent to `BytesRefHash.lastCount`.
    last_count: i32,
    bytes_used: Arc<AtomicI64>,
    /// `true` when the table is keyed by the pool offset of already-interned
    /// bytes (`BytesRefHash.addByPoolOffset`) rather than by the bytes
    /// themselves (`BytesRefHash.add`).
    keyed_by_pool_offset: bool,
}

impl BytesRefHash {
    /// Creates an empty table with [`HASH_INIT_SIZE`] slots, keyed by the term
    /// bytes.
    pub fn new(bytes_used: Arc<AtomicI64>) -> Self {
        Self::with_capacity(HASH_INIT_SIZE, bytes_used)
    }

    /// Creates an empty table with [`HASH_INIT_SIZE`] slots, keyed by the pool
    /// offset of bytes another table already interned.
    ///
    /// This is the table Lucene's secondary term hashes use: the term text has
    /// already been interned by the primary hash, so the offset alone
    /// identifies it and neither hashing nor comparing the bytes is needed.
    /// Only [`Self::add_by_pool_offset`] may add to such a table; the bytes
    /// themselves stay in the primary hash's pool, which
    /// [`Self::sort`] and [`Self::byte_start`] still address.
    pub fn new_by_pool_offset(bytes_used: Arc<AtomicI64>) -> Self {
        let mut hash = Self::with_capacity(HASH_INIT_SIZE, bytes_used);
        hash.keyed_by_pool_offset = true;
        hash
    }

    /// Creates an empty table with `capacity` slots, keyed by the term bytes.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero or not a power of two.
    pub fn with_capacity(capacity: usize, bytes_used: Arc<AtomicI64>) -> Self {
        assert!(
            capacity > 0 && capacity.is_power_of_two(),
            "capacity must be a positive power of two, got {capacity}"
        );
        bytes_used.fetch_add(
            (capacity * std::mem::size_of::<i32>()) as i64,
            Ordering::AcqRel,
        );
        Self {
            bytes_start: Vec::new(),
            ids: vec![-1; capacity],
            hash_half_size: capacity >> 1,
            hash_mask: capacity as i32 - 1,
            high_mask: !(capacity as i32 - 1),
            count: 0,
            last_count: -1,
            bytes_used,
            keyed_by_pool_offset: false,
        }
    }

    /// Returns `true` when this table is keyed by pool offset.
    pub fn is_keyed_by_pool_offset(&self) -> bool {
        self.keyed_by_pool_offset
    }

    /// Returns the number of distinct terms in this table.
    pub fn size(&self) -> i32 {
        self.count
    }

    /// Returns `true` if no term was added.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the global pool offset of the bytes of `term_id`.
    ///
    /// # Panics
    ///
    /// Panics if `term_id` was never returned by [`Self::add`].
    pub fn byte_start(&self, term_id: i32) -> i32 {
        self.bytes_start[term_id as usize]
    }

    /// Interns `bytes` and returns its term id.
    ///
    /// Equivalent to `BytesRefHash.add(BytesRef)`: a value seen for the first
    /// time yields its new term id (always `>= 0`), while a value already in
    /// the table yields `-(term_id + 1)`, so the sign of the result tells the
    /// caller which case it is in.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::LuceneError::IllegalArgument`] when `bytes`
    /// exceeds [`crate::util::byte_block_pool::MAX_TERM_LENGTH`], and
    /// [`crate::error::LuceneError::ResourceLimit`] when the pool runs out of
    /// addressable space.
    pub fn add(&mut self, pool: &mut ByteBlockPool, bytes: &[u8]) -> Result<i32> {
        debug_assert!(
            !self.keyed_by_pool_offset,
            "a pool-offset-keyed table must be fed through add_by_pool_offset"
        );
        let hashcode = hash_bytes(bytes);
        let hash_pos = self.find_hash(pool, bytes, hashcode);
        let entry = self.ids[hash_pos];
        if entry != -1 {
            return Ok(-((entry & self.hash_mask) + 1));
        }

        let text_start = pool.add_bytes_ref(bytes)?;
        let term_id = self.count;
        self.bytes_start.push(text_start);
        self.bytes_used
            .fetch_add(std::mem::size_of::<i32>() as i64, Ordering::AcqRel);
        self.ids[hash_pos] = term_id | (hashcode & self.high_mask);
        self.count += 1;
        if self.count as usize == self.hash_half_size {
            self.rehash(pool, self.ids.len() * 2);
        }
        Ok(term_id)
    }

    /// Records `text_start` — the pool offset of bytes another table already
    /// interned — and returns its term id.
    ///
    /// Equivalent to `BytesRefHash.addByPoolOffset(int)`, with the same sign
    /// convention as [`Self::add`]: a value seen for the first time yields its
    /// new term id (always `>= 0`), an offset already in the table yields
    /// `-(term_id + 1)`. The offset *is* the hash key, so no byte is read and
    /// no pool is needed.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when the table is keyed by the term bytes.
    pub fn add_by_pool_offset(&mut self, text_start: i32) -> i32 {
        debug_assert!(
            self.keyed_by_pool_offset,
            "a bytes-keyed table must be fed through add"
        );
        let mut code = text_start;
        let mut hash_pos = (code & self.hash_mask) as usize;
        let mut entry = self.ids[hash_pos];
        while entry != -1 && self.bytes_start[entry as usize] != text_start {
            code = code.wrapping_add(1);
            hash_pos = (code & self.hash_mask) as usize;
            entry = self.ids[hash_pos];
        }
        if entry != -1 {
            return -(entry + 1);
        }

        let term_id = self.count;
        self.bytes_start.push(text_start);
        self.bytes_used
            .fetch_add(std::mem::size_of::<i32>() as i64, Ordering::AcqRel);
        // Unlike the bytes-keyed path the slot holds the bare term id: the
        // probe compares `bytes_start[id]` with the offset, so there are no
        // high bits to shortcut it with (`BytesRefHash.java`, the `hashOnData
        // == false` branch of `rehash`).
        self.ids[hash_pos] = term_id;
        self.count += 1;
        if self.count as usize == self.hash_half_size {
            self.rehash_by_pool_offset(self.ids.len() * 2);
        }
        term_id
    }

    /// Returns the term id of `bytes`, or `-1` when it is not in the table.
    ///
    /// Equivalent to `BytesRefHash.find(BytesRef)`.
    pub fn find(&self, pool: &ByteBlockPool, bytes: &[u8]) -> i32 {
        debug_assert!(
            !self.keyed_by_pool_offset,
            "a pool-offset-keyed table cannot be probed by bytes"
        );
        let hashcode = hash_bytes(bytes);
        let entry = self.ids[self.find_hash(pool, bytes, hashcode)];
        if entry == -1 {
            -1
        } else {
            entry & self.hash_mask
        }
    }

    /// Returns every term id ordered by the unsigned byte-wise order of its
    /// term bytes.
    ///
    /// Equivalent to `BytesRefHash.sort()` (`BytesRefHash.java:198-262`): a
    /// `StringSorter` over `BytesRefComparator.NATURAL` whose radix sorter is
    /// an `MSBStringRadixSorter` caching each entry's bucket in the second half
    /// of the array, and whose fallback is the `StringSorter` intro sort.
    /// Unlike Lucene's version this does not consume the table, so it can be
    /// called again after inspection.
    pub fn sort(&self, pool: &ByteBlockPool) -> Vec<i32> {
        let count = self.count as usize;
        // `compact()` lays the ids out first.
        let mut compact: Vec<i32> = (0..self.count).collect();
        let mut ops = HashRadixOps {
            compact: &mut compact,
            bucket_cache: vec![Cell::new(0); count],
            pool,
            bytes_start: &self.bytes_start,
            k: Cell::new(0),
        };
        let mut radix = MSBRadixSorter::new(i32::MAX as usize);
        radix.sort(&mut ops, 0, count);
        compact
    }

    /// Shallow size of the Java class plus the `Counter` it shares, the fixed
    /// part of [`ram_bytes_used`](Self::ram_bytes_used).
    ///
    /// Equivalent to the private `BytesRefHash.BASE_RAM_BYTES`
    /// (`BytesRefHash.java:36-41`), which is
    /// `RamUsageEstimator.shallowSizeOfInstance(BytesRefHash.class)` — an object
    /// header, the five references `pool`, `bytesStart`, `ids`, `bytesUsed` and
    /// `bytesStartArray`, and the six `int` fields `hashSize`, `hashHalfSize`,
    /// `hashMask`, `highMask`, `count` and `lastCount` — plus
    /// `RamUsageEstimator.primitiveSizes.get(long.class)` for the counter's
    /// `long`. Measured against `lucene-core-10.5.0.jar` on a JVM with
    /// compressed references, the shallow size is 56 and the whole constant 64.
    pub const BASE_RAM_BYTES: i64 = RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + 5 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + 6 * 4,
    ) + 8;

    /// Returns the bytes interned under `bytes_id`.
    ///
    /// Equivalent to `org.apache.lucene.util.BytesRefHash#get(int, BytesRef)`
    /// (`BytesRefHash.java:163-168`), which fills the caller's `BytesRef` from
    /// `pool.fillBytesRef(ref, bytesStart[bytesID])` and hands it back. A
    /// Rucene [`BytesRef`](crate::util::BytesRef) owns its buffer, so there is
    /// no scratch to fill: the bytes are returned as a slice borrowed from the
    /// pool, which is what Java's `BytesRef` is a view over.
    ///
    /// # Panics
    ///
    /// Panics when `bytes_id` is not a term id this table handed out, which is
    /// what Java's two assertions check.
    pub fn get<'p>(&self, pool: &'p ByteBlockPool, bytes_id: i32) -> &'p [u8] {
        assert!(
            bytes_id >= 0 && (bytes_id as usize) < self.bytes_start.len(),
            "bytesID exceeds byteStart len: {}",
            self.bytes_start.len()
        );
        pool.term_bytes(self.bytes_start[bytes_id as usize])
    }

    /// Returns an estimate of the heap this table occupies, including the pool
    /// its terms live in.
    ///
    /// Equivalent to `org.apache.lucene.util.BytesRefHash#ramBytesUsed`
    /// (`BytesRefHash.java:505-512`), which sums
    /// [`BASE_RAM_BYTES`](Self::BASE_RAM_BYTES) with `sizeOfObject` of
    /// `bytesStart`, `ids` and `pool`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reaches the pool through a
    /// `pool` field; this port keeps the pool outside the table — every other
    /// method that needs it takes it as an argument — so the pool is a
    /// parameter here too. That also means this cannot be the
    /// [`Accountable`](crate::util::Accountable) implementation, whose
    /// `ram_bytes_used` takes nothing but `&self`.
    pub fn ram_bytes_used(&self, pool: &ByteBlockPool) -> i64 {
        use crate::util::Accountable;
        Self::BASE_RAM_BYTES
            + RamUsageEstimator::size_of_int(&self.bytes_start)
            + RamUsageEstimator::size_of_int(&self.ids)
            + pool.ram_bytes_used()
    }

    /// Empties the table, resets the pool and releases every byte this table
    /// charged to the shared counter.
    ///
    /// Equivalent to `org.apache.lucene.util.BytesRefHash#close`
    /// (`BytesRefHash.java:314-318`), which calls `clear(true)`, drops `ids`
    /// and returns `Integer.BYTES * hashSize` to the counter. `clear(true)` is
    /// Java's `clear()`; this port's [`clear`](Self::clear) is `clear(false)`,
    /// so the pool reset is spelled out here.
    ///
    /// The table is unusable afterwards until [`reinit`](Self::reinit) puts it
    /// back; calling this twice is harmless.
    pub fn close(&mut self, pool: &mut ByteBlockPool) {
        self.clear();
        pool.reset();
        let hash_size = self.ids.len();
        self.bytes_used.fetch_sub(
            (hash_size * std::mem::size_of::<i32>()) as i64,
            Ordering::AcqRel,
        );
        self.ids = Vec::new();
    }

    /// Drops both arrays and removes them from the shared RAM counter.
    ///
    /// This is [`close`](Self::close) without the pool: it returns the same
    /// bytes to the shared counter that Java's `close()` does — the `ids` and
    /// `bytesStart` arrays — but leaves the pool alone, for the callers that
    /// discard a table without owning the pool it interned into. Rust frees
    /// eagerly, so the owner must hand the bytes back when it drops the table,
    /// whereas the JVM reclaims a discarded `BytesRefHash` on its own. The
    /// table is empty and unusable afterwards, and calling this twice is
    /// harmless.
    pub fn release_accounting(&mut self) {
        let held = (self.ids.len() + self.bytes_start.len()) * std::mem::size_of::<i32>();
        self.bytes_used.fetch_sub(held as i64, Ordering::AcqRel);
        self.ids = Vec::new();
        self.bytes_start = Vec::new();
        self.count = 0;
    }

    /// Removes every term, keeping the table allocated for reuse.
    ///
    /// Equivalent to `BytesRefHash.clear(false)`.
    /// Restores the table's buffers after a [`close`](Self::clear) that
    /// released them.
    ///
    /// Equivalent to `BytesRefHash.reinit()` (`BytesRefHash.java:480-489`),
    /// which reallocates the id and byte-start arrays when a previous
    /// [`close`](Self::close) dropped them, and does nothing when they are
    /// still there.
    ///
    /// Java tests each array against `null`; this port tests each `Vec` for
    /// emptiness, which is the state [`close`](Self::close) and
    /// [`release_accounting`](Self::release_accounting) leave them in. The
    /// slot count comes from `hash_mask`, which those methods do not disturb,
    /// and is Java's `hashSize` field. Whichever way the table got here, it is
    /// left empty, sized, and ready to intern again.
    pub fn reinit(&mut self) {
        if self.ids.is_empty() {
            let hash_size = (self.hash_mask as usize) + 1;
            self.ids = vec![-1i32; hash_size];
            self.bytes_used.fetch_add(
                (hash_size * std::mem::size_of::<i32>()) as i64,
                Ordering::AcqRel,
            );
            self.count = 0;
            self.last_count = -1;
        } else {
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.bytes_used.fetch_sub(
            (self.bytes_start.len() * std::mem::size_of::<i32>()) as i64,
            Ordering::AcqRel,
        );
        self.bytes_start.clear();
        self.last_count = self.count;
        self.count = 0;
        // `shrink` clears the hash entries when it resizes; when it does not,
        // they must be cleared here.
        if self.last_count == -1 || !self.shrink(self.last_count) {
            self.ids.fill(-1);
        }
    }

    /// Shrinks the table towards `target_size`, keeping it a power of two.
    ///
    /// Equivalent to the private `BytesRefHash.shrink(int)`, which halves the
    /// table while a quarter of it would still hold `target_size`. Returns
    /// whether the table was resized — a resize also clears every slot.
    fn shrink(&mut self, target_size: i32) -> bool {
        // ArrayUtil.shrink cannot be used: a power of two is required.
        let hash_size = self.ids.len();
        let mut new_size = hash_size;
        while new_size >= 8 && (new_size / 4) as i32 > target_size {
            new_size /= 2;
        }
        if new_size == hash_size {
            return false;
        }
        self.bytes_used.fetch_sub(
            (std::mem::size_of::<i32>() * (hash_size - new_size)) as i64,
            Ordering::AcqRel,
        );
        self.ids = vec![-1; new_size];
        self.hash_half_size = new_size / 2;
        self.hash_mask = new_size as i32 - 1;
        self.high_mask = !self.hash_mask;
        true
    }

    /// Renumbers the slots so that the first `size()` of them hold the term ids
    /// in the order the terms were added, and returns the slot array.
    ///
    /// Equivalent to `BytesRefHash.compact()`, which [`sort`](Self::sort) runs
    /// first so it can use the freed tail of the array as scratch space.
    pub fn compact(&mut self) -> &mut [i32] {
        // The id is the sequence number the bytes were added to the pool with.
        for i in 0..self.count as usize {
            self.ids[i] = i as i32;
        }
        let count = self.count as usize;
        self.ids[count..].fill(-1);
        self.last_count = self.count;
        &mut self.ids
    }

    /// Hashes `bytes` the way the table does.
    ///
    /// Equivalent to the package-private `BytesRefHash.doHash(byte[], int,
    /// int)`, which is `StringHelper.murmurhash3_x86_32` under
    /// `GOOD_FAST_HASH_SEED`.
    pub fn do_hash(bytes: &[u8], offset: usize, length: usize) -> i32 {
        hash_bytes(&bytes[offset..offset + length])
    }

    /// Returns the slot `bytes` hashes to: either the slot holding it, or the
    /// first empty slot on its probe sequence.
    fn find_hash(&self, pool: &ByteBlockPool, bytes: &[u8], hashcode: i32) -> usize {
        let high_bits = hashcode & self.high_mask;
        let mut code = hashcode;
        let mut hash_pos = (code & self.hash_mask) as usize;
        let mut entry = self.ids[hash_pos];
        while entry != -1
            && ((entry & self.high_mask) != high_bits
                || pool.term_bytes(self.bytes_start[(entry & self.hash_mask) as usize]) != bytes)
        {
            code = code.wrapping_add(1);
            hash_pos = (code & self.hash_mask) as usize;
            entry = self.ids[hash_pos];
        }
        hash_pos
    }

    /// Grows the slot array to `new_size` and reinserts every term.
    ///
    /// Equivalent to `BytesRefHash.rehash(int, boolean)` with `hashOnData`
    /// set: the hash code is recomputed from the term bytes because the number
    /// of high bits stored alongside the term id changes with the table size.
    fn rehash(&mut self, pool: &ByteBlockPool, new_size: usize) {
        debug_assert!(new_size.is_power_of_two());
        self.bytes_used.fetch_add(
            ((new_size - self.ids.len()) * std::mem::size_of::<i32>()) as i64,
            Ordering::AcqRel,
        );
        let new_mask = new_size as i32 - 1;
        let new_high_mask = !new_mask;
        let mut new_ids = vec![-1i32; new_size];
        for slot in 0..self.ids.len() {
            let entry = self.ids[slot];
            if entry == -1 {
                continue;
            }
            let term_id = entry & self.hash_mask;
            let hashcode = pool.hash_at(self.bytes_start[term_id as usize]);
            let mut code = hashcode;
            let mut hash_pos = (code & new_mask) as usize;
            while new_ids[hash_pos] != -1 {
                code = code.wrapping_add(1);
                hash_pos = (code & new_mask) as usize;
            }
            new_ids[hash_pos] = term_id | (hashcode & new_high_mask);
        }
        self.ids = new_ids;
        self.hash_mask = new_mask;
        self.high_mask = new_high_mask;
        self.hash_half_size = new_size >> 1;
    }

    /// Grows the slot array to `new_size` and reinserts every term id.
    ///
    /// Equivalent to `BytesRefHash.rehash(int, boolean)` with `hashOnData`
    /// clear: the pool offset is the hash code, so nothing has to be read back
    /// from the pool and the slot keeps holding the bare term id.
    fn rehash_by_pool_offset(&mut self, new_size: usize) {
        debug_assert!(new_size.is_power_of_two());
        self.bytes_used.fetch_add(
            ((new_size - self.ids.len()) * std::mem::size_of::<i32>()) as i64,
            Ordering::AcqRel,
        );
        let new_mask = new_size as i32 - 1;
        let mut new_ids = vec![-1i32; new_size];
        for term_id in 0..self.count {
            let mut code = self.bytes_start[term_id as usize];
            let mut hash_pos = (code & new_mask) as usize;
            while new_ids[hash_pos] != -1 {
                code = code.wrapping_add(1);
                hash_pos = (code & new_mask) as usize;
            }
            new_ids[hash_pos] = term_id;
        }
        self.ids = new_ids;
        self.hash_mask = new_mask;
        self.high_mask = !new_mask;
        self.hash_half_size = new_size >> 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::byte_block_pool::MAX_TERM_LENGTH;

    fn fixture() -> (ByteBlockPool, BytesRefHash) {
        let bytes_used = Arc::new(AtomicI64::new(0));
        (
            ByteBlockPool::new(Arc::clone(&bytes_used)),
            BytesRefHash::new(bytes_used),
        )
    }

    #[test]
    fn add_returns_new_ids_in_first_seen_order() {
        let (mut pool, mut hash) = fixture();
        assert_eq!(hash.add(&mut pool, b"beta").expect("beta"), 0);
        assert_eq!(hash.add(&mut pool, b"alpha").expect("alpha"), 1);
        assert_eq!(hash.add(&mut pool, b"gamma").expect("gamma"), 2);
        assert_eq!(hash.size(), 3);
    }

    #[test]
    fn add_returns_negated_id_for_a_repeated_term() {
        let (mut pool, mut hash) = fixture();
        let first = hash.add(&mut pool, b"lucene").expect("first");
        let again = hash.add(&mut pool, b"lucene").expect("again");
        assert_eq!(first, 0);
        assert_eq!(again, -1, "-(id + 1) for an existing term");
        assert_eq!(hash.size(), 1);
    }

    #[test]
    fn find_locates_present_terms_and_rejects_absent_ones() {
        let (mut pool, mut hash) = fixture();
        hash.add(&mut pool, b"one").expect("one");
        let two = hash.add(&mut pool, b"two").expect("two");
        assert_eq!(hash.find(&pool, b"two"), two);
        assert_eq!(hash.find(&pool, b"three"), -1);
    }

    #[test]
    fn table_grows_and_keeps_every_term_findable() {
        let (mut pool, mut hash) = fixture();
        let terms: Vec<Vec<u8>> = (0..5000u32)
            .map(|i| format!("term-{i:07}").into_bytes())
            .collect();
        for (expected_id, term) in terms.iter().enumerate() {
            assert_eq!(hash.add(&mut pool, term).expect("add"), expected_id as i32);
        }
        assert_eq!(hash.size(), terms.len() as i32);
        for (expected_id, term) in terms.iter().enumerate() {
            assert_eq!(hash.find(&pool, term), expected_id as i32);
            assert_eq!(
                hash.add(&mut pool, term).expect("re-add"),
                -(expected_id as i32 + 1)
            );
        }
    }

    #[test]
    fn sort_orders_terms_by_unsigned_byte_value() {
        let (mut pool, mut hash) = fixture();
        // 0xFF must sort after 0x01: byte comparison is unsigned, unlike Java's
        // signed `byte`.
        let inputs: Vec<Vec<u8>> = vec![
            vec![0xFF],
            vec![0x01],
            b"a".to_vec(),
            b"ab".to_vec(),
            b"".to_vec(),
            b"A".to_vec(),
        ];
        for input in &inputs {
            hash.add(&mut pool, input).expect("add");
        }
        let sorted: Vec<Vec<u8>> = hash
            .sort(&pool)
            .into_iter()
            .map(|id| pool.term_bytes(hash.byte_start(id)).to_vec())
            .collect();
        let mut expected = inputs.clone();
        expected.sort();
        assert_eq!(sorted, expected);
    }

    /// rmp #151: `sort` runs Lucene's `MSBStringRadixSorter` with the bucket
    /// cache and the lowered fallback threshold (`BytesRefHash.java:198-262`).
    /// Terms with long shared prefixes, empty and one-byte terms, and bytes
    /// above 0x7F drive the radix levels, the common-prefix shortcut and the
    /// intro-sort fallback; the order must be the unsigned byte-wise order.
    #[test]
    fn sort_uses_the_radix_sorter_and_yields_unsigned_byte_order() {
        let (mut pool, mut hash) = fixture();
        let mut inputs: Vec<Vec<u8>> = vec![Vec::new(), vec![0x00], vec![0xFF], vec![0x80, 0x01]];
        for i in 0..3000u32 {
            let mut term = b"common/prefix/".to_vec();
            term.push(((i * 131) % 256) as u8);
            term.extend_from_slice(format!("{}", (i * 7919) % 3001).as_bytes());
            if i % 3 == 0 {
                term.extend_from_slice(b"/tail");
            }
            inputs.push(term);
        }
        inputs.sort();
        inputs.dedup();
        // Insert in a scrambled order so the ids do not already sort.
        let mut scrambled = inputs.clone();
        let n = scrambled.len();
        for i in 0..n {
            scrambled.swap(i, (i * 104_729) % n);
        }
        for input in &scrambled {
            hash.add(&mut pool, input).expect("add");
        }
        let sorted: Vec<Vec<u8>> = hash
            .sort(&pool)
            .into_iter()
            .map(|id| pool.term_bytes(hash.byte_start(id)).to_vec())
            .collect();
        assert_eq!(sorted, inputs);
    }

    #[test]
    fn sort_is_stable_across_repeated_calls_and_survives_growth() {
        let (mut pool, mut hash) = fixture();
        for i in (0..1000u32).rev() {
            hash.add(&mut pool, format!("t{i:05}").as_bytes())
                .expect("add");
        }
        let first = hash.sort(&pool);
        let second = hash.sort(&pool);
        assert_eq!(first, second);
        let terms: Vec<Vec<u8>> = first
            .iter()
            .map(|id| pool.term_bytes(hash.byte_start(*id)).to_vec())
            .collect();
        assert!(terms.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn clear_empties_the_table_but_keeps_it_usable() {
        let (mut pool, mut hash) = fixture();
        hash.add(&mut pool, b"x").expect("x");
        hash.clear();
        assert_eq!(hash.size(), 0);
        assert!(hash.is_empty());
        assert_eq!(hash.find(&pool, b"x"), -1);
        assert_eq!(hash.add(&mut pool, b"x").expect("x again"), 0);
    }

    #[test]
    fn add_rejects_a_term_longer_than_the_block_size() {
        let (mut pool, mut hash) = fixture();
        let too_long = vec![0u8; MAX_TERM_LENGTH + 1];
        assert!(hash.add(&mut pool, &too_long).is_err());
        assert_eq!(hash.size(), 0, "a rejected term must not be interned");
    }

    #[test]
    fn ram_accounting_grows_with_the_table_and_returns_on_clear() {
        // The pool is charged to a counter of its own so that the assertions
        // below observe only what the table itself charges.
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::new(AtomicI64::new(0)));
        let mut hash = BytesRefHash::new(Arc::clone(&bytes_used));
        let after_init = bytes_used.load(Ordering::Acquire);
        assert_eq!(after_init, (HASH_INIT_SIZE * 4) as i64);
        for i in 0..100u32 {
            hash.add(&mut pool, format!("t{i}").as_bytes())
                .expect("add");
        }
        let after_adds = bytes_used.load(Ordering::Acquire);
        assert!(after_adds > after_init);
        hash.clear();
        assert!(bytes_used.load(Ordering::Acquire) < after_adds);
        hash.release_accounting();
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            0,
            "releasing must hand back every byte the table charged"
        );
        hash.release_accounting();
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            0,
            "releasing twice must not double-count"
        );
    }

    // -- Pool-offset keying ------------------------------------------------

    #[test]
    fn add_by_pool_offset_dedupes_by_offset_and_hands_out_dense_ids() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut primary = BytesRefHash::new(Arc::clone(&bytes_used));
        let mut secondary = BytesRefHash::new_by_pool_offset(bytes_used);
        assert!(secondary.is_keyed_by_pool_offset());
        assert!(!primary.is_keyed_by_pool_offset());

        let alpha = primary.add(&mut pool, b"alpha").expect("alpha");
        let beta = primary.add(&mut pool, b"beta").expect("beta");
        let alpha_start = primary.byte_start(alpha);
        let beta_start = primary.byte_start(beta);

        assert_eq!(secondary.add_by_pool_offset(beta_start), 0);
        assert_eq!(secondary.add_by_pool_offset(alpha_start), 1);
        assert_eq!(
            secondary.add_by_pool_offset(beta_start),
            -1,
            "-(id + 1) for an offset already recorded"
        );
        assert_eq!(secondary.size(), 2);
        assert_eq!(secondary.byte_start(0), beta_start);
        assert_eq!(secondary.byte_start(1), alpha_start);
    }

    #[test]
    fn a_pool_offset_table_sorts_by_the_bytes_the_offsets_point_at() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut primary = BytesRefHash::new(Arc::clone(&bytes_used));
        let mut secondary = BytesRefHash::new_by_pool_offset(bytes_used);
        for term in ["zeta", "alpha", "mu"] {
            let id = primary.add(&mut pool, term.as_bytes()).expect("intern");
            secondary.add_by_pool_offset(primary.byte_start(id));
        }
        let sorted: Vec<&[u8]> = secondary
            .sort(&pool)
            .into_iter()
            .map(|id| pool.term_bytes(secondary.byte_start(id)))
            .collect();
        assert_eq!(sorted, vec![&b"alpha"[..], &b"mu"[..], &b"zeta"[..]]);
    }

    #[test]
    fn a_pool_offset_table_survives_its_rehash() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut primary = BytesRefHash::new(Arc::clone(&bytes_used));
        let mut secondary = BytesRefHash::new_by_pool_offset(bytes_used);

        // Enough terms to force several doublings of the four-slot table.
        let mut starts = Vec::new();
        for term in 0..500 {
            let id = primary
                .add(&mut pool, format!("term{term:04}").as_bytes())
                .expect("intern");
            starts.push(primary.byte_start(id));
        }
        for (expected, start) in starts.iter().enumerate() {
            assert_eq!(secondary.add_by_pool_offset(*start), expected as i32);
        }
        assert_eq!(secondary.size(), 500);
        for (id, start) in starts.iter().enumerate() {
            assert_eq!(
                secondary.add_by_pool_offset(*start),
                -(id as i32 + 1),
                "every offset must still be found after the rehash"
            );
            assert_eq!(secondary.byte_start(id as i32), *start);
        }
    }

    #[test]
    fn clearing_a_pool_offset_table_lets_it_be_refilled() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut primary = BytesRefHash::new(Arc::clone(&bytes_used));
        let mut secondary = BytesRefHash::new_by_pool_offset(bytes_used);
        let id = primary.add(&mut pool, b"alpha").expect("alpha");
        let start = primary.byte_start(id);
        assert_eq!(secondary.add_by_pool_offset(start), 0);
        secondary.clear();
        assert_eq!(secondary.size(), 0);
        assert_eq!(
            secondary.add_by_pool_offset(start),
            0,
            "the offset is new again after a clear"
        );
    }

    /// `get(bytesID)` must return exactly the bytes that `add` interned under
    /// that id, which is what Java's `get(int, BytesRef)` fills its scratch
    /// with.
    #[test]
    fn get_returns_the_bytes_interned_under_each_id() {
        let (mut pool, mut hash) = fixture();
        let terms: Vec<&[u8]> = vec![
            b"alpha",
            b"beta",
            b"",
            b"a-much-longer-term-than-the-others",
        ];
        let mut ids = Vec::new();
        for t in &terms {
            ids.push(hash.add(&mut pool, t).expect("add"));
        }
        for (id, expected) in ids.iter().zip(&terms) {
            assert_eq!(hash.get(&pool, *id), *expected, "id={id}");
        }
        // `get` agrees with the offset-based route the rest of the crate uses.
        for id in &ids {
            assert_eq!(hash.get(&pool, *id), pool.term_bytes(hash.byte_start(*id)));
        }
    }

    #[test]
    #[should_panic(expected = "bytesID exceeds byteStart len")]
    fn get_rejects_an_id_the_table_never_handed_out() {
        let (mut pool, mut hash) = fixture();
        hash.add(&mut pool, b"alpha").expect("add");
        hash.get(&pool, 7);
    }

    /// `ramBytesUsed` is `BASE_RAM_BYTES + sizeOfObject(bytesStart) +
    /// sizeOfObject(ids) + sizeOfObject(pool)`.
    #[test]
    fn ram_bytes_used_sums_javas_terms() {
        use crate::util::Accountable;
        let (mut pool, mut hash) = fixture();

        // Measured against lucene-core-10.5.0.jar on a JVM with compressed
        // references: shallowSizeOfInstance(BytesRefHash.class) == 56, and
        // BASE_RAM_BYTES adds the Counter's 8-byte long.
        assert_eq!(BytesRefHash::BASE_RAM_BYTES, 64);

        hash.add(&mut pool, b"hello").expect("hello");
        hash.add(&mut pool, b"world").expect("world");

        // Java's terms, spelled out rather than re-using the implementation's
        // expression. `sizeOf(int[n])` is `align(16 + 4n)`.
        let size_of_int_array = |n: usize| -> i64 {
            let raw = 16 + 4 * n as i64;
            raw + ((8 - raw % 8) % 8)
        };
        let expected = 64
            + size_of_int_array(hash.bytes_start.len())
            + size_of_int_array(hash.ids.len())
            + pool.ram_bytes_used();
        assert_eq!(hash.ram_bytes_used(&pool), expected);

        // Every term is charged: interning more can only raise the estimate.
        let before = hash.ram_bytes_used(&pool);
        for i in 0..1000 {
            hash.add(&mut pool, format!("term-{i:05}").as_bytes())
                .expect("add");
        }
        assert!(hash.ram_bytes_used(&pool) > before);
    }

    /// `close()` empties the table, resets the pool and returns every byte it
    /// charged, which is what Java's `clear(true)` plus the `ids` release does.
    #[test]
    fn close_releases_everything_the_table_charged() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut pool = ByteBlockPool::new(Arc::clone(&bytes_used));
        let mut hash = BytesRefHash::new(Arc::clone(&bytes_used));

        let baseline = bytes_used.load(Ordering::Acquire);
        for i in 0..200 {
            hash.add(&mut pool, format!("term-{i:04}").as_bytes())
                .expect("add");
        }
        assert!(bytes_used.load(Ordering::Acquire) > baseline);
        assert_eq!(hash.size(), 200);

        hash.close(&mut pool);

        assert_eq!(hash.size(), 0);
        // The pool was reset, so its blocks went back to the counter too.
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            0,
            "close must return every byte the table and its pool charged"
        );
        // Calling it twice is harmless.
        hash.close(&mut pool);
        assert_eq!(bytes_used.load(Ordering::Acquire), 0);

        // `reinit` puts the table back to work, as it does after Java's close.
        hash.reinit();
        assert_eq!(hash.add(&mut pool, b"again").expect("again"), 0);
        assert_eq!(hash.get(&pool, 0), b"again");
    }
}
