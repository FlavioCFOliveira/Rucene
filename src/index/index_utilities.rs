//! Index-level utilities ported from `org.apache.lucene.index`.
//!
//! Groups the small support types the indexing engine needs: the query-timeout
//! contract, the approximate priority queues that `DocumentsWriter` uses to pick
//! a thread state, and the directory wrapper that tracks temporary merge output.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use crate::error::Result;
use crate::store::{Directory, FilterDirectory, IOContext, IndexInput, IndexOutput, Lock};

// -----------------------------------------------------------------------------
// QueryTimeout
// -----------------------------------------------------------------------------

/// Asks whether a long-running operation should stop.
///
/// Equivalent to `org.apache.lucene.index.QueryTimeout`.
pub trait QueryTimeout: Send + Sync + std::fmt::Debug {
    /// Returns `true` once the operation has run past its allowance.
    ///
    /// Equivalent to `QueryTimeout.shouldExit()`.
    fn should_exit(&self) -> bool;
}

/// A [`QueryTimeout`] that expires a fixed number of milliseconds after it is
/// created.
///
/// Equivalent to `org.apache.lucene.index.QueryTimeoutImpl`.
///
/// **Divergence from Lucene 10.5.0.** Java stores an absolute `System.nanoTime()`
/// deadline in a mutable `Long` field that `reset()` sets to `null`. Rust has no
/// process-wide monotonic epoch to compare against, so this port stores an
/// `Instant` deadline behind a lock. The observable behaviour is the same:
/// `should_exit` is `false` before the deadline, `true` after it, and always
/// `false` once `reset` has been called.
#[derive(Debug)]
pub struct QueryTimeoutImpl {
    timeout_at: RwLock<Option<Instant>>,
}

impl QueryTimeoutImpl {
    /// Creates a timeout that expires `time_allowed` milliseconds from now.
    ///
    /// A negative `time_allowed` means no timeout, as `Long.MAX_VALUE` does in
    /// Java.
    pub fn new(time_allowed: i64) -> Self {
        let deadline = if time_allowed < 0 {
            None
        } else {
            Instant::now().checked_add(std::time::Duration::from_millis(time_allowed as u64))
        };
        Self {
            timeout_at: RwLock::new(deadline),
        }
    }

    /// Returns the deadline, if one is set.
    ///
    /// Equivalent to `QueryTimeoutImpl.getTimeoutAt()`.
    pub fn get_timeout_at(&self) -> Option<Instant> {
        self.timeout_at.read().ok().and_then(|guard| *guard)
    }

    /// Clears the deadline, so the timeout never fires again.
    ///
    /// Equivalent to `QueryTimeoutImpl.reset()`.
    pub fn reset(&self) {
        if let Ok(mut guard) = self.timeout_at.write() {
            *guard = None;
        }
    }
}

impl QueryTimeout for QueryTimeoutImpl {
    fn should_exit(&self) -> bool {
        match self.timeout_at.read() {
            Ok(guard) => match *guard {
                Some(deadline) => Instant::now() > deadline,
                None => false,
            },
            Err(_) => false,
        }
    }
}

// -----------------------------------------------------------------------------
// ApproximatePriorityQueue
// -----------------------------------------------------------------------------

/// Number of sparse slots, one per bit of the `used_slots` bitset.
const SPARSE_SLOTS: usize = u64::BITS as usize;

/// A priority queue that only approximately orders its entries by weight.
///
/// Equivalent to `org.apache.lucene.index.ApproximatePriorityQueue`. Slots `0` to
/// `63` are sparsely populated and indexed by the number of leading zeros of an
/// entry's weight, so heavier entries sit closer to the front; slots from `64`
/// onwards are densely populated and polled back to front.
pub struct ApproximatePriorityQueue<T> {
    slots: Vec<Option<T>>,
    used_slots: u64,
}

impl<T> Default for ApproximatePriorityQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ApproximatePriorityQueue<T> {
    /// Creates an empty queue.
    pub fn new() -> Self {
        let mut slots = Vec::with_capacity(SPARSE_SLOTS);
        slots.resize_with(SPARSE_SLOTS, || None);
        Self {
            slots,
            used_slots: 0,
        }
    }

    /// Adds `entry` with the given `weight`.
    ///
    /// Equivalent to `ApproximatePriorityQueue.add(T, long)`.
    pub fn add(&mut self, entry: T, weight: i64) {
        let expected_slot = (weight as u64).leading_zeros() as usize;
        let free_slots = !self.used_slots;
        let destination_slot =
            expected_slot + (free_slots >> expected_slot).trailing_zeros() as usize;
        if destination_slot < SPARSE_SLOTS {
            self.used_slots |= 1u64 << destination_slot;
            self.slots[destination_slot] = Some(entry);
        } else {
            self.slots.push(Some(entry));
        }
    }

    /// Removes and returns the first entry satisfying `predicate`, or `None`.
    ///
    /// Equivalent to `ApproximatePriorityQueue.poll(Predicate<T>)`.
    pub fn poll(&mut self, mut predicate: impl FnMut(&T) -> bool) -> Option<T> {
        let mut next_slot = 0usize;
        while next_slot < SPARSE_SLOTS {
            let next_used_slot =
                next_slot + (self.used_slots >> next_slot).trailing_zeros() as usize;
            if next_used_slot >= SPARSE_SLOTS {
                break;
            }
            let matches = self.slots[next_used_slot]
                .as_ref()
                .map(&mut predicate)
                .unwrap_or(false);
            if matches {
                self.used_slots &= !(1u64 << next_used_slot);
                return self.slots[next_used_slot].take();
            }
            next_slot = next_used_slot + 1;
        }

        // The dense region is polled in descending order, so that a shrinking
        // number of indexing threads keeps reusing the same entry and the list
        // only ever shortens from its end.
        let mut index = self.slots.len();
        while index > SPARSE_SLOTS {
            index -= 1;
            let matches = self.slots[index]
                .as_ref()
                .map(&mut predicate)
                .unwrap_or(false);
            if matches {
                return self.slots.remove(index);
            }
        }
        None
    }

    /// Returns `true` when the queue holds no entries.
    pub fn is_empty(&self) -> bool {
        self.used_slots == 0 && self.slots.len() == SPARSE_SLOTS
    }
}

impl<T: PartialEq> ApproximatePriorityQueue<T> {
    /// Returns `true` when `entry` is in the queue.
    pub fn contains(&self, entry: &T) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.as_ref().is_some_and(|value| value == entry))
    }

    /// Removes `entry`, returning whether it was present.
    pub fn remove(&mut self, entry: &T) -> bool {
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|value| value == entry))
        else {
            return false;
        };
        if index >= SPARSE_SLOTS {
            self.slots.remove(index);
        } else {
            self.used_slots &= !(1u64 << index);
            self.slots[index] = None;
        }
        true
    }
}

// -----------------------------------------------------------------------------
// ConcurrentApproximatePriorityQueue
// -----------------------------------------------------------------------------

/// Lowest concurrency a [`ConcurrentApproximatePriorityQueue`] accepts.
pub const MIN_CONCURRENCY: usize = 1;
/// Highest concurrency a [`ConcurrentApproximatePriorityQueue`] accepts.
pub const MAX_CONCURRENCY: usize = 256;

/// Returns the default concurrency: roughly four entries per slot when indexing
/// with one thread per core.
fn default_concurrency() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (cores / 4).clamp(MIN_CONCURRENCY, MAX_CONCURRENCY)
}

/// Shards an [`ApproximatePriorityQueue`] across several locks.
///
/// Equivalent to `org.apache.lucene.index.ConcurrentApproximatePriorityQueue`.
pub struct ConcurrentApproximatePriorityQueue<T> {
    queues: Vec<Mutex<ApproximatePriorityQueue<T>>>,
}

impl<T> Default for ConcurrentApproximatePriorityQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ConcurrentApproximatePriorityQueue<T> {
    /// Creates a queue with the default concurrency.
    pub fn new() -> Self {
        Self::with_concurrency(default_concurrency())
    }

    /// Creates a queue sharded `concurrency` ways.
    ///
    /// `concurrency` is clamped into `[MIN_CONCURRENCY, MAX_CONCURRENCY]`, where
    /// Java throws `IllegalArgumentException`: the only two callers in the crate
    /// derive it from the core count, so a value outside the range is a
    /// programming error rather than user input.
    pub fn with_concurrency(concurrency: usize) -> Self {
        let concurrency = concurrency.clamp(MIN_CONCURRENCY, MAX_CONCURRENCY);
        let mut queues = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            queues.push(Mutex::new(ApproximatePriorityQueue::new()));
        }
        Self { queues }
    }

    /// Returns how many shards this queue has.
    pub fn concurrency(&self) -> usize {
        self.queues.len()
    }

    /// Seeds the shard scan so entries spread across shards with a little thread
    /// affinity, as Java does with the thread's hash code.
    fn seed(&self) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        (hasher.finish() & 0xFFFF) as usize
    }

    /// Adds `entry` with the given `weight`, preferring an uncontended shard.
    pub fn add(&self, entry: T, weight: i64) {
        let seed = self.seed();
        let n = self.queues.len();
        for i in 0..n {
            let index = (seed + i) % n;
            if let Ok(mut queue) = self.queues[index].try_lock() {
                queue.add(entry, weight);
                return;
            }
        }
        let index = seed % n;
        if let Ok(mut queue) = self.queues[index].lock() {
            queue.add(entry, weight);
        }
    }

    /// Removes and returns an entry satisfying `predicate`, scanning uncontended
    /// shards first and then blocking on each in turn.
    pub fn poll(&self, mut predicate: impl FnMut(&T) -> bool) -> Option<T> {
        let seed = self.seed();
        let n = self.queues.len();
        for i in 0..n {
            let index = (seed + i) % n;
            if let Ok(mut queue) = self.queues[index].try_lock() {
                if let Some(entry) = queue.poll(&mut predicate) {
                    return Some(entry);
                }
            }
        }
        for i in 0..n {
            let index = (seed + i) % n;
            if let Ok(mut queue) = self.queues[index].lock() {
                if let Some(entry) = queue.poll(&mut predicate) {
                    return Some(entry);
                }
            }
        }
        None
    }
}

impl<T: PartialEq> ConcurrentApproximatePriorityQueue<T> {
    /// Returns `true` when `entry` is in any shard.
    pub fn contains(&self, entry: &T) -> bool {
        self.queues
            .iter()
            .any(|queue| queue.lock().map(|q| q.contains(entry)).unwrap_or(false))
    }

    /// Removes `entry` from whichever shard holds it.
    pub fn remove(&self, entry: &T) -> bool {
        for queue in &self.queues {
            if let Ok(mut queue) = queue.lock() {
                if queue.remove(entry) {
                    return true;
                }
            }
        }
        false
    }
}

// -----------------------------------------------------------------------------
// LockableConcurrentApproximatePriorityQueue
// -----------------------------------------------------------------------------

/// An entry that can be locked before being handed out by
/// [`LockableConcurrentApproximatePriorityQueue`].
///
/// **Divergence from Lucene 10.5.0.** Java bounds the queue's element type on
/// `java.util.concurrent.locks.Lock` and calls `tryLock`/`unlock` on the entry
/// itself. Rust's standard library has no object-safe lock trait to bound on, so
/// the port declares the two operations it actually uses.
pub trait TryLockable {
    /// Attempts to take the entry's lock without blocking.
    fn try_lock_entry(&self) -> bool;
    /// Releases the entry's lock.
    fn unlock_entry(&self);
}

/// A queue whose entries are handed out already locked.
///
/// Equivalent to
/// `org.apache.lucene.index.LockableConcurrentApproximatePriorityQueue`.
pub struct LockableConcurrentApproximatePriorityQueue<T: TryLockable> {
    queue: ConcurrentApproximatePriorityQueue<T>,
    add_and_unlock_counter: AtomicUsize,
}

impl<T: TryLockable> Default for LockableConcurrentApproximatePriorityQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: TryLockable> LockableConcurrentApproximatePriorityQueue<T> {
    /// Creates a queue with the default concurrency.
    pub fn new() -> Self {
        Self {
            queue: ConcurrentApproximatePriorityQueue::new(),
            add_and_unlock_counter: AtomicUsize::new(0),
        }
    }

    /// Creates a queue sharded `concurrency` ways.
    pub fn with_concurrency(concurrency: usize) -> Self {
        Self {
            queue: ConcurrentApproximatePriorityQueue::with_concurrency(concurrency),
            add_and_unlock_counter: AtomicUsize::new(0),
        }
    }

    /// Polls an entry whose lock could be taken, retrying while entries keep
    /// arriving.
    ///
    /// Equivalent to `LockableConcurrentApproximatePriorityQueue.lockAndPoll()`.
    pub fn lock_and_poll(&self) -> Option<T> {
        loop {
            let before = self.add_and_unlock_counter.load(Ordering::Acquire);
            if let Some(entry) = self.queue.poll(|entry| entry.try_lock_entry()) {
                return Some(entry);
            }
            if before == self.add_and_unlock_counter.load(Ordering::Acquire) {
                return None;
            }
        }
    }

    /// Adds `entry` and releases its lock.
    ///
    /// Equivalent to `LockableConcurrentApproximatePriorityQueue.addAndUnlock`.
    pub fn add_and_unlock(&self, entry: T, weight: i64) {
        self.queue.add(entry, weight);
        // The entry was unlocked by `add`'s move, so the counter bump is what
        // tells a concurrent `lock_and_poll` that retrying is worthwhile.
        self.add_and_unlock_counter.fetch_add(1, Ordering::Release);
    }
}

impl<T: TryLockable + PartialEq> LockableConcurrentApproximatePriorityQueue<T> {
    /// Returns `true` when `entry` is in the queue.
    pub fn contains(&self, entry: &T) -> bool {
        self.queue.contains(entry)
    }

    /// Removes `entry` from the queue.
    pub fn remove(&self, entry: &T) -> bool {
        self.queue.remove(entry)
    }
}

// -----------------------------------------------------------------------------
// TrackingTmpOutputDirectoryWrapper
// -----------------------------------------------------------------------------

/// A directory that redirects every `create_output` to a temporary file and
/// remembers the mapping.
///
/// Equivalent to `org.apache.lucene.index.TrackingTmpOutputDirectoryWrapper`,
/// which index sorting uses so a merge can write under the final names while the
/// bytes land in temporary files.
pub struct TrackingTmpOutputDirectoryWrapper {
    inner: FilterDirectory,
    file_names: RwLock<HashMap<String, String>>,
}

impl TrackingTmpOutputDirectoryWrapper {
    /// Wraps `inner`.
    pub fn new(inner: Box<dyn Directory>) -> Self {
        Self {
            inner: FilterDirectory::new(inner),
            file_names: RwLock::new(HashMap::new()),
        }
    }

    /// Returns the mapping from requested name to the temporary file that holds
    /// its bytes.
    ///
    /// Equivalent to `getTemporaryFiles()`.
    pub fn get_temporary_files(&self) -> HashMap<String, String> {
        self.file_names
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

impl std::fmt::Debug for TrackingTmpOutputDirectoryWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackingTmpOutputDirectoryWrapper")
            .finish_non_exhaustive()
    }
}

impl Directory for TrackingTmpOutputDirectoryWrapper {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        let output = self.inner.create_temp_output(name, "", context)?;
        if let Ok(mut guard) = self.file_names.write() {
            guard.insert(name.to_string(), output.name().to_string());
        }
        Ok(output)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        // Fall back to the original name: it may already be a temporary file.
        let tmp_name = self
            .file_names
            .read()
            .ok()
            .and_then(|guard| guard.get(name).cloned())
            .unwrap_or_else(|| name.to_string());
        self.inner.open_input(&tmp_name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }
}
