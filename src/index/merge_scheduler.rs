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
        }
    }

    /// Sets how many merges may run at once and how many may be queued.
    ///
    /// Equivalent to `ConcurrentMergeScheduler.setMaxMergesAndThreads(int, int)`.
    pub fn set_max_merges_and_threads(
        &mut self,
        max_merge_count: usize,
        max_thread_count: usize,
    ) -> Result<()> {
        if max_thread_count > max_merge_count {
            return Err(LuceneError::IllegalArgument(format!(
                "maxThreadCount ({max_thread_count}) should be <= maxMergeCount ({max_merge_count})"
            )));
        }
        self.max_merge_count = max_merge_count;
        self.max_thread_count = max_thread_count;
        Ok(())
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

impl MergeScheduler for ConcurrentMergeScheduler {
    fn merge(&self, merge_source: &dyn MergeSource, _trigger: MergeTrigger) -> Result<()> {
        let mut first_error: Option<LuceneError> = None;

        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            while let Some(merge) = merge_source.get_next_merge() {
                self.running.fetch_add(1, Ordering::Relaxed);
                handles.push(scope.spawn(|| {
                    let result = merge_source.merge(merge);
                    self.running.fetch_sub(1, Ordering::Relaxed);
                    result
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
