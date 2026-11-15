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

//! `java.lang.CharSequence` as the flexible query parser passes it around:
//! a `java.lang.String`, or another class implementing the interface (the
//! parser's own `UnescapedCharSequence`).
//!
//! **Divergence.** An emulation of the JDK 25 interface (`CharSequence` and
//! `String` read as specification), not a port of a Lucene file. Java's
//! interface is the [`CharSequence`] enum: a `String` is held as a Rust
//! string, any other implementation as an [`Arc`] of [`JavaCharSequence`],
//! so that `instanceof` tests ([`CharSequence::as_any`]) and identity
//! (`equals` and `hashCode` of a class that does not override them) are kept.
//! Indexes count UTF-16 code units, as Java's do; a Rust string cannot hold an
//! unpaired surrogate, so a `subSequence` of a `String` that splits a
//! surrogate pair replaces the half by U+FFFD.

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};

/// A class implementing `java.lang.CharSequence` other than `String`.
///
/// Equivalent to the interface's abstract methods, `toString()` and the
/// `instanceof` test of the implementing class.
pub trait JavaCharSequence: fmt::Debug + Send + Sync + 'static {
    /// `length()`: the number of UTF-16 code units.
    fn length(&self) -> usize;

    /// `charAt(int index)`.
    ///
    /// # Errors
    ///
    /// The `IndexOutOfBoundsException` of the class for an index outside the
    /// sequence.
    fn char_at(&self, index: usize) -> Result<u16>;

    /// `subSequence(int start, int end)`.
    ///
    /// # Errors
    ///
    /// The `IndexOutOfBoundsException` of the class for a range outside the
    /// sequence.
    fn sub_sequence(&self, start: usize, end: usize) -> Result<CharSequence>;

    /// `toString()`.
    fn to_java_string(&self) -> String;

    /// The object, for `instanceof` tests.
    fn as_any(&self) -> &dyn Any;
}

/// A `java.lang.CharSequence` reference (never `null`; a nullable one is an
/// [`Option`]).
///
/// Equivalent to `CharSequence`: a [`String`](CharSequence::String) is a
/// `java.lang.String`, an [`Object`](CharSequence::Object) any other class.
/// Cloning copies the reference, not the characters.
#[derive(Clone)]
pub enum CharSequence {
    /// A `java.lang.String`.
    String(Arc<str>),
    /// An object of another class implementing the interface.
    Object(Arc<dyn JavaCharSequence>),
}

impl CharSequence {
    /// `length()`: the number of UTF-16 code units.
    pub fn length(&self) -> usize {
        match self {
            CharSequence::String(s) => s.encode_utf16().count(),
            CharSequence::Object(o) => o.length(),
        }
    }

    /// `isEmpty()`: `length() == 0`.
    pub fn is_empty(&self) -> bool {
        self.length() == 0
    }

    /// `charAt(int index)`.
    ///
    /// # Errors
    ///
    /// `StringIndexOutOfBoundsException: Index <i> out of bounds for length
    /// <n>` for a `String`, as JDK 25's `String.charAt` throws; the class's
    /// own exception otherwise.
    pub fn char_at(&self, index: usize) -> Result<u16> {
        match self {
            CharSequence::String(s) => {
                let length = s.encode_utf16().count();
                s.encode_utf16().nth(index).ok_or_else(|| {
                    LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(format!(
                        "Index {index} out of bounds for length {length}"
                    )))
                })
            }
            CharSequence::Object(o) => o.char_at(index),
        }
    }

    /// `subSequence(int start, int end)`.
    ///
    /// # Errors
    ///
    /// `StringIndexOutOfBoundsException: begin <b>, end <e>, length <n>` for
    /// a `String` (`String.substring`); the class's own exception otherwise.
    pub fn sub_sequence(&self, start: usize, end: usize) -> Result<CharSequence> {
        match self {
            CharSequence::String(s) => {
                let units: Vec<u16> = s.encode_utf16().collect();
                if start > end || end > units.len() {
                    return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
                        format!("begin {start}, end {end}, length {}", units.len()),
                    )));
                }
                Ok(CharSequence::from(String::from_utf16_lossy(
                    &units[start..end],
                )))
            }
            CharSequence::Object(o) => o.sub_sequence(start, end),
        }
    }

    /// The UTF-16 code units of `toString()`.
    pub fn units(&self) -> Vec<u16> {
        match self {
            CharSequence::String(s) => s.encode_utf16().collect(),
            CharSequence::Object(o) => o.to_java_string().encode_utf16().collect(),
        }
    }

    /// The object implementing the interface, for an `instanceof` test of a
    /// class other than `String`; `None` for a `String`.
    pub fn as_any(&self) -> Option<&dyn Any> {
        match self {
            CharSequence::String(_) => None,
            CharSequence::Object(o) => Some(o.as_any()),
        }
    }

    /// `instanceof String`.
    pub fn is_string(&self) -> bool {
        matches!(self, CharSequence::String(_))
    }
}

impl fmt::Display for CharSequence {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CharSequence::String(s) => f.write_str(s),
            CharSequence::Object(o) => f.write_str(&o.to_java_string()),
        }
    }
}

impl fmt::Debug for CharSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CharSequence::String(s) => fmt::Debug::fmt(&**s, f),
            CharSequence::Object(o) => fmt::Debug::fmt(o, f),
        }
    }
}

impl PartialEq for CharSequence {
    /// `equals(Object)`: the characters for two `String`s
    /// (`String.equals`), identity otherwise (`Object.equals`, which the
    /// flexible parser's `CharSequence` classes do not override).
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CharSequence::String(a), CharSequence::String(b)) => a == b,
            (CharSequence::Object(a), CharSequence::Object(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for CharSequence {}

impl Hash for CharSequence {
    /// Consistent with [`PartialEq`]: the characters of a `String`, the
    /// identity of another object.
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            CharSequence::String(s) => {
                0u8.hash(state);
                s.hash(state);
            }
            CharSequence::Object(o) => {
                1u8.hash(state);
                (Arc::as_ptr(o) as *const () as usize).hash(state);
            }
        }
    }
}

impl From<&str> for CharSequence {
    fn from(s: &str) -> Self {
        CharSequence::String(Arc::from(s))
    }
}

impl From<String> for CharSequence {
    fn from(s: String) -> Self {
        CharSequence::String(Arc::from(s))
    }
}

impl From<&String> for CharSequence {
    fn from(s: &String) -> Self {
        CharSequence::String(Arc::from(s.as_str()))
    }
}

impl From<Arc<str>> for CharSequence {
    fn from(s: Arc<str>) -> Self {
        CharSequence::String(s)
    }
}

/// `String.valueOf(Object)` of a nullable `CharSequence`: its `toString()`,
/// or `"null"`.
pub fn value_of(text: Option<&CharSequence>) -> String {
    match text {
        Some(text) => text.to_string(),
        None => "null".to_string(),
    }
}
