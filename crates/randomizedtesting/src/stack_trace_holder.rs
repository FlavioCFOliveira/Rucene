//! Port of `com.carrotsearch.randomizedtesting.StackTraceHolder`
//! (randomizedtesting 2.8.1, `StackTraceHolder.java`).

use crate::jdk::exception::{StackTraceElement, Throwable};

/// Never thrown: carries a chained stack trace for information
/// (`StackTraceHolder extends Throwable`, package-private). The port's
/// throwables are [`Throwable`] values, so this type only names the class
/// and builds the value.
///
/// The stack trace it carries is the one the caller sets
/// (`setStackTrace`); a thread's stack trace is empty in the port (see
/// [`crate::jdk::thread`]).
pub(crate) struct StackTraceHolder;

impl StackTraceHolder {
    /// The binary name of `StackTraceHolder`.
    pub(crate) const CLASS: &'static str = "com.carrotsearch.randomizedtesting.StackTraceHolder";

    /// `new StackTraceHolder(message)` followed by `setStackTrace(stack)`.
    #[allow(clippy::new_ret_no_self)] // Java's constructor; the port's throwables are `Throwable` values.
    pub(crate) fn new(message: impl Into<String>, stack: Vec<StackTraceElement>) -> Throwable {
        let mut t = Throwable::new(Self::CLASS, message);
        t.set_stack_trace(stack);
        t
    }
}
