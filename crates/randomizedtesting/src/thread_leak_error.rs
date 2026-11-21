//! Port of `com.carrotsearch.randomizedtesting.ThreadLeakError`
//! (randomizedtesting 2.8.1, `ThreadLeakError.java`).

use crate::jdk::exception::Throwable;

/// Port of `com.carrotsearch.randomizedtesting.ThreadLeakError`.
///
/// "A thread went wild": `final class ThreadLeakError extends Error`
/// (package-private), which `ThreadLeakControl` reports for leaked and
/// zombie threads. A Java throwable is a value of the crate's
/// [`Throwable`]; this type names its class and builds it.
pub(crate) struct ThreadLeakError;

impl ThreadLeakError {
    /// The binary class name, `com.carrotsearch.randomizedtesting.ThreadLeakError`.
    pub(crate) const CLASS: &'static str = "com.carrotsearch.randomizedtesting.ThreadLeakError";

    /// `new ThreadLeakError(message)`.
    #[allow(clippy::new_ret_no_self)] // Java's constructor; the port's throwables are `Throwable` values.
    pub(crate) fn new(message: impl Into<String>) -> Throwable {
        Throwable::new(Self::CLASS, message)
    }
}
