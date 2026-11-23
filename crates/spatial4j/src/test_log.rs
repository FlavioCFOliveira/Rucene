/*******************************************************************************
 * Copyright (c) 2015 MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `TestLog.java`, of the Java package
//! `org.locationtech.spatial4j` (spatial4j 0.8 test sources, built with the
//! `test-helpers` feature).

use std::fmt::Display;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::system;
use randomizedtesting::rules::{TestRuleAdapter, TestRuleAdapterRule};

/// A log argument: Java's `Object`, rendered by its `toString()` when the
/// entry is printed.
pub type LogArg = Arc<dyn Display + Send + Sync>;

/// `logStack` (`private static ArrayList<LogEntry>`). Java's comment asks
/// whether it needs to be thread-safe; a Rust static must be, so it sits in
/// a [`Mutex`].
static LOG_STACK: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());

/// `MAX_LOGS`.
const MAX_LOGS: usize = 1000;

fn log_stack() -> std::sync::MutexGuard<'static, Vec<LogEntry>> {
    LOG_STACK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Equivalent to `org.locationtech.spatial4j.TestLog` (`public class TestLog
/// extends TestRuleAdapter`): a utility for logging that keeps the log
/// statements of a test and prints them only if the test fails.
pub struct TestLog {
    _private: (),
}

impl TestLog {
    /// `instance` (`public static final TestLog`).
    pub fn instance() -> Arc<TestLog> {
        static INSTANCE: std::sync::LazyLock<Arc<TestLog>> =
            std::sync::LazyLock::new(|| Arc::new(TestLog { _private: () }));
        Arc::clone(&INSTANCE)
    }

    /// [`instance`](Self::instance) as the `TestRule` a `@Rule` field holds.
    pub fn rule() -> TestRuleAdapterRule<TestLog> {
        TestRuleAdapterRule(Self::instance())
    }

    /// `logThenClear()`.
    fn log_then_clear() {
        let mut stack = log_stack();
        for entry in stack.iter() {
            system::out_println(&array_format(&entry.msg, &entry.args));
        }
        stack.clear();
    }

    /// `clear()`.
    pub fn clear() {
        log_stack().clear();
    }

    /// `log(String msg, Object... args)`: enqueues a log message with
    /// substitution arguments ala SLF4J (i.e. `{}` syntax). If the test fails
    /// then it'll be logged then, otherwise it'll be forgotten.
    ///
    /// # Panics
    ///
    /// Java's `RuntimeException` (`Too many log statements: …`) when more
    /// than `MAX_LOGS` entries are queued.
    pub fn log(msg: &str, args: Vec<LogArg>) {
        let mut stack = log_stack();
        if stack.len() > MAX_LOGS {
            let message = format!("Too many log statements: {} > {}", stack.len(), MAX_LOGS);
            drop(stack);
            Throwable::new("java.lang.RuntimeException", &message).throw();
        }
        stack.push(LogEntry {
            msg: msg.to_string(),
            args,
        });
    }
}

impl TestRuleAdapter for TestLog {
    /// `before()`.
    fn before(&self) {
        log_stack().clear();
    }

    /// `afterAlways(List<Throwable> errors)`.
    fn after_always(&self, errors: &mut Vec<Throwable>) {
        if !errors.is_empty() {
            TestLog::log_then_clear();
        }
    }
}

/// Equivalent to `org.locationtech.spatial4j.TestLog.LogEntry` (`private
/// static class`).
pub(crate) struct LogEntry {
    msg: String,
    args: Vec<LogArg>,
}

/// SLF4J's `MessageFormatter.arrayFormat(messagePattern, argArray)
/// .getMessage()` (slf4j-api 1.7): each `{}` anchor is replaced by the next
/// argument; `\{}` escapes an anchor and `\\{}` escapes the escape.
fn array_format(message_pattern: &str, arg_array: &[LogArg]) -> String {
    const DELIM: &str = "{}";
    let pattern = message_pattern.as_bytes();
    let mut i = 0usize;
    let mut sbuf = String::with_capacity(message_pattern.len() + 50);
    let mut l = 0usize;
    while l < arg_array.len() {
        let j = match message_pattern[i..].find(DELIM) {
            Some(off) => i + off,
            None => {
                // no more variables
                if i == 0 {
                    // this is a simple string
                    return message_pattern.to_string();
                }
                // add the tail string which contains no variables and return the result.
                sbuf.push_str(&message_pattern[i..]);
                return sbuf;
            }
        };
        let escaped = j >= 1 && pattern[j - 1] == b'\\';
        if escaped {
            let double_escaped = j >= 2 && pattern[j - 2] == b'\\';
            if !double_escaped {
                // escaped: the anchor is literal and the argument is kept
                sbuf.push_str(&message_pattern[i..j - 1]);
                sbuf.push('{');
                i = j + 1;
                continue;
            }
            // The escape character preceding the delimiter start is itself
            // escaped: "abc x:\\{}"; we have to consume one backward slash
            sbuf.push_str(&message_pattern[i..j - 1]);
            sbuf.push_str(&arg_array[l].to_string());
            i = j + 2;
        } else {
            // normal case
            sbuf.push_str(&message_pattern[i..j]);
            sbuf.push_str(&arg_array[l].to_string());
            i = j + 2;
        }
        l += 1;
    }
    // append the characters following the last {} pair.
    sbuf.push_str(&message_pattern[i..]);
    sbuf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_format_substitutes_and_escapes() {
        let a: LogArg = Arc::new("A");
        let b: LogArg = Arc::new(2);
        assert_eq!(
            array_format("x {} y {}", &[a.clone(), b.clone()]),
            "x A y 2"
        );
        assert_eq!(
            array_format("x \\{} y {}", std::slice::from_ref(&a)),
            "x {} y A"
        );
        assert_eq!(array_format("x \\\\{}", std::slice::from_ref(&a)), "x \\A");
        assert_eq!(
            array_format("no anchors", std::slice::from_ref(&a)),
            "no anchors"
        );
        assert_eq!(array_format("{} {} {}", &[a, b]), "A 2 {}");
    }
}
