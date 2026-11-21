//! Port of `com.carrotsearch.randomizedtesting.UncaughtExceptionError`
//! (randomizedtesting 2.8.1, `UncaughtExceptionError.java`).

use crate::jdk::exception::Throwable;

/// Port of `com.carrotsearch.randomizedtesting.UncaughtExceptionError`.
///
/// Thrown on uncaught exceptions during suite or test execution:
/// `final class UncaughtExceptionError extends Error` (package-private),
/// which `ThreadLeakControl` reports with the uncaught throwable as its
/// cause. A Java throwable is a value of the crate's [`Throwable`]; this type
/// names its class and builds it.
pub(crate) struct UncaughtExceptionError;

impl UncaughtExceptionError {
    /// The binary class name,
    /// `com.carrotsearch.randomizedtesting.UncaughtExceptionError`.
    pub(crate) const CLASS: &'static str =
        "com.carrotsearch.randomizedtesting.UncaughtExceptionError";

    /// `new UncaughtExceptionError(message, cause)`.
    #[allow(clippy::new_ret_no_self)] // Java's constructor; the port's throwables are `Throwable` values.
    pub(crate) fn new(message: impl Into<String>, cause: Throwable) -> Throwable {
        Throwable::with_cause(Self::CLASS, Some(message.into()), cause)
    }
}
