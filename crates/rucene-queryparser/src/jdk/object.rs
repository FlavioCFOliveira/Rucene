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

//! `java.lang.Object` as an argument of `java.text.MessageFormat` and of the
//! flexible query parser's `MessageImpl`: `null`, a `String`, a `Number`, or
//! any other object.
//!
//! **Divergence.** An emulation, not a port of a Lucene file. Java passes an
//! `Object[]`; MessageFormat (JDK 25 `MessageFormat.subformat`) formats a
//! `Number` with the locale's `NumberFormat`, a `Date` with a date-time
//! format, a `String` as is and any other object as its `toString()`. The
//! port keeps the classes that change the outcome and that Lucene passes: a
//! [`JavaObject::String`] (a `String` or any other `CharSequence`, whose
//! `toString()` is its characters), a [`JavaObject::Number`], and a
//! [`JavaObject::Other`] holding the `toString()` of any other object (a
//! `Character`, a `Class`, …), taken when the argument is built; a `Date`
//! argument, which Lucene never passes, has no variant.

use std::fmt;

use super::char_sequence::CharSequence;
use super::class::JavaClass;
use super::format::JavaNumber;

/// A `java.lang.Object` reference passed as a message argument.
#[derive(Clone, Debug, PartialEq)]
pub enum JavaObject {
    /// `null`.
    Null,
    /// A `java.lang.String` (or another `CharSequence`, by its characters).
    String(String),
    /// A `java.lang.Number`.
    Number(JavaNumber),
    /// Any other object, by the result of its `toString()`.
    Other(String),
}

impl JavaObject {
    /// `String.valueOf(Object)`: `"null"`, the string, the number as
    /// `Integer.toString` (and its kin) renders it, or the object's
    /// `toString()`.
    pub fn to_java_string(&self) -> String {
        match self {
            JavaObject::Null => "null".to_string(),
            JavaObject::String(s) | JavaObject::Other(s) => s.clone(),
            JavaObject::Number(n) => number_to_string(n),
        }
    }

    /// A `java.lang.Character` argument.
    pub fn character(c: u16) -> Self {
        JavaObject::Other(String::from_utf16_lossy(&[c]))
    }

    /// A `java.lang.Class` argument.
    pub fn class(class: &JavaClass) -> Self {
        JavaObject::Other(class.to_java_string())
    }
}

/// `toString()` of a boxed number: `Integer.toString`, `Long.toString`,
/// `Float.toString` or `Double.toString`.
pub fn number_to_string(n: &JavaNumber) -> String {
    match *n {
        JavaNumber::Integer(v) => v.to_string(),
        JavaNumber::Long(v) => v.to_string(),
        JavaNumber::Float(v) => rucene::util::java_fmt::float_to_string(v),
        JavaNumber::Double(v) => rucene::util::java_fmt::double_to_string(v),
    }
}

impl fmt::Display for JavaObject {
    /// `String.valueOf(Object)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

impl From<&str> for JavaObject {
    fn from(s: &str) -> Self {
        JavaObject::String(s.to_string())
    }
}

impl From<String> for JavaObject {
    fn from(s: String) -> Self {
        JavaObject::String(s)
    }
}

impl From<&CharSequence> for JavaObject {
    fn from(s: &CharSequence) -> Self {
        JavaObject::String(s.to_string())
    }
}

impl From<CharSequence> for JavaObject {
    fn from(s: CharSequence) -> Self {
        JavaObject::String(s.to_string())
    }
}

impl From<Option<&CharSequence>> for JavaObject {
    fn from(s: Option<&CharSequence>) -> Self {
        match s {
            Some(s) => JavaObject::String(s.to_string()),
            None => JavaObject::Null,
        }
    }
}

impl From<Option<String>> for JavaObject {
    fn from(s: Option<String>) -> Self {
        match s {
            Some(s) => JavaObject::String(s),
            None => JavaObject::Null,
        }
    }
}

impl From<i32> for JavaObject {
    fn from(v: i32) -> Self {
        JavaObject::Number(JavaNumber::Integer(v))
    }
}

impl From<i64> for JavaObject {
    fn from(v: i64) -> Self {
        JavaObject::Number(JavaNumber::Long(v))
    }
}

impl From<f32> for JavaObject {
    fn from(v: f32) -> Self {
        JavaObject::Number(JavaNumber::Float(v))
    }
}

impl From<f64> for JavaObject {
    fn from(v: f64) -> Self {
        JavaObject::Number(JavaNumber::Double(v))
    }
}

impl From<JavaNumber> for JavaObject {
    fn from(v: JavaNumber) -> Self {
        JavaObject::Number(v)
    }
}
