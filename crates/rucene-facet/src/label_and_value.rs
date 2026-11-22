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

//! Port of `LabelAndValue.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::fmt;

use rucene::document::NumericValue;
use rucene::util::java_collections::{
    java_double_hash_code, java_float_hash_code, java_string_hash_code,
};

/// `Number.equals(Object)`: the same boxed class and the same value, with
/// `Float.equals`/`Double.equals` comparing bit patterns (so `NaN` equals
/// `NaN` and `0.0` differs from `-0.0`).
pub fn number_equals(a: &NumericValue, b: &NumericValue) -> bool {
    match (a, b) {
        (NumericValue::Int(x), NumericValue::Int(y)) => x == y,
        (NumericValue::Long(x), NumericValue::Long(y)) => x == y,
        (NumericValue::Float(x), NumericValue::Float(y)) => {
            java_float_hash_code(*x) == java_float_hash_code(*y)
        }
        (NumericValue::Double(x), NumericValue::Double(y)) => {
            let bits = |v: f64| {
                if v.is_nan() {
                    0x7ff8_0000_0000_0000u64
                } else {
                    v.to_bits()
                }
            };
            bits(*x) == bits(*y)
        }
        _ => false,
    }
}

/// `Number.hashCode()` of the boxed class.
pub fn number_hash_code(value: &NumericValue) -> i32 {
    match *value {
        NumericValue::Int(v) => v,
        NumericValue::Long(v) => (v ^ ((v as u64) >> 32) as i64) as i32,
        NumericValue::Float(v) => java_float_hash_code(v),
        NumericValue::Double(v) => java_double_hash_code(v),
    }
}

/// `Number.intValue()`.
pub fn number_int_value(value: &NumericValue) -> i32 {
    match *value {
        NumericValue::Int(v) => v,
        NumericValue::Long(v) => v as i32,
        NumericValue::Float(v) => v as i32,
        NumericValue::Double(v) => v as i32,
    }
}

/// `Number.floatValue()`.
pub fn number_float_value(value: &NumericValue) -> f32 {
    match *value {
        NumericValue::Int(v) => v as f32,
        NumericValue::Long(v) => v as f32,
        NumericValue::Float(v) => v,
        NumericValue::Double(v) => v as f32,
    }
}

/// `Number.longValue()`.
pub fn number_long_value(value: &NumericValue) -> i64 {
    match *value {
        NumericValue::Int(v) => v as i64,
        NumericValue::Long(v) => v,
        NumericValue::Float(v) => v as i64,
        NumericValue::Double(v) => v as i64,
    }
}

/// `Number.doubleValue()`.
pub fn number_double_value(value: &NumericValue) -> f64 {
    match *value {
        NumericValue::Int(v) => v as f64,
        NumericValue::Long(v) => v as f64,
        NumericValue::Float(v) => v as f64,
        NumericValue::Double(v) => v,
    }
}

/// Single label and its value, usually contained in a
/// [`FacetResult`](crate::facet_result::FacetResult).
///
/// Equivalent to `org.apache.lucene.facet.LabelAndValue`.
#[derive(Debug, Clone)]
pub struct LabelAndValue {
    /// Facet's label.
    pub label: String,
    /// Value associated with this label.
    pub value: NumericValue,
    /// Number of occurrences for this label.
    pub count: i32,
}

impl LabelAndValue {
    /// Constructor for `NumericValue` value; `count` is the value's
    /// `intValue()`.
    ///
    /// Equivalent to `LabelAndValue(String, Number)`.
    pub fn new(label: impl Into<String>, value: NumericValue) -> Self {
        let count = number_int_value(&value);
        Self {
            label: label.into(),
            value,
            count,
        }
    }

    /// Constructor for value and count.
    ///
    /// Equivalent to `LabelAndValue(String, Number, int)`.
    pub fn with_count(label: impl Into<String>, value: NumericValue, count: i32) -> Self {
        Self {
            label: label.into(),
            value,
            count,
        }
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        java_string_hash_code(&self.label).wrapping_add(1439i32.wrapping_mul(number_hash_code(&self.value)))
    }
}

impl fmt::Display for LabelAndValue {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.label, self.value)
    }
}

impl PartialEq for LabelAndValue {
    /// Equivalent to `equals(Object)`: the label and the value; not the count.
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label && number_equals(&self.value, &other.value)
    }
}
/// `Objects.hash(Object...)`: `31 * h + element.hashCode()` from `1`.
pub fn objects_hash(values: &[i32]) -> i32 {
    values
        .iter()
        .fold(1i32, |h, v| h.wrapping_mul(31).wrapping_add(*v))
}
