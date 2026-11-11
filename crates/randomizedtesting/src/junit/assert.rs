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

    /// `assertEquals(message, expected, actual)` for non-`String` objects
    /// either of which may be `null` (`None`): equal when both are `null` or
    /// both are present and equal; a `null` side prints as `null`.
    pub fn assert_equals_nullable_msg<T: PartialEq + Display>(
        message: Option<&str>,
        expected: Option<T>,
        actual: Option<T>,
    ) {
        if expected != actual {
            let show = |v: &Option<T>| v.as_ref().map_or_else(|| "null".to_string(), T::to_string);
            let prefix = match message {
                Some(m) if !m.is_empty() => format!("{m} "),
                _ => String::new(),
            };
            Assert::fail(&format!(
                "{prefix}expected:<{}> but was:<{}>",
                show(&expected),
                show(&actual)
            ))
        }
    }

    /// `assertEquals(expected, actual)` for non-`String` objects either of
    /// which may be `null` (`None`).
    pub fn assert_equals_nullable<T: PartialEq + Display>(expected: Option<T>, actual: Option<T>) {
        Assert::assert_equals_nullable_msg(None, expected, actual)
    }

    /// `assertEquals(message, expected, actual)` for two `String`s (`None`
    /// standing for `null`): when they differ, JUnit throws
    /// `org.junit.ComparisonFailure`, whose message brackets the differing
    /// middle of each string between their shared prefix and suffix, each
    /// shortened to 20 characters of context behind `...`
    /// (`ComparisonFailure.ComparisonCompactor`), as in `term 0
    /// expected:<[foo]> but was:<[bar]>`.
    ///
    /// The strings are compared as UTF-16 code units, as Java's `charAt`
    /// does; a bracket that would split a surrogate pair renders the lone
    /// half as U+FFFD, which a Rust `String` cannot hold.
    pub fn assert_equals_str(message: Option<&str>, expected: Option<&str>, actual: Option<&str>) {
        if expected == actual {
            return;
        }
        let message = message.unwrap_or("");
        let (expected, actual) = match (expected, actual) {
            (Some(e), Some(a)) => (e, a),
            (e, a) => {
                // `ComparisonFailure` with a null side: `Assert.format` of
                // the plain values.
                let show = |v: Option<&str>| v.unwrap_or("null").to_string();
                Throwable::new(
                    COMPARISON_FAILURE,
                    format_values(message, &show(e), &show(a)),
                )
                .throw()
            }
        };
        let e: Vec<u16> = expected.encode_utf16().collect();
        let a: Vec<u16> = actual.encode_utf16().collect();
        let end = e.len().min(a.len());
        let prefix_len = (0..end).find(|&i| e[i] != a[i]).unwrap_or(end);
        let max_suffix = e.len().min(a.len()) as isize - prefix_len as isize - 1;
        let mut suffix_len = 0usize;
        while (suffix_len as isize) <= max_suffix
            && e[e.len() - 1 - suffix_len] == a[a.len() - 1 - suffix_len]
        {
            suffix_len += 1;
        }
        let s = |units: &[u16]| String::from_utf16_lossy(units);
        let prefix = &e[..prefix_len];
        let suffix = &e[e.len() - suffix_len..];
        let compact_prefix = if prefix.len() <= MAX_CONTEXT_LENGTH {
            s(prefix)
        } else {
            format!("...{}", s(&prefix[prefix.len() - MAX_CONTEXT_LENGTH..]))
        };
        let compact_suffix = if suffix.len() <= MAX_CONTEXT_LENGTH {
            s(suffix)
        } else {
            format!("{}...", s(&suffix[..MAX_CONTEXT_LENGTH]))
        };
        let diff = |units: &[u16]| format!("[{}]", s(&units[prefix_len..units.len() - suffix_len]));
        Throwable::new(
            COMPARISON_FAILURE,
            format_values(
                message,
                &format!("{compact_prefix}{}{compact_suffix}", diff(&e)),
                &format!("{compact_prefix}{}{compact_suffix}", diff(&a)),
            ),
        )
        .throw()
    }
}

/// `org.junit.ComparisonFailure`.
pub const COMPARISON_FAILURE: &str = "org.junit.ComparisonFailure";

/// `ComparisonFailure.MAX_CONTEXT_LENGTH`.
const MAX_CONTEXT_LENGTH: usize = 20;

/// `Assert.format(message, expected, actual)` for two different strings.
fn format_values(message: &str, expected: &str, actual: &str) -> String {
    let prefix = if message.is_empty() {
        String::new()
    } else {
        format!("{message} ")
    };
    format!("{prefix}expected:<{expected}> but was:<{actual}>")
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
