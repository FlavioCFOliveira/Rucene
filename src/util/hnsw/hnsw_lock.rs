//! Port of `org.apache.lucene.util.hnsw.HnswLock`.

use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Number of stripes.
///
/// Equivalent to `HnswLock.NUM_LOCKS`.
const NUM_LOCKS: usize = 512;

/// Provides read-and-write striped locks for access to the nodes of an
/// [`OnHeapHnswGraph`](super::on_heap::OnHeapHnswGraph), for use by
/// [`HnswConcurrentMergeBuilder`](super::HnswConcurrentMergeBuilder) and its
/// builders.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswLock`. The concurrent merge
/// creates one (`HnswConcurrentMergeBuilder.java:63`) and hands it to every
/// worker: a worker adds a back-link to a neighbour's array under
/// [`write`](Self::write) (`HnswGraphBuilder.java:432-444`) and copies an
/// array it searches under [`read`](Self::read)
/// (`HnswConcurrentMergeBuilder.java:250-263`).
///
/// Java's stripes are `ReentrantReadWriteLock`s, which Lucene never takes
/// twice on one thread; a `std` [`RwLock`] serves, minus the reentrancy no
/// caller uses.
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

    /// Acquires the read lock guarding `(level, node)`; dropping the guard
    /// releases it.
    ///
    /// Equivalent to `HnswLock.read(int, int)`, which locks and returns the
    /// stripe's read lock. A stripe a panicking thread left poisoned is
    /// acquired all the same: Java's lock has no such state, and Java's
    /// holders release it in `finally` whatever they threw.
    pub fn read(&self, level: i32, node: i32) -> RwLockReadGuard<'_, ()> {
        let lock_id = Self::lock_id(level, node);
        self.locks[lock_id]
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquires the write lock guarding `(level, node)`; dropping the guard
    /// releases it.
    ///
    /// Equivalent to `HnswLock.write(int, int)`; a poisoned stripe is
    /// acquired as [`read`](Self::read) acquires it.
    pub fn write(&self, level: i32, node: i32) -> RwLockWriteGuard<'_, ()> {
        let lock_id = Self::lock_id(level, node);
        self.locks[lock_id]
            .write()
            .unwrap_or_else(PoisonError::into_inner)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Java's `ReentrantReadWriteLock` has no poisoned state: a stripe whose
    /// holder failed is acquired again, as Java's holders release it in
    /// `finally`.
    #[test]
    fn a_stripe_a_panicking_holder_left_is_acquired_again() {
        let lock = HnswLock::new();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lock.write(0, 7);
            panic!("the holder fails");
        }));
        assert!(panicked.is_err());
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(lock.read(0, 7));
            drop(lock.write(0, 7));
        }));
        assert!(outcome.is_ok(), "the stripe is acquired after the panic");
    }
}
