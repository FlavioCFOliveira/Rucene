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

//! Port of `DimRange.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use rucene::error::{LuceneError, Result};
use rucene::util::NumericUtils;

use crate::range::double_range::{next_down, next_up};

/// `Math.nextUp(float)`.
fn next_up_f32(f: f32) -> f32 {
    if f.is_nan() || f == f32::INFINITY {
        f
    } else if f == 0.0 {
        f32::from_bits(1)
    } else if f > 0.0 {
        f32::from_bits(f.to_bits() + 1)
    } else {
        f32::from_bits(f.to_bits() - 1)
    }
}

/// `Math.nextDown(float)`.
fn next_down_f32(f: f32) -> f32 {
    if f.is_nan() || f == f32::NEG_INFINITY {
        f
    } else if f == 0.0 {
        -f32::from_bits(1)
    } else if f > 0.0 {
        f32::from_bits(f.to_bits() - 1)
    } else {
        f32::from_bits(f.to_bits() + 1)
    }
}

/// Defines a single range in a `FacetSet` dimension.
///
/// Equivalent to the record `org.apache.lucene.facet.facetset.DimRange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DimRange {
    /// Inclusive min.
    pub min: i64,
    /// Inclusive max.
    pub max: i64,
}

impl DimRange {
    /// The record's canonical constructor.
    pub fn new(min: i64, max: i64) -> Self {
        Self { min, max }
    }

    /// Equivalent to `min()`.
    pub fn min(&self) -> i64 {
        self.min
    }

    /// Equivalent to `max()`.
    pub fn max(&self) -> i64 {
        self.max
    }

    /// Creates a `DimRange` for the given min and max long values.
    ///
    /// Equivalent to `fromLongs(long, boolean, long, boolean)`.
    pub fn from_longs(mut min: i64, min_inclusive: bool, mut max: i64, max_inclusive: bool) -> Result<Self> {
        if !min_inclusive {
            if min != i64::MAX {
                min += 1;
            } else {
                return Err(LuceneError::illegal_argument(format!("Invalid min input: {min}")));
            }
        }
        if !max_inclusive {
            if max != i64::MIN {
                max -= 1;
            } else {
                return Err(LuceneError::illegal_argument(format!("Invalid max input: {max}")));
            }
        }
        if min > max {
            return Err(LuceneError::illegal_argument(format!(
                "Minimum cannot be greater than maximum, max={max}, min={min}"
            )));
        }
        Ok(Self::new(min, max))
    }

    /// Creates a `DimRange` for the given min and max double values.
    ///
    /// Equivalent to `fromDoubles(double, boolean, double, boolean)`.
    pub fn from_doubles(mut min: f64, min_inclusive: bool, mut max: f64, max_inclusive: bool) -> Result<Self> {
        if min.is_nan() {
            return Err(LuceneError::illegal_argument("min cannot be NaN"));
        }
        if !min_inclusive {
            min = next_up(min);
        }

        if max.is_nan() {
            return Err(LuceneError::illegal_argument("max cannot be NaN"));
        }
        if !max_inclusive {
            max = next_down(max);
        }

        if min > max {
            return Err(LuceneError::illegal_argument("Minimum cannot be greater than maximum"));
        }
        Ok(Self::new(
            NumericUtils::double_to_sortable_long(min),
            NumericUtils::double_to_sortable_long(max),
        ))
    }

    /// Creates a `DimRange` for the given min and max float values.
    ///
    /// Equivalent to `fromFloats(float, boolean, float, boolean)`.
    pub fn from_floats(mut min: f32, min_inclusive: bool, mut max: f32, max_inclusive: bool) -> Result<Self> {
        if min.is_nan() {
            return Err(LuceneError::illegal_argument("min cannot be NaN"));
        }
        if !min_inclusive {
            min = next_up_f32(min);
        }

        if max.is_nan() {
            return Err(LuceneError::illegal_argument("max cannot be NaN"));
        }
        if !max_inclusive {
            max = next_down_f32(max);
        }

        if min > max {
            return Err(LuceneError::illegal_argument("Minimum cannot be greater than maximum"));
        }
        Ok(Self::new(
            NumericUtils::float_to_sortable_int(min) as i64,
            NumericUtils::float_to_sortable_int(max) as i64,
        ))
    }
}