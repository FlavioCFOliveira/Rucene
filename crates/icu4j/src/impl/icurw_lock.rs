// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICURWLock`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Condvar, Mutex};
use std::thread::ThreadId;

/// Internal class used to gather statistics on the `ICURWLock`.
///
/// Equivalent to `com.ibm.icu.impl.ICURWLock.Stats`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// `_rc`: number of times read access granted (read count).
    pub rc: i32,
    /// `_mrc`: number of times concurrent read access granted (multiple
    /// read count).
    pub mrc: i32,
    /// `_wrc`: number of times blocked by a writer (writer read count).
    pub wrc: i32,
    /// `_wc`: number of times write access granted (writer count).
    pub wc: i32,
    /// `_wwc`: number of times write access waited (writer wait count).
    pub wwc: i32,
}

impl fmt::Display for Stats {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            " rc: {} mrc: {} wrc: {} wc: {} wwc: {}",
            self.rc, self.mrc, self.wrc, self.wc, self.wwc
        )
    }
}

/// The state of the reentrant read-write lock: the read holds per thread
/// and the writing thread with its hold count.
#[derive(Default)]
struct RwState {
    readers: HashMap<ThreadId, i32>,
    writer: Option<(ThreadId, i32)>,
}

impl RwState {
    fn read_lock_count(&self) -> i32 {
        self.readers.values().sum()
    }
}

/// A reader/writer lock that can gather statistics on its use.
///
/// Equivalent to `com.ibm.icu.impl.ICURWLock`, which wraps a non-fair
/// `java.util.concurrent.locks.ReentrantReadWriteLock`. The lock is
/// reproduced over a mutex and a condition variable with that lock's
/// reentrancy rules: a thread may take the read lock again, may take the
/// write lock again, and may take the read lock while it holds the write
/// lock; any other thread's write lock blocks a reader, and any read lock
/// or other thread's write lock blocks a writer. Acquire and release are
/// separate calls, as in Java, not a guard.
pub struct ICURWLock {
    rwl: Mutex<RwState>,
    cond: Condvar,
    stats: Mutex<Option<Stats>>,
}

impl Default for ICURWLock {
    fn default() -> Self {
        Self::new()
    }
}

impl ICURWLock {
    /// `new ICURWLock()`: unlocked, statistics off.
    pub fn new() -> Self {
        Self {
            rwl: Mutex::new(RwState::default()),
            cond: Condvar::new(),
            stats: Mutex::new(None),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, RwState> {
        self.rwl.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn stats(&self) -> std::sync::MutexGuard<'_, Option<Stats>> {
        self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `resetStats()` (synchronized): starts gathering statistics anew and
    /// returns the previous ones.
    pub fn reset_stats(&self) -> Option<Stats> {
        self.stats().replace(Stats::default())
    }

    /// `clearStats()` (synchronized): stops gathering statistics and
    /// returns the previous ones.
    pub fn clear_stats(&self) -> Option<Stats> {
        self.stats().take()
    }

    /// `getStats()` (synchronized): a copy of the statistics, `None` when
    /// they are not gathered.
    pub fn get_stats(&self) -> Option<Stats> {
        *self.stats()
    }

    /// `acquireRead()`: blocks while another thread holds the write lock.
    pub fn acquire_read(&self) {
        {
            let mut stats = self.stats();
            if let Some(stats) = stats.as_mut() {
                let state = self.state();
                stats.rc += 1;
                if state.read_lock_count() > 0 {
                    stats.mrc += 1;
                }
                if state.writer.is_some() {
                    stats.wrc += 1;
                }
            }
        }
        let me = std::thread::current().id();
        let mut state = self.state();
        while matches!(state.writer, Some((w, _)) if w != me) {
            state = self.cond.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        *state.readers.entry(me).or_insert(0) += 1;
    }

    /// `releaseRead()`.
    ///
    /// # Panics
    ///
    /// Where Java throws `IllegalMonitorStateException`: the thread holds
    /// no read lock.
    pub fn release_read(&self) {
        let me = std::thread::current().id();
        let mut state = self.state();
        let count = state
            .readers
            .get_mut(&me)
            .expect("java.lang.IllegalMonitorStateException: attempt to unlock read lock, not locked by current thread");
        *count -= 1;
        if *count == 0 {
            state.readers.remove(&me);
        }
        drop(state);
        self.cond.notify_all();
    }

    /// `acquireWrite()`: blocks while any read lock or another thread's
    /// write lock is held.
    pub fn acquire_write(&self) {
        {
            let mut stats = self.stats();
            if let Some(stats) = stats.as_mut() {
                let state = self.state();
                stats.wc += 1;
                if state.read_lock_count() > 0 || state.writer.is_some() {
                    stats.wwc += 1;
                }
            }
        }
        let me = std::thread::current().id();
        let mut state = self.state();
        loop {
            if let Some((w, n)) = state.writer {
                if w == me {
                    state.writer = Some((w, n + 1));
                    return;
                }
            } else if state.readers.is_empty() {
                state.writer = Some((me, 1));
                return;
            }
            state = self.cond.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// `releaseWrite()`.
    ///
    /// # Panics
    ///
    /// Where Java throws `IllegalMonitorStateException`: the thread does not
    /// hold the write lock.
    pub fn release_write(&self) {
        let me = std::thread::current().id();
        let mut state = self.state();
        match state.writer {
            Some((w, ref mut n)) if w == me => {
                *n -= 1;
                if *n == 0 {
                    state.writer = None;
                }
            }
            _ => panic!("java.lang.IllegalMonitorStateException"),
        }
        drop(state);
        self.cond.notify_all();
    }
}
