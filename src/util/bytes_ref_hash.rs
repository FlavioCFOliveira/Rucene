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
//! * **[`BytesRefHash::sort`] is a comparison sort.** Lucene uses an MSB radix
//!   sort with a comparison fallback. Both produce exactly the same order —
//!   ascending unsigned byte-wise order — which is all the index format
//!   depends on; the radix sort is a constant-factor optimisation that can be
//!   revisited once term-heavy segments are benchmarked.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::error::Result;
use crate::util::byte_block_pool::{hash_bytes, ByteBlockPool};

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
    bytes_used: Arc<AtomicI64>,
}

impl BytesRefHash {
    /// Creates an empty table with [`HASH_INIT_SIZE`] slots.
    pub fn new(bytes_used: Arc<AtomicI64>) -> Self {
        Self::with_capacity(HASH_INIT_SIZE, bytes_used)
    }

    /// Creates an empty table with `capacity` slots.
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
            bytes_used,
        }
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

    /// Returns the term id of `bytes`, or `-1` when it is not in the table.
    ///
    /// Equivalent to `BytesRefHash.find(BytesRef)`.
    pub fn find(&self, pool: &ByteBlockPool, bytes: &[u8]) -> i32 {
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
    /// Equivalent to `BytesRefHash.sort()`. Unlike Lucene's version this does
    /// not consume the table, so it can be called again after inspection.
    pub fn sort(&self, pool: &ByteBlockPool) -> Vec<i32> {
        let mut sorted: Vec<i32> = (0..self.count).collect();
        sorted.sort_unstable_by(|left, right| {
            pool.term_bytes(self.bytes_start[*left as usize])
                .cmp(pool.term_bytes(self.bytes_start[*right as usize]))
        });
        sorted
    }

    /// Drops both arrays and removes them from the shared RAM counter.
    ///
    /// Java never needs this: the JVM reclaims a discarded `BytesRefHash` and
    /// its `Counter` is per-`DocumentsWriterPerThread`, so a leaked charge
    /// disappears with the thread state. Rust frees eagerly, so the owner must
    /// hand the bytes back when it discards the table. The table is empty and
    /// unusable afterwards, and calling this twice is harmless.
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
    pub fn clear(&mut self) {
        self.bytes_used.fetch_sub(
            (self.bytes_start.len() * std::mem::size_of::<i32>()) as i64,
            Ordering::AcqRel,
        );
        self.bytes_start.clear();
        self.count = 0;
        self.ids.fill(-1);
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
}
