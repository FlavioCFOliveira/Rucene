//! Java's unchecked exceptions, as Rust panics.
//!
//! randomizedtesting signals misuse with unchecked exceptions
//! (`IllegalArgumentException`, `IllegalStateException`, `RuntimeException`,
//! `AssertionError`), which fail the test that triggered them. A Rust test
//! fails the same way on a panic, so the port panics where Java throws. The
//! panic message is the exception's `Throwable.toString()`: the binary class
//! name, then `": "` and the message when there is one, so both the error
//! class and its message are observable and can be compared with Java's.
//!
//! **Divergence.** Java exceptions are values that a caller can
//! catch by class; a Rust panic is caught with `std::panic::catch_unwind` and
//! carries the class only in its message text.

use std::fmt::Display;

/// `java.lang.IllegalArgumentException`.
pub const ILLEGAL_ARGUMENT_EXCEPTION: &str = "java.lang.IllegalArgumentException";
/// `java.lang.IllegalStateException`.
pub const ILLEGAL_STATE_EXCEPTION: &str = "java.lang.IllegalStateException";
/// `java.lang.RuntimeException`.
pub const RUNTIME_EXCEPTION: &str = "java.lang.RuntimeException";
/// `java.lang.AssertionError`.
pub const ASSERTION_ERROR: &str = "java.lang.AssertionError";
/// `java.lang.NullPointerException`.
pub const NULL_POINTER_EXCEPTION: &str = "java.lang.NullPointerException";
/// `java.lang.NegativeArraySizeException`.
pub const NEGATIVE_ARRAY_SIZE_EXCEPTION: &str = "java.lang.NegativeArraySizeException";
/// `java.lang.ArithmeticException`.
pub const ARITHMETIC_EXCEPTION: &str = "java.lang.ArithmeticException";
/// `java.lang.IllegalThreadStateException`.
pub const ILLEGAL_THREAD_STATE_EXCEPTION: &str = "java.lang.IllegalThreadStateException";

/// Formats an exception as `Throwable.toString()` does: the class name, then
/// `": "` and the message when the message is not `null`.
pub fn to_string(class: &str, message: Option<&dyn Display>) -> String {
    match message {
        Some(m) => format!("{class}: {m}"),
        None => class.to_string(),
    }
}

/// Throws an exception of `class` with `message`: panics with its
/// `Throwable.toString()`.
#[track_caller]
pub fn throw(class: &str, message: impl Display) -> ! {
    panic!("{}", to_string(class, Some(&message)))
}

/// Throws an exception of `class` whose message is `null`.
#[track_caller]
pub fn throw_without_message(class: &str) -> ! {
    panic!("{}", class)
}

/// `throw new IllegalArgumentException(message)`.
#[track_caller]
pub fn illegal_argument(message: impl Display) -> ! {
    throw(ILLEGAL_ARGUMENT_EXCEPTION, message)
}

/// `throw new IllegalStateException(message)`.
#[track_caller]
pub fn illegal_state(message: impl Display) -> ! {
    throw(ILLEGAL_STATE_EXCEPTION, message)
}

/// `throw new RuntimeException(message)`.
#[track_caller]
pub fn runtime(message: impl Display) -> ! {
    throw(RUNTIME_EXCEPTION, message)
}

/// Java's `assert condition : message;` under `-ea`.
///
/// **Divergence.** Java evaluates `assert` only when assertions are
/// enabled (`-ea`, as Lucene's and randomizedtesting's builds run tests); the
/// port evaluates it when `debug_assertions` are on, the Rust counterpart.
#[track_caller]
#[inline]
pub fn java_assert(condition: bool, message: impl FnOnce() -> String) {
    if cfg!(debug_assertions) && !condition {
        throw(ASSERTION_ERROR, message())
    }
}

/// Java's `assert condition;` (no message) under `-ea`; see [`java_assert`].
#[track_caller]
#[inline]
pub fn java_assert_no_message(condition: bool) {
    if cfg!(debug_assertions) && !condition {
        throw_without_message(ASSERTION_ERROR)
    }
}

/// The message of a panic payload, as the tests and the harness read it.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else {
        "<non-string panic payload>".to_string()
    }
}
