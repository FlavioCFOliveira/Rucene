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
//! `org.apache.lucene.facet.range`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::long_values::LongValues;
use rucene::search::long_values_source::LongValuesSource;
use rucene::search::Query;
use rucene::util::java_collections::java_string_hash_code;

use crate::label_and_value::objects_hash;
use crate::multi_long_values::MultiLongValues;
use crate::multi_long_values_source::MultiLongValuesSource;
use crate::range::range::{Range, RangeFilterQuery, RangeFilterSource, RangeMatcher};

/// Represents a range over long values.
///
/// Equivalent to `org.apache.lucene.facet.range.LongRange`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongRange {
    /// The `Range` this extends: its label.
    pub range: Range,
    /// Minimum (inclusive).
    pub min: i64,
    /// Maximum (inclusive).
    pub max: i64,
}

/// `Long.hashCode(long)`.
pub(crate) fn long_hash(v: i64) -> i32 {
    (v ^ ((v as u64) >> 32) as i64) as i32
}

impl LongRange {
    /// Create a LongRange.
    ///
    /// Equivalent to `LongRange(String, long, boolean, long, boolean)`.
    pub fn new(
        label: &str,
        mut min_in: i64,
        min_inclusive: bool,
        mut max_in: i64,
        max_inclusive: bool,
    ) -> Result<Self> {
        let range = Range::new(label);

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
    /// Equivalent to `accept(long)`.
    pub fn accept(&self, value: i64) -> bool {
        value >= self.min && value <= self.max
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        objects_hash(&[
            java_string_hash_code(&self.range.label),
            long_hash(self.min),
            long_hash(self.max),
        ])
    }

    /// Create a new `Query` matching only the documents whose
    /// `LongValuesSource` value falls in this range.
    ///
    /// Equivalent to `getQuery(Query, LongValuesSource)`.
    pub fn get_query(
        &self,
        fast_match_query: Option<Arc<dyn Query>>,
        value_source: Arc<dyn LongValuesSource>,
    ) -> Arc<dyn Query> {
        Arc::new(ValueSourceQuery {
            source: Arc::new(LongSingleSource {
                range: self.clone(),
                value_source,
            }),
            fast_match_query,
        })
    }

    /// Create a new `Query` matching only the documents with a
    /// `MultiLongValuesSource` value in this range.
    ///
    /// Equivalent to `getQuery(Query, MultiLongValuesSource)`.
    pub fn get_query_multi(
        &self,
        fast_match_query: Option<Arc<dyn Query>>,
        values_source: Arc<dyn MultiLongValuesSource>,
    ) -> Arc<dyn Query> {
        match <dyn MultiLongValuesSource>::unwrap_singleton(&*values_source) {
            Some(single_values) => self.get_query(fast_match_query, single_values),
            None => Arc::new(MultiValueSourceQuery {
                source: Arc::new(LongMultiSource {
                    range: self.clone(),
                    values_source,
                }),
                fast_match_query,
            }),
        }
    }
}

impl fmt::Display for LongRange {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LongRange({}: {} to {})",
            self.range.label, self.min, self.max
        )
    }
}

/// The range and single-valued source of a `ValueSourceQuery`.
#[derive(Debug)]
pub(crate) struct LongSingleSource {
    range: LongRange,
    value_source: Arc<dyn LongValuesSource>,
}

struct LongSingleMatcher {
    range: LongRange,
    values: Box<dyn LongValues>,
}

impl RangeMatcher for LongSingleMatcher {
    fn matches(&mut self, doc: i32) -> Result<bool> {
        Ok(self.values.advance_exact(doc)? && self.range.accept(self.values.long_value()?))
    }
}

impl RangeFilterSource for LongSingleSource {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.range.LongRange$ValueSourceQuery"
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
        Ok(Box::new(LongSingleMatcher {
            range: self.range.clone(),
            values,
        }))
    }
}

/// The range and multi-valued source of a `MultiValueSourceQuery`.
#[derive(Debug)]
pub(crate) struct LongMultiSource {
    range: LongRange,
    values_source: Arc<dyn MultiLongValuesSource>,
}

struct LongMultiMatcher {
    range: LongRange,
    values: Box<dyn MultiLongValues>,
}

impl RangeMatcher for LongMultiMatcher {
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

impl RangeFilterSource for LongMultiSource {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.range.LongRange$MultiValueSourceQuery"
    }

    fn range_to_string(&self) -> String {
        self.range.to_string()
    }

    fn source_eq(&self, other: &Self) -> bool {
        self.range == other.range && self.values_source.source_eq(&*other.values_source)
    }

    fn hash_parts(&self) -> (i32, i32) {
        (self.range.hash_code(), self.values_source.source_hash())
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.values_source.is_cacheable(ctx)
    }

    fn matcher(&self, context: &LeafReaderContext) -> Result<Box<dyn RangeMatcher>> {
        let values = self.values_source.get_values(context)?;
        Ok(Box::new(LongMultiMatcher {
            range: self.range.clone(),
            values,
        }))
    }
}

/// Equivalent to `org.apache.lucene.facet.range.LongRange.ValueSourceQuery`.
pub(crate) type ValueSourceQuery = RangeFilterQuery<LongSingleSource>;

/// Equivalent to `org.apache.lucene.facet.range.LongRange.MultiValueSourceQuery`.
pub(crate) type MultiValueSourceQuery = RangeFilterQuery<LongMultiSource>;
