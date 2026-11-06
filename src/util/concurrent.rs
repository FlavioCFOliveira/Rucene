//! Concurrency helpers ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`Counter`] | `Counter` and its two nested implementations |
//! | [`SetOnce`] | `SetOnce<T>` |
//! | [`SameThreadExecutorService`] | `SameThreadExecutorService` |
//! | [`NamedThreadFactory`] | `NamedThreadFactory` |
//! | [`WeakIdentityMap`] | `WeakIdentityMap<K, V>` |
//!
//! It also models the part of `java.lang.Thread` that Lucene's waits observe:
//! [`InterruptHandle`] is a thread's interrupt status (`Thread.interrupt`,
//! `Thread.isInterrupted`, `Thread.interrupted`), [`wait_interruptibly`] is
//! `Object.wait` on a monitor, and [`InterruptedException`] is the exception
//! that wait throws. [`set_thread_start_hook`] models what a Java thread
//! inherits from the thread that creates it.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LockResult, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

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
            return Err(LuceneError::illegal_state(ALREADY_SET_MESSAGE.to_string()));
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
            return Err(LuceneError::illegal_state(
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
/// priority, so only the naming carries over. In particular the private
/// `NamedThreadFactory.getThreadGroup()` (`NamedThreadFactory.java:51-59`) has
/// no port: it reads `System.getSecurityManager()` and
/// `Thread.currentThread().getThreadGroup()`, and its result is only ever
/// passed to the `Thread(ThreadGroup, Runnable, String, long)` constructor,
/// none of which exists in Rust.
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
        let prefix = Self::check_prefix(thread_name_prefix);
        Self {
            thread_name_prefix: format!(
                "{prefix}-{}-thread",
                THREAD_POOL_NUMBER.fetch_add(1, Ordering::SeqCst)
            ),
            thread_number: AtomicUsize::new(1),
        }
    }

    /// Returns `prefix`, or `Lucene` when it is empty.
    ///
    /// Equivalent to the private `NamedThreadFactory.checkPrefix(String)`.
    /// Java also accepts a null prefix, which `&str` cannot be.
    fn check_prefix(prefix: &str) -> &str {
        if prefix.is_empty() {
            "Lucene"
        } else {
            prefix
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
            .spawn(with_thread_start_context(r))
            .map_err(LuceneError::Io)
    }
}

// ---------------------------------------------------------------------------
// Thread creation context
// ---------------------------------------------------------------------------

/// What a [`ThreadStartHook`] hands from a creating thread to the thread it
/// creates: run on the new thread before its body.
pub type ThreadStartContext = Box<dyn FnOnce() + Send>;

/// Captures, on the creating thread, the context a new thread inherits.
pub type ThreadStartHook = fn() -> ThreadStartContext;

/// The hook [`set_thread_start_hook`] installed, once per process.
static THREAD_START_HOOK: OnceLock<ThreadStartHook> = OnceLock::new();

/// Installs the hook every thread this crate starts runs: it is called on
/// the creating thread, and what it returns runs first on the new thread.
///
/// This models what a Java `Thread` takes from the thread that constructs
/// it — its `ThreadGroup`, which the JVM gives every thread Lucene starts
/// (`ConcurrentMergeScheduler`'s merge threads, its intra-merge executor's
/// workers, `NamedThreadFactory`'s threads, `ControlledRealTimeReopenThread`).
/// A Rust thread inherits nothing from its creator; a test runner that keys
/// state by thread group (randomizedtesting's `RandomizedContext`) installs
/// this hook so that code running on those threads finds its context, as it
/// does on the JVM. Without a hook, threads start with nothing to run first.
///
/// Returns `false`, leaving the installed hook in place, when one is already
/// installed.
pub fn set_thread_start_hook(hook: ThreadStartHook) -> bool {
    THREAD_START_HOOK.set(hook).is_ok()
}

/// Wraps `body`, on the creating thread, so that the new thread first runs
/// what the installed [`ThreadStartHook`] captured.
pub(crate) fn with_thread_start_context<F>(body: F) -> impl FnOnce() + Send + 'static
where
    F: FnOnce() + Send + 'static,
{
    let context = THREAD_START_HOOK.get().map(|hook| hook());
    move || {
        if let Some(context) = context {
            context();
        }
        body();
    }
}

// ---------------------------------------------------------------------------
// Thread interruption
// ---------------------------------------------------------------------------

/// The exception a wait throws when its thread is interrupted.
///
/// Equivalent to `java.lang.InterruptedException` as `Object.wait` throws it:
/// built without a message, so its `toString()` — what
/// [`ThreadInterruptedException`](crate::util::ThreadInterruptedException)
/// reports as its message — is the class name alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterruptedException;

impl Display for InterruptedException {
    /// Java's `Throwable.toString()` of an exception without a message.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("java.lang.InterruptedException")
    }
}

impl std::error::Error for InterruptedException {}

/// One thread's interrupt status, and the monitor it is waiting in.
#[derive(Debug, Default)]
struct InterruptState {
    /// Java's interrupt status of the thread.
    interrupted: AtomicBool,
    /// The condition variable of the monitor the thread waits in, while it
    /// waits in [`wait_interruptibly`].
    blocker: Mutex<Option<Arc<Condvar>>>,
}

impl InterruptState {
    /// Tests and clears the interrupt status, as Java's wait does when it
    /// throws `InterruptedException` and as `Thread.interrupted()` does.
    fn take_interrupt(&self) -> bool {
        // SeqCst orders this load after the publication of the blocker, so an
        // interrupter that missed the blocker set the status before it.
        self.interrupted.swap(false, Ordering::SeqCst)
    }

    fn set_blocker(&self, blocker: Option<Arc<Condvar>>) {
        *self.blocker.lock().unwrap_or_else(PoisonError::into_inner) = blocker;
    }
}

thread_local! {
    /// `Thread.currentThread()`'s interrupt state.
    static CURRENT_THREAD: Arc<InterruptState> = Arc::new(InterruptState::default());
}

/// A handle to one thread's interrupt status: the part of `java.lang.Thread`
/// that Lucene's interruptible waits observe.
///
/// Equivalent to `Thread.interrupt()`, `Thread.isInterrupted()` and
/// `Thread.interrupted()`. A thread obtains its own handle with
/// [`current`](Self::current) and hands it to the threads that may interrupt
/// it. Interrupting sets the status; a thread waiting in
/// [`wait_interruptibly`] is woken and its wait returns
/// [`InterruptedException`] with the status cleared, and a wait entered with
/// the status set returns it at once, as `Object.wait` does
/// (JLS 17.2.1, `Object.wait` Javadoc). Rust threads have no interruption of
/// their own, so only the waits built on [`wait_interruptibly`] observe it.
#[derive(Debug, Clone)]
pub struct InterruptHandle {
    state: Arc<InterruptState>,
}

impl InterruptHandle {
    /// Returns the handle of the calling thread.
    ///
    /// Equivalent to `Thread.currentThread()`, as far as interruption goes.
    pub fn current() -> Self {
        Self {
            state: CURRENT_THREAD.with(Arc::clone),
        }
    }

    /// Interrupts the thread: sets its interrupt status and wakes it out of
    /// the wait it is blocked in, if any.
    ///
    /// Equivalent to `Thread.interrupt()`. The interrupter does not take the
    /// monitor the thread waits in, as Java's does not; see
    /// [`wait_interruptibly`] for how a wake-up that races the thread's entry
    /// into the wait is still delivered.
    pub fn interrupt(&self) {
        self.state.interrupted.store(true, Ordering::SeqCst);
        let blocker = self
            .state
            .blocker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(blocker) = blocker {
            blocker.notify_all();
        }
    }

    /// Returns whether the thread's interrupt status is set, leaving it
    /// unchanged.
    ///
    /// Equivalent to `Thread.isInterrupted()`.
    pub fn is_interrupted(&self) -> bool {
        self.state.interrupted.load(Ordering::SeqCst)
    }

    /// Tests whether the calling thread has been interrupted, and clears its
    /// interrupt status.
    ///
    /// Equivalent to the static `Thread.interrupted()`.
    pub fn interrupted() -> bool {
        CURRENT_THREAD.with(|state| state.take_interrupt())
    }

    /// Returns whether the thread is waiting in [`wait_interruptibly`].
    #[cfg(test)]
    pub(crate) fn is_waiting(&self) -> bool {
        self.state
            .blocker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

/// The longest stretch a waiting thread goes without checking its interrupt
/// status; see [`wait_interruptibly`].
const INTERRUPT_CHECK_INTERVAL: Duration = Duration::from_millis(10);

/// Clears the blocker a waiting thread published, however the wait ends.
struct BlockerRegistration<'a> {
    state: &'a InterruptState,
}

impl Drop for BlockerRegistration<'_> {
    fn drop(&mut self) {
        self.state.set_blocker(None);
    }
}

/// Waits on `condvar`, releasing the monitor `guard` holds, until the thread is
/// notified, `timeout` elapses (`None` waits without a bound) or the thread is
/// interrupted; returns the monitor re-acquired, with
/// `Err(`[`InterruptedException`]`)` when the thread was interrupted.
///
/// Equivalent to `Object.wait(long)` and `Object.wait()` on the monitor that
/// `guard`'s mutex and `condvar` together stand for. As in Java, a thread whose
/// interrupt status is set when it enters, or that is interrupted while it
/// waits, gets the exception and has its status cleared; a normal return may
/// be a notification, the timeout or a spurious wake-up, so the caller
/// re-checks its condition, as every Java caller of `wait` does.
///
/// **Divergence from Lucene 10.5.0 (the JDK).** Java's `Thread.interrupt()`
/// wakes the waiting thread atomically with its entry into the wait. An
/// interrupter here notifies `condvar` without holding the monitor — as Java's
/// does not hold it, and must not, since it may itself be holding the monitor
/// — so a notification sent between the thread's last check of its status and
/// its entry into the wait would be lost. The thread therefore waits in
/// stretches of at most 10 ms and checks its status after each; the stretches
/// are not returned to the caller, so the wait ends only as Java's does, and an
/// interrupt is delivered at most 10 ms late in that race.
///
/// # Errors
///
/// Returns `Err(`[`PoisonError`]`)`, carrying the guard and the wait's outcome,
/// when another thread panicked while holding the monitor.
pub fn wait_interruptibly<'a, T>(
    condvar: &Arc<Condvar>,
    guard: MutexGuard<'a, T>,
    timeout: Option<Duration>,
) -> LockResult<(
    MutexGuard<'a, T>,
    std::result::Result<(), InterruptedException>,
)> {
    let state = CURRENT_THREAD.with(Arc::clone);
    // Publish the monitor before checking the status: an interrupter sets the
    // status before it reads the blocker, so it either finds the blocker and
    // notifies, or set the status before this check sees it.
    state.set_blocker(Some(Arc::clone(condvar)));
    let _registration = BlockerRegistration { state: &state };
    if state.take_interrupt() {
        return Ok((guard, Err(InterruptedException)));
    }
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let mut guard = guard;
    let mut poisoned = false;
    let outcome = loop {
        let stretch = match deadline {
            None => INTERRUPT_CHECK_INTERVAL,
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline {
                    break Ok(());
                }
                (deadline - now).min(INTERRUPT_CHECK_INTERVAL)
            }
        };
        let (reacquired, result) = match condvar.wait_timeout(guard, stretch) {
            Ok(woken) => woken,
            Err(error) => {
                poisoned = true;
                error.into_inner()
            }
        };
        guard = reacquired;
        if state.take_interrupt() {
            break Err(InterruptedException);
        }
        if !result.timed_out() {
            // Notified, or woken spuriously: Java returns either way.
            break Ok(());
        }
    };
    if poisoned {
        Err(PoisonError::new((guard, outcome)))
    } else {
        Ok((guard, outcome))
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

#[cfg(test)]
mod interrupt_tests {
    use super::*;
    use std::sync::mpsc;

    /// A monitor: Java's object with its intrinsic lock and wait set.
    struct Monitor {
        lock: Mutex<()>,
        condvar: Arc<Condvar>,
    }

    fn monitor() -> Arc<Monitor> {
        Arc::new(Monitor {
            lock: Mutex::new(()),
            condvar: Arc::new(Condvar::new()),
        })
    }

    /// A wait's outcome and the thread's interrupt status after it.
    type WaitOutcome = (std::result::Result<(), InterruptedException>, bool);

    /// Starts a thread that waits on `monitor` once, without a timeout, and
    /// reports its handle and then the wait's outcome and its interrupt
    /// status afterwards.
    fn start_waiter(monitor: &Arc<Monitor>) -> (InterruptHandle, mpsc::Receiver<WaitOutcome>) {
        let (handle_tx, handle_rx) = mpsc::channel();
        let (outcome_tx, outcome_rx) = mpsc::channel();
        let monitor = Arc::clone(monitor);
        std::thread::spawn(move || {
            handle_tx.send(InterruptHandle::current()).unwrap();
            let guard = monitor.lock.lock().unwrap();
            let (_guard, outcome) = wait_interruptibly(&monitor.condvar, guard, None).unwrap();
            let _ = outcome_tx.send((outcome, InterruptHandle::current().is_interrupted()));
        });
        (handle_rx.recv().unwrap(), outcome_rx)
    }

    fn wait_until_waiting(handle: &InterruptHandle) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !handle.is_waiting() {
            assert!(
                Instant::now() < deadline,
                "the thread never started waiting"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn interrupting_a_waiting_thread_ends_its_wait_with_the_status_cleared() {
        let monitor = monitor();
        let (handle, outcome) = start_waiter(&monitor);
        wait_until_waiting(&handle);
        handle.interrupt();
        let (outcome, interrupted_after) = outcome
            .recv_timeout(Duration::from_secs(10))
            .expect("the interrupted wait did not return");
        assert_eq!(outcome, Err(InterruptedException));
        assert!(!interrupted_after);
    }

    #[test]
    fn a_wait_entered_with_the_status_set_fails_at_once_and_clears_it() {
        let monitor = monitor();
        InterruptHandle::current().interrupt();
        assert!(InterruptHandle::current().is_interrupted());
        let guard = monitor.lock.lock().unwrap();
        let started = Instant::now();
        let (_guard, outcome) =
            wait_interruptibly(&monitor.condvar, guard, Some(Duration::from_secs(5))).unwrap();
        assert_eq!(outcome, Err(InterruptedException));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!InterruptHandle::current().is_interrupted());
    }

    #[test]
    fn a_notification_ends_the_wait_normally() {
        let monitor = monitor();
        let (handle, outcome) = start_waiter(&monitor);
        wait_until_waiting(&handle);
        {
            let _guard = monitor.lock.lock().unwrap();
            monitor.condvar.notify_all();
        }
        let (outcome, interrupted_after) = outcome
            .recv_timeout(Duration::from_secs(10))
            .expect("the notified wait did not return");
        assert_eq!(outcome, Ok(()));
        assert!(!interrupted_after);
    }

    #[test]
    fn a_timed_wait_returns_normally_after_its_timeout() {
        let monitor = monitor();
        let guard = monitor.lock.lock().unwrap();
        let started = Instant::now();
        let (_guard, outcome) =
            wait_interruptibly(&monitor.condvar, guard, Some(Duration::from_millis(50))).unwrap();
        assert_eq!(outcome, Ok(()));
        assert!(started.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn interrupted_tests_and_clears_the_status() {
        assert!(!InterruptHandle::interrupted());
        InterruptHandle::current().interrupt();
        assert!(InterruptHandle::interrupted());
        assert!(!InterruptHandle::interrupted());
        assert!(!InterruptHandle::current().is_interrupted());
    }

    #[test]
    fn interrupting_a_thread_that_holds_the_monitor_does_not_block() {
        // Java's `Thread.interrupt()` does not take the waiter's monitor, so
        // the interrupter may hold it.
        let monitor = monitor();
        let (handle, outcome) = start_waiter(&monitor);
        wait_until_waiting(&handle);
        {
            let _guard = monitor.lock.lock().unwrap();
            handle.interrupt();
        }
        let (outcome, _) = outcome
            .recv_timeout(Duration::from_secs(10))
            .expect("the interrupted wait did not return");
        assert_eq!(outcome, Err(InterruptedException));
    }
}
