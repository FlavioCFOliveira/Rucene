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

//! Port of `LongRange.java`, of the Java package
//! `org.apache.lucene.facet.rangeonrange`.

use std::fmt;

use rucene::error::Result;
use rucene::util::java_collections::java_string_hash_code;

use crate::label_and_value::objects_hash;
use crate::range::long_range::long_hash;
use crate::rangeonrange::range::Range;
use crate::taxonomy::facet_label::arrays_to_string;

/// Represents a long range for RangeOnRange faceting.
///
/// Equivalent to `org.apache.lucene.facet.rangeonrange.LongRange`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongRange {
    /// The `Range` this extends: label and dims.
    pub range: Range,
    /// Minimum (inclusive) per dimension.
    pub min: Vec<i64>,
    /// Maximum (inclusive) per dimension.
    pub max: Vec<i64>,
}

impl LongRange {
    /// Represents a single dimensional long range for RangeOnRange faceting.
    ///
    /// Equivalent to `LongRange(String, long, boolean, long, boolean)`.
    pub fn new(
        label: &str,
        mut min_in: i64,
        min_inclusive: bool,
        mut max_in: i64,
        max_inclusive: bool,
    ) -> Result<Self> {
        let range = Range::new(label, 1);

        if !min_inclusive {
            if min_in != i64::MAX {
                min_in += 1;
            } else {
                return Err(range.fail_no_match());
            }
        }

        if !max_inclusive {
            if max_in != i64::MIN {
                max_in -= 1;
            } else {
                return Err(range.fail_no_match());
            }
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

    /// Represents a multidimensional long range for RangeOnRange faceting.
    ///
    /// Equivalent to `LongRange(String, long[], long[])`.
    pub fn new_multi(label: &str, min: Vec<i64>, max: Vec<i64>) -> Result<Self> {
        let range = Range::new(label, min.len() as i32);
        Self::check_args(&range, &min, &max)?;
        Ok(Self { range, min, max })
    }

    /// Equivalent to `checkArgs(long[], long[])`.
    fn check_args(range: &Range, min: &[i64], max: &[i64]) -> Result<()> {
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
        let array_hash = |a: &[i64]| {
            a.iter()
                .fold(1i32, |h, v| h.wrapping_mul(31).wrapping_add(long_hash(*v)))
        };
        objects_hash(&[
            java_string_hash_code(&self.range.label),
            array_hash(&self.min),
            array_hash(&self.max),
            self.range.dims,
        ])
    }
}

impl fmt::Display for LongRange {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let min: Vec<String> = self.min.iter().map(|v| v.to_string()).collect();
        let max: Vec<String> = self.max.iter().map(|v| v.to_string()).collect();
        write!(
            f,
            "LongRange(label: {}, min: {}, max: {})",
            self.range.label,
            arrays_to_string(&min),
            arrays_to_string(&max)
        )
    }
}
