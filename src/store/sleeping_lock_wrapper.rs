//! A directory that sleeps and retries when obtaining a lock fails.
//!
//! Ported from `org.apache.lucene.store.SleepingLockWrapper`.

use std::collections::HashSet;
use std::thread;
use std::time::Duration;

use super::{Directory, FilterDirectory, IOContext, IndexInput, IndexOutput, Lock};
use crate::error::{LuceneError, Result};
use crate::store::exceptions::LockObtainFailedException;

/// Pass this as the lock wait timeout to try forever to obtain the lock.
///
/// Equivalent to `SleepingLockWrapper.LOCK_OBTAIN_WAIT_FOREVER`.
pub const LOCK_OBTAIN_WAIT_FOREVER: i64 = -1;

/// How long [`SleepingLockWrapper::obtain_lock`] waits, in milliseconds,
/// between attempts to acquire the lock.
///
/// Equivalent to `SleepingLockWrapper.DEFAULT_POLL_INTERVAL`.
pub const DEFAULT_POLL_INTERVAL: i64 = 1000;

/// A [`Directory`] that wraps another and sleeps and retries if obtaining the
/// lock fails.
///
/// Equivalent to `org.apache.lucene.store.SleepingLockWrapper`. As the Lucene
/// javadoc puts it: *this is not a good idea*. It exists because some
/// filesystems and legacy setups need a grace period before a stale lock
/// disappears.
///
/// # Divergence from Lucene 10.5.0
///
/// Java extends `FilterDirectory`; Rust has no inheritance, so this type
/// contains one and delegates. Java also converts an `InterruptedException`
/// raised by `Thread.sleep` into a `ThreadInterruptedException`;
/// [`std::thread::sleep`] cannot be interrupted, so that branch has no
/// counterpart here.
pub struct SleepingLockWrapper {
    inner: FilterDirectory,
    lock_wait_timeout: i64,
    poll_interval: i64,
}

impl SleepingLockWrapper {
    /// Creates a wrapper polling at [`DEFAULT_POLL_INTERVAL`].
    ///
    /// Equivalent to `SleepingLockWrapper(Directory, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `lock_wait_timeout` is
    /// negative and is not [`LOCK_OBTAIN_WAIT_FOREVER`].
    pub fn new(delegate: Box<dyn Directory>, lock_wait_timeout: i64) -> Result<Self> {
        Self::with_poll_interval(delegate, lock_wait_timeout, DEFAULT_POLL_INTERVAL)
    }

    /// Creates a wrapper with an explicit poll interval, both in milliseconds.
    ///
    /// Equivalent to `SleepingLockWrapper(Directory, long, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `lock_wait_timeout` is
    /// negative and is not [`LOCK_OBTAIN_WAIT_FOREVER`], or if `poll_interval`
    /// is negative.
    pub fn with_poll_interval(
        delegate: Box<dyn Directory>,
        lock_wait_timeout: i64,
        poll_interval: i64,
    ) -> Result<Self> {
        if lock_wait_timeout < 0 && lock_wait_timeout != LOCK_OBTAIN_WAIT_FOREVER {
            return Err(LuceneError::IllegalArgument(format!(
                "lockWaitTimeout should be LOCK_OBTAIN_WAIT_FOREVER or a non-negative number (got {lock_wait_timeout})"
            )));
        }
        if poll_interval < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pollInterval must be a non-negative number (got {poll_interval})"
            )));
        }
        Ok(Self {
            inner: FilterDirectory::new(delegate),
            lock_wait_timeout,
            poll_interval,
        })
    }

    /// Returns the wrapped directory.
    ///
    /// Equivalent to `FilterDirectory.getDelegate()`.
    pub fn get_delegate(&self) -> &dyn Directory {
        self.inner.get_delegate()
    }

    /// Returns the configured lock wait timeout in milliseconds.
    pub fn lock_wait_timeout(&self) -> i64 {
        self.lock_wait_timeout
    }

    /// Returns the configured poll interval in milliseconds.
    pub fn poll_interval(&self) -> i64 {
        self.poll_interval
    }
}

impl Directory for SleepingLockWrapper {
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
        self.inner.create_output(name, context)
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

    /// Obtains the lock, retrying every [`poll_interval`](Self::poll_interval)
    /// milliseconds until [`lock_wait_timeout`](Self::lock_wait_timeout)
    /// elapses.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::LockObtainFailed`] once the deadline passes,
    /// carrying the first failure observed as the source. Any error other than
    /// a lock-obtain failure aborts the retry loop immediately, matching Java,
    /// where only `LockObtainFailedException` is caught.
    ///
    /// A `poll_interval` of zero makes Java's `lockWaitTimeout / pollInterval`
    /// throw `ArithmeticException` from this method; this port reports the same
    /// failure at the same point as [`LuceneError::IllegalArgument`] rather
    /// than panicking.
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        let mut failure_reason: Option<String> = None;
        let max_sleep_count = self
            .lock_wait_timeout
            .checked_div(self.poll_interval)
            .ok_or_else(|| {
                LuceneError::IllegalArgument("pollInterval must not be zero: / by zero".to_string())
            })?;
        let mut sleep_count: i64 = 0;

        loop {
            match self.inner.obtain_lock(name) {
                Ok(lock) => return Ok(lock),
                Err(failed) if LockObtainFailedException::is(&failed) => {
                    if failure_reason.is_none() {
                        failure_reason = Some(failed.to_string());
                    }
                }
                Err(other) => return Err(other),
            }

            thread::sleep(Duration::from_millis(self.poll_interval as u64));

            let keep_going =
                sleep_count < max_sleep_count || self.lock_wait_timeout == LOCK_OBTAIN_WAIT_FOREVER;
            sleep_count = sleep_count.saturating_add(1);
            if !keep_going {
                break;
            }
        }

        // We failed to obtain the lock in the required time.
        let reason = format!(
            "Lock obtain timed out: {}: {}",
            self,
            failure_reason.as_deref().unwrap_or("null")
        );
        Err(LockObtainFailedException::with_cause(
            reason.clone(),
            std::io::Error::other(reason),
        ))
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn directory_type_name(&self) -> &'static str {
        "SleepingLockWrapper"
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

impl std::fmt::Display for SleepingLockWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SleepingLockWrapper({})",
            self.inner.get_delegate().directory_type_name()
        )
    }
}

impl std::fmt::Debug for SleepingLockWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SleepingLockWrapper")
            .field("inner", &self.inner.get_delegate().directory_type_name())
            .field("lock_wait_timeout", &self.lock_wait_timeout)
            .field("poll_interval", &self.poll_interval)
            .finish()
    }
}
