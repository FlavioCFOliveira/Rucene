//! Merge schedulers ported from `org.apache.lucene.index`.
//!
//! Covers `MergeScheduler` with its `MergeSource`, `SerialMergeScheduler`,
//! `NoMergeScheduler`, `ConcurrentMergeScheduler` with its `MergeThread`,
//! `MergeRateLimiter`, and `MultiIndexMergeScheduler` with its
//! `CombinedMergeScheduler`.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use crate::error::{LuceneError, Result};
use crate::index::merge_policy::{MergeTrigger, OneMerge};
use crate::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock,
    RateLimitedIndexOutput,
};
use crate::util::InfoStream;

/// Where a scheduler gets its merges and how it runs them.
///
/// Equivalent to `MergeScheduler.MergeSource`, which `IndexWriter` implements
/// through its `IndexWriterMergeSource` record.
///
/// A scheduler that runs merges on threads of its own keeps the source for as
/// long as those threads run — Java's `MergeThread` holds the reference it was
/// handed — so [`MergeScheduler::merge`] takes it as an owned
/// `Arc<dyn MergeSource>`. The [`Any`] supertrait is what lets
/// `CombinedMergeScheduler.sync(Directory)` ask, as Java's `instanceof` does,
/// whether a merge thread's source is a [`TaggedMergeSource`]; the
/// [`Display`](fmt::Display) supertrait is Java's `toString()`, which
/// `ConcurrentMergeScheduler.merge` logs.
pub trait MergeSource: Any + Send + Sync + fmt::Display {
    /// Returns the next merge the policy asked for, or `None`.
    ///
    /// Equivalent to `MergeSource.getNextMerge()`.
    ///
    /// # Errors
    ///
    /// `IndexWriter`'s source returns [`LuceneError::IllegalState`] once the
    /// writer hit a tragedy, where Java throws `IllegalStateException`
    /// (`IndexWriter.java:2498-2502`).
    fn get_next_merge(&self) -> Result<Option<OneMerge>>;

    /// Reports that a merge finished.
    ///
    /// Equivalent to `MergeSource.onMergeFinished(OneMerge)`.
    fn on_merge_finished(&self, merge: &OneMerge);

    /// Returns whether merges are waiting to be scheduled.
    ///
    /// Equivalent to `MergeSource.hasPendingMerges()`.
    ///
    /// # Errors
    ///
    /// `IndexWriter`'s source returns [`LuceneError::IllegalState`] once the
    /// writer hit a tragedy, where Java throws `IllegalStateException`
    /// (`IndexWriter.java:2519-2524`).
    fn has_pending_merges(&self) -> Result<bool>;

    /// Runs `merge`, replacing its segments with the merged one.
    ///
    /// Equivalent to `MergeSource.merge(OneMerge)`.
    fn merge(&self, merge: OneMerge) -> Result<()>;
}

/// Decides when and on which thread merges run.
///
/// Equivalent to `org.apache.lucene.index.MergeScheduler`.
pub trait MergeScheduler: Send + Sync + fmt::Display {
    /// Runs whatever merges `merge_source` currently offers.
    ///
    /// Equivalent to `MergeScheduler.merge(MergeSource, MergeTrigger)`. The
    /// source is owned (`Arc`) because a scheduler such as
    /// [`ConcurrentMergeScheduler`] hands it to merge threads that outlive this
    /// call, as Java's `MergeThread` keeps its `mergeSource` reference.
    ///
    /// # Errors
    ///
    /// Propagates whatever the scheduler or the source raises.
    fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()>;

    /// Wraps the directory a merge writes to, so the scheduler can throttle
    /// the merge's writes.
    ///
    /// Equivalent to `MergeScheduler.wrapForMerge(OneMerge, Directory)`, a
    /// no-op by default.
    ///
    /// # Errors
    ///
    /// [`ConcurrentMergeScheduler`] fails when it is not called from one of its
    /// merge threads, where Java throws `AssertionError`.
    fn wrap_for_merge(
        &self,
        merge: &OneMerge,
        directory: Arc<dyn Directory>,
    ) -> Result<Arc<dyn Directory>> {
        let _ = merge;
        Ok(directory)
    }

    /// Releases the scheduler's resources.
    ///
    /// Equivalent to `MergeScheduler.close()`.
    ///
    /// # Errors
    ///
    /// Propagates whatever closing the scheduler raises.
    fn close(&self) -> Result<()> {
        Ok(())
    }

    /// Hands the scheduler the writer's info stream and directory.
    ///
    /// Equivalent to the package-private
    /// `MergeScheduler.initialize(InfoStream, Directory)`, which `IndexWriter`
    /// calls once at construction (`IndexWriter.java:965`). The base
    /// implementation only records the info stream;
    /// [`ConcurrentMergeScheduler`] overrides it to resolve its thread
    /// defaults.
    ///
    /// # Errors
    ///
    /// Propagates whatever inspecting `directory` raises.
    fn initialize(
        &self,
        info_stream: Arc<dyn InfoStream>,
        directory: &Arc<dyn Directory>,
    ) -> Result<()> {
        let _ = directory;
        self.set_info_stream(info_stream);
        Ok(())
    }

    /// Installs the info stream messages are logged to.
    ///
    /// Equivalent to assigning `MergeScheduler.infoStream`, a protected field
    /// in Java. Rust traits carry no fields, so the assignment is a method a
    /// scheduler that keeps an info stream overrides; the default drops it,
    /// which is what a scheduler that never logs would do with the field.
    fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        let _ = info_stream;
    }

    /// Returns the info stream messages are logged to, if one was installed.
    ///
    /// Equivalent to reading the protected `MergeScheduler.infoStream` field.
    fn get_info_stream(&self) -> Option<Arc<dyn InfoStream>> {
        None
    }

    /// Returns whether info-stream messages are enabled for the `"MS"`
    /// component.
    ///
    /// Equivalent to the protected `org.apache.lucene.index.MergeScheduler#verbose`,
    /// which callers pair with [`message`](Self::message) so that an expensive
    /// message is not built when nothing will read it.
    fn verbose(&self) -> bool {
        self.get_info_stream()
            .is_some_and(|stream| stream.is_enabled(MS))
    }

    /// Logs `message` under the `"MS"` component.
    ///
    /// Equivalent to the protected `org.apache.lucene.index.MergeScheduler#message`.
    /// Java dereferences the field without a null check, assuming
    /// [`verbose`](Self::verbose) was consulted first; this port simply does
    /// nothing when no stream is installed.
    fn message(&self, message: &str) {
        if let Some(stream) = self.get_info_stream() {
            stream.message(MS, message);
        }
    }
}

/// The info-stream component name Lucene logs merge scheduling under.
///
/// Equivalent to the `"MS"` literal `MergeScheduler.verbose()` and
/// `MergeScheduler.message(String)` pass to the info stream.
pub const MS: &str = "MS";

/// A scheduler that runs every merge on the calling thread, one after another.
///
/// Equivalent to `org.apache.lucene.index.SerialMergeScheduler`.
#[derive(Debug, Default)]
pub struct SerialMergeScheduler {
    /// Serialises callers, as Java's `synchronized merge` does.
    lock: Mutex<()>,
}

impl SerialMergeScheduler {
    /// Creates the scheduler.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MergeScheduler for SerialMergeScheduler {
    /// Equivalent to `SerialMergeScheduler.merge(MergeSource, MergeTrigger)`
    /// (`SerialMergeScheduler.java:31-40`).
    fn merge(&self, merge_source: Arc<dyn MergeSource>, _trigger: MergeTrigger) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| LuceneError::IllegalState("merge scheduler lock poisoned".to_string()))?;
        while let Some(merge) = merge_source.get_next_merge()? {
            merge_source.merge(merge)?;
        }
        Ok(())
    }
}

/// A scheduler that never runs a merge.
///
/// Equivalent to `org.apache.lucene.index.NoMergeScheduler`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMergeScheduler;

impl MergeScheduler for NoMergeScheduler {
    fn merge(&self, _merge_source: Arc<dyn MergeSource>, _trigger: MergeTrigger) -> Result<()> {
        Ok(())
    }
}

/// Default number of merges that may run at once when the count is not derived
/// from the machine.
pub const DEFAULT_MAX_MERGE_COUNT: i32 = 6;
/// Default number of merge threads.
pub const DEFAULT_MAX_THREAD_COUNT: i32 = 3;
/// Sentinel telling `set_max_merges_and_threads` to derive both counts from the
/// machine's core count.
///
/// Equivalent to `ConcurrentMergeScheduler.AUTO_DETECT_MERGES_AND_THREADS`.
pub const AUTO_DETECT_MERGES_AND_THREADS: i32 = -1;
/// Initial IO throttle rate once auto-throttling is enabled.
///
/// Equivalent to `ConcurrentMergeScheduler.START_MB_PER_SEC`.
const START_MB_PER_SEC: f64 = 20.0;
/// Floor of the auto IO throttle.
///
/// Equivalent to `ConcurrentMergeScheduler.MIN_MERGE_MB_PER_SEC`.
const MIN_MERGE_MB_PER_SEC: f64 = 5.0;
/// Ceiling of the auto IO throttle.
///
/// Equivalent to `ConcurrentMergeScheduler.MAX_MERGE_MB_PER_SEC`.
const MAX_MERGE_MB_PER_SEC: f64 = 10240.0;
/// Merges below this size are neither throttled nor counted as backlog.
///
/// Equivalent to `ConcurrentMergeScheduler.MIN_BIG_MERGE_MB`.
const MIN_BIG_MERGE_MB: f64 = 50.0;

/// Overrides `ConcurrentMergeScheduler.doMerge(MergeSource, OneMerge)`.
///
/// Java subclasses the scheduler and overrides the protected method; Rust has
/// no subclassing, so the override is a closure installed with
/// [`ConcurrentMergeScheduler::set_do_merge_override`] — the pattern
/// [`OneMerge`]'s hooks already use. The closure receives the scheduler so it
/// can call [`ConcurrentMergeScheduler::default_do_merge`], Java's
/// `super.doMerge(mergeSource, merge)`.
pub type DoMergeOverride = Arc<
    dyn Fn(&ConcurrentMergeScheduler, &Arc<dyn MergeSource>, OneMerge) -> Result<()> + Send + Sync,
>;

/// Overrides `ConcurrentMergeScheduler.handleMergeException(Throwable)`, as
/// [`DoMergeOverride`] overrides `doMerge`.
pub type HandleMergeExceptionOverride = Arc<dyn Fn(LuceneError) + Send + Sync>;

/// A scheduler that runs each merge on a background thread of its own.
///
/// Equivalent to `org.apache.lucene.index.ConcurrentMergeScheduler`:
/// [`merge`](MergeScheduler::merge) starts one long-lived [`MergeThread`] per
/// merge and returns at once; the threads are kept in a list the scheduler
/// re-rates whenever it changes (`updateMergeThreads`), pausing the biggest
/// merges beyond `maxThreadCount` and throttling the rest when auto IO
/// throttling is on; a producer is stalled once `maxMergeCount` merges are
/// running and more are pending (`maybeStall`); and [`sync`](Self::sync) waits
/// for the threads.
///
/// Java's monitor — every `synchronized` method, `wait` and `notifyAll` — is
/// the mutex over the scheduler's state together with a condition variable.
/// Java's monitor is reentrant and `runOnMergeFinished` calls `merge` while
/// holding it; a Rust mutex is not, so the locked bodies take the guard and the
/// public methods lock once and call them.
///
/// The value is a handle: the state is shared, through an `Arc`, with the
/// merge threads, which call back into the scheduler as Java's inner
/// `MergeThread` class calls its enclosing instance.
///
/// **Divergences from Lucene 10.5.0.**
///
/// * Java's protected `doMerge` and `handleMergeException` are overridden by
///   subclassing; here they are overridden with
///   [`set_do_merge_override`](Self::set_do_merge_override) and
///   [`set_handle_merge_exception_override`](Self::set_handle_merge_exception_override).
/// * `getIntraMergeExecutor` and its `CachedExecutor` are not ported:
///   `SegmentMerger` takes no executor in this port, so a merge always runs on
///   its own thread (see [`get_intra_merge_executor`](Self::get_intra_merge_executor)).
/// * The `lucene.cms.override_core_count` system property Java's tests use to
///   override the core count has no counterpart.
pub struct ConcurrentMergeScheduler {
    shared: Arc<CmsShared>,
}

/// The state a [`ConcurrentMergeScheduler`] handle shares with its merge
/// threads.
struct CmsShared {
    /// The fields Java guards with the scheduler's monitor.
    monitor: Mutex<CmsState>,
    /// The monitor's `wait`/`notifyAll`.
    monitor_cond: Condvar,
    /// The sink `verbose()`/`message()` report to.
    ///
    /// Equivalent to the protected `MergeScheduler.infoStream` field, which
    /// Java reads without the monitor; it is kept outside the monitor here so
    /// that a merge source may log through the scheduler while the scheduler's
    /// monitor is held, as `IndexWriterMergeSource.getNextMerge` does.
    info_stream: Mutex<Option<Arc<dyn InfoStream>>>,
    /// Set by the test hook that swallows merge failures.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.suppressExceptions`, which Java
    /// reads from the merge threads without the monitor.
    suppress_exceptions: AtomicBool,
    /// The `doMerge` override, if one was installed.
    do_merge_override: Mutex<Option<DoMergeOverride>>,
    /// The `handleMergeException` override, if one was installed.
    handle_merge_exception_override: Mutex<Option<HandleMergeExceptionOverride>>,
}

/// The fields of `ConcurrentMergeScheduler` Java guards with its monitor.
struct CmsState {
    /// Equivalent to `ConcurrentMergeScheduler.mergeThreads`.
    merge_threads: Vec<Arc<MergeThread>>,
    /// Equivalent to `ConcurrentMergeScheduler.maxThreadCount`.
    max_thread_count: i32,
    /// Equivalent to `ConcurrentMergeScheduler.maxMergeCount`.
    max_merge_count: i32,
    /// Equivalent to `ConcurrentMergeScheduler.mergeThreadCounter`.
    merge_thread_counter: u64,
    /// Equivalent to `ConcurrentMergeScheduler.targetMBPerSec`.
    target_mb_per_sec: f64,
    /// Equivalent to `ConcurrentMergeScheduler.doAutoIOThrottle`.
    do_auto_io_throttle: bool,
    /// Equivalent to `ConcurrentMergeScheduler.forceMergeMBPerSec`.
    force_merge_mb_per_sec: f64,
}

thread_local! {
    /// The [`MergeThread`] the current thread runs, if it is one.
    ///
    /// Java asks `Thread.currentThread()` and casts it to `MergeThread`
    /// (`ConcurrentMergeScheduler.java:291-302`); a Rust thread is not an
    /// object the scheduler can subclass, so each merge thread records itself
    /// here when it starts.
    static CURRENT_MERGE_THREAD: RefCell<Option<Arc<MergeThread>>> = const { RefCell::new(None) };
}

/// Where a [`MergeThread`] is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadLife {
    /// Created, not yet started.
    New,
    /// Started and still running.
    Alive,
    /// Its body has returned.
    Terminated,
}

/// Runs one merge, then exits.
///
/// Equivalent to `ConcurrentMergeScheduler.MergeThread`. Java's class is a
/// `Thread` subclass; this is the object the scheduler keeps in its list, and
/// it runs on a named OS thread (`Lucene Merge Thread #N`) that
/// [`ConcurrentMergeScheduler`] spawns for it. `isAlive()` and `join()` are
/// answered from its life cycle, which the thread updates as it starts and
/// ends.
pub struct MergeThread {
    /// Equivalent to `MergeThread.mergeSource`; `None` once the thread has
    /// terminated.
    ///
    /// **Divergence from Lucene 10.5.0, with no visible effect.** Java keeps
    /// the reference for the life of the `MergeThread` object, and a thread
    /// whose merge failed stays in `mergeThreads` until `updateMergeThreads`
    /// prunes it (`ConcurrentMergeScheduler.java:327-335`); the garbage
    /// collector reclaims the writer whatever cycle that closes. With
    /// reference counting, `IndexWriter` → config → scheduler →
    /// `mergeThreads` → this source → `IndexWriter` is a cycle that would
    /// keep the writer, and its directory's write lock, alive for ever. The
    /// thread therefore drops the source when its body ends; nothing reads it
    /// afterwards, since every reader of a merge thread's source asks for an
    /// alive thread first.
    merge_source: Mutex<Option<Arc<dyn MergeSource>>>,
    /// Equivalent to `MergeThread.merge`: a clone sharing the writer's
    /// merge's abort flag, start time and result.
    merge: OneMerge,
    /// Equivalent to `MergeThread.rateLimiter`.
    rate_limiter: Arc<MergeRateLimiter>,
    /// Equivalent to `Thread.getName()`.
    name: String,
    /// The OS thread running this merge, once spawned.
    thread_id: OnceLock<std::thread::ThreadId>,
    /// The life cycle `isAlive()` and `join()` read.
    life: Mutex<ThreadLife>,
    /// Signalled when the thread terminates.
    terminated: Condvar,
}

impl fmt::Debug for MergeThread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MergeThread")
            .field("name", &self.name)
            .field("merge", &self.merge)
            .field("life", &self.life())
            .finish_non_exhaustive()
    }
}

impl MergeThread {
    /// Creates the thread object for one merge.
    ///
    /// Equivalent to `MergeThread(MergeSource, OneMerge)`
    /// (`ConcurrentMergeScheduler.java:715-720`), whose rate limiter is built
    /// over the merge's progress; here it shares the merge's abort flag.
    fn new(merge_source: Arc<dyn MergeSource>, merge: OneMerge, name: String) -> Self {
        let rate_limiter = Arc::new(MergeRateLimiter::for_merge(&merge));
        Self {
            merge_source: Mutex::new(Some(merge_source)),
            merge,
            rate_limiter,
            name,
            thread_id: OnceLock::new(),
            life: Mutex::new(ThreadLife::New),
            terminated: Condvar::new(),
        }
    }

    fn life(&self) -> ThreadLife {
        *self.life.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_life(&self, life: ThreadLife) {
        *self.life.lock().unwrap_or_else(PoisonError::into_inner) = life;
        if life == ThreadLife::Terminated {
            self.terminated.notify_all();
        }
    }

    /// Returns the merge this thread runs.
    pub fn merge(&self) -> &OneMerge {
        &self.merge
    }

    /// Returns the limiter throttling this merge's writes.
    pub fn rate_limiter(&self) -> &Arc<MergeRateLimiter> {
        &self.rate_limiter
    }

    /// Returns the merge source this thread merges for, or `None` once the
    /// thread has terminated (see the field's divergence note).
    pub fn merge_source(&self) -> Option<Arc<dyn MergeSource>> {
        self.merge_source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Returns the thread's name, `Lucene Merge Thread #N`.
    ///
    /// Equivalent to `Thread.getName()`.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Returns whether the thread has started and not yet finished.
    ///
    /// Equivalent to `Thread.isAlive()`.
    pub fn is_alive(&self) -> bool {
        self.life() == ThreadLife::Alive
    }

    /// Returns whether this is the calling thread.
    ///
    /// Equivalent to comparing with `Thread.currentThread()`.
    fn is_current_thread(&self) -> bool {
        self.thread_id.get() == Some(&std::thread::current().id())
    }

    /// Waits until the thread has finished.
    ///
    /// Equivalent to `Thread.join()`; returns at once for a thread that never
    /// started or already finished.
    pub fn join(&self) {
        let mut life = self.life.lock().unwrap_or_else(PoisonError::into_inner);
        while *life == ThreadLife::Alive {
            life = self
                .terminated
                .wait(life)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Orders merges largest first.
    ///
    /// Equivalent to `MergeThread.compareTo(MergeThread)`, which
    /// `updateMergeThreads` sorts by so that the biggest merges keep running
    /// and the smallest are the ones paused.
    pub fn compare_to(&self, other: &Self) -> std::cmp::Ordering {
        other
            .merge
            .estimated_merge_bytes
            .cmp(&self.merge.estimated_merge_bytes)
    }
}

/// Marks a [`MergeThread`] terminated however its body ends, panics included,
/// so that `join` and `isAlive` never wait on a thread that is gone.
struct TerminateOnDrop(Arc<MergeThread>);

impl Drop for TerminateOnDrop {
    fn drop(&mut self) {
        CURRENT_MERGE_THREAD.with(|current| current.borrow_mut().take());
        // Break the writer → scheduler → thread → source → writer cycle (see
        // `MergeThread::merge_source`) before the thread is reported dead.
        let source = self
            .0
            .merge_source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(source);
        self.0.set_life(ThreadLife::Terminated);
    }
}

impl Default for ConcurrentMergeScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrentMergeScheduler {
    /// Creates a scheduler with all settings at their defaults.
    ///
    /// Equivalent to `ConcurrentMergeScheduler()`.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(CmsShared {
                monitor: Mutex::new(CmsState {
                    merge_threads: Vec::new(),
                    max_thread_count: AUTO_DETECT_MERGES_AND_THREADS,
                    max_merge_count: AUTO_DETECT_MERGES_AND_THREADS,
                    merge_thread_counter: 0,
                    target_mb_per_sec: START_MB_PER_SEC,
                    do_auto_io_throttle: false,
                    force_merge_mb_per_sec: f64::INFINITY,
                }),
                monitor_cond: Condvar::new(),
                info_stream: Mutex::new(None),
                suppress_exceptions: AtomicBool::new(false),
                do_merge_override: Mutex::new(None),
                handle_merge_exception_override: Mutex::new(None),
            }),
        }
    }

    /// Returns another handle on this same scheduler, for a merge thread.
    fn handle(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Takes the scheduler's monitor.
    ///
    /// Java has no lock poisoning; recovering the guard keeps a panic on one
    /// thread from turning every later call into a different failure.
    fn lock(&self) -> MutexGuard<'_, CmsState> {
        self.shared
            .monitor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Installs the info stream this scheduler logs to.
    ///
    /// Equivalent to assigning the protected `MergeScheduler.infoStream`
    /// field, which `CombinedMergeScheduler.setInfoStream` does directly
    /// because a combined scheduler is never handed to an `IndexWriter` and so
    /// never sees `initialize`.
    pub fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        *self
            .shared
            .info_stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(info_stream);
    }

    /// Returns the info stream this scheduler logs to, if one was installed.
    pub fn get_info_stream(&self) -> Option<Arc<dyn InfoStream>> {
        self.shared
            .info_stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Returns whether `"MS"` messages are enabled.
    ///
    /// Equivalent to `MergeScheduler.verbose()`.
    fn verbose(&self) -> bool {
        self.get_info_stream()
            .is_some_and(|stream| stream.is_enabled(MS))
    }

    /// Logs `message` under `"MS"`.
    ///
    /// Equivalent to `MergeScheduler.message(String)`.
    fn message(&self, message: &str) {
        if let Some(stream) = self.get_info_stream() {
            stream.message(MS, message);
        }
    }

    /// Installs, or with `None` removes, the override of `doMerge`.
    ///
    /// See [`DoMergeOverride`].
    pub fn set_do_merge_override(&self, hook: Option<DoMergeOverride>) {
        *self
            .shared
            .do_merge_override
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = hook;
    }

    /// Installs, or with `None` removes, the override of
    /// `handleMergeException`.
    ///
    /// See [`HandleMergeExceptionOverride`].
    pub fn set_handle_merge_exception_override(&self, hook: Option<HandleMergeExceptionOverride>) {
        *self
            .shared
            .handle_merge_exception_override
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = hook;
    }

    /// Sets how many merges may run at once and how many may be queued.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setMaxMergesAndThreads(int, int)`
    /// (`ConcurrentMergeScheduler.java:131-157`). Passing
    /// [`AUTO_DETECT_MERGES_AND_THREADS`] for both arguments resets both counts
    /// to the sentinel, which `initialize` resolves from the machine's core
    /// count.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when only one argument is the
    /// auto-detect sentinel, when either count is below 1, or when
    /// `max_thread_count` exceeds `max_merge_count`.
    pub fn set_max_merges_and_threads(
        &self,
        max_merge_count: i32,
        max_thread_count: i32,
    ) -> Result<()> {
        let mut state = self.lock();
        if max_merge_count == AUTO_DETECT_MERGES_AND_THREADS
            && max_thread_count == AUTO_DETECT_MERGES_AND_THREADS
        {
            // OK
            state.max_merge_count = AUTO_DETECT_MERGES_AND_THREADS;
            state.max_thread_count = AUTO_DETECT_MERGES_AND_THREADS;
        } else if max_merge_count == AUTO_DETECT_MERGES_AND_THREADS
            || max_thread_count == AUTO_DETECT_MERGES_AND_THREADS
        {
            return Err(LuceneError::IllegalArgument(
                "both maxMergeCount and maxThreadCount must be AUTO_DETECT_MERGES_AND_THREADS"
                    .to_string(),
            ));
        } else {
            if max_thread_count < 1 {
                return Err(LuceneError::IllegalArgument(
                    "maxThreadCount should be at least 1".to_string(),
                ));
            }
            if max_merge_count < 1 {
                return Err(LuceneError::IllegalArgument(
                    "maxMergeCount should be at least 1".to_string(),
                ));
            }
            if max_thread_count > max_merge_count {
                return Err(LuceneError::IllegalArgument(format!(
                    "maxThreadCount should be <= maxMergeCount (= {max_merge_count})"
                )));
            }
            state.max_thread_count = max_thread_count;
            state.max_merge_count = max_merge_count;
        }
        Ok(())
    }

    /// Sets `max_merge_count`/`max_thread_count` to the defaults appropriate for
    /// the storage kind.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setDefaultMaxMergesAndThreads(boolean)`
    /// (`ConcurrentMergeScheduler.java:165-193`). `spins` selects the
    /// conservative defaults for rotational storage; `false` derives both
    /// counts from [`std::thread::available_parallelism`], Java's
    /// `Runtime.availableProcessors()`.
    pub fn set_default_max_merges_and_threads(&self, spins: bool) {
        let mut state = self.lock();
        Self::set_default_max_merges_and_threads_locked(&mut state, spins);
    }

    fn set_default_max_merges_and_threads_locked(state: &mut CmsState, spins: bool) {
        if spins {
            state.max_thread_count = 1;
            state.max_merge_count = DEFAULT_MAX_MERGE_COUNT;
        } else {
            let core_count = std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(1);
            // If you are indexing at full throttle, how many merge threads do
            // you need to keep up? Merging costs about as much as
            // indexing/flushing overall for some data structures (kNN vectors),
            // so half the cores go to merges.
            state.max_thread_count = i32::try_from(core_count / 2).unwrap_or(i32::MAX).max(1);
            state.max_merge_count = state.max_thread_count.saturating_add(5);
        }
    }

    /// Sets the per-merge IO throttle rate for forced merges.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setForceMergeMBPerSec(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a negative `v` reaches
    /// the rate limiter of a running forced merge, as Java's
    /// `MergeRateLimiter.setMBPerSec` throws from `updateMergeThreads`; `v` is
    /// recorded all the same.
    pub fn set_force_merge_mb_per_sec(&self, v: f64) -> Result<()> {
        let mut state = self.lock();
        state.force_merge_mb_per_sec = v;
        self.update_merge_threads_locked(&mut state)
    }

    /// Returns the per-merge IO throttle rate for forced merges.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getForceMergeMBPerSec()`.
    pub fn get_force_merge_mb_per_sec(&self) -> f64 {
        self.lock().force_merge_mb_per_sec
    }

    /// Turns on dynamic IO throttling, which rate-limits merge writes to the
    /// lowest rate that keeps merges from falling behind.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.enableAutoIOThrottle()`.
    ///
    /// # Errors
    ///
    /// Propagates what `update_merge_threads` raises.
    pub fn enable_auto_io_throttle(&self) -> Result<()> {
        let mut state = self.lock();
        state.do_auto_io_throttle = true;
        state.target_mb_per_sec = START_MB_PER_SEC;
        self.update_merge_threads_locked(&mut state)
    }

    /// Turns off dynamic IO throttling.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.disableAutoIOThrottle()`.
    ///
    /// # Errors
    ///
    /// Propagates what `update_merge_threads` raises.
    pub fn disable_auto_io_throttle(&self) -> Result<()> {
        let mut state = self.lock();
        state.do_auto_io_throttle = false;
        self.update_merge_threads_locked(&mut state)
    }

    /// Returns whether dynamic IO throttling is on.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getAutoIOThrottle()`.
    pub fn get_auto_io_throttle(&self) -> bool {
        self.lock().do_auto_io_throttle
    }

    /// Returns the currently active per-merge IO rate limit.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getIORateLimitMBPerSec()`.
    pub fn get_io_rate_limit_mb_per_sec(&self) -> f64 {
        let state = self.lock();
        if state.do_auto_io_throttle {
            state.target_mb_per_sec
        } else {
            f64::INFINITY
        }
    }

    /// Returns `maxThreadCount`.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getMaxThreadCount()`.
    pub fn get_max_thread_count(&self) -> i32 {
        self.lock().max_thread_count
    }

    /// Returns `maxMergeCount`.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getMaxMergeCount()`.
    pub fn get_max_merge_count(&self) -> i32 {
        self.lock().max_merge_count
    }

    /// Returns the number of merge threads that are alive and not aborted,
    /// ignoring the calling thread if it is a merge thread.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.mergeThreadCount()`
    /// (`ConcurrentMergeScheduler.java:519-530`).
    pub fn merge_thread_count(&self) -> usize {
        let state = self.lock();
        Self::merge_thread_count_locked(&state)
    }

    fn merge_thread_count_locked(state: &CmsState) -> usize {
        state
            .merge_threads
            .iter()
            .filter(|t| !t.is_current_thread() && t.is_alive() && !t.merge.is_aborted())
            .count()
    }

    /// Returns the merge threads the scheduler currently tracks.
    ///
    /// Equivalent to reading the protected `ConcurrentMergeScheduler.mergeThreads`.
    pub fn merge_threads(&self) -> Vec<Arc<MergeThread>> {
        self.lock().merge_threads.clone()
    }

    /// Removes the calling thread from the active merge threads.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.removeMergeThread()`.
    fn remove_merge_thread_locked(state: &mut CmsState) {
        let index = state
            .merge_threads
            .iter()
            .position(|t| t.is_current_thread());
        debug_assert!(
            index.is_some(),
            "merge thread {:?} was not found",
            std::thread::current().name()
        );
        if let Some(index) = index {
            state.merge_threads.remove(index);
        }
    }

    /// Returns whether a merge should be parallelised internally.
    ///
    /// Equivalent to the size test of
    /// `ConcurrentMergeScheduler.getIntraMergeExecutor(OneMerge)`, which hands
    /// merges below [`MIN_BIG_MERGE_MB`] the caller's thread and bigger ones
    /// the intra-merge pool. This port's `SegmentMerger` takes no executor, so
    /// only the predicate is kept.
    pub fn get_intra_merge_executor(&self, merge: &OneMerge) -> bool {
        (merge.estimated_merge_bytes as f64) >= MIN_BIG_MERGE_MB * 1024.0 * 1024.0
    }

    /// Re-rates the running merges whenever the set of merges changes.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.updateMergeThreads()`
    /// (`ConcurrentMergeScheduler.java:312-420`): dead threads are pruned, the
    /// rest are sorted largest first, the biggest merges beyond
    /// `maxThreadCount` are stopped (rate `0`), and every other merge is rated
    /// by the throttle in force — so smaller merges are guaranteed to run
    /// before larger ones.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the rate a merge is given
    /// is negative — only a negative forced-merge rate can be — as Java's
    /// `MergeRateLimiter.setMBPerSec` throws.
    pub fn update_merge_threads(&self) -> Result<()> {
        let mut state = self.lock();
        self.update_merge_threads_locked(&mut state)
    }

    fn update_merge_threads_locked(&self, state: &mut CmsState) -> Result<()> {
        // Only look at threads that are alive & not in the process of stopping
        // (ie have an active merge); prune any dead threads.
        state.merge_threads.retain(|t| t.is_alive());
        let mut active_merges: Vec<Arc<MergeThread>> = state.merge_threads.clone();

        // Sort the merge threads, largest first (a stable sort, as Java's
        // `CollectionUtil.timSort`).
        active_merges.sort_by(|a, b| a.compare_to(b));

        let active_merge_count = active_merges.len();
        let mut big_merge_count = 0usize;
        for thread_idx in (0..active_merge_count).rev() {
            if (active_merges[thread_idx].merge.estimated_merge_bytes as f64)
                > MIN_BIG_MERGE_MB * 1024.0 * 1024.0
            {
                big_merge_count = 1 + thread_idx;
                break;
            }
        }

        let now = now_ns();
        let verbose = self.verbose();
        let mut message = if verbose {
            Some(format!(
                "updateMergeThreads ioThrottle={} targetMBPerSec={:.1} MB/sec",
                state.do_auto_io_throttle, state.target_mb_per_sec
            ))
        } else {
            None
        };

        for (thread_idx, merge_thread) in active_merges.iter().enumerate() {
            let merge = &merge_thread.merge;

            // Pause the thread if maxThreadCount is smaller than the number of
            // merge threads.
            let do_pause =
                (thread_idx as i64) < big_merge_count as i64 - i64::from(state.max_thread_count);

            let new_mb_per_sec = if do_pause {
                0.0
            } else if merge.max_num_segments() != -1 {
                state.force_merge_mb_per_sec
            } else if !state.do_auto_io_throttle {
                f64::INFINITY
            } else if (merge.estimated_merge_bytes as f64) < MIN_BIG_MERGE_MB * 1024.0 * 1024.0 {
                // Don't rate limit small merges.
                f64::INFINITY
            } else {
                state.target_mb_per_sec
            };

            let rate_limiter = &merge_thread.rate_limiter;
            let cur_mb_per_sec = rate_limiter.get_mb_per_sec();

            if let Some(message) = message.as_mut() {
                let mut merge_start_ns = merge.merge_start_ns();
                if merge_start_ns == -1 {
                    // IndexWriter didn't start the merge yet.
                    merge_start_ns = now;
                }
                message.push('\n');
                message.push_str(&format!(
                    "merge thread {} estSize={:.1} MB (written={:.1} MB) runTime={:.1}s (stopped={:.1}s, paused={:.1}s) rate={}\n",
                    merge_thread.get_name(),
                    bytes_to_mb(merge.estimated_merge_bytes),
                    bytes_to_mb(rate_limiter.get_total_bytes_written()),
                    ns_to_sec(now - merge_start_ns),
                    ns_to_sec(rate_limiter.get_total_stopped_ns()),
                    ns_to_sec(rate_limiter.get_total_paused_ns()),
                    rate_to_string(rate_limiter.get_mb_per_sec())
                ));

                if new_mb_per_sec != cur_mb_per_sec {
                    if new_mb_per_sec == 0.0 {
                        message.push_str("  now stop");
                    } else if cur_mb_per_sec == 0.0 {
                        if new_mb_per_sec == f64::INFINITY {
                            message.push_str("  now resume");
                        } else {
                            message
                                .push_str(&format!("  now resume to {new_mb_per_sec:.1} MB/sec"));
                        }
                    } else {
                        message.push_str(&format!(
                            "  now change from {cur_mb_per_sec:.1} MB/sec to {new_mb_per_sec:.1} MB/sec"
                        ));
                    }
                } else if cur_mb_per_sec == 0.0 {
                    message.push_str("  leave stopped");
                } else {
                    message.push_str(&format!("  leave running at {cur_mb_per_sec:.1} MB/sec"));
                }
            }

            rate_limiter.set_mb_per_sec(new_mb_per_sec)?;
        }
        if let Some(message) = message {
            self.message(&message);
        }
        Ok(())
    }

    /// Resolves the thread and merge counts from the machine when they were
    /// left on [`AUTO_DETECT_MERGES_AND_THREADS`].
    ///
    /// Equivalent to `ConcurrentMergeScheduler.initDynamicDefaults(Directory)`
    /// (`ConcurrentMergeScheduler.java:422-432`), whose `Directory` argument
    /// Java does not read.
    pub fn init_dynamic_defaults(&self) {
        let mut state = self.lock();
        if state.max_thread_count == AUTO_DETECT_MERGES_AND_THREADS {
            Self::set_default_max_merges_and_threads_locked(&mut state, false);
            if self.verbose() {
                self.message(&format!(
                    "initDynamicDefaults maxThreadCount={} maxMergeCount={}",
                    state.max_thread_count, state.max_merge_count
                ));
            }
        }
    }

    /// Waits for every running merge thread to finish.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.sync()`
    /// (`ConcurrentMergeScheduler.java:461-498`): joins each live merge thread
    /// in turn, skipping the calling thread, since a merge thread waiting for
    /// itself would never finish.
    pub fn sync(&self) {
        self.sync_matching(|_| true);
    }

    /// [`sync`](Self::sync) restricted to the merge threads `include` selects,
    /// which is how `CombinedMergeScheduler.sync(Directory)` waits for one
    /// index only.
    fn sync_matching(&self, include: impl Fn(&MergeThread) -> bool) {
        loop {
            let to_sync = {
                let state = self.lock();
                state
                    .merge_threads
                    .iter()
                    .find(|t| t.is_alive() && !t.is_current_thread() && include(t))
                    .cloned()
            };
            match to_sync {
                Some(thread) => thread.join(),
                None => break,
            }
        }
    }

    /// Blocks the calling thread while merging has fallen too far behind.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.maybeStall(MergeSource)`
    /// (`ConcurrentMergeScheduler.java:614-645`). Returns `false` to tell the
    /// caller not to start any new merge — what Java answers when the caller
    /// is itself a merge thread, because stalling it would block the very
    /// work the stall is waiting for.
    ///
    /// # Errors
    ///
    /// Propagates what `merge_source.has_pending_merges()` raises — Java's
    /// `IllegalStateException` once the writer hit a tragedy.
    pub fn maybe_stall(&self, merge_source: &Arc<dyn MergeSource>) -> Result<bool> {
        let state = self.lock();
        self.maybe_stall_locked(state, merge_source).1
    }

    fn maybe_stall_locked<'a>(
        &'a self,
        mut state: MutexGuard<'a, CmsState>,
        merge_source: &Arc<dyn MergeSource>,
    ) -> (MutexGuard<'a, CmsState>, Result<bool>) {
        let mut start_stall_time: i64 = 0;
        loop {
            let has_pending_merges = match merge_source.has_pending_merges() {
                Ok(has_pending_merges) => has_pending_merges,
                Err(error) => return (state, Err(error)),
            };
            if !(has_pending_merges
                && Self::merge_thread_count_locked(&state) as i64
                    >= i64::from(state.max_merge_count))
            {
                break;
            }
            // This means merging has fallen too far behind: we have already
            // created maxMergeCount threads, and now there's at least one more
            // merge pending. Only maxThreadCount of those threads actually run;
            // the rest are paused (see updateMergeThreads). We stall this
            // producer thread to prevent creation of new segments, until
            // merging has caught up.
            if state.merge_threads.iter().any(|t| t.is_current_thread()) {
                // Never stall a merge thread since this blocks the thread from
                // finishing and calling updateMergeThreads, and blocking it
                // accomplishes nothing anyway (it's not really a segment
                // producer).
                return (state, Ok(false));
            }
            if start_stall_time == 0 {
                start_stall_time = current_time_millis();
                if self.verbose() {
                    self.message("    too many merges; stalling...");
                }
            }
            state = self.do_stall_locked(state);
        }
        if self.verbose() && start_stall_time != 0 {
            self.message(&format!(
                "  stalled for {} ms",
                current_time_millis() - start_stall_time
            ));
        }
        (state, Ok(true))
    }

    /// Pauses the calling thread for a bit, releasing the monitor.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.doStall()`, which waits at most
    /// 250 ms defensively, in case a notification is missed.
    fn do_stall_locked<'a>(&'a self, state: MutexGuard<'a, CmsState>) -> MutexGuard<'a, CmsState> {
        self.shared
            .monitor_cond
            .wait_timeout(state, Duration::from_millis(250))
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    /// Runs one merge.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.doMerge(MergeSource, OneMerge)`:
    /// calls the override installed with
    /// [`set_do_merge_override`](Self::set_do_merge_override), if any, else
    /// [`default_do_merge`](Self::default_do_merge).
    ///
    /// # Errors
    ///
    /// Propagates whatever the merge raises.
    pub fn do_merge(&self, merge_source: &Arc<dyn MergeSource>, merge: OneMerge) -> Result<()> {
        let hook = self
            .shared
            .do_merge_override
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match hook {
            Some(hook) => hook(self, merge_source, merge),
            None => self.default_do_merge(merge_source, merge),
        }
    }

    /// The base `doMerge`: calls `merge_source.merge(merge)`.
    ///
    /// Equivalent to `super.doMerge(mergeSource, merge)` in an overriding
    /// subclass (`ConcurrentMergeScheduler.java:661-664`).
    ///
    /// # Errors
    ///
    /// Propagates whatever the merge raises.
    pub fn default_do_merge(
        &self,
        merge_source: &Arc<dyn MergeSource>,
        merge: OneMerge,
    ) -> Result<()> {
        merge_source.merge(merge)
    }

    /// Creates and names a new merge thread.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getMergeThread(MergeSource, OneMerge)`
    /// (`ConcurrentMergeScheduler.java:667-673`). Java also marks the thread a
    /// daemon; a Rust thread never keeps the process alive, so there is
    /// nothing to set.
    fn get_merge_thread_locked(
        state: &mut CmsState,
        merge_source: Arc<dyn MergeSource>,
        merge: OneMerge,
    ) -> Arc<MergeThread> {
        let name = format!("Lucene Merge Thread #{}", state.merge_thread_counter);
        state.merge_thread_counter += 1;
        Arc::new(MergeThread::new(merge_source, merge, name))
    }

    /// Starts `thread`: Java's `Thread.start()` on a `MergeThread`.
    fn start_merge_thread(&self, thread: &Arc<MergeThread>) -> Result<()> {
        thread.set_life(ThreadLife::Alive);
        let scheduler = self.handle();
        let body = Arc::clone(thread);
        match std::thread::Builder::new()
            .name(thread.name.clone())
            .spawn(move || scheduler.run_merge_thread(body))
        {
            Ok(handle) => {
                // The handle is dropped, detaching the thread, as Java's daemon
                // merge threads are never joined except through `sync`.
                let _ = thread.thread_id.set(handle.thread().id());
                Ok(())
            }
            Err(error) => {
                thread.set_life(ThreadLife::Terminated);
                Err(LuceneError::Io(error))
            }
        }
    }

    /// The body of a merge thread.
    ///
    /// Equivalent to `MergeThread.run()` (`ConcurrentMergeScheduler.java:728-768`):
    /// runs the merge, then [`run_on_merge_finished`](Self::run_on_merge_finished);
    /// an aborted merge is ignored, and any other failure goes to
    /// [`handle_merge_exception`](Self::handle_merge_exception) unless
    /// exceptions are suppressed.
    fn run_merge_thread(&self, thread: Arc<MergeThread>) {
        let _ = thread.thread_id.set(std::thread::current().id());
        CURRENT_MERGE_THREAD.with(|current| *current.borrow_mut() = Some(Arc::clone(&thread)));
        let _terminate = TerminateOnDrop(Arc::clone(&thread));
        let Some(merge_source) = thread.merge_source() else {
            return;
        };

        let result = (|| -> Result<()> {
            if self.verbose() {
                self.message(&format!("merge thread {} start", thread.get_name()));
            }
            self.do_merge(&merge_source, thread.merge.clone())?;
            if self.verbose() {
                let merge = &thread.merge;
                let rate_limiter = &thread.rate_limiter;
                self.message(&format!(
                    "merge thread {} merge segment [{}] done estSize={:.1} MB (written={:.1} MB) runTime={:.1}s (stopped={:.1}s, paused={:.1}s) rate={}",
                    thread.get_name(),
                    Self::get_segment_name(merge),
                    bytes_to_mb(merge.estimated_merge_bytes),
                    bytes_to_mb(rate_limiter.get_total_bytes_written()),
                    ns_to_sec(now_ns() - merge.merge_start_ns()),
                    ns_to_sec(rate_limiter.get_total_stopped_ns()),
                    ns_to_sec(rate_limiter.get_total_paused_ns()),
                    rate_to_string(rate_limiter.get_mb_per_sec())
                ));
            }
            self.run_on_merge_finished(&merge_source, &thread.merge, &thread.rate_limiter)?;
            if self.verbose() {
                self.message(&format!("merge thread {} end", thread.get_name()));
            }
            Ok(())
        })();

        drop(merge_source);
        if let Err(exc) = result {
            if matches!(exc, LuceneError::MergeAborted(_)) {
                // OK to ignore.
            } else if !self.shared.suppress_exceptions.load(Ordering::Acquire) {
                // suppressExceptions is normally only set during testing.
                self.handle_merge_exception(exc);
            }
        }
    }

    /// Called by a merge thread once its merge is done: lets the scheduler run
    /// new merges, then drops the thread from the active set, re-rates the
    /// rest and wakes any stalled producer.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.runOnMergeFinished(MergeSource, OneMerge, MergeRateLimiter)`
    /// (`ConcurrentMergeScheduler.java:681-707`). The merge call and the
    /// thread bookkeeping run under one hold of the monitor, otherwise a
    /// stalling decision could miss a pending merge.
    ///
    /// # Errors
    ///
    /// Propagates what scheduling the new merges raised, except an
    /// already-closed writer, which Java ignores.
    pub fn run_on_merge_finished(
        &self,
        merge_source: &Arc<dyn MergeSource>,
        merge: &OneMerge,
        rate_limiter: &MergeRateLimiter,
    ) -> Result<()> {
        let _ = (merge, rate_limiter);
        let state = self.lock();
        debug_assert!(
            state.merge_threads.iter().any(|t| t.is_current_thread()),
            "caller is not a merge thread"
        );
        // Let CMS run new merges if necessary.
        let (mut state, outcome) =
            self.merge_locked(state, Arc::clone(merge_source), MergeTrigger::MergeFinished);
        let outcome = match outcome {
            Err(LuceneError::AlreadyClosed(_)) => Ok(()),
            other => other,
        };
        Self::remove_merge_thread_locked(&mut state);
        let updated = self.update_merge_threads_locked(&mut state);
        // In case we had stalled indexing, we can now wake up and possibly
        // unstall.
        self.shared.monitor_cond.notify_all();
        // An exception thrown from Java's `finally` replaces the one the
        // merge raised.
        updated?;
        outcome
    }

    /// Called when an error escapes a merge thread.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.handleMergeException(Throwable)`
    /// (`ConcurrentMergeScheduler.java:772-774`), which throws
    /// `MergePolicy.MergeException` out of the merge thread's `run()`, where
    /// the JVM reports it as an uncaught exception and the thread dies. The
    /// Rust counterpart of an uncaught exception ending a thread is a panic on
    /// that thread, so this panics with the `MergeException` — after the
    /// writer has already recorded the failure (`IndexWriter.merge` turns it
    /// into a tragic event before rethrowing). An override installed with
    /// [`set_handle_merge_exception_override`](Self::set_handle_merge_exception_override)
    /// replaces this, as a subclass's override does.
    ///
    /// **Divergence from Lucene 10.5.0 (user-approved).** An uncaught Java
    /// exception ends only the merge thread. A Rust panic ends only the thread
    /// as well when the crate is built with `panic = "unwind"` (the default),
    /// but under `panic = "abort"` it ends the whole process. An application
    /// built with `panic = "abort"` that must survive a failed merge installs
    /// an override.
    ///
    /// # Panics
    ///
    /// Panics with the merge exception when no override is installed, as Java
    /// throws it. Under `panic = "abort"` this aborts the process.
    pub fn handle_merge_exception(&self, exc: LuceneError) {
        let hook = self
            .shared
            .handle_merge_exception_override
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match hook {
            Some(hook) => hook(exc),
            None => panic!("{}", LuceneError::Merge(exc.to_string())),
        }
    }

    /// Swallows merge failures instead of reporting them.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setSuppressExceptions()`, which
    /// Lucene exposes to its tests through `TestSecrets`.
    pub fn set_suppress_exceptions(&self) {
        if self.verbose() {
            self.message("will suppress merge exceptions");
        }
        self.shared
            .suppress_exceptions
            .store(true, Ordering::Release);
    }

    /// Reports merge failures again.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.clearSuppressExceptions()`.
    pub fn clear_suppress_exceptions(&self) {
        if self.verbose() {
            self.message("will not suppress merge exceptions");
        }
        self.shared
            .suppress_exceptions
            .store(false, Ordering::Release);
    }

    /// Returns whether a merge of comparable size is already running and has
    /// been running long enough to count as a backlog.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.isBacklog(long, OneMerge)`
    /// (`ConcurrentMergeScheduler.java:801-822`). Java compares the `OneMerge`
    /// objects by identity; a clone here shares its identity with the merge it
    /// was cloned from ([`OneMerge::is_same_merge`]).
    fn is_backlog(state: &CmsState, now: i64, merge: &OneMerge) -> bool {
        let merge_mb = bytes_to_mb(merge.estimated_merge_bytes);
        for merge_thread in &state.merge_threads {
            let merge_start_ns = merge_thread.merge.merge_start_ns();
            if merge_thread.is_alive()
                && !merge_thread.merge.is_same_merge(merge)
                && merge_start_ns != -1
                && (merge_thread.merge.estimated_merge_bytes as f64)
                    >= MIN_BIG_MERGE_MB * 1024.0 * 1024.0
                && ns_to_sec(now - merge_start_ns) > 3.0
            {
                let other_merge_mb = bytes_to_mb(merge_thread.merge.estimated_merge_bytes);
                let ratio = other_merge_mb / merge_mb;
                if ratio > 0.3 && ratio < 3.0 {
                    return true;
                }
            }
        }
        false
    }

    /// Tunes the IO throttle when a new merge starts.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.updateIOThrottle(OneMerge, MergeRateLimiter)`
    /// (`ConcurrentMergeScheduler.java:825-923`): a simple closed loop in
    /// which a new backlog raises the target rate by 20% up to
    /// [`MAX_MERGE_MB_PER_SEC`], no backlog lowers it by 10% down to
    /// [`MIN_MERGE_MB_PER_SEC`], and an existing backlog leaves it alone.
    fn update_io_throttle_locked(
        &self,
        state: &mut CmsState,
        new_merge: &OneMerge,
        rate_limiter: &MergeRateLimiter,
    ) -> Result<()> {
        if !state.do_auto_io_throttle {
            return Ok(());
        }

        let merge_mb = bytes_to_mb(new_merge.estimated_merge_bytes);
        if merge_mb < MIN_BIG_MERGE_MB {
            // Only watch non-trivial merges for throttling; this is safe
            // because the MP must eventually have to do larger merges.
            return Ok(());
        }

        let now = now_ns();

        // Simplistic closed-loop feedback control: if we find any other
        // similarly sized merges running, then we are falling behind, so we
        // bump up the IO throttle, else we lower it.
        let new_backlog = Self::is_backlog(state, now, new_merge);

        let mut cur_backlog = false;

        if !new_backlog {
            if state.merge_threads.len() as i64 > i64::from(state.max_thread_count) {
                // If there are already more than the maximum merge threads
                // allowed, count that as backlog.
                cur_backlog = true;
            } else {
                // Now see if any still-running merges are backlog'd.
                for merge_thread in &state.merge_threads {
                    if Self::is_backlog(state, now, &merge_thread.merge) {
                        cur_backlog = true;
                        break;
                    }
                }
            }
        }

        let cur_mb_per_sec = state.target_mb_per_sec;

        if new_backlog {
            // This new merge adds to the backlog: increase IO throttle by 20%.
            state.target_mb_per_sec *= 1.20;
            if state.target_mb_per_sec > MAX_MERGE_MB_PER_SEC {
                state.target_mb_per_sec = MAX_MERGE_MB_PER_SEC;
            }
            if self.verbose() {
                if cur_mb_per_sec == state.target_mb_per_sec {
                    self.message(&format!(
                        "io throttle: new merge backlog; leave IO rate at ceiling {:.1} MB/sec",
                        state.target_mb_per_sec
                    ));
                } else {
                    self.message(&format!(
                        "io throttle: new merge backlog; increase IO rate to {:.1} MB/sec",
                        state.target_mb_per_sec
                    ));
                }
            }
        } else if cur_backlog {
            // We still have an existing backlog; leave the rate as is.
            if self.verbose() {
                self.message(&format!(
                    "io throttle: current merge backlog; leave IO rate at {:.1} MB/sec",
                    state.target_mb_per_sec
                ));
            }
        } else {
            // We are not falling behind: decrease IO throttle by 10%.
            state.target_mb_per_sec /= 1.10;
            if state.target_mb_per_sec < MIN_MERGE_MB_PER_SEC {
                state.target_mb_per_sec = MIN_MERGE_MB_PER_SEC;
            }
            if self.verbose() {
                if cur_mb_per_sec == state.target_mb_per_sec {
                    self.message(&format!(
                        "io throttle: no merge backlog; leave IO rate at floor {:.1} MB/sec",
                        state.target_mb_per_sec
                    ));
                } else {
                    self.message(&format!(
                        "io throttle: no merge backlog; decrease IO rate to {:.1} MB/sec",
                        state.target_mb_per_sec
                    ));
                }
            }
        }

        let rate = if new_merge.max_num_segments() != -1 {
            state.force_merge_mb_per_sec
        } else {
            state.target_mb_per_sec
        };
        rate_limiter.set_mb_per_sec(rate)?;
        self.target_mb_per_sec_changed();
        Ok(())
    }

    /// Hook called after the auto throttle moved.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.targetMBPerSecChanged()`, whose
    /// body is empty and which exists so a subclass can observe the change.
    pub fn target_mb_per_sec_changed(&self) {}

    /// Returns the segment name of a merge, or `_na_` before the writer has
    /// named it.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getSegmentName(OneMerge)`.
    pub fn get_segment_name(merge: &OneMerge) -> String {
        merge
            .info()
            .as_ref()
            .map(|info| info.info.name.clone())
            .unwrap_or_else(|| "_na_".to_string())
    }

    /// `merge` under a held monitor; returns the monitor with the outcome.
    ///
    /// Equivalent to the body of the `synchronized`
    /// `ConcurrentMergeScheduler.merge(MergeSource, MergeTrigger)`
    /// (`ConcurrentMergeScheduler.java:541-603`).
    fn merge_locked<'a>(
        &'a self,
        mut state: MutexGuard<'a, CmsState>,
        merge_source: Arc<dyn MergeSource>,
        trigger: MergeTrigger,
    ) -> (MutexGuard<'a, CmsState>, Result<()>) {
        if trigger == MergeTrigger::Closing {
            // Disable throttling on close.
            state.target_mb_per_sec = MAX_MERGE_MB_PER_SEC;
            if let Err(error) = self.update_merge_threads_locked(&mut state) {
                return (state, Err(error));
            }
        }

        // First, quickly run through the newly proposed merges and add any
        // orthogonal merges (ie a merge not involving segments already pending
        // to be merged) to the queue. If we are way behind on merging, many of
        // these newly proposed merges will likely already be registered.
        if self.verbose() {
            self.message("now merge");
            self.message(&format!("  index(source): {merge_source}"));
        }

        // Iterate, pulling from the IndexWriter's queue of pending merges,
        // until it's empty.
        loop {
            let (next_state, proceed) = self.maybe_stall_locked(state, &merge_source);
            state = next_state;
            match proceed {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => return (state, Err(error)),
            }

            let next_merge = match merge_source.get_next_merge() {
                Ok(next_merge) => next_merge,
                Err(error) => return (state, Err(error)),
            };
            let Some(merge) = next_merge else {
                if self.verbose() {
                    self.message("  no more merges pending; now return");
                }
                return (state, Ok(()));
            };

            // OK to spawn a new merge thread to handle this merge.
            let new_merge_thread =
                Self::get_merge_thread_locked(&mut state, Arc::clone(&merge_source), merge.clone());
            state.merge_threads.push(Arc::clone(&new_merge_thread));

            // `ConcurrentMergeScheduler.java:585-601`: whatever fails from
            // here on hands the merge back to its source.
            if let Err(error) = self.update_io_throttle_locked(
                &mut state,
                &new_merge_thread.merge,
                &new_merge_thread.rate_limiter,
            ) {
                merge_source.on_merge_finished(&merge);
                return (state, Err(error));
            }

            if self.verbose() {
                self.message(&format!(
                    "    launch new thread [{}]",
                    new_merge_thread.get_name()
                ));
            }

            if let Err(error) = self
                .start_merge_thread(&new_merge_thread)
                .and_then(|()| self.update_merge_threads_locked(&mut state))
            {
                merge_source.on_merge_finished(&merge);
                return (state, Err(error));
            }
        }
        (state, Ok(()))
    }

    /// Renders the scheduler as Java's `toString()` does, under the simple
    /// class name `simple_name` (`getClass().getSimpleName()`).
    fn to_string_as(&self, simple_name: &str) -> String {
        let state = self.lock();
        format!(
            "{simple_name}: maxThreadCount={}, maxMergeCount={}, ioThrottle={}",
            state.max_thread_count, state.max_merge_count, state.do_auto_io_throttle
        )
    }
}

/// Renders a throttle rate the way the info stream does.
///
/// Equivalent to `ConcurrentMergeScheduler.rateToString(double)`.
pub fn rate_to_string(mb_per_sec: f64) -> String {
    if mb_per_sec == 0.0 {
        "stopped".to_string()
    } else if mb_per_sec == f64::INFINITY {
        "unlimited".to_string()
    } else {
        format!("{mb_per_sec:.1} MB/sec")
    }
}

/// Returns a monotonic nanosecond reading, the counterpart of Java's
/// `System.nanoTime()`.
///
/// `OneMerge::merge_start_ns` and every elapsed-time comparison in this module
/// are taken from this one clock, so they are always comparable.
pub fn now_ns() -> i64 {
    static ORIGIN: std::sync::LazyLock<std::time::Instant> =
        std::sync::LazyLock::new(std::time::Instant::now);
    ORIGIN.elapsed().as_nanos().min(i64::MAX as u128) as i64
}

/// Returns the wall-clock time in milliseconds, Java's
/// `System.currentTimeMillis()`.
pub(crate) fn current_time_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Converts nanoseconds to seconds.
///
/// Equivalent to `ConcurrentMergeScheduler.nsToSec(long)`.
pub fn ns_to_sec(ns: i64) -> f64 {
    ns as f64 / 1_000_000_000.0
}

/// Converts bytes to megabytes.
///
/// Equivalent to `ConcurrentMergeScheduler.bytesToMB(long)`.
pub fn bytes_to_mb(bytes: i64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

impl fmt::Display for ConcurrentMergeScheduler {
    /// Equivalent to `ConcurrentMergeScheduler.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_as("ConcurrentMergeScheduler"))
    }
}

impl fmt::Debug for ConcurrentMergeScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl MergeScheduler for ConcurrentMergeScheduler {
    /// Equivalent to `ConcurrentMergeScheduler.merge(MergeSource, MergeTrigger)`
    /// (`ConcurrentMergeScheduler.java:541-603`): starts one merge thread per
    /// pending merge, stalling the caller while too many are running, and
    /// returns without waiting for them.
    fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()> {
        let state = self.lock();
        self.merge_locked(state, merge_source, trigger).1
    }

    /// Equivalent to `ConcurrentMergeScheduler.wrapForMerge(OneMerge, Directory)`
    /// (`ConcurrentMergeScheduler.java:288-311`): every output the merge
    /// creates is rate-limited by the calling merge thread's limiter. The
    /// limiter is per thread, so with several merges running the combined IO
    /// rate can exceed the target.
    fn wrap_for_merge(
        &self,
        merge: &OneMerge,
        directory: Arc<dyn Directory>,
    ) -> Result<Arc<dyn Directory>> {
        let _ = merge;
        let rate_limiter = CURRENT_MERGE_THREAD.with(|current| {
            current
                .borrow()
                .as_ref()
                .map(|thread| Arc::clone(&thread.rate_limiter))
        });
        match rate_limiter {
            Some(rate_limiter) => Ok(Arc::new(RateLimitedMergeDirectory {
                inner: directory,
                rate_limiter,
            })),
            None => Err(LuceneError::IllegalState(format!(
                "wrapForMerge should be called from MergeThread. Current thread: {}",
                std::thread::current().name().unwrap_or("<unnamed>")
            ))),
        }
    }

    /// Equivalent to `ConcurrentMergeScheduler.close()`: waits for the merge
    /// threads with [`sync`](ConcurrentMergeScheduler::sync).
    fn close(&self) -> Result<()> {
        self.sync();
        Ok(())
    }

    /// Equivalent to `ConcurrentMergeScheduler.initialize(InfoStream, Directory)`
    /// (`ConcurrentMergeScheduler.java:532-538`): records the info stream and
    /// resolves the auto-detected thread and merge counts.
    fn initialize(
        &self,
        info_stream: Arc<dyn InfoStream>,
        directory: &Arc<dyn Directory>,
    ) -> Result<()> {
        let _ = directory;
        self.set_info_stream(info_stream);
        self.init_dynamic_defaults();
        Ok(())
    }

    fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        ConcurrentMergeScheduler::set_info_stream(self, info_stream);
    }

    fn get_info_stream(&self) -> Option<Arc<dyn InfoStream>> {
        ConcurrentMergeScheduler::get_info_stream(self)
    }
}

/// The directory a [`ConcurrentMergeScheduler`] merge writes through: every
/// output it creates is throttled by the merge thread's rate limiter.
///
/// Equivalent to the anonymous `FilterDirectory` subclass
/// `ConcurrentMergeScheduler.wrapForMerge` returns
/// (`ConcurrentMergeScheduler.java:299-309`), which overrides only
/// `createOutput`.
struct RateLimitedMergeDirectory {
    inner: Arc<dyn Directory>,
    rate_limiter: Arc<MergeRateLimiter>,
}

impl Directory for RateLimitedMergeDirectory {
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
        self.ensure_open()?;
        // This Directory is only supposed to be used during merging, so all
        // writes should have MERGE context, else there is a bug somewhere that
        // is failing to pass down the right IOContext.
        debug_assert_eq!(
            context.context(),
            crate::store::Context::Merge,
            "got context={:?}",
            context.context()
        );
        let rate_limiter: Arc<dyn crate::store::RateLimiter> = self.rate_limiter.clone();
        Ok(Box::new(RateLimitedIndexOutput::new(
            rate_limiter,
            self.inner.create_output(name, context)?,
        )))
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
        self.inner.open_input(name, context)
    }

    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.inner.open_checksum_input(name)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    /// A no-op: the wrapper does not own the directory it shares, exactly as
    /// the writer's other shared-directory adapters.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn fs_directory_path(&self) -> Option<&std::path::Path> {
        self.inner.fs_directory_path()
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

/// How often the limiter is consulted, in milliseconds, when a rate is set.
///
/// Equivalent to `MergeRateLimiter.MIN_PAUSE_CHECK_MSEC`.
const MIN_PAUSE_CHECK_MSEC: i64 = 25;
/// Shortest pause the rate limiter bothers with.
///
/// Equivalent to `MergeRateLimiter.MIN_PAUSE_NS`.
const MIN_PAUSE: Duration = Duration::from_millis(2);
/// Longest single pause, so an aborted merge is noticed promptly.
///
/// Equivalent to `MergeRateLimiter.MAX_PAUSE_NS`.
const MAX_PAUSE: Duration = Duration::from_millis(250);

/// Why a merge thread was paused.
///
/// Equivalent to `org.apache.lucene.index.MergePolicy.OneMergeProgress.PauseReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// Stopped, because the throughput rate was set to zero.
    Stopped,
    /// Temporarily paused, because the throughput rate was exceeded.
    Paused,
    /// Any other reason.
    Other,
}

/// Throttles the bytes one merge writes, and lets the writer abort it.
///
/// Equivalent to `org.apache.lucene.index.MergeRateLimiter`.
#[derive(Debug)]
pub struct MergeRateLimiter {
    /// Target throughput in MB/s; `f64::INFINITY` means unthrottled.
    mb_per_sec: Mutex<f64>,
    /// How many bytes may pass before the limiter checks again.
    min_pause_check_bytes: AtomicI64,
    total_bytes_written: AtomicI64,
    total_paused_ns: AtomicI64,
    total_stopped_ns: AtomicI64,
    /// The instant, on [`now_ns`]'s clock, that the merge may next write at.
    ///
    /// Equivalent to `MergeRateLimiter.lastNS`, an `AtomicLong` seeded with
    /// `0` and compared against `System.nanoTime()`.
    last_ns: Mutex<i64>,
    /// Whether the merge was aborted.
    ///
    /// Equivalent to `OneMergeProgress.aborted`, which Java's limiter reads
    /// through `mergeProgress.isAborted()`. A limiter built with
    /// [`for_merge`](Self::for_merge) shares the merge's own flag.
    aborted: Arc<AtomicBool>,
    /// The pause lock and its condition variable.
    ///
    /// Equivalent to `OneMergeProgress.pauseLock` and `pausing`, which Java's
    /// limiter reaches through its `mergeProgress`. `OneMerge` carries no
    /// progress object in this port (see the note on [`OneMerge`]), so the
    /// per-reason pause totals live on the limiter; the lock and condition are
    /// shared with the merge, so that `OneMerge::set_aborted` wakes a paused
    /// merge as `OneMergeProgress.abort()` does. A waiter sleeps on the
    /// condition until its time expires, the merge is aborted, or the rate
    /// changes.
    pausing: Arc<(Mutex<()>, Condvar)>,
}

impl Default for MergeRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl MergeRateLimiter {
    /// Creates an unthrottled limiter with an abort flag of its own.
    pub fn new() -> Self {
        Self::with_progress(
            Arc::new(AtomicBool::new(false)),
            Arc::new((Mutex::new(()), Condvar::new())),
        )
    }

    /// Creates an unthrottled limiter for `merge`.
    ///
    /// Equivalent to `MergeRateLimiter(OneMergeProgress)`, which
    /// `MergeThread` calls with `merge.getMergeProgress()`
    /// (`ConcurrentMergeScheduler.java:719`): the limiter observes the merge's
    /// abort flag.
    pub fn for_merge(merge: &OneMerge) -> Self {
        Self::with_progress(merge.aborted_flag(), merge.pause_signal())
    }

    fn with_progress(aborted: Arc<AtomicBool>, pausing: Arc<(Mutex<()>, Condvar)>) -> Self {
        let limiter = Self {
            mb_per_sec: Mutex::new(f64::INFINITY),
            min_pause_check_bytes: AtomicI64::new(0),
            total_bytes_written: AtomicI64::new(0),
            total_paused_ns: AtomicI64::new(0),
            total_stopped_ns: AtomicI64::new(0),
            last_ns: Mutex::new(0),
            aborted,
            pausing,
        };
        // Initially no IO limit; use the setter so `min_pause_check_bytes` is
        // set (`MergeRateLimiter.java:50-52`).
        limiter.store_mb_per_sec(f64::INFINITY);
        limiter
    }

    /// Sets the target throughput.
    ///
    /// Equivalent to `MergeRateLimiter.setMBPerSec(double)`. A rate of `0.0` is
    /// allowed and means the merge is stopped; `f64::INFINITY` means it is not
    /// throttled at all. Java synchronises so that the rate and the check
    /// interval change together, and wakes any thread already pausing so that
    /// it re-evaluates against the new rate; so does this.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a negative rate, as Java's
    /// `IllegalArgumentException("mbPerSec must be positive; got: …")`
    /// (`MergeRateLimiter.java:58-62`); the rate is left unchanged.
    pub fn set_mb_per_sec(&self, mb_per_sec: f64) -> Result<()> {
        // 0.0 is allowed: it means the merge is paused.
        if mb_per_sec < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "mbPerSec must be positive; got: {}",
                crate::util::java_fmt::double_to_string(mb_per_sec)
            )));
        }
        self.store_mb_per_sec(mb_per_sec);
        Ok(())
    }

    /// Stores a rate already known to be non-negative, with its check
    /// interval, and wakes any pausing thread: the body of
    /// `MergeRateLimiter.setMBPerSec(double)` past its argument check.
    fn store_mb_per_sec(&self, mb_per_sec: f64) {
        debug_assert!(
            mb_per_sec >= 0.0 || mb_per_sec.is_nan(),
            "negative rate {mb_per_sec}"
        );
        {
            // Java holds `synchronized (this)` across both stores so that a
            // reader never sees one without the other.
            let mut guard = match self.mb_per_sec.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *guard = mb_per_sec;
            // `Math.min(1024 * 1024, (long) ((MIN_PAUSE_CHECK_MSEC / 1000.0) *
            // mbPerSec * 1024 * 1024))`; an infinite rate casts to
            // `Long.MAX_VALUE` in Java, so the minimum picks 1 MB.
            let scaled = (MIN_PAUSE_CHECK_MSEC as f64 / 1000.0) * mb_per_sec * 1024.0 * 1024.0;
            let scaled = if scaled >= i64::MAX as f64 {
                i64::MAX
            } else {
                scaled as i64
            };
            self.min_pause_check_bytes
                .store(scaled.min(1024 * 1024), Ordering::Relaxed);
        }
        // `mergeProgress.wakeup()`.
        self.wakeup();
    }

    /// Wakes any thread pausing in [`maybe_pause`](Self::maybe_pause).
    ///
    /// Equivalent to `MergePolicy.OneMergeProgress.wakeup()`.
    fn wakeup(&self) {
        let (lock, pausing) = &*self.pausing;
        let _guard = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        pausing.notify_all();
    }

    /// Returns the target throughput.
    pub fn get_mb_per_sec(&self) -> f64 {
        self.mb_per_sec
            .lock()
            .map(|guard| *guard)
            .unwrap_or(f64::INFINITY)
    }

    /// Returns how many bytes have passed through the limiter.
    pub fn get_total_bytes_written(&self) -> i64 {
        self.total_bytes_written.load(Ordering::Relaxed)
    }

    /// Returns how long the merge spent paused.
    pub fn get_total_paused_ns(&self) -> i64 {
        self.total_paused_ns.load(Ordering::Relaxed)
    }

    /// Returns how long the merge spent fully stopped.
    pub fn get_total_stopped_ns(&self) -> i64 {
        self.total_stopped_ns.load(Ordering::Relaxed)
    }

    /// Returns how many bytes may pass before the next check.
    pub fn get_min_pause_check_bytes(&self) -> i64 {
        self.min_pause_check_bytes.load(Ordering::Relaxed)
    }

    /// Marks the merge aborted, so the next `pause` fails.
    ///
    /// Equivalent to `MergePolicy.OneMergeProgress.abort()`, which sets the flag
    /// and then wakes any paused merge thread.
    pub fn set_aborted(&self) {
        self.aborted.store(true, Ordering::Release);
        self.wakeup();
    }

    /// Returns whether the merge was aborted.
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
    }

    /// Accounts `bytes` and sleeps if the merge is running ahead of its rate.
    ///
    /// Equivalent to `MergeRateLimiter.pause(long)`. Returns the total time
    /// paused, in nanoseconds. The loop re-checks because the rate may be
    /// changed while the caller is pausing.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::MergeAborted`] once the merge has been aborted,
    /// which is Java's `MergePolicy.MergeAbortedException`.
    pub fn pause(&self, bytes: i64) -> Result<i64> {
        self.total_bytes_written.fetch_add(bytes, Ordering::Relaxed);

        // While loop because we may wake up and check again when our rate limit
        // is changed while we were pausing:
        let mut paused = 0i64;
        loop {
            let delta = self.maybe_pause(bytes)?;
            if delta < 0 {
                break;
            }
            paused += delta;
        }
        Ok(paused)
    }

    /// Pauses the calling thread once, if the merge is running ahead of its
    /// rate, and returns how long it paused — or `-1` when no pause applied.
    ///
    /// Equivalent to the private `org.apache.lucene.index.MergeRateLimiter#maybePause`.
    /// Only the calling thread is paused: several intra-merge threads each
    /// pause independently, exactly as in Java.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::MergeAborted`] when the merge has been aborted,
    /// which is where Java throws `MergePolicy.MergeAbortedException`.
    fn maybe_pause(&self, bytes: i64) -> Result<i64> {
        // Now is a good time to abort the merge:
        if self.is_aborted() {
            return Err(LuceneError::MergeAborted("Merge aborted.".to_string()));
        }

        // Read from the volatile rate once.
        let rate = self.get_mb_per_sec();
        let seconds_to_pause = (bytes as f64 / 1024. / 1024.) / rate;

        // Java folds this into `lastNS.updateAndGet`; the mutex plays the part
        // of the compare-and-set, and the branch that keeps `last` unchanged is
        // the one that reports a pause.
        let cur_pause_ns = {
            let mut last = match self.last_ns.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let cur_ns = now_ns();
            // Time we should sleep until; this is purely the instantaneous
            // rate, just adding seconds onto the last time we paused to.
            //
            // `last + (long) (1000000000 * secondsToPause)` and
            // `targetNS - curNS` are Java `long` arithmetic, which wraps; a
            // Rust `f64 as i64` cast is Java's `(long)` cast (NaN to 0,
            // infinities and out-of-range values clamped). With the rate at
            // `0` and `bytes > 0` the cast yields `i64::MAX`, the addition
            // wraps and the subtraction wraps back, so the pause is
            // `i64::MAX - (cur_ns - last)`: a full `MAX_PAUSE`, as in Java.
            // With `bytes == 0` the quotient is NaN, the cast yields `0`, and
            // no pause applies.
            let target_ns = last.wrapping_add((1_000_000_000.0 * seconds_to_pause) as i64);
            let cur_pause_ns = target_ns.wrapping_sub(cur_ns);
            if cur_pause_ns <= MIN_PAUSE.as_nanos() as i64 {
                // Set to cur_ns, not target_ns, to enforce the instant rate,
                // not the rate averaged over all history:
                *last = cur_ns;
                0
            } else {
                cur_pause_ns
            }
        };

        if cur_pause_ns == 0 {
            return Ok(-1);
        }
        // Defensive: don't sleep for too long; the loop above will call us
        // again if we should keep sleeping and the rate may be adjusted in
        // between.
        let cur_pause_ns = cur_pause_ns.min(MAX_PAUSE.as_nanos() as i64);

        let start = now_ns();
        let reason = if rate == 0.0 {
            PauseReason::Stopped
        } else {
            PauseReason::Paused
        };
        self.pause_nanos(cur_pause_ns, reason, rate);
        Ok(now_ns() - start)
    }

    /// Pauses the calling thread for at least `pause_nanos` nanoseconds unless
    /// the merge is aborted or the rate changes.
    ///
    /// Equivalent to `MergePolicy.OneMergeProgress.pauseNanos(long, PauseReason,
    /// BooleanSupplier)`. The condition Java passes from `maybePause` is
    /// `rate == mbPerSec`, so a rate change ends the pause immediately; that is
    /// `expected_rate` here.
    fn pause_nanos(&self, pause_nanos: i64, reason: PauseReason, expected_rate: f64) {
        let start = now_ns();
        let mut remaining = Duration::from_nanos(pause_nanos.max(0) as u64);
        let (lock, pausing) = &*self.pausing;
        let mut guard = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        while !remaining.is_zero() && !self.is_aborted() && self.get_mb_per_sec() == expected_rate {
            let waited = match pausing.wait_timeout(guard, remaining) {
                Ok(result) => result,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard = waited.0;
            // `Condition.awaitNanos` returns what is left of the deadline; a
            // `WaitTimeoutResult` that timed out means nothing is left.
            remaining = if waited.1.timed_out() {
                Duration::ZERO
            } else {
                let elapsed = Duration::from_nanos((now_ns() - start).max(0) as u64);
                Duration::from_nanos(pause_nanos.max(0) as u64).saturating_sub(elapsed)
            };
        }
        drop(guard);
        let elapsed = now_ns() - start;
        match reason {
            PauseReason::Stopped => {
                self.total_stopped_ns.fetch_add(elapsed, Ordering::Relaxed);
            }
            PauseReason::Paused => {
                self.total_paused_ns.fetch_add(elapsed, Ordering::Relaxed);
            }
            PauseReason::Other => {}
        }
    }
}

impl crate::store::RateLimiter for MergeRateLimiter {
    /// Equivalent to `MergeRateLimiter.setMBPerSec(double)`.
    fn set_mb_per_sec(&self, mb_per_sec: f64) -> Result<()> {
        MergeRateLimiter::set_mb_per_sec(self, mb_per_sec)
    }

    /// Equivalent to `MergeRateLimiter.getMBPerSec()`.
    fn get_mb_per_sec(&self) -> f64 {
        MergeRateLimiter::get_mb_per_sec(self)
    }

    /// Equivalent to `MergeRateLimiter.pause(long)`.
    fn pause(&self, bytes: i64) -> Result<i64> {
        MergeRateLimiter::pause(self, bytes)
    }

    /// Equivalent to `MergeRateLimiter.getMinPauseCheckBytes()`.
    fn get_min_pause_check_bytes(&self) -> i64 {
        MergeRateLimiter::get_min_pause_check_bytes(self)
    }
}

/// Multi-index, or multi-tenant, merge scheduling.
///
/// Equivalent to `org.apache.lucene.index.MultiIndexMergeScheduler`: one
/// instance per index, each tagging its index's [`MergeSource`] with the index
/// [`Directory`] ([`TaggedMergeSource`]) and shunting it into a single
/// [`CombinedMergeScheduler`], so the merge threads of every index share one
/// `maxThreadCount`/`maxMergeCount` budget. Closing one index waits only for
/// that index's merge threads.
pub struct MultiIndexMergeScheduler {
    /// Equivalent to `MultiIndexMergeScheduler.directory`.
    directory: Arc<dyn Directory>,
    /// Equivalent to `MultiIndexMergeScheduler.combinedMergeScheduler`.
    combined_merge_scheduler: Arc<CombinedMergeScheduler>,
    /// Equivalent to `MultiIndexMergeScheduler.manageSingleton`.
    manage_singleton: bool,
    /// The protected `MergeScheduler.infoStream` field of this scheduler
    /// itself, which the base `initialize` fills in; the combined scheduler
    /// keeps its own, set through [`set_info_stream`](Self::set_info_stream).
    info_stream: Mutex<Option<Arc<dyn InfoStream>>>,
}

impl MultiIndexMergeScheduler {
    /// Creates the scheduler for the index in `directory`, sharing the
    /// process-wide [`CombinedMergeScheduler`] singleton.
    ///
    /// Equivalent to `MultiIndexMergeScheduler(Directory)`
    /// (`MultiIndexMergeScheduler.java:42-47`); [`close`](MergeScheduler::close)
    /// releases the singleton reference this takes.
    pub fn new(directory: Arc<dyn Directory>) -> Self {
        Self {
            directory,
            combined_merge_scheduler: CombinedMergeScheduler::acquire_singleton(),
            manage_singleton: true,
            info_stream: Mutex::new(None),
        }
    }

    /// Creates the scheduler for the index in `directory` over the given
    /// combined scheduler, which closing this one does not close.
    ///
    /// Equivalent to the package-private
    /// `MultiIndexMergeScheduler(Directory, CombinedMergeScheduler)`
    /// (`MultiIndexMergeScheduler.java:50-55`), meant for testing or for
    /// partitioning tenants into groups.
    pub fn with_combined_merge_scheduler(
        directory: Arc<dyn Directory>,
        combined_merge_scheduler: Arc<CombinedMergeScheduler>,
    ) -> Self {
        Self {
            directory,
            combined_merge_scheduler,
            manage_singleton: false,
            info_stream: Mutex::new(None),
        }
    }

    /// Returns the directory of this scheduler's index.
    ///
    /// Equivalent to `MultiIndexMergeScheduler.getDirectory()`.
    pub fn get_directory(&self) -> &Arc<dyn Directory> {
        &self.directory
    }

    /// Returns the scheduler shared with the other indexes.
    ///
    /// Equivalent to `MultiIndexMergeScheduler.getCombinedMergeScheduler()`.
    pub fn get_combined_merge_scheduler(&self) -> &Arc<CombinedMergeScheduler> {
        &self.combined_merge_scheduler
    }

    /// Installs the info stream on the combined scheduler.
    ///
    /// Equivalent to `MultiIndexMergeScheduler.setInfoStream(InfoStream)`
    /// (`MultiIndexMergeScheduler.java:83-85`), which forwards because the
    /// combined scheduler is the one running the merges.
    pub fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        self.combined_merge_scheduler.set_info_stream(info_stream);
    }
}

impl MergeScheduler for MultiIndexMergeScheduler {
    /// Equivalent to `MultiIndexMergeScheduler.merge(MergeSource, MergeTrigger)`
    /// (`MultiIndexMergeScheduler.java:65-70`): tags the source with this
    /// index's directory and hands it to the combined scheduler.
    fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()> {
        let tagged_merge_source: Arc<dyn MergeSource> = Arc::new(TaggedMergeSource::new(
            merge_source,
            Arc::clone(&self.directory),
        ));
        MergeScheduler::merge(
            self.combined_merge_scheduler.as_ref(),
            tagged_merge_source,
            trigger,
        )
    }

    /// Equivalent to `MultiIndexMergeScheduler.wrapForMerge(OneMerge, Directory)`.
    fn wrap_for_merge(
        &self,
        merge: &OneMerge,
        directory: Arc<dyn Directory>,
    ) -> Result<Arc<dyn Directory>> {
        self.combined_merge_scheduler
            .wrap_for_merge(merge, directory)
    }

    /// Equivalent to `MultiIndexMergeScheduler.close()`
    /// (`MultiIndexMergeScheduler.java:73-79`): waits for this index's merge
    /// threads, then releases the singleton reference when this scheduler
    /// took one.
    fn close(&self) -> Result<()> {
        self.combined_merge_scheduler
            .sync_directory(&self.directory);
        if self.manage_singleton {
            CombinedMergeScheduler::release_singleton()?;
        }
        Ok(())
    }

    fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        *self
            .info_stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(info_stream);
    }

    fn get_info_stream(&self) -> Option<Arc<dyn InfoStream>> {
        self.info_stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The [`ConcurrentMergeScheduler`] every [`MultiIndexMergeScheduler`] shunts
/// its tagged merge sources into.
///
/// Equivalent to `MultiIndexMergeScheduler.CombinedMergeScheduler`, a
/// `ConcurrentMergeScheduler` subclass; Rust has no subclassing, so it wraps
/// one and dereferences to it for the inherited API. It should not be handed
/// to an `IndexWriter` directly.
pub struct CombinedMergeScheduler {
    cms: ConcurrentMergeScheduler,
}

/// The process-wide combined scheduler and its reference count.
///
/// Equivalent to `CombinedMergeScheduler.singleton` and `singletonRefCount`,
/// guarded by the class monitor in Java.
static COMBINED_SINGLETON: Mutex<(Option<Arc<CombinedMergeScheduler>>, usize)> =
    Mutex::new((None, 0));

impl Default for CombinedMergeScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl CombinedMergeScheduler {
    /// Creates a combined scheduler with default settings.
    pub fn new() -> Self {
        Self {
            cms: ConcurrentMergeScheduler::new(),
        }
    }

    /// Returns the singleton, creating it on first use, and counts the
    /// reference.
    ///
    /// Equivalent to `CombinedMergeScheduler.acquireSingleton()`.
    fn acquire_singleton() -> Arc<Self> {
        let mut singleton = COMBINED_SINGLETON
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let scheduler = Arc::clone(singleton.0.get_or_insert_with(|| Arc::new(Self::new())));
        singleton.1 += 1;
        scheduler
    }

    /// Drops one reference to the singleton, closing it with the last.
    ///
    /// Equivalent to `CombinedMergeScheduler.releaseSingleton()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when called more often than
    /// [`acquire_singleton`](Self::acquire_singleton).
    fn release_singleton() -> Result<()> {
        let closing = {
            let mut singleton = COMBINED_SINGLETON
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if singleton.1 < 1 {
                return Err(LuceneError::IllegalState(
                    "decrementSingletonReference() called too many times".to_string(),
                ));
            }
            singleton.1 -= 1;
            if singleton.1 == 0 {
                singleton.0.take()
            } else {
                None
            }
        };
        // Java closes the singleton while holding the class monitor; closing
        // waits for merge threads, which never take that monitor, so closing
        // after releasing it is equivalent.
        if let Some(scheduler) = closing {
            MergeScheduler::close(scheduler.as_ref())?;
        }
        Ok(())
    }

    /// Returns the singleton, if one exists.
    ///
    /// Equivalent to `CombinedMergeScheduler.peekSingleton()`.
    pub fn peek_singleton() -> Option<Arc<Self>> {
        COMBINED_SINGLETON
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .clone()
    }

    /// Waits for the merge threads of the index in `directory` to finish.
    ///
    /// Equivalent to `CombinedMergeScheduler.sync(Directory)`
    /// (`MultiIndexMergeScheduler.java:160-193`): only threads whose source is
    /// a [`TaggedMergeSource`] for that directory are joined. Java compares
    /// the directories with `equals`, which `Directory` inherits from `Object`
    /// as identity; so does this.
    pub fn sync_directory(&self, directory: &Arc<dyn Directory>) {
        self.cms.sync_matching(|thread| {
            thread.merge_source().is_some_and(|source| {
                let source: &dyn Any = source.as_ref();
                source
                    .downcast_ref::<TaggedMergeSource>()
                    .is_some_and(|tagged| {
                        std::ptr::addr_eq(Arc::as_ptr(&tagged.directory), Arc::as_ptr(directory))
                    })
            })
        });
    }
}

impl std::ops::Deref for CombinedMergeScheduler {
    type Target = ConcurrentMergeScheduler;

    fn deref(&self) -> &ConcurrentMergeScheduler {
        &self.cms
    }
}

impl fmt::Display for CombinedMergeScheduler {
    /// The inherited `ConcurrentMergeScheduler.toString()`, which prints
    /// `getClass().getSimpleName()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.cms.to_string_as("CombinedMergeScheduler"))
    }
}

impl MergeScheduler for CombinedMergeScheduler {
    fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()> {
        MergeScheduler::merge(&self.cms, merge_source, trigger)
    }

    fn wrap_for_merge(
        &self,
        merge: &OneMerge,
        directory: Arc<dyn Directory>,
    ) -> Result<Arc<dyn Directory>> {
        self.cms.wrap_for_merge(merge, directory)
    }

    fn close(&self) -> Result<()> {
        MergeScheduler::close(&self.cms)
    }

    fn initialize(
        &self,
        info_stream: Arc<dyn InfoStream>,
        directory: &Arc<dyn Directory>,
    ) -> Result<()> {
        MergeScheduler::initialize(&self.cms, info_stream, directory)
    }

    fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        self.cms.set_info_stream(info_stream);
    }

    fn get_info_stream(&self) -> Option<Arc<dyn InfoStream>> {
        self.cms.get_info_stream()
    }
}

/// A [`MergeSource`] tagged with the directory of the index it belongs to.
///
/// Equivalent to `MultiIndexMergeScheduler.CombinedMergeScheduler.TaggedMergeSource`,
/// a filter that forwards every call to the wrapped source.
pub struct TaggedMergeSource {
    /// Equivalent to `TaggedMergeSource.in`.
    inner: Arc<dyn MergeSource>,
    /// Equivalent to `TaggedMergeSource.directory`.
    directory: Arc<dyn Directory>,
}

impl TaggedMergeSource {
    /// Tags `inner` with `directory`.
    pub fn new(inner: Arc<dyn MergeSource>, directory: Arc<dyn Directory>) -> Self {
        Self { inner, directory }
    }

    /// Returns the tag.
    ///
    /// Equivalent to `TaggedMergeSource.getDirectory()`.
    pub fn get_directory(&self) -> &Arc<dyn Directory> {
        &self.directory
    }
}

impl fmt::Display for TaggedMergeSource {
    /// `TaggedMergeSource` declares no `toString()`, so Java falls back to
    /// `Object.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.MultiIndexMergeScheduler$CombinedMergeScheduler$TaggedMergeSource",
            self,
        ))
    }
}

impl MergeSource for TaggedMergeSource {
    fn get_next_merge(&self) -> Result<Option<OneMerge>> {
        self.inner.get_next_merge()
    }

    fn on_merge_finished(&self, merge: &OneMerge) {
        self.inner.on_merge_finished(merge);
    }

    fn has_pending_merges(&self) -> Result<bool> {
        self.inner.has_pending_merges()
    }

    fn merge(&self, merge: OneMerge) -> Result<()> {
        self.inner.merge(merge)
    }
}

impl fmt::Display for SerialMergeScheduler {
    /// `SerialMergeScheduler` declares no `toString()`, so Java falls back to
    /// `Object.toString()` — `getClass().getName() + "@" + hex identity hash`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SerialMergeScheduler",
            self,
        ))
    }
}

impl fmt::Display for NoMergeScheduler {
    /// `NoMergeScheduler` declares no `toString()`, so Java falls back to
    /// `Object.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.NoMergeScheduler",
            self,
        ))
    }
}

impl fmt::Display for MultiIndexMergeScheduler {
    /// `MultiIndexMergeScheduler` declares no `toString()`, so Java falls back
    /// to `Object.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.MultiIndexMergeScheduler",
            self,
        ))
    }
}

// -----------------------------------------------------------------------------
// TestSecrets registration
// -----------------------------------------------------------------------------

/// The [`ConcurrentMergeSchedulerAccess`](crate::internal::tests_hooks::ConcurrentMergeSchedulerAccess)
/// `ConcurrentMergeScheduler` registers with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets): the method
/// reference `ConcurrentMergeScheduler::setSuppressExceptions`.
#[derive(Debug)]
struct ConcurrentMergeSchedulerAccessImpl;

impl crate::internal::tests_hooks::ConcurrentMergeSchedulerAccess
    for ConcurrentMergeSchedulerAccessImpl
{
    fn set_suppress_exceptions(&self, cms: &mut ConcurrentMergeScheduler) {
        cms.set_suppress_exceptions();
    }
}

/// Registers `ConcurrentMergeScheduler`'s accessor with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets), once.
///
/// Equivalent to `ConcurrentMergeScheduler`'s `static {}` block
/// (`ConcurrentMergeScheduler.java:944-946`), which
/// `TestSecrets.ensureInitialized(ConcurrentMergeScheduler.class)` triggers.
pub(crate) fn register_test_secrets() {
    static INITIALIZED: std::sync::Once = std::sync::Once::new();
    INITIALIZED.call_once(|| {
        crate::internal::tests_hooks::TestSecrets::set_concurrent_merge_scheduler_access(
            std::sync::Arc::new(ConcurrentMergeSchedulerAccessImpl),
        )
        .expect(
            "INVARIANT: ConcurrentMergeScheduler's static initializer runs once, from this file",
        );
    });
}
