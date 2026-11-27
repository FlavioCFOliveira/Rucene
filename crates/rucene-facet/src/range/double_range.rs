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
//! `org.apache.lucene.facet.range`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::Query;
use rucene::util::java_collections::{java_double_hash_code, java_string_hash_code};
use rucene::util::java_fmt::double_to_string;
use rucene::util::NumericUtils;

use crate::label_and_value::objects_hash;
use crate::multi_double_values::MultiDoubleValues;
use crate::multi_double_values_source::MultiDoubleValuesSource;
use crate::range::long_range::LongRange;
use crate::range::range::{Range, RangeFilterQuery, RangeFilterSource, RangeMatcher};

/// `Math.nextUp(double)`.
pub(crate) fn next_up(d: f64) -> f64 {
    if d.is_nan() || d == f64::INFINITY {
        d
    } else if d == 0.0 {
        f64::from_bits(1)
    } else if d > 0.0 {
        f64::from_bits(d.to_bits() + 1)
    } else {
        f64::from_bits(d.to_bits() - 1)
    }
}

/// `Math.nextAfter(d, Double.NEGATIVE_INFINITY)`, i.e. `Math.nextDown`.
pub(crate) fn next_down(d: f64) -> f64 {
    if d.is_nan() || d == f64::NEG_INFINITY {
        d
    } else if d == 0.0 {
        -f64::from_bits(1)
    } else if d > 0.0 {
        f64::from_bits(d.to_bits() - 1)
    } else {
        f64::from_bits(d.to_bits() + 1)
    }
}

/// `Double.compare(double, double)`.
pub(crate) fn java_double_compare(a: f64, b: f64) -> std::cmp::Ordering {
    if a < b {
        return std::cmp::Ordering::Less;
    }
    if a > b {
        return std::cmp::Ordering::Greater;
    }
    let bits = |v: f64| -> i64 {
        if v.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            v.to_bits() as i64
        }
    };
    bits(a).cmp(&bits(b))
}

/// Represents a range over double values.
///
/// Equivalent to `org.apache.lucene.facet.range.DoubleRange`.
#[derive(Debug, Clone)]
pub struct DoubleRange {
    /// The `Range` this extends: its label.
    pub range: Range,
    /// Minimum (inclusive).
    pub min: f64,
    /// Maximum (inclusive).
    pub max: f64,
}

impl DoubleRange {
    /// Create a DoubleRange.
    ///
    /// Equivalent to `DoubleRange(String, double, boolean, double, boolean)`.
    pub fn new(
        label: &str,
        mut min_in: f64,
        min_inclusive: bool,
        mut max_in: f64,
        max_inclusive: bool,
    ) -> Result<Self> {
        let range = Range::new(label);

        // TODO: if DoubleDocValuesField used
        // LegacyNumericUtils.doubleToSortableLong format (instead of
        // Double.doubleToRawLongBits) we could do comparisons in long space

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
            // Why no Math.nextDown?
            max_in = next_down(max_in);
        }

        if min_in > max_in {
            return Err(range.fail_no_match());
        }

        Ok(Self {
            range,
            min: min_in,
            max: max_in,
        })
    }

    /// The range's label.
    pub fn label(&self) -> &str {
        &self.range.label
    }

    /// True if this range accepts the provided value.
    ///
    /// Equivalent to `accept(double)`.
    pub fn accept(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }

    /// Equivalent to the package-private `toLongRange()`.
    // Public for the port of the facet tests (Java package-private).
    pub fn to_long_range(&self) -> Result<LongRange> {
        LongRange::new(
            &self.range.label,
            NumericUtils::double_to_sortable_long(self.min),
            true,
            NumericUtils::double_to_sortable_long(self.max),
            true,
        )
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        objects_hash(&[
            java_string_hash_code(&self.range.label),
            java_double_hash_code(self.min),
            java_double_hash_code(self.max),
        ])
    }

    /// Create a new `Query` matching only the documents whose
    /// `DoubleValuesSource` value falls in this range.
    ///
    /// Equivalent to `getQuery(Query, DoubleValuesSource)`.
    pub fn get_query(
        &self,
        fast_match_query: Option<Arc<dyn Query>>,
        value_source: Arc<dyn DoubleValuesSource>,
    ) -> Arc<dyn Query> {
        Arc::new(ValueSourceQuery {
            source: Arc::new(DoubleSingleSource {
                range: self.clone(),
                value_source,
            }),
            fast_match_query,
        })
    }

    /// Create a new `Query` matching only the documents with a
    /// `MultiDoubleValuesSource` value in this range.
    ///
    /// Equivalent to `getQuery(Query, MultiDoubleValuesSource)`.
    pub fn get_query_multi(
        &self,
        fast_match_query: Option<Arc<dyn Query>>,
        value_source: Arc<dyn MultiDoubleValuesSource>,
    ) -> Arc<dyn Query> {
        match <dyn MultiDoubleValuesSource>::unwrap_singleton(&*value_source) {
            Some(single_values) => self.get_query(fast_match_query, single_values),
            None => Arc::new(MultiValueSourceQuery {
                source: Arc::new(DoubleMultiSource {
                    range: self.clone(),
                    value_source,
                }),
                fast_match_query,
            }),
        }
    }
}

impl PartialEq for DoubleRange {
    /// Equivalent to `equals(Object)`: `Double.compare` of the bounds.
    fn eq(&self, other: &Self) -> bool {
        other.range.label == self.range.label
            && java_double_compare(other.min, self.min) == std::cmp::Ordering::Equal
            && java_double_compare(other.max, self.max) == std::cmp::Ordering::Equal
    }
}

impl fmt::Display for DoubleRange {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DoubleRange({}: {} to {})",
            self.range.label,
            double_to_string(self.min),
            double_to_string(self.max)
        )
    }
}

/// The range and single-valued source of a `ValueSourceQuery`.
#[derive(Debug)]
pub(crate) struct DoubleSingleSource {
    range: DoubleRange,
    value_source: Arc<dyn DoubleValuesSource>,
}

struct DoubleSingleMatcher {
    range: DoubleRange,
    values: Box<dyn DoubleValues>,
}

impl RangeMatcher for DoubleSingleMatcher {
    fn matches(&mut self, doc: i32) -> Result<bool> {
        Ok(self.values.advance_exact(doc)? && self.range.accept(self.values.double_value()?))
    }
}

impl RangeFilterSource for DoubleSingleSource {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.range.DoubleRange$ValueSourceQuery"
    }

    fn range_to_string(&self) -> String {
        self.range.to_string()
    }

    fn source_eq(&self, other: &Self) -> bool {
        self.range == other.range && self.value_source.source_eq(&*other.value_source)
    }

    fn hash_parts(&self) -> (i32, i32) {
        (self.range.hash_code(), self.value_source.source_hash())
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.value_source.is_cacheable(ctx)
    }

    fn matcher(&self, context: &LeafReaderContext) -> Result<Box<dyn RangeMatcher>> {
        let values = Arc::clone(&self.value_source).get_values(context, None)?;
        Ok(Box::new(DoubleSingleMatcher {
            range: self.range.clone(),
            values,
        }))
    }
}

/// The range and multi-valued source of a `MultiValueSourceQuery`.
#[derive(Debug)]
pub(crate) struct DoubleMultiSource {
    range: DoubleRange,
    value_source: Arc<dyn MultiDoubleValuesSource>,
}

struct DoubleMultiMatcher {
    range: DoubleRange,
    values: Box<dyn MultiDoubleValues>,
}

impl RangeMatcher for DoubleMultiMatcher {
    fn matches(&mut self, doc: i32) -> Result<bool> {
        if !self.values.advance_exact(doc)? {
            return Ok(false);
        }
        let count = self.values.get_value_count()?;
        let mut i = 0i32;
        while (i as i64) < count {
            if self.range.accept(self.values.next_value()?) {
                return Ok(true);
            }
            i += 1;
        }
        Ok(false)
    }
}

impl RangeFilterSource for DoubleMultiSource {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.range.DoubleRange$MultiValueSourceQuery"
    }

    fn range_to_string(&self) -> String {
        self.range.to_string()
    }

    fn source_eq(&self, other: &Self) -> bool {
        self.range == other.range && self.value_source.source_eq(&*other.value_source)
    }

    fn hash_parts(&self) -> (i32, i32) {
        (self.range.hash_code(), self.value_source.source_hash())
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.value_source.is_cacheable(ctx)
    }

    fn matcher(&self, context: &LeafReaderContext) -> Result<Box<dyn RangeMatcher>> {
        let values = self.value_source.get_values(context)?;
        Ok(Box::new(DoubleMultiMatcher {
            range: self.range.clone(),
            values,
        }))
    }
}

/// Equivalent to `org.apache.lucene.facet.range.DoubleRange.ValueSourceQuery`.
pub(crate) type ValueSourceQuery = RangeFilterQuery<DoubleSingleSource>;

/// Equivalent to `org.apache.lucene.facet.range.DoubleRange.MultiValueSourceQuery`.
pub(crate) type MultiValueSourceQuery = RangeFilterQuery<DoubleMultiSource>;
