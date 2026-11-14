/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The Java semantics the code generated from Lucene's JavaCC parsers relies
//! on: object references, `null` checks, and the `java.lang.String`
//! operations the grammars call, over UTF-16 code units.
//!
//! **Divergence.** Not a port of a Lucene file: the support of
//! `tools/translate/javacc_java_to_rust.py`, which translates the generated
//! Java of `classic.QueryParser`, `surround.parser.QueryParser` and their
//! token managers. Each function states the Java operation it stands for.
//! Rust strings cannot hold an unpaired surrogate: where a Java operation
//! would split a surrogate pair (`substring` inside a pair), the half is
//! replaced by U+FFFD.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::error::{LuceneError, LuceneErrorKind, Result};

/// A Java object reference to a mutable value: shared, compared by identity,
/// and rewritten in place by whoever holds it.
pub struct Shared<T>(Arc<Mutex<T>>);

impl<T> Shared<T> {
    /// `new T(...)`: a fresh object.
    pub fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }

    /// The object, for reading or writing its fields.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads the object: `f` sees it while it is locked, and the lock is
    /// released before this returns.
    pub fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.lock())
    }

    /// Java's `==` on two references.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }
}

impl<T> Clone for Shared<T> {
    /// Copies the reference, not the object.
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.try_lock() {
            Ok(value) => value.fmt(f),
            Err(_) => f.write_str("Shared(<locked>)"),
        }
    }
}

/// Java's `==` on two nullable references.
pub fn opt_ptr_eq<T>(a: &Option<Shared<T>>, b: &Option<Shared<T>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Shared::ptr_eq(a, b),
        _ => false,
    }
}

/// A dereference of a reference: the `NullPointerException` the JVM throws
/// for `null`.
pub fn nn<T>(value: Option<&T>) -> Result<&T> {
    value.ok_or_else(LuceneError::null_pointer)
}

/// A dereference of a reference for writing: the `NullPointerException` the
/// JVM throws for `null`.
pub fn nn_mut<T>(value: Option<&mut T>) -> Result<&mut T> {
    value.ok_or_else(LuceneError::null_pointer)
}

/// A dereference of a `String` reference: the `NullPointerException` the
/// JVM throws for `null`.
pub fn nn_str(value: Option<&str>) -> Result<&str> {
    value.ok_or_else(LuceneError::null_pointer)
}

/// A non-null value taken from a nullable one: the `NullPointerException`
/// the JVM throws for `null`.
pub fn nn_owned<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(LuceneError::null_pointer)
}

/// Java's string concatenation of the string conversions `parts`.
pub fn concat(parts: &[&str]) -> String {
    parts.concat()
}

/// `String.valueOf(Object)` of a `String`: the string, or `"null"`.
pub fn string_value_of(value: Option<&str>) -> String {
    value.unwrap_or("null").to_string()
}

/// `String.length()`: the number of UTF-16 code units.
pub fn length(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

/// `String.charAt(int)`.
///
/// # Errors
///
/// The `StringIndexOutOfBoundsException` of an index outside the string.
pub fn char_at(s: &str, index: i32) -> Result<u16> {
    let units: Vec<u16> = s.encode_utf16().collect();
    if index < 0 || index as usize >= units.len() {
        return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
            format!("Index {index} out of bounds for length {}", units.len()),
        )));
    }
    Ok(units[index as usize])
}

/// `String.substring(int, int)`, over UTF-16 code units.
///
/// # Errors
///
/// The `StringIndexOutOfBoundsException` of `String.checkBoundsBeginEnd`.
pub fn substring(s: &str, begin: i32, end: i32) -> Result<String> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let length = units.len() as i32;
    if begin < 0 || begin > end || end > length {
        return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
            format!("begin {begin}, end {end}, length {length}"),
        )));
    }
    Ok(String::from_utf16_lossy(
        &units[begin as usize..end as usize],
    ))
}

/// `String.substring(int)`, over UTF-16 code units.
///
/// # Errors
///
/// The `StringIndexOutOfBoundsException` of `String.substring(int)`.
pub fn substring_from(s: &str, begin: i32) -> Result<String> {
    substring(s, begin, length(s))
}

/// `Float.parseFloat(String)`.
///
/// # Errors
///
/// The `NumberFormatException` Java throws: `empty String` for a value that
/// trims to nothing, `For input string: "<s>"` otherwise.
pub fn parse_float(s: &str) -> Result<f32> {
    if s.trim_matches(|c: char| c <= '\u{20}').is_empty() {
        return Err(LuceneError::number_format("empty String"));
    }
    rucene::util::java_fmt::parse_float(s)
        .ok_or_else(|| LuceneError::number_format(format!("For input string: \"{s}\"")))
}

/// `Integer.parseInt(String)`.
///
/// # Errors
///
/// The `NumberFormatException` of `NumberFormatException.forInputString`.
pub fn parse_int(s: &str) -> Result<i32> {
    rucene::util::extra::java_parse_int(s)
        .ok_or_else(|| LuceneError::number_format_for_input_string(s, 10))
}

/// `new java.lang.Error(message)`.
pub fn java_error(message: String) -> LuceneError {
    LuceneError::unchecked("java.lang.Error", message)
}

/// The `java.lang.Error` subclasses the port carries as a
/// [`LuceneErrorKind::Unchecked`] class name.
const ERRORS: &[&str] = &[
    "java.lang.Error",
    "java.lang.VirtualMachineError",
    "java.lang.OutOfMemoryError",
    "java.lang.StackOverflowError",
    "java.lang.InternalError",
    "java.lang.UnknownError",
    "java.lang.LinkageError",
    "java.lang.ThreadDeath",
];

/// Whether `catch (Exception e)` catches `error`: every throwable but a
/// `java.lang.Error` (an `AssertionError`, a JDK `Error`, or the token
/// managers' `TokenMgrError`).
pub fn is_exception(error: &LuceneError) -> bool {
    match error.kind() {
        LuceneErrorKind::AssertionError(_) => false,
        LuceneErrorKind::Unchecked { class, .. } => !ERRORS.contains(class),
        LuceneErrorKind::Throwable(t) => !t.java_class().ends_with(".TokenMgrError"),
        _ => true,
    }
}

/// `addEscapes(String)` of the JavaCC 7 `TokenMgrError` and `add_escapes`
/// of its `ParseException`: the escapes of a Java string literal for the
/// UTF-16 code units below U+0020 or above U+007E, `\uXXXX` in lower-case
/// hexadecimal for those without a short escape.
pub fn add_escapes(s: &str) -> String {
    let mut out = String::new();
    for unit in s.encode_utf16() {
        match unit {
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0A => out.push_str("\\n"),
            0x0C => out.push_str("\\f"),
            0x0D => out.push_str("\\r"),
            0x22 => out.push_str("\\\""),
            0x27 => out.push_str("\\'"),
            0x5C => out.push_str("\\\\"),
            u if !(0x20..=0x7e).contains(&u) => out.push_str(&format!("\\u{u:04x}")),
            u => out.push(char::from(u as u8)),
        }
    }
    out
}
