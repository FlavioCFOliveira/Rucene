//! Concurrency helpers ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`Counter`] | `Counter` and its two nested implementations |
//! | [`SetOnce`] | `SetOnce<T>` |
//! | [`SameThreadExecutorService`] | `SameThreadExecutorService` |
//! | [`NamedThreadFactory`] | `NamedThreadFactory` |
//! | [`WeakIdentityMap`] | `WeakIdentityMap<K, V>` |

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use crate::error::{LuceneError, Result};

// ---------------------------------------------------------------------------
// Counter
// ---------------------------------------------------------------------------

/// A simple shared counter.
///
/// Port of `org.apache.lucene.util.Counter`, which Lucene 10.5.0 marks
/// `@lucene.internal` and `@lucene.experimental`.
///
/// **Divergence from Lucene 10.5.0.** Java has two implementations behind an
/// abstract class: `SerialCounter`, a plain `long` field, and `AtomicCounter`,
/// an `AtomicLong`. A plain field shared by reference is not expressible in
/// safe Rust: the closest form, a `Cell<i64>`, would make the type `!Sync` and
/// so would forbid sharing even the thread-safe variant across threads. Both
/// variants therefore share one atomic cell and differ only in memory ordering
/// — `Relaxed` for the serial one, `SeqCst` for the thread-safe one. Under
/// single-threaded use, which is the only use the serial counter promises, the
/// observable behaviour is identical.
#[derive(Debug)]
pub struct Counter {
    count: AtomicI64,
    thread_safe: bool,
}

impl Counter {
    /// Returns a new counter that is not intended for concurrent use.
    ///
    /// Equivalent to `Counter.newCounter()`.
    pub fn new_counter() -> Self {
        Self::new_counter_with(false)
    }

    /// Returns a new counter, thread-safe when `thread_safe` is `true`.
    ///
    /// Equivalent to `Counter.newCounter(boolean)`.
    pub fn new_counter_with(thread_safe: bool) -> Self {
        Self {
            count: AtomicI64::new(0),
            thread_safe,
        }
    }

    /// Adds `delta` to the counter's current value and returns the new value.
    pub fn add_and_get(&self, delta: i64) -> i64 {
        let ordering = if self.thread_safe {
            Ordering::SeqCst
        } else {
            Ordering::Relaxed
        };
        self.count.fetch_add(delta, ordering) + delta
    }

    /// Returns the counter's current value.
    pub fn get(&self) -> i64 {
        let ordering = if self.thread_safe {
            Ordering::SeqCst
        } else {
            Ordering::Relaxed
        };
        self.count.load(ordering)
    }

    /// Returns whether this counter was created for concurrent use.
    pub fn is_thread_safe(&self) -> bool {
        self.thread_safe
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new_counter()
    }
}

// ---------------------------------------------------------------------------
// SetOnce
// ---------------------------------------------------------------------------

/// A semi-immutable wrapper whose value may be set exactly once and read many
/// times.
///
/// Port of `org.apache.lucene.util.SetOnce`, which Lucene 10.5.0 marks
/// `@lucene.experimental`. Java's nested `AlreadySetException` extends
/// `IllegalStateException`; the Rust equivalent is
/// [`LuceneError::IllegalState`] carrying the same message.
#[derive(Debug)]
pub struct SetOnce<T> {
    cell: OnceLock<T>,
}

/// The message Java's `SetOnce.AlreadySetException` carries.
pub const ALREADY_SET_MESSAGE: &str = "The object cannot be set twice!";

impl<T> SetOnce<T> {
    /// Creates an unset instance.
    pub fn new() -> Self {
        Self {
            cell: OnceLock::new(),
        }
    }

    /// Creates an instance already holding `obj`; any later
    /// [`SetOnce::set`] fails.
    pub fn with_value(obj: T) -> Self {
        let cell = OnceLock::new();
        let _ = cell.set(obj);
        Self { cell }
    }

    /// Sets the value.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the value was already set,
    /// which is Java's `AlreadySetException`.
    pub fn set(&self, obj: T) -> Result<()> {
        if !self.try_set(obj) {
            return Err(LuceneError::IllegalState(ALREADY_SET_MESSAGE.to_string()));
        }
        Ok(())
    }

    /// Sets the value if none was set before, returning whether it was stored.
    pub fn try_set(&self, obj: T) -> bool {
        self.cell.set(obj).is_ok()
    }

    /// Returns the value, or `None` when it was never set.
    ///
    /// Java returns `null` in that case.
    pub fn get(&self) -> Option<&T> {
        self.cell.get()
    }
}

impl<T> Default for SetOnce<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> Clone for SetOnce<T> {
    /// Java's `SetOnce` implements `Cloneable`; the clone shares the
    /// already-set state, which is what a shallow Java clone of the internal
    /// `AtomicReference` snapshot produces.
    fn clone(&self) -> Self {
        match self.cell.get() {
            Some(v) => Self::with_value(v.clone()),
            None => Self::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// SameThreadExecutorService
// ---------------------------------------------------------------------------

/// An executor that runs every task immediately on the calling thread.
///
/// Port of `org.apache.lucene.util.SameThreadExecutorService`.
///
/// **Divergence from Lucene 10.5.0.** Java extends `AbstractExecutorService`,
/// inheriting `submit`, `invokeAll` and the `Future` plumbing. Rust's standard
/// library has no `ExecutorService`, so this exposes exactly the methods the
/// Java class itself defines; a caller that wants a future runs the task and
/// keeps its result, which is all the same-thread executor ever did.
#[derive(Debug, Default)]
pub struct SameThreadExecutorService {
    shutdown: AtomicBool,
}

impl SameThreadExecutorService {
    /// Creates a running executor.
    pub fn new() -> Self {
        Self {
            shutdown: AtomicBool::new(false),
        }
    }

    /// Runs `command` on the calling thread and returns its result.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once the executor is shut down,
    /// which is Java's `RejectedExecutionException("Executor is shut down.")`.
    pub fn execute<R, F: FnOnce() -> R>(&self, command: F) -> Result<R> {
        self.check_shutdown()?;
        Ok(command())
    }

    /// Shuts the executor down.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Shuts the executor down and returns the tasks that never ran — always
    /// none, because tasks run on submission.
    pub fn shutdown_now(&self) -> Vec<()> {
        self.shutdown();
        Vec::new()
    }

    /// Returns whether the executor has terminated.
    ///
    /// Simplified exactly as Java is: no attempt is made to detect a thread
    /// still inside `execute`.
    pub fn is_terminated(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Returns whether the executor has been shut down.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Waits for termination; always returns `true` immediately, as Java's
    /// override does.
    pub fn await_termination(&self, _timeout: std::time::Duration) -> bool {
        true
    }

    fn check_shutdown(&self) -> Result<()> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(LuceneError::IllegalState(
                "Executor is shut down.".to_string(),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// NamedThreadFactory
// ---------------------------------------------------------------------------

/// Counts the factories created so far. `NamedThreadFactory.threadPoolNumber`.
static THREAD_POOL_NUMBER: AtomicUsize = AtomicUsize::new(1);

/// A thread factory that gives every thread a recognisable name.
///
/// Port of `org.apache.lucene.util.NamedThreadFactory`, including its
/// `"%s-%d-thread"` prefix pattern and the `"%s-%d"` per-thread suffix.
///
/// **Divergence from Lucene 10.5.0.** Java also places the thread in a
/// `ThreadGroup`, clears the daemon flag and sets `Thread.NORM_PRIORITY`.
/// Rust's `std::thread` has no thread groups, no daemon flag and no portable
/// priority, so only the naming carries over.
#[derive(Debug)]
pub struct NamedThreadFactory {
    thread_name_prefix: String,
    thread_number: AtomicUsize,
}

impl NamedThreadFactory {
    /// Creates a factory naming threads after `thread_name_prefix`.
    ///
    /// An empty prefix becomes `Lucene`, as `NamedThreadFactory.checkPrefix`
    /// specifies.
    pub fn new(thread_name_prefix: &str) -> Self {
        let prefix = if thread_name_prefix.is_empty() {
            "Lucene"
        } else {
            thread_name_prefix
        };
        Self {
            thread_name_prefix: format!(
                "{prefix}-{}-thread",
                THREAD_POOL_NUMBER.fetch_add(1, Ordering::SeqCst)
            ),
            thread_number: AtomicUsize::new(1),
        }
    }

    /// Returns the prefix every thread this factory creates is named after.
    pub fn thread_name_prefix(&self) -> &str {
        &self.thread_name_prefix
    }

    /// Returns the name the next thread would receive.
    pub fn next_thread_name(&self) -> String {
        format!(
            "{}-{}",
            self.thread_name_prefix,
            self.thread_number.fetch_add(1, Ordering::SeqCst)
        )
    }

    /// Spawns a named thread running `r`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] when the thread cannot be spawned.
    pub fn new_thread<F>(&self, r: F) -> Result<std::thread::JoinHandle<()>>
    where
        F: FnOnce() + Send + 'static,
    {
        let name = self.next_thread_name();
        std::thread::Builder::new()
            .name(name)
            .spawn(r)
            .map_err(LuceneError::Io)
    }
}

// ---------------------------------------------------------------------------
// WeakIdentityMap
// ---------------------------------------------------------------------------

/// A weak key reference compared and hashed by identity.
///
/// Port of the nested `WeakIdentityMap.IdentityWeakReference`. Java's identity
/// is `System.identityHashCode` plus reference equality; Rust's is the address
/// the [`Weak`] points at, which stays readable after the value is dropped
/// because the map keeps the weak handle alive.
#[derive(Debug)]
struct IdentityWeakReference<K> {
    weak: Weak<K>,
    hash: usize,
}

impl<K> IdentityWeakReference<K> {
    fn new(key: &Arc<K>) -> Self {
        Self {
            weak: Arc::downgrade(key),
            hash: Arc::as_ptr(key) as *const () as usize,
        }
    }

    fn is_alive(&self) -> bool {
        self.weak.strong_count() > 0
    }
}

impl<K> Hash for IdentityWeakReference<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl<K> PartialEq for IdentityWeakReference<K> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl<K> Eq for IdentityWeakReference<K> {}

/// A hash map keyed by weak references compared by identity.
///
/// Port of `org.apache.lucene.util.WeakIdentityMap`.
///
/// **Divergences from Lucene 10.5.0.**
///
/// * Java's keys are raw objects and the map supports a `null` key through an
///   internal sentinel. Rust keys are [`Arc<K>`] handles, which cannot be null,
///   so the sentinel has nothing to stand for and is gone.
/// * Java clears entries through a `ReferenceQueue` fed by the garbage
///   collector, so `reap()` is O(collected). Rust has no reference queue:
///   [`WeakIdentityMap::reap`] scans the map for keys whose strong count has
///   reached zero. The set of entries removed is the same.
/// * `newConcurrentHashMap` backs the Java map with a `ConcurrentHashMap`.
///   Rust's standard library has no concurrent map, and adding a dependency is
///   a decision for the user, so both constructors build the same structure;
///   wrap it in a `Mutex` or `RwLock` for concurrent use.
/// * Reaping mutates, so [`WeakIdentityMap::get`],
///   [`WeakIdentityMap::contains_key`] and [`WeakIdentityMap::size`] take
///   `&mut self` when `reap_on_read` is enabled — Java hides the same mutation
///   behind a concurrent map.
#[derive(Debug)]
pub struct WeakIdentityMap<K, V> {
    backing_store: HashMap<IdentityWeakReference<K>, V>,
    reap_on_read: bool,
}

impl<K, V> WeakIdentityMap<K, V> {
    /// Creates a map that reaps stale keys on every read.
    ///
    /// Equivalent to `WeakIdentityMap.newHashMap()`.
    pub fn new_hash_map() -> Self {
        Self::new_hash_map_with(true)
    }

    /// Creates a map, reaping stale keys on every read when `reap_on_read`.
    ///
    /// Equivalent to `WeakIdentityMap.newHashMap(boolean)`.
    pub fn new_hash_map_with(reap_on_read: bool) -> Self {
        Self {
            backing_store: HashMap::new(),
            reap_on_read,
        }
    }

    /// Equivalent to `WeakIdentityMap.newConcurrentHashMap()`; see the type
    /// documentation for how concurrency is handled here.
    pub fn new_concurrent_hash_map() -> Self {
        Self::new_hash_map_with(true)
    }

    /// Equivalent to `WeakIdentityMap.newConcurrentHashMap(boolean)`.
    pub fn new_concurrent_hash_map_with(reap_on_read: bool) -> Self {
        Self::new_hash_map_with(reap_on_read)
    }

    /// Empties the map.
    pub fn clear(&mut self) {
        self.backing_store.clear();
        self.reap();
    }

    /// Returns whether `key` is present.
    pub fn contains_key(&mut self, key: &Arc<K>) -> bool {
        if self.reap_on_read {
            self.reap();
        }
        self.backing_store
            .contains_key(&IdentityWeakReference::new(key))
    }

    /// Returns the value stored for `key`.
    pub fn get(&mut self, key: &Arc<K>) -> Option<&V> {
        if self.reap_on_read {
            self.reap();
        }
        self.backing_store.get(&IdentityWeakReference::new(key))
    }

    /// Stores `value` under `key`, returning the previous value.
    pub fn put(&mut self, key: &Arc<K>, value: V) -> Option<V> {
        self.reap();
        self.backing_store
            .insert(IdentityWeakReference::new(key), value)
    }

    /// Returns whether the map is empty.
    pub fn is_empty(&mut self) -> bool {
        self.size() == 0
    }

    /// Removes `key`, returning the value it held.
    pub fn remove(&mut self, key: &Arc<K>) -> Option<V> {
        self.reap();
        self.backing_store.remove(&IdentityWeakReference::new(key))
    }

    /// Returns how many live entries the map holds.
    pub fn size(&mut self) -> usize {
        if self.backing_store.is_empty() {
            return 0;
        }
        if self.reap_on_read {
            self.reap();
        }
        self.backing_store.len()
    }

    /// Iterates over the live keys, dropping the entries whose key is gone.
    ///
    /// Equivalent to `WeakIdentityMap.keyIterator()`; Java holds a strong
    /// reference to the current key while it is exposed, which upgrading the
    /// [`Weak`] to an [`Arc`] does here.
    pub fn key_iterator(&mut self) -> impl Iterator<Item = Arc<K>> + '_ {
        self.reap();
        self.backing_store.keys().filter_map(|k| k.weak.upgrade())
    }

    /// Iterates over the values.
    ///
    /// Equivalent to `WeakIdentityMap.valueIterator()`.
    pub fn value_iterator(&mut self) -> impl Iterator<Item = &V> {
        if self.reap_on_read {
            self.reap();
        }
        self.backing_store.values()
    }

    /// Drops the entries whose key has been released.
    ///
    /// Equivalent to `WeakIdentityMap.reap()`.
    pub fn reap(&mut self) {
        self.backing_store.retain(|k, _| k.is_alive());
    }
}

impl<K, V> Default for WeakIdentityMap<K, V> {
    fn default() -> Self {
        Self::new_hash_map()
    }
}
