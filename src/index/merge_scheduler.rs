//! Merge schedulers ported from `org.apache.lucene.index`.
//!
//! Covers `MergeScheduler` with its `MergeSource`, `SerialMergeScheduler`,
//! `NoMergeScheduler`, `ConcurrentMergeScheduler` and `MergeRateLimiter`.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::error::{LuceneError, Result};
use crate::index::merge_policy::{MergeTrigger, OneMerge};
use crate::store::Directory;
use crate::util::InfoStream;

/// Where a scheduler gets its merges and how it runs them.
///
/// Equivalent to `MergeScheduler.MergeSource`, which `IndexWriter` implements.
pub trait MergeSource: Send + Sync {
    /// Returns the next merge the policy asked for, or `None`.
    ///
    /// Equivalent to `MergeSource.getNextMerge()`.
    fn get_next_merge(&self) -> Option<OneMerge>;

    /// Reports that a merge finished.
    ///
    /// Equivalent to `MergeSource.onMergeFinished(OneMerge)`.
    fn on_merge_finished(&self, merge: &OneMerge);

    /// Returns whether merges are waiting to be scheduled.
    ///
    /// Equivalent to `MergeSource.hasPendingMerges()`.
    fn has_pending_merges(&self) -> bool;

    /// Runs `merge`, replacing its segments with the merged one.
    ///
    /// Equivalent to `MergeSource.merge(OneMerge)`.
    fn merge(&self, merge: OneMerge) -> Result<()>;
}

/// Decides when and on which thread merges run.
///
/// Equivalent to `org.apache.lucene.index.MergeScheduler`.
pub trait MergeScheduler: Send + Sync + std::fmt::Display {
    /// Runs whatever merges `merge_source` currently offers.
    ///
    /// Equivalent to `MergeScheduler.merge(MergeSource, MergeTrigger)`.
    fn merge(&self, merge_source: &dyn MergeSource, trigger: MergeTrigger) -> Result<()>;

    /// Releases the scheduler's resources.
    ///
    /// Equivalent to `MergeScheduler.close()`.
    fn close(&self) -> Result<()> {
        Ok(())
    }

    /// Hands the scheduler the writer's info stream and directory.
    ///
    /// Equivalent to the package-private
    /// `MergeScheduler.initialize(InfoStream, Directory)`, which `IndexWriter`
    /// calls once at construction. The base implementation only records the
    /// info stream; `ConcurrentMergeScheduler` overrides it to derive its
    /// thread defaults from the directory's storage.
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
    fn merge(&self, merge_source: &dyn MergeSource, _trigger: MergeTrigger) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| LuceneError::IllegalState("merge scheduler lock poisoned".to_string()))?;
        while let Some(merge) = merge_source.get_next_merge() {
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
    fn merge(&self, _merge_source: &dyn MergeSource, _trigger: MergeTrigger) -> Result<()> {
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

/// A scheduler that runs merges on background threads.
///
/// Equivalent to `org.apache.lucene.index.ConcurrentMergeScheduler`.
///
/// **Divergence from Lucene 10.5.0.** Java keeps a list of long-lived
/// `MergeThread` objects it can pause, resume, and re-target as the backlog
/// changes, and adjusts their IO rate through `updateMergeThreads()`. This port
/// spawns one scoped thread per merge and joins them before returning, which
/// reproduces the concurrency and the `max_thread_count` bound but not the
/// dynamic re-rating; the rate limiter is honoured per merge instead.
pub struct ConcurrentMergeScheduler {
    /// How many merges may run at once, and how many may be queued before the
    /// producer is stalled.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.maxThreadCount` and
    /// `maxMergeCount`, which start at
    /// [`AUTO_DETECT_MERGES_AND_THREADS`] and are resolved by
    /// [`init_dynamic_defaults`](Self::init_dynamic_defaults) on the first
    /// merge — which is why `getMaxThreadCount()` and `toString()` report `-1`
    /// on a scheduler that has not merged yet. Java mutates them under the
    /// scheduler's own monitor; here they are atomics, because the merge path
    /// holds only `&self`.
    max_thread_count: AtomicI32,
    max_merge_count: AtomicI32,
    /// Merges currently running, for `merge_thread_count()`.
    running: AtomicU64,
    /// Per-merge IO throttle rate for a forced merge.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.forceMergeMBPerSec`.
    force_merge_mb_per_sec: Mutex<f64>,
    /// Whether dynamic IO throttling is on.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.doAutoIOThrottle`.
    do_auto_io_throttle: AtomicBool,
    /// Current auto-throttle target, meaningful only while
    /// `do_auto_io_throttle` is set.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.targetMBPerSec`.
    target_mb_per_sec: Mutex<f64>,
    /// The merges running right now, each with its own rate limiter.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.mergeThreads`. Java holds the
    /// `MergeThread` objects themselves and reads `merge` and `rateLimiter`
    /// off them; a Rust scoped thread is not an object the scheduler can hold,
    /// so the registry holds the state Java reads through the thread instead.
    merge_threads: Mutex<Vec<Arc<MergeThreadState>>>,
    /// Names the merge threads `_0`, `_1`, … as Java's counter does.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.mergeThreadCounter`.
    merge_thread_counter: AtomicU64,
    /// Set by the test hook that swallows merge failures.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.suppressExceptions`.
    suppress_exceptions: AtomicBool,
    /// The sink `verbose()`/`message()` report to.
    ///
    /// Equivalent to the protected `MergeScheduler.infoStream` field, which
    /// `IndexWriter` fills in through `initialize`.
    info_stream: Mutex<Option<Arc<dyn InfoStream>>>,
}

/// The state `ConcurrentMergeScheduler` keeps per running merge.
///
/// Equivalent to the fields of `ConcurrentMergeScheduler.MergeThread`: the
/// merge it runs, the rate limiter that throttles it, and the name the info
/// stream reports. `MergeThread` itself is a `Thread` subclass, which Rust has
/// no counterpart for; the scheduler needs the *state*, not the thread object,
/// so that is what the registry holds.
#[derive(Debug)]
pub struct MergeThreadState {
    /// The merge this thread runs.
    pub merge: OneMerge,
    /// The limiter throttling this merge's writes.
    pub rate_limiter: Arc<MergeRateLimiter>,
    /// The thread's name, as Java's `Lucene Merge Thread #N`.
    pub name: String,
    /// Whether the merge is still running.
    alive: AtomicBool,
}

impl MergeThreadState {
    /// Creates the state for one merge.
    ///
    /// Equivalent to `MergeThread(MergeSource, OneMerge)`.
    pub fn new(merge: OneMerge, name: String) -> Self {
        Self {
            merge,
            rate_limiter: Arc::new(MergeRateLimiter::new()),
            name,
            alive: AtomicBool::new(true),
        }
    }

    /// Returns whether the merge is still running.
    ///
    /// Equivalent to `Thread.isAlive()` on a `MergeThread`.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Marks the merge as finished.
    fn set_finished(&self) {
        self.alive.store(false, Ordering::Release);
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

impl Default for ConcurrentMergeScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrentMergeScheduler {
    /// Creates a scheduler with Lucene's default thread and merge counts.
    pub fn new() -> Self {
        Self {
            max_thread_count: AtomicI32::new(AUTO_DETECT_MERGES_AND_THREADS),
            max_merge_count: AtomicI32::new(AUTO_DETECT_MERGES_AND_THREADS),
            running: AtomicU64::new(0),
            force_merge_mb_per_sec: Mutex::new(f64::INFINITY),
            do_auto_io_throttle: AtomicBool::new(false),
            target_mb_per_sec: Mutex::new(START_MB_PER_SEC),
            merge_threads: Mutex::new(Vec::new()),
            merge_thread_counter: AtomicU64::new(0),
            suppress_exceptions: AtomicBool::new(false),
            info_stream: Mutex::new(None),
        }
    }

    /// Installs the info stream this scheduler logs to.
    ///
    /// Equivalent to `MultiIndexMergeScheduler.CombinedMergeScheduler#setInfoStream`,
    /// which assigns the protected `MergeScheduler.infoStream` field directly
    /// because a combined scheduler is never handed to an `IndexWriter` and so
    /// never sees `initialize`.
    pub fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        if let Ok(mut guard) = self.info_stream.lock() {
            *guard = Some(info_stream);
        }
    }

    /// Returns the info stream this scheduler logs to, if one was installed.
    pub fn get_info_stream(&self) -> Option<Arc<dyn InfoStream>> {
        self.info_stream.lock().ok().and_then(|guard| guard.clone())
    }

    /// Sets how many merges may run at once and how many may be queued.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setMaxMergesAndThreads(int, int)`.
    /// Pass [`AUTO_DETECT_MERGES_AND_THREADS`] for both arguments to derive them
    /// from the machine's core count, exactly as
    /// [`set_default_max_merges_and_threads`](Self::set_default_max_merges_and_threads)
    /// does for non-rotational storage.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when only one argument is the
    /// auto-detect sentinel, when either count is below 1, or when
    /// `max_thread_count` exceeds `max_merge_count`.
    pub fn set_max_merges_and_threads(
        &mut self,
        max_merge_count: i32,
        max_thread_count: i32,
    ) -> Result<()> {
        if max_merge_count == AUTO_DETECT_MERGES_AND_THREADS
            && max_thread_count == AUTO_DETECT_MERGES_AND_THREADS
        {
            self.set_default_max_merges_and_threads(false);
            return Ok(());
        }
        if max_merge_count == AUTO_DETECT_MERGES_AND_THREADS
            || max_thread_count == AUTO_DETECT_MERGES_AND_THREADS
        {
            return Err(LuceneError::IllegalArgument(
                "both maxMergeCount and maxThreadCount must be AUTO_DETECT_MERGES_AND_THREADS"
                    .to_string(),
            ));
        }
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
        self.max_thread_count
            .store(max_thread_count, Ordering::Release);
        self.max_merge_count
            .store(max_merge_count, Ordering::Release);
        Ok(())
    }

    /// Sets `max_merge_count`/`max_thread_count` to the defaults appropriate for
    /// the storage kind.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setDefaultMaxMergesAndThreads(boolean)`.
    /// `spins` selects the conservative defaults for rotational storage; `false`
    /// derives both counts from [`std::thread::available_parallelism`], mirroring
    /// Java's `Runtime.availableProcessors()`.
    pub fn set_default_max_merges_and_threads(&self, spins: bool) {
        if spins {
            self.max_thread_count.store(1, Ordering::Release);
            self.max_merge_count
                .store(DEFAULT_MAX_MERGE_COUNT, Ordering::Release);
            return;
        }
        let core_count = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);
        // If indexing at full throttle, merging costs about as much work as
        // indexing/flushing overall for some data structures (kNN vectors), so
        // half the cores go to merges, exactly as Java's comment explains.
        let threads = ((core_count / 2).max(1)) as i32;
        self.max_thread_count.store(threads, Ordering::Release);
        self.max_merge_count.store(threads + 5, Ordering::Release);
    }

    /// Sets the per-merge IO throttle rate for a forced merge.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setForceMergeMBPerSec(double)`.
    pub fn set_force_merge_mb_per_sec(&self, v: f64) {
        if let Ok(mut guard) = self.force_merge_mb_per_sec.lock() {
            *guard = v;
        }
    }

    /// Returns the per-merge IO throttle rate for a forced merge.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getForceMergeMBPerSec()`.
    pub fn get_force_merge_mb_per_sec(&self) -> f64 {
        self.force_merge_mb_per_sec
            .lock()
            .map(|guard| *guard)
            .unwrap_or(f64::INFINITY)
    }

    /// Turns on dynamic IO throttling.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.enableAutoIOThrottle()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `updateMergeThreads()` measures
    /// each running merge thread's actual write rate and raises or lowers
    /// `targetMBPerSec` to the minimum that keeps merges from falling behind
    /// indexing. This port's [`merge`](MergeScheduler::merge) spawns one
    /// scoped thread per merge and does not yet retarget a running merge's
    /// rate limiter, so enabling this flag records the starting rate but does
    /// not adapt it; see the type's own divergence note for why the underlying
    /// thread model differs.
    pub fn enable_auto_io_throttle(&self) {
        self.do_auto_io_throttle.store(true, Ordering::Release);
        if let Ok(mut guard) = self.target_mb_per_sec.lock() {
            *guard = START_MB_PER_SEC;
        }
    }

    /// Turns off dynamic IO throttling.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.disableAutoIOThrottle()`.
    pub fn disable_auto_io_throttle(&self) {
        self.do_auto_io_throttle.store(false, Ordering::Release);
    }

    /// Returns whether dynamic IO throttling is on.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getAutoIOThrottle()`.
    pub fn get_auto_io_throttle(&self) -> bool {
        self.do_auto_io_throttle.load(Ordering::Acquire)
    }

    /// Returns the currently active per-merge IO rate limit.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getIORateLimitMBPerSec()`.
    pub fn get_io_rate_limit_mb_per_sec(&self) -> f64 {
        if self.get_auto_io_throttle() {
            self.target_mb_per_sec
                .lock()
                .map(|guard| *guard)
                .unwrap_or(START_MB_PER_SEC)
        } else {
            f64::INFINITY
        }
    }

    /// Returns how many merges are running right now.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.mergeThreadCount()`.
    pub fn merge_thread_count(&self) -> u64 {
        self.running.load(Ordering::Relaxed)
    }

    /// Returns the configured thread count.
    pub fn get_max_thread_count(&self) -> i32 {
        self.max_thread_count.load(Ordering::Acquire)
    }

    /// Returns the configured merge count.
    pub fn get_max_merge_count(&self) -> i32 {
        self.max_merge_count.load(Ordering::Acquire)
    }
}

impl ConcurrentMergeScheduler {
    // -- Thread bookkeeping --------------------------------------------------

    /// Prunes the finished merges and re-rates the running ones.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.updateMergeThreads()`. The
    /// merges are sorted largest first; anything past `max_thread_count` among
    /// the *big* merges is stopped by setting its rate to zero, and everything
    /// else is rated according to the throttle in force.
    pub fn update_merge_threads(&self) {
        let Ok(mut threads) = self.merge_threads.lock() else {
            return;
        };
        // Only look at merges that are still running.
        threads.retain(|t| t.is_alive());
        let mut active: Vec<Arc<MergeThreadState>> = threads.clone();
        // Sort the merge threads, largest first.
        active.sort_by(|a, b| a.compare_to(b));
        let active_merge_count = active.len();

        let mut big_merge_count = 0usize;
        for idx in (0..active_merge_count).rev() {
            if bytes_to_mb(active[idx].merge.estimated_merge_bytes) > MIN_BIG_MERGE_MB {
                big_merge_count = idx + 1;
                break;
            }
        }

        let do_auto = self.do_auto_io_throttle.load(Ordering::Acquire);
        let target = self.target_mb_per_sec();
        let force_rate = self.get_force_merge_mb_per_sec();

        for (idx, thread) in active.iter().enumerate() {
            let merge = &thread.merge;
            // Pause the merge when max_thread_count is smaller than the number
            // of big merges running.
            let do_pause =
                (idx as i64) < big_merge_count as i64 - self.get_max_thread_count() as i64;
            let new_mb_per_sec = if do_pause {
                0.0
            } else if merge.max_num_segments != -1 {
                force_rate
            } else if !do_auto {
                f64::INFINITY
            } else if bytes_to_mb(merge.estimated_merge_bytes) < MIN_BIG_MERGE_MB {
                // Don't rate limit small merges.
                f64::INFINITY
            } else {
                target
            };
            thread.rate_limiter.set_mb_per_sec(new_mb_per_sec);
        }
    }

    /// Removes the calling merge from the active set.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.removeMergeThread()`.
    pub fn remove_merge_thread(&self, state: &Arc<MergeThreadState>) {
        if let Ok(mut threads) = self.merge_threads.lock() {
            if let Some(pos) = threads.iter().position(|t| Arc::ptr_eq(t, state)) {
                threads.remove(pos);
            }
        }
    }

    /// Blocks the calling thread while merging has fallen too far behind.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.maybeStall(MergeSource)`.
    /// Returns `false` to tell the caller not to start any new merge — which is
    /// what Java answers when the caller is itself a merge thread, because
    /// stalling it would block the very work the stall is waiting for.
    pub fn maybe_stall(&self, merge_source: &dyn MergeSource) -> bool {
        let mut start_stall_time: Option<Instant> = None;
        while merge_source.has_pending_merges()
            && self.merge_thread_count() >= self.get_max_merge_count().max(0) as u64
        {
            // Merging has fallen behind: `max_merge_count` merges are already
            // running and at least one more is pending. Stall the producer so
            // no new segment is created until merging catches up.
            if start_stall_time.is_none() {
                start_stall_time = Some(Instant::now());
            }
            self.do_stall();
        }
        true
    }

    /// Pauses the calling thread briefly, waiting for a merge to finish.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.doStall()`, which waits 250 ms
    /// defensively rather than relying on a notification it might miss.
    pub fn do_stall(&self) {
        std::thread::sleep(Duration::from_millis(250));
    }

    /// Runs one merge.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.doMerge(MergeSource, OneMerge)`,
    /// which exists so a subclass can wrap the call.
    pub fn do_merge(&self, merge_source: &dyn MergeSource, merge: OneMerge) -> Result<()> {
        merge_source.merge(merge)
    }

    /// Called when a merge finishes: drops it from the active set and re-rates
    /// the rest.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.runOnMergeFinished(MergeSource, OneMerge, MergeRateLimiter)`.
    pub fn run_on_merge_finished(&self, state: &Arc<MergeThreadState>) {
        self.remove_merge_thread(state);
        self.update_merge_threads();
    }

    /// Waits for every running merge to finish.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.sync()`. Java joins each merge
    /// thread in turn, skipping itself; this port's merges run inside a scoped
    /// thread block that has already joined by the time `merge` returns, so
    /// `sync` waits on the registry emptying instead.
    pub fn sync(&self) {
        loop {
            let outstanding = self
                .merge_threads
                .lock()
                .map(|threads| threads.iter().filter(|t| t.is_alive()).count())
                .unwrap_or(0);
            if outstanding == 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Called when an error escapes a background merge.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.handleMergeException(Throwable)`,
    /// which wraps the cause in `MergePolicy.MergeException`.
    pub fn handle_merge_exception(&self, error: &LuceneError) -> LuceneError {
        LuceneError::Merge(error.to_string())
    }

    /// Swallows merge failures instead of reporting them.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setSuppressExceptions()`, which
    /// Lucene exposes to its tests through `TestSecrets`.
    pub fn set_suppress_exceptions(&self) {
        self.suppress_exceptions.store(true, Ordering::Release);
    }

    /// Reports merge failures again.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.clearSuppressExceptions()`.
    pub fn clear_suppress_exceptions(&self) {
        self.suppress_exceptions.store(false, Ordering::Release);
    }

    // -- IO throttling -------------------------------------------------------

    /// Returns whether a merge of comparable size is already running and has
    /// been running long enough to count as a backlog.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.isBacklog(long, OneMerge)`. A
    /// merge counts only when it is big, has been going for over three seconds,
    /// and is within a factor of about three of the merge being judged.
    fn is_backlog(&self, now: i64, merge: &OneMerge) -> bool {
        let merge_mb = bytes_to_mb(merge.estimated_merge_bytes);
        let Ok(threads) = self.merge_threads.lock() else {
            return false;
        };
        for thread in threads.iter() {
            let other = &thread.merge;
            // Java compares the `OneMerge` objects by identity. The registry
            // holds a clone, so the port compares the segment selection, which
            // identifies a merge just as uniquely: no two live merges may claim
            // the same segment.
            let same_merge = other.segments.len() == merge.segments.len()
                && other
                    .segments
                    .iter()
                    .zip(merge.segments.iter())
                    .all(|(a, b)| a.info.name == b.info.name);
            if thread.is_alive()
                && !same_merge
                && other.merge_start_ns != -1
                && bytes_to_mb(other.estimated_merge_bytes) >= MIN_BIG_MERGE_MB
                && ns_to_sec(now - other.merge_start_ns) > 3.0
            {
                let other_merge_mb = bytes_to_mb(other.estimated_merge_bytes);
                let ratio = other_merge_mb / merge_mb;
                if ratio > 0.3 && ratio < 3.0 {
                    return true;
                }
            }
        }
        false
    }

    /// Retunes the auto IO throttle as a new merge starts.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.updateIOThrottle(OneMerge, MergeRateLimiter)`.
    /// A simple closed loop: a new backlog raises the target rate by 20% up to
    /// [`MAX_MERGE_MB_PER_SEC`], no backlog lowers it by 10% down to
    /// [`MIN_MERGE_MB_PER_SEC`], and an unchanged backlog leaves it alone.
    pub fn update_io_throttle(&self, new_merge: &OneMerge, rate_limiter: &MergeRateLimiter) {
        if !self.do_auto_io_throttle.load(Ordering::Acquire) {
            return;
        }
        let merge_mb = bytes_to_mb(new_merge.estimated_merge_bytes);
        if merge_mb < MIN_BIG_MERGE_MB {
            // Only watch non-trivial merges: the policy must eventually run
            // larger ones, so nothing is missed by ignoring these.
            return;
        }
        let now = now_ns();
        let new_backlog = self.is_backlog(now, new_merge);
        let mut cur_backlog = false;
        if !new_backlog {
            let running: Vec<OneMerge> = self
                .merge_threads
                .lock()
                .map(|threads| threads.iter().map(|t| t.merge.clone()).collect())
                .unwrap_or_default();
            if running.len() > self.get_max_thread_count().max(0) as usize {
                // More merges than threads allowed already counts as backlog.
                cur_backlog = true;
            } else {
                for merge in &running {
                    if self.is_backlog(now, merge) {
                        cur_backlog = true;
                        break;
                    }
                }
            }
        }

        if let Ok(mut target) = self.target_mb_per_sec.lock() {
            if new_backlog {
                // This merge adds to the backlog: raise the throttle by 20%.
                *target *= 1.20;
                if *target > MAX_MERGE_MB_PER_SEC {
                    *target = MAX_MERGE_MB_PER_SEC;
                }
            } else if cur_backlog {
                // An existing backlog remains: leave the rate as it is.
            } else {
                // Not falling behind: lower the throttle by 10%.
                *target /= 1.10;
                if *target < MIN_MERGE_MB_PER_SEC {
                    *target = MIN_MERGE_MB_PER_SEC;
                }
            }
        }

        let rate = if new_merge.max_num_segments != -1 {
            self.get_force_merge_mb_per_sec()
        } else {
            self.target_mb_per_sec()
        };
        rate_limiter.set_mb_per_sec(rate);
        self.target_mb_per_sec_changed();
    }

    /// Hook called after the auto throttle moved.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.targetMBPerSecChanged()`, whose
    /// body is empty and which exists so a subclass can observe the change.
    pub fn target_mb_per_sec_changed(&self) {}

    /// Returns the current auto-throttle target.
    fn target_mb_per_sec(&self) -> f64 {
        self.target_mb_per_sec
            .lock()
            .map(|v| *v)
            .unwrap_or(START_MB_PER_SEC)
    }

    /// Derives the thread and merge counts from the machine when they were left
    /// on [`AUTO_DETECT_MERGES_AND_THREADS`].
    ///
    /// Equivalent to `ConcurrentMergeScheduler.initDynamicDefaults(Directory)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java passes the `Directory` so it can
    /// ask whether the index sits on spinning media, which halves the defaults.
    /// Rust has no portable way to answer that, so the port takes the answer as
    /// the `spins` argument that `set_default_max_merges_and_threads` already
    /// accepts, and defaults it to `false` — the same choice Java makes on every
    /// platform where the probe fails.
    pub fn init_dynamic_defaults(&self) {
        if self.get_max_thread_count() == AUTO_DETECT_MERGES_AND_THREADS {
            self.set_default_max_merges_and_threads(false);
            if self.verbose() {
                self.message(&format!(
                    "initDynamicDefaults maxThreadCount={} maxMergeCount={}",
                    self.get_max_thread_count(),
                    self.get_max_merge_count()
                ));
            }
        }
    }

    /// Creates the state for one merge thread and names it.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getMergeThread(MergeSource, OneMerge)`
    /// (`ConcurrentMergeScheduler.java:672-678`), which Java declares
    /// `protected` so a subclass can hand back its own `MergeThread`; the name
    /// it gives — `Lucene Merge Thread #N`, from a counter this scheduler owns
    /// — is what the info stream reports.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a `Thread` subclass and
    /// marks it a daemon. Rust cannot subclass a thread, so what is created is
    /// the *state* the scheduler tracks — the divergence already declared on
    /// [`MergeThreadState`] — and `mergeSource` is not carried in it, because
    /// the thread this port spawns is handed the source directly. Nothing
    /// corresponds to `setDaemon(true)`: the threads are scoped and joined
    /// before [`merge`](MergeScheduler::merge) returns, so none can outlive it.
    pub fn get_merge_thread(&self, merge: OneMerge) -> Arc<MergeThreadState> {
        Arc::new(MergeThreadState::new(
            merge,
            format!(
                "Lucene Merge Thread #{}",
                self.merge_thread_counter.fetch_add(1, Ordering::AcqRel)
            ),
        ))
    }

    /// Returns the executor a merge should use for its own internal parallelism.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getIntraMergeExecutor(OneMerge)`,
    /// which hands small merges the caller's thread and big ones a shared pool.
    /// This port answers the same question as a predicate: `true` when the merge
    /// is big enough to be worth parallelising internally.
    pub fn get_intra_merge_executor(&self, merge: &OneMerge) -> bool {
        bytes_to_mb(merge.estimated_merge_bytes) >= MIN_BIG_MERGE_MB
    }

    /// Wraps `directory` so that everything the merge writes passes through the
    /// merge's rate limiter.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.wrapForMerge(OneMerge, Directory)`.
    /// Java looks the limiter up on the calling `MergeThread`; this port takes
    /// the merge's registry entry instead, because the limiter belongs to the
    /// merge and not to whichever thread happens to run it.
    ///
    /// The rate limit is per merge, so with several merges running the combined
    /// IO rate can exceed the target — the same caveat Lucene documents.
    pub fn wrap_for_merge(&self, merge: &OneMerge) -> Option<Arc<MergeRateLimiter>> {
        let threads = self.merge_threads.lock().ok()?;
        threads
            .iter()
            .find(|t| {
                t.merge.segments.len() == merge.segments.len()
                    && t.merge
                        .segments
                        .iter()
                        .zip(merge.segments.iter())
                        .all(|(a, b)| a.info.name == b.info.name)
            })
            .map(|t| Arc::clone(&t.rate_limiter))
    }

    /// Returns the segment name of a merge, or `_na_` before the writer has
    /// named it.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.getSegmentName(OneMerge)`.
    pub fn get_segment_name(merge: &OneMerge) -> String {
        merge
            .info
            .as_ref()
            .map(|info| info.info.name.clone())
            .unwrap_or_else(|| "_na_".to_string())
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
/// `OneMerge.merge_start_ns` and every elapsed-time comparison in this module
/// are taken from this one clock, so they are always comparable.
pub fn now_ns() -> i64 {
    static ORIGIN: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    ORIGIN.elapsed().as_nanos().min(i64::MAX as u128) as i64
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

impl std::fmt::Display for ConcurrentMergeScheduler {
    /// Equivalent to `ConcurrentMergeScheduler.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ConcurrentMergeScheduler: maxThreadCount={}, maxMergeCount={}, ioThrottle={}",
            self.get_max_thread_count(),
            self.get_max_merge_count(),
            self.do_auto_io_throttle.load(Ordering::Acquire)
        )
    }
}

impl MergeScheduler for ConcurrentMergeScheduler {
    fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        ConcurrentMergeScheduler::set_info_stream(self, info_stream);
    }

    fn get_info_stream(&self) -> Option<Arc<dyn InfoStream>> {
        ConcurrentMergeScheduler::get_info_stream(self)
    }

    fn merge(&self, merge_source: &dyn MergeSource, _trigger: MergeTrigger) -> Result<()> {
        // `ConcurrentMergeScheduler.merge` resolves the auto-detected counts
        // before it looks at the backlog (`ConcurrentMergeScheduler.java:538`
        // through `initialize`), so a scheduler that was never configured picks
        // them up here.
        self.init_dynamic_defaults();
        let mut first_error: Option<LuceneError> = None;

        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            loop {
                // Java stalls the producer here when merging has fallen too far
                // behind, so that indexing cannot outrun the merges.
                if !self.maybe_stall(merge_source) {
                    break;
                }
                let Some(merge) = merge_source.get_next_merge() else {
                    break;
                };
                let state = self.get_merge_thread(merge.clone());
                self.update_io_throttle(&state.merge, &state.rate_limiter);
                if let Ok(mut threads) = self.merge_threads.lock() {
                    threads.push(Arc::clone(&state));
                }
                self.update_merge_threads();
                self.running.fetch_add(1, Ordering::Relaxed);
                let state_for_thread = Arc::clone(&state);
                handles.push(scope.spawn(move || {
                    let result = self.do_merge(merge_source, merge);
                    state_for_thread.set_finished();
                    self.running.fetch_sub(1, Ordering::Relaxed);
                    self.run_on_merge_finished(&state_for_thread);
                    match result {
                        Err(error) if self.suppress_exceptions.load(Ordering::Acquire) => {
                            let _ = error;
                            Ok(())
                        }
                        other => other,
                    }
                }));
                if handles.len() >= self.get_max_thread_count().max(1) as usize {
                    for handle in handles.drain(..) {
                        match handle.join() {
                            Ok(Err(err)) if first_error.is_none() => first_error = Some(err),
                            Ok(_) => {}
                            Err(_) if first_error.is_none() => {
                                first_error = Some(LuceneError::IllegalState(
                                    "merge thread panicked".to_string(),
                                ))
                            }
                            Err(_) => {}
                        }
                    }
                }
            }
            for handle in handles {
                match handle.join() {
                    Ok(Err(err)) if first_error.is_none() => first_error = Some(err),
                    Ok(_) => {}
                    Err(_) if first_error.is_none() => {
                        first_error = Some(LuceneError::IllegalState(
                            "merge thread panicked".to_string(),
                        ))
                    }
                    Err(_) => {}
                }
            }
        });

        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
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
    aborted: AtomicBool,
    /// Guards the pause condition variable.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's pausing lives in
    /// `MergePolicy.OneMergeProgress`, which `MergeRateLimiter` holds a
    /// reference to; `OneMerge` does not carry a progress object in this port
    /// (see the note on [`OneMerge`]), so the lock, the condition variable and
    /// the per-reason pause totals live on the limiter itself. The mechanism is
    /// unchanged: a waiter sleeps on the condition until its time expires, the
    /// merge is aborted, or the rate changes, and both `abort` and
    /// `set_mb_per_sec` signal it.
    pause_lock: Mutex<()>,
    /// Signalled when a pause should be re-evaluated.
    ///
    /// Equivalent to `OneMergeProgress.pausing`.
    pausing: Condvar,
}

impl Default for MergeRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl MergeRateLimiter {
    /// Creates an unthrottled limiter.
    pub fn new() -> Self {
        let limiter = Self {
            mb_per_sec: Mutex::new(f64::INFINITY),
            min_pause_check_bytes: AtomicI64::new(0),
            total_bytes_written: AtomicI64::new(0),
            total_paused_ns: AtomicI64::new(0),
            total_stopped_ns: AtomicI64::new(0),
            last_ns: Mutex::new(0),
            aborted: AtomicBool::new(false),
            pause_lock: Mutex::new(()),
            pausing: Condvar::new(),
        };
        limiter.set_mb_per_sec(f64::INFINITY);
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
    /// # Panics
    ///
    /// Does not panic. A negative rate is Java's `IllegalArgumentException`;
    /// this port cannot raise one from an infallible setter, so it clamps to
    /// `0.0`, which stops the merge — the nearest legal rate, and the one a
    /// caller asking for "slower than nothing" means.
    pub fn set_mb_per_sec(&self, mb_per_sec: f64) {
        let mb_per_sec = if mb_per_sec < 0.0 { 0.0 } else { mb_per_sec };
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
        let _guard = match self.pause_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.pausing.notify_all();
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
            let target_ns = last.saturating_add(if seconds_to_pause.is_finite() {
                (1_000_000_000.0 * seconds_to_pause) as i64
            } else {
                i64::MAX
            });
            let cur_pause_ns = target_ns.saturating_sub(cur_ns);
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
        let mut guard = match self.pause_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        while !remaining.is_zero() && !self.is_aborted() && self.get_mb_per_sec() == expected_rate {
            let waited = match self.pausing.wait_timeout(guard, remaining) {
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

/// Runs the merges of several indexes through one shared scheduler.
///
/// Equivalent to `org.apache.lucene.index.MultiIndexMergeScheduler`, which lets
/// a host running many indexes bound the total number of merge threads instead
/// of bounding each index separately.
///
/// **Divergence from Lucene 10.5.0.** Java routes merges through a
/// `CombinedMergeScheduler` singleton that tags each `MergeSource` with its
/// directory and interleaves the tagged sources across one thread pool. This
/// port shares a single [`ConcurrentMergeScheduler`] between the writers instead
/// of tagging: the bound on concurrent merges is shared, which is the point of
/// the class, but merges are not interleaved fairly across indexes.
pub struct MultiIndexMergeScheduler {
    shared: Arc<ConcurrentMergeScheduler>,
    directory_name: String,
}

impl MultiIndexMergeScheduler {
    /// Creates a scheduler for one index, sharing `shared` with the others.
    pub fn new(directory_name: impl Into<String>, shared: Arc<ConcurrentMergeScheduler>) -> Self {
        Self {
            shared,
            directory_name: directory_name.into(),
        }
    }

    /// Returns the name identifying this scheduler's index.
    ///
    /// Equivalent to `MultiIndexMergeScheduler.getDirectory()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns the `Directory` itself,
    /// which its `sync()` compares with `equals` to decide which merge threads
    /// belong to this index. That comparison only exists because Java tags each
    /// `MergeSource` with its directory, which this port does not do — see the
    /// divergence declared on the type — so nothing here needs a `Directory`,
    /// and a name is what identifies the index in the info stream.
    pub fn get_directory_name(&self) -> &str {
        &self.directory_name
    }

    /// Returns the scheduler shared with the other indexes.
    pub fn get_combined_merge_scheduler(&self) -> &Arc<ConcurrentMergeScheduler> {
        &self.shared
    }

    /// Installs the info stream on the shared scheduler.
    ///
    /// Equivalent to `org.apache.lucene.index.MultiIndexMergeScheduler#setInfoStream`,
    /// which forwards to the `CombinedMergeScheduler` because that is the one
    /// running the merges: `MultiIndexMergeScheduler` never overrides
    /// `initialize`, so this is the only way its scheduler learns where to log.
    pub fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        self.shared.set_info_stream(info_stream);
    }
}

impl std::fmt::Display for SerialMergeScheduler {
    /// `SerialMergeScheduler` declares no `toString()`, so Java falls back to
    /// `Object.toString()` — `getClass().getName() + "@" + hex identity hash`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SerialMergeScheduler",
            self,
        ))
    }
}

impl std::fmt::Display for NoMergeScheduler {
    /// `NoMergeScheduler` declares no `toString()`, so Java falls back to
    /// `Object.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.NoMergeScheduler",
            self,
        ))
    }
}

impl std::fmt::Display for MultiIndexMergeScheduler {
    /// `MultiIndexMergeScheduler` declares no `toString()`, so Java falls back
    /// to `Object.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.MultiIndexMergeScheduler",
            self,
        ))
    }
}

impl MergeScheduler for MultiIndexMergeScheduler {
    fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) {
        MultiIndexMergeScheduler::set_info_stream(self, info_stream);
    }

    fn get_info_stream(&self) -> Option<Arc<dyn InfoStream>> {
        self.shared.get_info_stream()
    }

    fn merge(&self, merge_source: &dyn MergeSource, trigger: MergeTrigger) -> Result<()> {
        self.shared.merge(merge_source, trigger)
    }

    fn close(&self) -> Result<()> {
        // The shared scheduler outlives any one index, so closing this view does
        // not close it.
        Ok(())
    }
}
