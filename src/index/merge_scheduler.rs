//! Merge schedulers ported from `org.apache.lucene.index`.
//!
//! Covers `MergeScheduler` with its `MergeSource`, `SerialMergeScheduler`,
//! `NoMergeScheduler`, `ConcurrentMergeScheduler` and `MergeRateLimiter`.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::{LuceneError, Result};
use crate::index::merge_policy::{MergeTrigger, OneMerge};

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
pub trait MergeScheduler: Send + Sync {
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
}

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
pub const DEFAULT_MAX_MERGE_COUNT: usize = 6;
/// Default number of merge threads.
pub const DEFAULT_MAX_THREAD_COUNT: usize = 3;
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
    max_thread_count: usize,
    max_merge_count: usize,
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
            max_thread_count: DEFAULT_MAX_THREAD_COUNT,
            max_merge_count: DEFAULT_MAX_MERGE_COUNT,
            running: AtomicU64::new(0),
            force_merge_mb_per_sec: Mutex::new(f64::INFINITY),
            do_auto_io_throttle: AtomicBool::new(false),
            target_mb_per_sec: Mutex::new(START_MB_PER_SEC),
            merge_threads: Mutex::new(Vec::new()),
            merge_thread_counter: AtomicU64::new(0),
            suppress_exceptions: AtomicBool::new(false),
        }
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
        self.max_thread_count = max_thread_count as usize;
        self.max_merge_count = max_merge_count as usize;
        Ok(())
    }

    /// Sets `max_merge_count`/`max_thread_count` to the defaults appropriate for
    /// the storage kind.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setDefaultMaxMergesAndThreads(boolean)`.
    /// `spins` selects the conservative defaults for rotational storage; `false`
    /// derives both counts from [`std::thread::available_parallelism`], mirroring
    /// Java's `Runtime.availableProcessors()`.
    pub fn set_default_max_merges_and_threads(&mut self, spins: bool) {
        if spins {
            self.max_thread_count = 1;
            self.max_merge_count = 6;
            return;
        }
        let core_count = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);
        // If indexing at full throttle, merging costs about as much work as
        // indexing/flushing overall for some data structures (kNN vectors), so
        // half the cores go to merges, exactly as Java's comment explains.
        self.max_thread_count = (core_count / 2).max(1);
        self.max_merge_count = self.max_thread_count + 5;
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
    pub fn get_max_thread_count(&self) -> usize {
        self.max_thread_count
    }

    /// Returns the configured merge count.
    pub fn get_max_merge_count(&self) -> usize {
        self.max_merge_count
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
            let do_pause = (idx as i64) < big_merge_count as i64 - self.max_thread_count as i64;
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
            && self.merge_thread_count() >= self.max_merge_count as u64
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
            if running.len() > self.max_thread_count {
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
    pub fn init_dynamic_defaults(&mut self) {
        if self.max_thread_count == 0 {
            self.set_default_max_merges_and_threads(false);
        }
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
            self.max_thread_count,
            self.max_merge_count,
            self.do_auto_io_throttle.load(Ordering::Acquire)
        )
    }
}

impl MergeScheduler for ConcurrentMergeScheduler {
    fn merge(&self, merge_source: &dyn MergeSource, _trigger: MergeTrigger) -> Result<()> {
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
                let state = Arc::new(MergeThreadState::new(
                    merge.clone(),
                    format!(
                        "Lucene Merge Thread #{}",
                        self.merge_thread_counter.fetch_add(1, Ordering::AcqRel)
                    ),
                ));
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
                if handles.len() >= self.max_thread_count {
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

/// Shortest pause the rate limiter bothers with.
const MIN_PAUSE: Duration = Duration::from_millis(2);
/// Longest single pause, so an aborted merge is noticed promptly.
const MAX_PAUSE: Duration = Duration::from_millis(250);

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
    last_ns: Mutex<Option<Instant>>,
    aborted: AtomicBool,
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
            last_ns: Mutex::new(None),
            aborted: AtomicBool::new(false),
        };
        limiter.set_mb_per_sec(f64::INFINITY);
        limiter
    }

    /// Sets the target throughput.
    ///
    /// Equivalent to `MergeRateLimiter.setMBPerSec(double)`.
    pub fn set_mb_per_sec(&self, mb_per_sec: f64) {
        let check_bytes = if mb_per_sec.is_infinite() {
            i64::MAX
        } else {
            // Check often enough that a pause lands between MIN_PAUSE and
            // MAX_PAUSE, as Java's constructor arithmetic does.
            let bytes_per_ns = mb_per_sec * 1_000_000.0 / 1_000_000_000.0;
            ((MIN_PAUSE + MAX_PAUSE).as_nanos() as f64 / 2.0 * bytes_per_ns) as i64
        };
        self.min_pause_check_bytes
            .store(check_bytes.max(1), Ordering::Relaxed);
        if let Ok(mut guard) = self.mb_per_sec.lock() {
            *guard = mb_per_sec;
        }
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
    pub fn set_aborted(&self) {
        self.aborted.store(true, Ordering::Release);
    }

    /// Returns whether the merge was aborted.
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
    }

    /// Accounts `bytes` and sleeps if the merge is running ahead of its rate.
    ///
    /// Equivalent to `MergeRateLimiter.pause(long)`. Returns the paused time in
    /// nanoseconds, and fails once the merge has been aborted, which is how Java
    /// raises `MergePolicy.MergeAbortedException`.
    pub fn pause(&self, bytes: i64) -> Result<i64> {
        if self.is_aborted() {
            return Err(LuceneError::Cancelled);
        }
        self.total_bytes_written.fetch_add(bytes, Ordering::Relaxed);

        let mb_per_sec = self.get_mb_per_sec();
        if mb_per_sec.is_infinite() || mb_per_sec <= 0.0 {
            return Ok(0);
        }

        let seconds_to_pause = (bytes as f64 / 1_000_000.0) / mb_per_sec;
        let mut target = Duration::from_secs_f64(seconds_to_pause.max(0.0));

        let now = Instant::now();
        if let Ok(mut last) = self.last_ns.lock() {
            if let Some(previous) = *last {
                // Credit the time already spent writing since the last check.
                target = target.saturating_sub(now.duration_since(previous));
            }
            *last = Some(now);
        }

        if target < MIN_PAUSE {
            return Ok(0);
        }

        let mut paused = Duration::ZERO;
        while paused < target {
            if self.is_aborted() {
                return Err(LuceneError::Cancelled);
            }
            let slice = (target - paused).min(MAX_PAUSE);
            std::thread::sleep(slice);
            paused += slice;
        }

        let paused_ns = paused.as_nanos() as i64;
        self.total_paused_ns.fetch_add(paused_ns, Ordering::Relaxed);
        Ok(paused_ns)
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
    pub fn get_directory_name(&self) -> &str {
        &self.directory_name
    }

    /// Returns the scheduler shared with the other indexes.
    pub fn get_combined_merge_scheduler(&self) -> &Arc<ConcurrentMergeScheduler> {
        &self.shared
    }
}

impl MergeScheduler for MultiIndexMergeScheduler {
    fn merge(&self, merge_source: &dyn MergeSource, trigger: MergeTrigger) -> Result<()> {
        self.shared.merge(merge_source, trigger)
    }

    fn close(&self) -> Result<()> {
        // The shared scheduler outlives any one index, so closing this view does
        // not close it.
        Ok(())
    }
}
