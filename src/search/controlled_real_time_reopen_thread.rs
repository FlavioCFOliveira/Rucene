//! Controlled near-real-time reopening, ported from
//! `org.apache.lucene.search.ControlledRealTimeReopenThread`.

#![deny(unsafe_code)]

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::{LuceneError, Result};
use crate::search::reference_manager::{ManagedReference, ReferenceManager, RefreshListener};

/// The one thing [`ControlledRealTimeReopenThread`] needs from an
/// `IndexWriter`: the sequence number of the last completed operation.
///
/// **Divergence from Lucene 10.5.0.** Java's constructor takes an
/// `IndexWriter` and calls `writer.getMaxCompletedSequenceNumber()` from its
/// refresh listener. The port's writer seam,
/// [`crate::index::DirectoryReaderIndexWriter`], does not expose sequence
/// numbers yet, so the thread depends on this one-method trait instead; the
/// real `IndexWriter` implements it once it is ported, and nothing else about
/// the class changes.
pub trait MaxCompletedSequenceNumberSource: Send + Sync {
    /// Returns the sequence number of the last completed operation.
    ///
    /// Equivalent to `IndexWriter.getMaxCompletedSequenceNumber()`.
    fn get_max_completed_sequence_number(&self) -> i64;
}

/// The mutable state a [`ControlledRealTimeReopenThread`] shares with its
/// thread and its refresh listener.
///
/// **Divergence from Lucene 10.5.0.** Java splits this state across two
/// monitors — `reopenLock`/`reopenCond` for the reopen schedule and the
/// instance monitor for `waitForGeneration` — with every field also declared
/// `volatile` so that either side can read it outside its lock. This port uses
/// one mutex and two condition variables, which is a superset of Java's
/// synchronisation: every field is guarded, and each condition variable wakes
/// exactly the waiters Java's matching monitor wakes. It also removes a
/// lock-ordering hazard: Java's `close()` holds the instance monitor while it
/// joins the thread, which the thread's own `afterRefresh` needs.
#[derive(Debug, Default)]
struct SharedState {
    finish: bool,
    waiting_gen: i64,
    searching_gen: i64,
    refresh_start_gen: i64,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<SharedState>,
    /// Signalled when a caller starts waiting for a generation, and on close.
    ///
    /// Equivalent to `reopenCond`.
    reopen_cond: Condvar,
    /// Signalled when `searchingGen` advances.
    ///
    /// Equivalent to the instance monitor's `notifyAll()`.
    search_cond: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, SharedState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The refresh listener that records which generation the reopen covers.
///
/// Equivalent to the inner class
/// `ControlledRealTimeReopenThread.HandleRefresh`.
struct HandleRefresh {
    shared: Arc<Shared>,
    writer: Arc<dyn MaxCompletedSequenceNumberSource>,
}

impl RefreshListener for HandleRefresh {
    fn before_refresh(&self) -> Result<()> {
        // Save the generation as of when the reopen started; `after_refresh`
        // copies it to `searching_gen` once the reopen completes.
        self.shared.lock().refresh_start_gen = self.writer.get_max_completed_sequence_number();
        Ok(())
    }

    fn after_refresh(&self, _did_refresh: bool) -> Result<()> {
        {
            let mut state = self.shared.lock();
            state.searching_gen = state.refresh_start_gen;
        }
        self.shared.search_cond.notify_all();
        Ok(())
    }
}

/// Runs a thread that periodically reopens a
/// [`ReferenceManager`], with a method to wait for a specific index change to
/// become visible.
///
/// Equivalent to `org.apache.lucene.search.ControlledRealTimeReopenThread<T>`,
/// which extends `Thread` and implements `Closeable`. When a search request
/// needs to see a specific index change, call
/// [`wait_for_generation`](Self::wait_for_generation). This only scales well if
/// most searches do not need to wait for a specific generation.
///
/// **Divergence from Lucene 10.5.0.** Java's class *is* a `Thread`, started
/// with `start()`. Rust cannot subclass a thread, so the thread is spawned by
/// [`start`](Self::start) and owned by this value; [`close`](Self::close) joins
/// it, and [`Drop`] closes an instance that was never closed explicitly, so
/// that the thread cannot outlive its owner.
pub struct ControlledRealTimeReopenThread<G>
where
    G: ManagedReference + ?Sized + 'static,
{
    manager: Arc<ReferenceManager<G>>,
    shared: Arc<Shared>,
    target_max_stale_ns: i64,
    target_min_stale_ns: i64,
    listener: Arc<dyn RefreshListener>,
    handle: Option<JoinHandle<()>>,
}

impl<G> std::fmt::Debug for ControlledRealTimeReopenThread<G>
where
    G: ManagedReference + ?Sized + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlledRealTimeReopenThread")
            .field("targetMaxStaleNS", &self.target_max_stale_ns)
            .field("targetMinStaleNS", &self.target_min_stale_ns)
            .field("running", &self.handle.is_some())
            .finish()
    }
}

impl<G> ControlledRealTimeReopenThread<G>
where
    G: ManagedReference + ?Sized + 'static,
{
    /// Creates the reopen thread, which periodically reopens `manager`.
    ///
    /// Equivalent to
    /// `new ControlledRealTimeReopenThread(IndexWriter, ReferenceManager<T>, double, double)`.
    /// Call [`start`](Self::start) to actually run it, which is what Java's
    /// `Thread.start()` does.
    ///
    /// * `target_max_stale_sec` — the maximum time until a new reader must be
    ///   opened; the upper bound on how slowly reopens may occur when nobody is
    ///   waiting for a specific generation;
    /// * `target_min_stale_sec` — the minimum time until a new reader can be
    ///   opened; the lower bound on how quickly reopens may occur when a caller
    ///   is waiting for a specific generation.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's message — when
    /// `target_max_stale_sec` is smaller than `target_min_stale_sec`.
    pub fn new(
        writer: Arc<dyn MaxCompletedSequenceNumberSource>,
        manager: Arc<ReferenceManager<G>>,
        target_max_stale_sec: f64,
        target_min_stale_sec: f64,
    ) -> Result<Self> {
        if target_max_stale_sec < target_min_stale_sec {
            return Err(LuceneError::IllegalArgument(format!(
                "targetMaxScaleSec (= {target_max_stale_sec}) < targetMinStaleSec (={target_min_stale_sec})"
            )));
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(SharedState::default()),
            reopen_cond: Condvar::new(),
            search_cond: Condvar::new(),
        });
        let listener: Arc<dyn RefreshListener> = Arc::new(HandleRefresh {
            shared: Arc::clone(&shared),
            writer,
        });
        manager.add_listener(Arc::clone(&listener));
        Ok(Self {
            manager,
            shared,
            target_max_stale_ns: (1_000_000_000.0 * target_max_stale_sec) as i64,
            target_min_stale_ns: (1_000_000_000.0 * target_min_stale_sec) as i64,
            listener,
            handle: None,
        })
    }

    /// Starts the reopen thread.
    ///
    /// Equivalent to `Thread.start()` on the Java instance. Starting an
    /// already-started thread is a no-op.
    pub fn start(&mut self) {
        if self.handle.is_some() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let manager = Arc::clone(&self.manager);
        let target_max_stale_ns = self.target_max_stale_ns;
        let target_min_stale_ns = self.target_min_stale_ns;
        self.handle = Some(std::thread::spawn(move || {
            run(shared, manager, target_max_stale_ns, target_min_stale_ns);
        }));
    }

    /// Returns the generation the current searcher is guaranteed to include.
    ///
    /// Equivalent to
    /// `ControlledRealTimeReopenThread.getSearchingGen()`.
    pub fn get_searching_gen(&self) -> i64 {
        self.shared.lock().searching_gen
    }

    /// Waits, indefinitely, for the target generation to become visible.
    ///
    /// Equivalent to
    /// `ControlledRealTimeReopenThread.waitForGeneration(long)`, which passes
    /// `-1`.
    pub fn wait_for_generation(&self, target_gen: i64) -> bool {
        self.wait_for_generation_with_timeout(target_gen, -1)
    }

    /// Waits for the target generation to become visible, up to `max_ms`
    /// milliseconds; `-1` waits indefinitely.
    ///
    /// Equivalent to
    /// `ControlledRealTimeReopenThread.waitForGeneration(long, int)`. If the
    /// current searcher is older than the target generation, this blocks until
    /// the searcher is reopened by another thread through
    /// [`ReferenceManager::maybe_refresh`], the waiting time elapses, or the
    /// manager is closed. Returns `true` when the target generation is
    /// available, and `false` when the wait time was exceeded — in which case
    /// the current searcher is the one still in place.
    pub fn wait_for_generation_with_timeout(&self, target_gen: i64, max_ms: i64) -> bool {
        let mut state = self.shared.lock();
        if target_gen > state.searching_gen {
            // Notify the reopen thread that waiting_gen has changed, so that it
            // may wake up and realise it should not sleep for much or any
            // longer before reopening.
            state.waiting_gen = state.waiting_gen.max(target_gen);
            self.shared.reopen_cond.notify_one();

            let start = Instant::now();
            while target_gen > state.searching_gen {
                if max_ms < 0 {
                    state = self
                        .shared
                        .search_cond
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                } else {
                    let elapsed_ms = start.elapsed().as_millis() as i64;
                    let ms_left = max_ms - elapsed_ms;
                    if ms_left <= 0 {
                        return false;
                    }
                    let (guard, _timeout) = self
                        .shared
                        .search_cond
                        .wait_timeout(state, Duration::from_millis(ms_left as u64))
                        .unwrap_or_else(PoisonError::into_inner);
                    state = guard;
                }
            }
        }
        true
    }

    /// Stops the reopen thread and unregisters the refresh listener.
    ///
    /// Equivalent to the `synchronized ControlledRealTimeReopenThread.close()`.
    pub fn close(&mut self) {
        {
            let mut state = self.shared.lock();
            if state.finish && self.handle.is_none() {
                return;
            }
            state.finish = true;
        }
        // So that the thread wakes up and notices it should finish.
        self.shared.reopen_cond.notify_all();

        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.manager.remove_listener(&self.listener);

        // Max it out so that any waiting search thread returns.
        self.shared.lock().searching_gen = i64::MAX;
        self.shared.search_cond.notify_all();
    }
}

impl<G> Drop for ControlledRealTimeReopenThread<G>
where
    G: ManagedReference + ?Sized + 'static,
{
    /// Closes the thread if the caller did not.
    ///
    /// Java relies on an explicit `close()`; a Rust value that owns a thread
    /// must join it before it goes away, so dropping performs the same close.
    fn drop(&mut self) {
        self.close();
    }
}

/// The body of `ControlledRealTimeReopenThread.run()`.
fn run<G>(
    shared: Arc<Shared>,
    manager: Arc<ReferenceManager<G>>,
    target_max_stale_ns: i64,
    target_min_stale_ns: i64,
) where
    G: ManagedReference + ?Sized + 'static,
{
    let mut last_reopen_start = Instant::now();

    loop {
        if shared.lock().finish {
            return;
        }

        // Loop until we have waited long enough before the next reopen.
        loop {
            let mut state = shared.lock();
            if state.finish {
                return;
            }
            // True if someone is waiting for a reopened searcher.
            let has_waiting = state.waiting_gen > state.searching_gen;
            let stale_ns = if has_waiting {
                target_min_stale_ns
            } else {
                target_max_stale_ns
            };
            let elapsed_ns = last_reopen_start.elapsed().as_nanos() as i64;
            let sleep_ns = stale_ns - elapsed_ns;
            if sleep_ns > 0 {
                let (guard, _timeout) = shared
                    .reopen_cond
                    .wait_timeout(state, Duration::from_nanos(sleep_ns as u64))
                    .unwrap_or_else(PoisonError::into_inner);
                state = guard;
                if state.finish {
                    return;
                }
            } else {
                break;
            }
        }

        if shared.lock().finish {
            return;
        }

        last_reopen_start = Instant::now();
        if manager.maybe_refresh_blocking().is_err() {
            // Java wraps the IOException in a RuntimeException, which kills the
            // thread. Rust cannot unwind into the caller, so the thread stops
            // instead; `finish` is set so that a waiting caller is released by
            // the manager being closed.
            shared.lock().finish = true;
            shared.search_cond.notify_all();
            return;
        }
    }
}
