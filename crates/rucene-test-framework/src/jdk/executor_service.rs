//! The part of `java.util.concurrent.ExecutorService` that
//! `TestUtil.shutdownExecutorService` calls: `shutdown()` and
//! `awaitTermination(long, TimeUnit)`.
//!
//! **Divergence.** `ExecutorService` is a JDK interface; the port has no
//! common trait for its executors, so this one names the two methods, and is
//! implemented for the port's public executor service,
//! [`SameThreadExecutorService`]. An executor of a test implements it to be
//! shut down through `TestUtil`.

use std::time::Duration;

use rucene::util::concurrent::SameThreadExecutorService;
use rucene::util::InterruptedException;

/// `java.util.concurrent.ExecutorService`, the two methods
/// `TestUtil.shutdownExecutorService` calls.
pub trait ExecutorService {
    /// `shutdown()`: previously submitted tasks run, no new task is
    /// accepted.
    fn shutdown(&self);

    /// `awaitTermination(timeout, unit)`: blocks until every task completed
    /// after a shutdown, the timeout elapses or the current thread is
    /// interrupted; whether the executor terminated.
    ///
    /// # Errors
    ///
    /// `InterruptedException` when the waiting thread is interrupted.
    fn await_termination(&self, timeout: Duration) -> Result<bool, InterruptedException>;
}

impl ExecutorService for SameThreadExecutorService {
    fn shutdown(&self) {
        SameThreadExecutorService::shutdown(self);
    }

    /// `SameThreadExecutorService.awaitTermination`: `true` at once.
    fn await_termination(&self, timeout: Duration) -> Result<bool, InterruptedException> {
        Ok(SameThreadExecutorService::await_termination(self, timeout))
    }
}
