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

//! Port of `DoubleRange.java`, of the Java package
//! `org.apache.lucene.facet.rangeonrange`.

use std::fmt;

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::{java_double_hash_code, java_string_hash_code};
use rucene::util::java_fmt::double_to_string;

use crate::label_and_value::objects_hash;
use crate::range::double_range::{next_down, next_up};
use crate::rangeonrange::range::Range;
use crate::taxonomy::facet_label::arrays_to_string;

/// Represent a double range for RangeOnRange faceting.
///
/// Equivalent to `org.apache.lucene.facet.rangeonrange.DoubleRange`.
#[derive(Debug, Clone)]
pub struct DoubleRange {
    /// The `Range` this extends: label and dims.
    pub range: Range,
    /// Minimum (inclusive) per dimension.
    pub min: Vec<f64>,
    /// Maximum (inclusive) per dimension.
    pub max: Vec<f64>,
}

impl DoubleRange {
    /// Represents a double range for RangeOnRange faceting.
    ///
    /// Equivalent to `DoubleRange(String, double, boolean, double, boolean)`.
    pub fn new(label: &str, mut min_in: f64, min_inclusive: bool, mut max_in: f64, max_inclusive: bool) -> Result<Self> {
        let range = Range::new(label, 1);

        if min_in.is_nan() {
            return Err(LuceneError::illegal_argument("min cannot be NaN"));
        }
        if !min_inclusive {
            min_in = next_up(min_in);
        }

        if max_in.is_nan() {
            return Err(LuceneError::illegal_argument("max cannot be NaN"));
        }
        if !max_inclusive {
            max_in = next_down(max_in);
        }

        if min_in > max_in {
            return Err(range.fail_no_match());
        }

        Ok(Self {
            range,
            min: vec![min_in],
            max: vec![max_in],
        })
    }

    /// Represents a multidimensional double range for RangeOnRange faceting.
    ///
    /// Equivalent to `DoubleRange(String, double[], double[])`.
    pub fn new_multi(label: &str, min: Vec<f64>, max: Vec<f64>) -> Result<Self> {
        let range = Range::new(label, min.len() as i32);
        Self::check_args(&range, &min, &max)?;
        Ok(Self { range, min, max })
    }

    /// Equivalent to `checkArgs(double[], double[])`.
    fn check_args(range: &Range, min: &[f64], max: &[f64]) -> Result<()> {
        if min.is_empty() || max.is_empty() {
            return Err(range.fail_no_match());
        }
        if min.len() != max.len() {
            return Err(range.fail_no_match());
        }

        for i in 0..min.len() {
            if min[i] > max[i] {
                return Err(range.fail_no_match());
            }
        }
        Ok(())
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let array_hash =
            |a: &[f64]| a.iter().fold(1i32, |h, v| h.wrapping_mul(31).wrapping_add(java_double_hash_code(*v)));
        objects_hash(&[
            java_string_hash_code(&self.range.label),
            array_hash(&self.min),
            array_hash(&self.max),
            self.range.dims,
        ])
    }
}

impl PartialEq for DoubleRange {
    /// Equivalent to `equals(Object)`: `Arrays.equals(double[], double[])`
    /// compares `Double.doubleToLongBits`.
    fn eq(&self, other: &Self) -> bool {
        let bits = |v: &f64| if v.is_nan() { 0x7ff8_0000_0000_0000u64 } else { v.to_bits() };
        self.min.iter().map(bits).eq(other.min.iter().map(bits))
            && self.max.iter().map(bits).eq(other.max.iter().map(bits))
            && self.range.label == other.range.label
            && self.range.dims == other.range.dims
    }
}

impl fmt::Display for DoubleRange {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let min: Vec<String> = self.min.iter().map(|v| double_to_string(*v)).collect();
        let max: Vec<String> = self.max.iter().map(|v| double_to_string(*v)).collect();
        write!(
            f,
            "DoubleRange(label: {}, min: {}, max: {})",
            self.range.label,
            arrays_to_string(&min),
            arrays_to_string(&max)
        )
    }
}