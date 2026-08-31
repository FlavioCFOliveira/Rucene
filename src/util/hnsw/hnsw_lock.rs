//! Port of `org.apache.lucene.util.hnsw.HnswLock`.

use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Number of stripes.
///
/// Equivalent to `HnswLock.NUM_LOCKS`.
const NUM_LOCKS: usize = 512;

/// Provides read-and-write striped locks for access to the nodes of an
/// [`OnHeapHnswGraph`](super::on_heap::OnHeapHnswGraph).
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswLock`. For use by
/// [`HnswConcurrentMergeBuilder`](super::concurrent_merge_builder::HnswConcurrentMergeBuilder)
/// and its graph builders.
#[derive(Debug)]
pub struct HnswLock {
    locks: Vec<RwLock<()>>,
}

impl Default for HnswLock {
    fn default() -> Self {
        Self::new()
    }
}

impl HnswLock {
    /// Creates the stripes.
    pub fn new() -> Self {
        let mut locks = Vec::with_capacity(NUM_LOCKS);
        for _ in 0..NUM_LOCKS {
            locks.push(RwLock::new(()));
        }
        Self { locks }
    }

    /// Acquires the read lock guarding `(level, node)`.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned, which can only happen after another thread
    /// panicked while holding it; Java's `ReentrantReadWriteLock` has no such state.
    pub fn read(&self, level: i32, node: i32) -> RwLockReadGuard<'_, ()> {
        let lock_id = Self::lock_id(level, node);
        self.locks[lock_id]
            .read()
            .expect("INVARIANT: the guarded unit value cannot be left inconsistent by a panic")
    }

    /// Acquires the write lock guarding `(level, node)`.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned; see [`HnswLock::read`].
    pub fn write(&self, level: i32, node: i32) -> RwLockWriteGuard<'_, ()> {
        let lock_id = Self::lock_id(level, node);
        self.locks[lock_id]
            .write()
            .expect("INVARIANT: the guarded unit value cannot be left inconsistent by a panic")
    }

    fn lock_id(level: i32, node: i32) -> usize {
        // Java computes `hash(level, node) % NUM_LOCKS` on a signed int, which
        // would index out of bounds if the hash were negative; levels and nodes are
        // non-negative, so it never is. Reducing the unsigned bit pattern gives the
        // same stripe over that domain and stays in range for any input.
        (Self::hash(level, node) as u32 as usize) % NUM_LOCKS
    }

    fn hash(v1: i32, v2: i32) -> i32 {
        v1.wrapping_mul(31).wrapping_add(v2)
    }
}
