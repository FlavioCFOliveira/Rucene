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

//! Reproductions of the JDK behaviours the port relies on, shared by the
//! modules of the crate.

use std::sync::Arc;

use rucene::error::LuceneError;
use spatial4j::shape::shape::Shape;

/// `Double.doubleToLongBits(double)`.
pub(crate) fn double_to_long_bits(v: f64) -> i64 {
    if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        v.to_bits() as i64
    }
}

/// `Double.hashCode(double)`: `(int) (bits ^ (bits >>> 32))` of
/// `doubleToLongBits`.
pub(crate) fn hash_double(v: f64) -> i32 {
    let bits = double_to_long_bits(v);
    (bits ^ ((bits as u64) >> 32) as i64) as i32
}

/// `Double.compare(a, b)` as an `int`.
pub(crate) fn double_compare(a: f64, b: f64) -> i32 {
    match rucene::util::java_double_compare(a, b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `String.valueOf(double)`.
pub(crate) fn d2s(v: f64) -> String {
    rucene::util::java_fmt::double_to_string(v)
}

/// `String.valueOf(float)`.
pub(crate) fn f2s(v: f32) -> String {
    rucene::util::java_fmt::float_to_string(v)
}

/// `String.hashCode()`.
pub(crate) fn string_hash(s: &str) -> i32 {
    rucene::util::java_collections::java_string_hash_code(s)
}

/// The hash code of an object that does not override `Object.hashCode()`:
/// the identity-hash stand-in of the core (`String.hashCode()` of its Java
/// class name), consistent with the identity `equals` the callers apply.
pub(crate) fn identity_hash(class_name: &str) -> i32 {
    rucene::util::java_collections::java_identity_hash_code_stand_in(class_name)
}

/// `new UnsupportedOperationException()`, whose message is `null` (the
/// core's `UnsupportedOperation` kind carries an empty message for it).
pub(crate) fn unsupported() -> LuceneError {
    LuceneError::unsupported_operation(String::new())
}

/// The `UnsupportedOperationException` a method without a fallible Rust
/// signature throws in Java (`getBoundingBox`, `getCenter`, `getArea`): the
/// port panics with the exception's text, as the spatial4j port does for the
/// shape methods it keeps infallible.
#[track_caller]
pub(crate) fn panic_unsupported() -> ! {
    panic!("java.lang.UnsupportedOperationException")
}

/// `Objects.equals(a, b)` over two optional shapes.
pub(crate) fn shapes_equal(a: Option<&Arc<dyn Shape>>, b: Option<&Arc<dyn Shape>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.equals(&**b),
        _ => false,
    }
}

/// `getClass().getSimpleName()` of a Java binary class name.
pub(crate) fn simple_name(class_name: &str) -> &str {
    let simple = class_name.rsplit('.').next().unwrap_or(class_name);
    simple.rsplit('$').next().unwrap_or(simple)
}

/// The data pointer of a trait object, for Java's `==` reference identity.
pub(crate) fn addr<T: ?Sized>(r: &T) -> *const () {
    r as *const T as *const ()
}

/// `Double.parseDouble(String)` / `Double.valueOf(String)`, with Java's
/// `NumberFormatException`.
pub(crate) fn parse_double(s: &str) -> rucene::error::Result<f64> {
    if s.trim_matches(|c: char| c <= ' ').is_empty() {
        return Err(LuceneError::number_format("empty String"));
    }
    rucene::util::java_fmt::parse_double(s)
        .ok_or_else(|| LuceneError::number_format(format!("For input string: \"{s}\"")))
}

/// `Integer.parseInt(String)` / `Integer.valueOf(String)`: an optional sign
/// then decimal digits (`Character.digit(c, 10)`, which accepts every
/// Unicode `Nd` digit), with Java's `NumberFormatException`.
pub(crate) fn parse_int(s: &str) -> rucene::error::Result<i32> {
    let err = || {
        if s.is_empty() {
            LuceneError::number_format("For input string: \"\"")
        } else {
            LuceneError::number_format_for_input_string(s, 10)
        }
    };
    let mut chars = s.chars().peekable();
    let negative = match chars.peek() {
        Some('-') => {
            chars.next();
            true
        }
        Some('+') => {
            chars.next();
            false
        }
        _ => false,
    };
    let mut result: i64 = 0;
    let mut any = false;
    for c in chars {
        let d = rucene::util::java_character_digit(c, 10).ok_or_else(err)? as i64;
        result = result * 10 + d;
        if result > i64::from(i32::MAX) + 1 {
            return Err(err());
        }
        any = true;
    }
    if !any {
        return Err(err());
    }
    let v = if negative { -result } else { result };
    i32::try_from(v).map_err(|_| err())
}

/// `Boolean.parseBoolean(String)`.
pub(crate) fn parse_boolean(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

/// A `double` field that can be reassigned through a shared reference, as
/// the setters of a shared Java object do. Plain (relaxed) loads and stores.
#[derive(Debug, Default)]
pub(crate) struct AtomicF64(std::sync::atomic::AtomicU64);

impl AtomicF64 {
    pub(crate) fn new(v: f64) -> Self {
        Self(std::sync::atomic::AtomicU64::new(v.to_bits()))
    }

    pub(crate) fn get(&self) -> f64 {
        f64::from_bits(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub(crate) fn set(&self, v: f64) {
        self.0
            .store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

/// `Math.round(double)`: the closest `long`, ties rounding up; `NaN` is 0
/// and out-of-range values saturate.
pub(crate) fn java_round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    let f = a.floor();
    let r = if a - f >= 0.5 { f + 1.0 } else { f };
    r as i64
}
