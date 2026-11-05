//! `org.junit.Assert` and `org.junit.Assume`, the members the runner's
//! users call, rebuilt from their documented behaviour.

use std::fmt::Display;

use super::statement::ASSUMPTION_VIOLATED;
use crate::jdk::exception::{Throwable, ASSERTION_ERROR};

/// `org.junit.Assert`.
pub struct Assert;

impl Assert {
    /// `fail(message)`: throws `AssertionError(message)`.
    pub fn fail(message: &str) -> ! {
        Throwable::new(ASSERTION_ERROR, message).throw()
    }

    /// `fail()`: throws `AssertionError` without a message.
    pub fn fail_without_message() -> ! {
        Throwable::with_message(ASSERTION_ERROR, None).throw()
    }

    /// `assertTrue(message, condition)`.
    pub fn assert_true_msg(message: &str, condition: bool) {
        if !condition {
            Assert::fail(message)
        }
    }

    /// `assertTrue(condition)`.
    pub fn assert_true(condition: bool) {
        if !condition {
            Assert::fail_without_message()
        }
    }

    /// `assertFalse(condition)`.
    pub fn assert_false(condition: bool) {
        Assert::assert_true(!condition)
    }

    /// `assertFalse(message, condition)`.
    pub fn assert_false_msg(message: &str, condition: bool) {
        Assert::assert_true_msg(message, !condition)
    }

    /// `assertEquals(message, float expected, float actual, float delta)`:
    /// fails unless `Float.compare(expected, actual) == 0` or
    /// `Math.abs(expected - actual) <= delta`, with
    /// `message expected:<x> but was:<y>` (`Float.toString`).
    pub fn assert_equals_float_msg(message: Option<&str>, expected: f32, actual: f32, delta: f32) {
        let same = expected.to_bits() == actual.to_bits()
            || (expected.is_nan() && actual.is_nan())
            || (expected - actual).abs() <= delta;
        if !same {
            Assert::fail_not_equals(
                message,
                &crate::jdk::lang::float_to_string(expected),
                &crate::jdk::lang::float_to_string(actual),
            )
        }
    }

    /// `assertEquals(message, double expected, double actual, double delta)`:
    /// as [`assert_equals_float_msg`](Self::assert_equals_float_msg) over
    /// doubles (`Double.compare`, `Double.toString`).
    pub fn assert_equals_double_msg(message: Option<&str>, expected: f64, actual: f64, delta: f64) {
        let same = expected.to_bits() == actual.to_bits()
            || (expected.is_nan() && actual.is_nan())
            || (expected - actual).abs() <= delta;
        if !same {
            Assert::fail_not_equals(
                message,
                &crate::jdk::lang::double_to_string(expected),
                &crate::jdk::lang::double_to_string(actual),
            )
        }
    }

    /// The private `failNotEquals(message, expected, actual)` over the two
    /// values' strings, which differ for every caller of this module.
    fn fail_not_equals(message: Option<&str>, expected: &str, actual: &str) -> ! {
        let prefix = match message {
            Some(m) if !m.is_empty() => format!("{m} "),
            _ => String::new(),
        };
        Assert::fail(&format!("{prefix}expected:<{expected}> but was:<{actual}>"))
    }

    /// `assertEquals(message, expected, actual)` for non-`String` objects:
    /// `message expected:<x> but was:<y>`.
    pub fn assert_equals_msg<T: PartialEq + Display>(
        message: Option<&str>,
        expected: T,
        actual: T,
    ) {
        if expected != actual {
            let prefix = match message {
                Some(m) if !m.is_empty() => format!("{m} "),
                _ => String::new(),
            };
            Assert::fail(&format!("{prefix}expected:<{expected}> but was:<{actual}>"))
        }
    }

    /// `assertEquals(expected, actual)` for non-`String` objects.
    pub fn assert_equals<T: PartialEq + Display>(expected: T, actual: T) {
        Assert::assert_equals_msg(None, expected, actual)
    }
}

/// `org.junit.Assume`.
pub struct Assume;

impl Assume {
    /// `assumeTrue(boolean b)`: `AssumptionViolatedException` (`got:
    /// <false>, expected: is <true>`) when `b` is false.
    pub fn assume_true(b: bool) {
        if !b {
            Throwable::new(ASSUMPTION_VIOLATED, "got: <false>, expected: is <true>").throw()
        }
    }

    /// `assumeTrue(String message, boolean b)`.
    pub fn assume_true_msg(message: &str, b: bool) {
        if !b {
            Throwable::new(ASSUMPTION_VIOLATED, message).throw()
        }
    }

    /// `assumeFalse(boolean b)`: `got: <true>, expected: is <false>`.
    pub fn assume_false(b: bool) {
        if b {
            Throwable::new(ASSUMPTION_VIOLATED, "got: <true>, expected: is <false>").throw()
        }
    }

    /// `throw new AssumptionViolatedException(assumption)`.
    pub fn violated(assumption: &str) -> ! {
        Throwable::new(ASSUMPTION_VIOLATED, assumption).throw()
    }
}
