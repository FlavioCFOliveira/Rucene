//! Searcher lifetime tracking, ported from
//! `org.apache.lucene.search.SearcherLifetimeManager`.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::error::{LuceneError, Result};
use crate::search::searcher_manager::ManagedSearcher;

/// Nanoseconds in a second, as a `double`.
///
/// Equivalent to `SearcherLifetimeManager.NANOS_PER_SEC`.
pub const NANOS_PER_SEC: f64 = 1_000_000_000.0;

/// One recorded searcher and the moment it was recorded.
///
/// Equivalent to the private static
/// `SearcherLifetimeManager.SearcherTracker`.
#[derive(Debug)]
struct SearcherTracker {
    searcher: Arc<ManagedSearcher>,
    record_time_sec: f64,
    version: i64,
    closed: AtomicBool,
}

impl SearcherTracker {
    /// Records `searcher`, taking a reference on its reader.
    ///
    /// Equivalent to `new SearcherTracker(IndexSearcher)`.
    fn new(searcher: Arc<ManagedSearcher>, record_time_sec: f64) -> Result<Self> {
        let version = searcher.reader().version()?;
        searcher.reader().inc_ref()?;
        Ok(Self {
            searcher,
            record_time_sec,
            version,
            closed: AtomicBool::new(false),
        })
    }

    /// Releases the reference this tracker holds.
    ///
    /// Equivalent to the `synchronized SearcherTracker.close()`; the
    /// `AtomicBool` plays the role of the monitor, so that a double close
    /// cannot over-decrement.
    fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.searcher.reader().dec_ref()
    }
}

/// Decides which searchers [`SearcherLifetimeManager::prune`] drops.
///
/// Equivalent to the nested interface
/// `SearcherLifetimeManager.Pruner`.
pub trait Pruner {
    /// Returns `true` when this searcher should be removed.
    ///
    /// Equivalent to `Pruner.doPrune(double, IndexSearcher)`.
    ///
    /// * `age_sec` — how much time has passed since this searcher was the
    ///   current (live) searcher;
    /// * `searcher` — the searcher itself.
    fn do_prune(&self, age_sec: f64, searcher: &ManagedSearcher) -> bool;
}

/// Drops any searcher older, by more than the given number of seconds, than the
/// newest searcher.
///
/// Equivalent to the `final` nested class
/// `SearcherLifetimeManager.PruneByAge`.
#[derive(Debug, Clone, Copy)]
pub struct PruneByAge {
    max_age_sec: f64,
}

impl PruneByAge {
    /// Creates the pruner.
    ///
    /// Equivalent to `new PruneByAge(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's message — when
    /// `max_age_sec` is negative.
    pub fn new(max_age_sec: f64) -> Result<Self> {
        if max_age_sec < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxAgeSec must be > 0 (got {max_age_sec})"
            )));
        }
        Ok(Self { max_age_sec })
    }
}

impl Pruner for PruneByAge {
    fn do_prune(&self, age_sec: f64, _searcher: &ManagedSearcher) -> bool {
        age_sec > self.max_age_sec
    }
}

/// Keeps track of the current searcher plus the older ones, closing the old
/// ones once they have timed out.
///
/// Equivalent to `org.apache.lucene.search.SearcherLifetimeManager`.
///
/// Per search request, if it is a "new" search, obtain the latest searcher —
/// for example from a [`SearcherManager`](crate::search::SearcherManager) — and
/// [`record`](Self::record) it; keep the returned token in the results sent to
/// the user. When a follow-up search arrives, pass the token to
/// [`acquire`](Self::acquire) to get the same searcher back, and
/// [`release`](Self::release) it when done. Separately, and ideally from the
/// same thread that reopens the searchers, call [`prune`](Self::prune)
/// periodically.
///
/// Keeping many searchers around uses more resources — open files and RAM —
/// than a single searcher, but as long as the readers are reopened with
/// `DirectoryReader.openIfChanged` they usually share almost every segment.
///
/// **Divergence from Lucene 10.5.0.** Java reads the clock as
/// `System.nanoTime() / NANOS_PER_SEC` and uses `0.0` as the "no previous
/// tracker" sentinel inside `prune`. This port measures from an [`Instant`]
/// captured when the manager is created and carries the sentinel as an
/// [`Option`] instead, so that a searcher recorded at exactly time zero cannot
/// be mistaken for the sentinel. Every age the pruner sees is the same.
#[derive(Debug)]
pub struct SearcherLifetimeManager {
    closed: AtomicBool,
    searchers: Mutex<HashMap<i64, Arc<SearcherTracker>>>,
    prune_lock: Mutex<()>,
    epoch: Instant,
}

impl Default for SearcherLifetimeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SearcherLifetimeManager {
    /// Creates an empty manager.
    ///
    /// Equivalent to `new SearcherLifetimeManager()`.
    pub fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            searchers: Mutex::new(HashMap::new()),
            prune_lock: Mutex::new(()),
            epoch: Instant::now(),
        }
    }

    /// Equivalent to `SearcherLifetimeManager.ensureOpen()`.
    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(LuceneError::AlreadyClosed(
                "this SearcherLifetimeManager instance is closed".to_string(),
            ));
        }
        Ok(())
    }

    fn now_sec(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// Records that this searcher is now in use, returning the token that
    /// [`acquire`](Self::acquire) takes.
    ///
    /// Equivalent to `SearcherLifetimeManager.record(IndexSearcher)`. It is
    /// fine to pass the same searcher again.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] when the manager is closed, and
    /// [`LuceneError::IllegalArgument`] — with Java's message — when a
    /// different searcher instance is recorded under a version that is already
    /// tracked.
    pub fn record(&self, searcher: &Arc<ManagedSearcher>) -> Result<i64> {
        self.ensure_open()?;
        let version = searcher.reader().version()?;
        let mut searchers = self
            .searchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match searchers.get(&version) {
            None => {
                let tracker = SearcherTracker::new(Arc::clone(searcher), self.now_sec())?;
                searchers.insert(version, Arc::new(tracker));
            }
            Some(tracker) => {
                if !Arc::ptr_eq(&tracker.searcher, searcher) {
                    return Err(LuceneError::IllegalArgument(format!(
                        "the provided searcher has the same underlying reader version yet the \
                         searcher instance differs from before (new={searcher:?} vs old={:?}",
                        tracker.searcher
                    )));
                }
            }
        }
        Ok(version)
    }

    /// Retrieves a previously recorded searcher, if it has not been closed yet.
    ///
    /// Equivalent to `SearcherLifetimeManager.acquire(long)`, which returns
    /// `null` when the searcher has already timed out — the caller should then
    /// notify the user that the session timed out, or pull a fresh searcher.
    ///
    /// A non-`None` result must later be matched by
    /// [`release`](Self::release).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] when the manager is closed.
    pub fn acquire(&self, version: i64) -> Result<Option<Arc<ManagedSearcher>>> {
        self.ensure_open()?;
        let searchers = self
            .searchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(tracker) = searchers.get(&version) {
            if tracker.searcher.reader().try_inc_ref() {
                return Ok(Some(Arc::clone(&tracker.searcher)));
            }
        }
        Ok(None)
    }

    /// Releases a searcher previously obtained from
    /// [`acquire`](Self::acquire).
    ///
    /// Equivalent to `SearcherLifetimeManager.release(IndexSearcher)`. It is
    /// fine to call this after [`close`](Self::close).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while closing the reader.
    pub fn release(&self, searcher: &Arc<ManagedSearcher>) -> Result<()> {
        searcher.reader().dec_ref()
    }

    /// Calls the provided [`Pruner`] to prune entries, newest searcher first.
    ///
    /// Equivalent to the `synchronized SearcherLifetimeManager.prune(Pruner)`.
    /// You must call it periodically, ideally from the same background thread
    /// that opens new searchers.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while closing a pruned searcher.
    pub fn prune(&self, pruner: &dyn Pruner) -> Result<()> {
        let _guard = self
            .prune_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut trackers: Vec<Arc<SearcherTracker>> = {
            let searchers = self
                .searchers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            searchers.values().map(Arc::clone).collect()
        };
        // Newer searchers sort before older ones.
        trackers.sort_by(|a, b| {
            b.record_time_sec
                .partial_cmp(&a.record_time_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut last_record_time_sec: Option<f64> = None;
        let now = self.now_sec();
        for tracker in trackers {
            // The first tracker always has an age of 0 seconds, since it is
            // still live; the second tracker's age — the seconds since it was
            // live — is now minus the first tracker's record time, and so on.
            let age_sec = match last_record_time_sec {
                None => 0.0,
                Some(last) => now - last,
            };
            if pruner.do_prune(age_sec, tracker.searcher.as_ref()) {
                self.searchers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&tracker.version);
                tracker.close()?;
            }
            last_record_time_sec = Some(tracker.record_time_sec);
        }
        Ok(())
    }

    /// Closes this manager to future searching; searches still in progress on
    /// other threads are unaffected and should still call
    /// [`release`](Self::release).
    ///
    /// Equivalent to the `synchronized SearcherLifetimeManager.close()`. You
    /// must ensure that no other thread calls [`record`](Self::record) while
    /// this runs, otherwise not all searcher references will be freed.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while closing a searcher, and returns
    /// [`LuceneError::IllegalState`] — with Java's message — when another
    /// thread recorded a searcher during the close.
    pub fn close(&self) -> Result<()> {
        let _guard = self
            .prune_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.closed.store(true, Ordering::Release);
        let to_close: Vec<Arc<SearcherTracker>> = {
            let mut searchers = self
                .searchers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // Remove up front, so that a failure below cannot over-decrement on
            // a double close.
            let trackers: Vec<Arc<SearcherTracker>> = searchers.values().map(Arc::clone).collect();
            for tracker in &trackers {
                searchers.remove(&tracker.version);
            }
            trackers
        };

        let mut first_error = None;
        for tracker in to_close {
            if let Err(error) = tracker.close() {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }

        // Make some effort to catch misuse.
        if !self
            .searchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
        {
            return Err(LuceneError::IllegalState(
                "another thread called record while this SearcherLifetimeManager instance was \
                 being closed; not all searchers were closed"
                    .to_string(),
            ));
        }
        Ok(())
    }
}
