//! Port of `com.carrotsearch.randomizedtesting.StackTraceHolder`
//! (randomizedtesting 2.8.1, `StackTraceHolder.java`).

use std::backtrace::Backtrace;
use std::fmt;
use std::sync::Arc;

/// Never thrown: carries a chained stack trace for information
/// (`StackTraceHolder extends Throwable`, package-private).
///
/// **Divergence.** Java sets the holder's stack to an
/// arbitrary `StackTraceElement[]`; the port carries a captured
/// `std::backtrace::Backtrace`, which is empty unless `RUST_BACKTRACE` (or
/// `RUST_LIB_BACKTRACE`) enables capture.
#[derive(Debug, Clone)]
pub(crate) struct StackTraceHolder {
    message: String,
    stack: Arc<Backtrace>,
}

impl StackTraceHolder {
    /// `new StackTraceHolder(message)` followed by `setStackTrace(stack)`.
    pub(crate) fn new(message: String, stack: Arc<Backtrace>) -> StackTraceHolder {
        StackTraceHolder { message, stack }
    }
}

impl fmt::Display for StackTraceHolder {
    /// `Throwable.toString()`, then the stack when one was captured.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "com.carrotsearch.randomizedtesting.StackTraceHolder: {}",
            self.message
        )?;
        if self.stack.status() == std::backtrace::BacktraceStatus::Captured {
            write!(f, "\n{}", self.stack)?;
        }
        Ok(())
    }
}
