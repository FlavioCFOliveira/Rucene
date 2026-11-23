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

//! Port of `LongRangeFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.range`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::long_values_source::LongValuesSource;
use rucene::search::{Query, NO_MORE_DOCS};

use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::multi_long_values_source::MultiLongValuesSource;
use crate::range::long_range::LongRange;
use crate::range::long_range_counter::LongRangeCounter;
use crate::range::range_facet_counts::{range_facets_delegate, RangeFacetCounts, Ranges};

/// `Facets` implementation that computes counts for dynamic long ranges.
///
/// Equivalent to `org.apache.lucene.facet.range.LongRangeFacetCounts`.
pub struct LongRangeFacetCounts {
    base: RangeFacetCounts,
}

impl LongRangeFacetCounts {
    /// Create `LongRangeFacetCounts`, using either single-valued
    /// `NumericDocValues` or multi-valued `SortedNumericDocValues` from the
    /// specified field.
    ///
    /// Equivalent to `LongRangeFacetCounts(String, FacetsCollector,
    /// LongRange...)`.
    pub fn new(field: &str, hits: &FacetsCollector, ranges: Vec<LongRange>) -> Result<Self> {
        Self::with_value_source_and_fast_match(field, None, hits, None, ranges)
    }

    /// Create `LongRangeFacetCounts` using the provided `LongValuesSource`.
    ///
    /// Equivalent to `LongRangeFacetCounts(String, LongValuesSource,
    /// FacetsCollector, LongRange...)`.
    pub fn with_value_source(
        field: &str,
        value_source: Option<Arc<dyn LongValuesSource>>,
        hits: &FacetsCollector,
        ranges: Vec<LongRange>,
    ) -> Result<Self> {
        Self::with_value_source_and_fast_match(field, value_source, hits, None, ranges)
    }

    /// Create `LongRangeFacetCounts` using the provided
    /// `MultiLongValuesSource`.
    ///
    /// Equivalent to `LongRangeFacetCounts(String, MultiLongValuesSource,
    /// FacetsCollector, LongRange...)`.
    pub fn with_multi_values_source(
        field: &str,
        values_source: Option<Arc<dyn MultiLongValuesSource>>,
        hits: &FacetsCollector,
        ranges: Vec<LongRange>,
    ) -> Result<Self> {
        Self::with_multi_values_source_and_fast_match(field, values_source, hits, None, ranges)
    }

    /// Create `LongRangeFacetCounts`, using the provided `LongValuesSource`
    /// if non-null, and an optional fast-match query.
    ///
    /// Equivalent to `LongRangeFacetCounts(String, LongValuesSource,
    /// FacetsCollector, Query, LongRange...)`.
    pub fn with_value_source_and_fast_match(
        field: &str,
        value_source: Option<Arc<dyn LongValuesSource>>,
        hits: &FacetsCollector,
        fast_match_query: Option<Arc<dyn Query>>,
        ranges: Vec<LongRange>,
    ) -> Result<Self> {
        let mut this = Self {
            base: RangeFacetCounts::new(field, Ranges::Long(ranges), fast_match_query),
        };
        match value_source {
            Some(value_source) => this.count_source(value_source, hits.get_matching_docs()?)?,
            None => this.base.count_field(field, hits.get_matching_docs()?)?,
        }
        Ok(this)
    }

    /// Create `LongRangeFacetCounts`, using the provided
    /// `MultiLongValuesSource` if non-null, and an optional fast-match query.
    ///
    /// Equivalent to `LongRangeFacetCounts(String, MultiLongValuesSource,
    /// FacetsCollector, Query, LongRange...)`.
    pub fn with_multi_values_source_and_fast_match(
        field: &str,
        values_source: Option<Arc<dyn MultiLongValuesSource>>,
        hits: &FacetsCollector,
        fast_match_query: Option<Arc<dyn Query>>,
        ranges: Vec<LongRange>,
    ) -> Result<Self> {
        let mut this = Self {
            base: RangeFacetCounts::new(field, Ranges::Long(ranges), fast_match_query),
        };
        match values_source {
            Some(values_source) => {
                match <dyn MultiLongValuesSource>::unwrap_singleton(&*values_source) {
                    Some(single_values) => {
                        this.count_source(single_values, hits.get_matching_docs()?)?
                    }
                    None => this.count_multi_source(&*values_source, hits.get_matching_docs()?)?,
                }
            }
            None => this.base.count_field(field, hits.get_matching_docs()?)?,
        }
        Ok(this)
    }

    /// Counts from the provided `LongValuesSource`.
    ///
    /// Equivalent to `count(LongValuesSource, List)`.
    fn count_source(
        &mut self,
        value_source: Arc<dyn LongValuesSource>,
        matching_docs: &[MatchingDocs],
    ) -> Result<()> {
        let mut counter: Option<Box<dyn LongRangeCounter>> = None;
        let mut missing_count = 0;
        for hits in matching_docs {
            if hits.total_hits == 0 {
                continue;
            }

            let Some(mut it) = self.base.filter.create_iterator(hits, Vec::new())? else {
                continue;
            };

            if counter.is_none() {
                counter = Some(self.base.setup_counter()?);
            }
            let c = counter.as_mut().expect("INVARIANT: set above");

            let mut fv = Arc::clone(&value_source).get_values(&hits.context, None)?;
            self.base.tot_count += hits.total_hits;

            let mut doc = it.next_doc()?;
            while doc != NO_MORE_DOCS {
                // Skip missing docs:
                if fv.advance_exact(doc)? {
                    c.add_single_valued(fv.long_value()?);
                } else {
                    missing_count += 1;
                }

                doc = it.next_doc()?;
            }
        }

        if let Some(counter) = counter {
            missing_count += self.base.finish_counter(counter);
            self.base.tot_count -= missing_count;
        }
        Ok(())
    }

    /// Counts from the provided `MultiLongValuesSource`.
    ///
    /// Equivalent to `count(MultiLongValuesSource, List)`.
    fn count_multi_source(
        &mut self,
        value_source: &dyn MultiLongValuesSource,
        matching_docs: &[MatchingDocs],
    ) -> Result<()> {
        let mut counter: Option<Box<dyn LongRangeCounter>> = None;
        for hits in matching_docs {
            if hits.total_hits == 0 {
                continue;
            }

            let Some(mut it) = self.base.filter.create_iterator(hits, Vec::new())? else {
                continue;
            };

            if counter.is_none() {
                counter = Some(self.base.setup_counter()?);
            }
            let c = counter.as_mut().expect("INVARIANT: set above");

            let mut multi_values = value_source.get_values(&hits.context)?;

            let mut doc = it.next_doc()?;
            while doc != NO_MORE_DOCS {
                // Skip missing docs:
                if multi_values.advance_exact(doc)? {
                    let limit = multi_values.get_value_count()?;
                    // optimize single-value case
                    if limit == 1 {
                        c.add_single_valued(multi_values.next_value()?);
                        self.base.tot_count += 1;
                    } else {
                        c.start_multi_valued_doc();
                        let mut previous = 0i64;
                        for i in 0..limit {
                            let val = multi_values.next_value()?;
                            if i == 0 || val != previous {
                                c.add_multi_valued(val);
                                previous = val;
                            }
                        }
                        if c.end_multi_valued_doc() {
                            self.base.tot_count += 1;
                        }
                    }
                }

                doc = it.next_doc()?;
            }
        }

        if let Some(counter) = counter {
            let missing_count = self.base.finish_counter(counter);
            self.base.tot_count -= missing_count;
        }
        Ok(())
    }

    /// Equivalent to `getLongRanges()`.
    pub fn get_long_ranges(&self) -> &[LongRange] {
        match &self.base.ranges {
            Ranges::Long(r) => r,
            Ranges::Double(_) => unreachable!("LongRangeFacetCounts holds LongRanges"),
        }
    }
}

range_facets_delegate!(LongRangeFacetCounts, base);
