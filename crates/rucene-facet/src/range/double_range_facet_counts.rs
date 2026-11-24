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

//! Port of `DoubleRangeFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.range`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{Query, NO_MORE_DOCS};
use rucene::util::NumericUtils;

use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::multi_double_values_source::MultiDoubleValuesSource;
use crate::range::double_range::DoubleRange;
use crate::range::long_range_counter::LongRangeCounter;
use crate::range::range_facet_counts::{range_facets_delegate, RangeFacetCounts, Ranges};

/// `Facets` implementation that computes counts for dynamic double ranges.
///
/// Equivalent to `org.apache.lucene.facet.range.DoubleRangeFacetCounts`.
pub struct DoubleRangeFacetCounts {
    base: RangeFacetCounts,
}

impl DoubleRangeFacetCounts {
    /// Create `RangeFacetCounts` using the provided field.
    ///
    /// Equivalent to `DoubleRangeFacetCounts(String, FacetsCollector,
    /// DoubleRange...)`.
    pub fn new(field: &str, hits: &FacetsCollector, ranges: Vec<DoubleRange>) -> Result<Self> {
        Self::with_value_source_and_fast_match(field, None, hits, None, ranges)
    }

    /// Create `RangeFacetCounts` using the provided `DoubleValuesSource`.
    ///
    /// Equivalent to `DoubleRangeFacetCounts(String, DoubleValuesSource,
    /// FacetsCollector, DoubleRange...)`.
    pub fn with_value_source(
        field: &str,
        value_source: Option<Arc<dyn DoubleValuesSource>>,
        hits: &FacetsCollector,
        ranges: Vec<DoubleRange>,
    ) -> Result<Self> {
        Self::with_value_source_and_fast_match(field, value_source, hits, None, ranges)
    }

    /// Create `RangeFacetCounts` using the provided `MultiDoubleValuesSource`.
    ///
    /// Equivalent to `DoubleRangeFacetCounts(String, MultiDoubleValuesSource,
    /// FacetsCollector, DoubleRange...)`.
    pub fn with_multi_values_source(
        field: &str,
        values_source: Option<Arc<dyn MultiDoubleValuesSource>>,
        hits: &FacetsCollector,
        ranges: Vec<DoubleRange>,
    ) -> Result<Self> {
        Self::with_multi_values_source_and_fast_match(field, values_source, hits, None, ranges)
    }

    /// Create `RangeFacetCounts` using the provided `DoubleValuesSource` if
    /// non-null and an optional fast-match query.
    ///
    /// Equivalent to `DoubleRangeFacetCounts(String, DoubleValuesSource,
    /// FacetsCollector, Query, DoubleRange...)`.
    pub fn with_value_source_and_fast_match(
        field: &str,
        value_source: Option<Arc<dyn DoubleValuesSource>>,
        hits: &FacetsCollector,
        fast_match_query: Option<Arc<dyn Query>>,
        ranges: Vec<DoubleRange>,
    ) -> Result<Self> {
        let mut this = Self {
            base: RangeFacetCounts::new(field, Ranges::Double(ranges), fast_match_query),
        };
        match value_source {
            Some(value_source) => this.count_source(value_source, hits.get_matching_docs()?)?,
            None => this.base.count_field(field, hits.get_matching_docs()?)?,
        }
        Ok(this)
    }

    /// Create `RangeFacetCounts` using the provided
    /// `MultiDoubleValuesSource` if non-null and an optional fast-match
    /// query.
    ///
    /// Equivalent to `DoubleRangeFacetCounts(String, MultiDoubleValuesSource,
    /// FacetsCollector, Query, DoubleRange...)`.
    pub fn with_multi_values_source_and_fast_match(
        field: &str,
        values_source: Option<Arc<dyn MultiDoubleValuesSource>>,
        hits: &FacetsCollector,
        fast_match_query: Option<Arc<dyn Query>>,
        ranges: Vec<DoubleRange>,
    ) -> Result<Self> {
        let mut this = Self {
            base: RangeFacetCounts::new(field, Ranges::Double(ranges), fast_match_query),
        };
        match values_source {
            Some(values_source) => {
                match <dyn MultiDoubleValuesSource>::unwrap_singleton(&*values_source) {
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

    /// Counts from the provided `DoubleValuesSource`.
    ///
    /// Equivalent to `count(DoubleValuesSource, List)`.
    fn count_source(
        &mut self,
        value_source: Arc<dyn DoubleValuesSource>,
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
                    c.add_single_valued(NumericUtils::double_to_sortable_long(fv.double_value()?));
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

    /// Counts from the provided `MultiDoubleValuesSource`.
    ///
    /// Equivalent to `count(MultiDoubleValuesSource, List)`.
    fn count_multi_source(
        &mut self,
        value_source: &dyn MultiDoubleValuesSource,
        matching_docs: &[MatchingDocs],
    ) -> Result<()> {
        let mut counter: Option<Box<dyn LongRangeCounter>> = None; // LongRangeCounter.create(longRanges, counts);
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

            let mut multi_values = value_source.get_values(&hits.context)?;

            let mut doc = it.next_doc()?;
            while doc != NO_MORE_DOCS {
                // Skip missing docs:
                if multi_values.advance_exact(doc)? {
                    let limit = multi_values.get_value_count()?;
                    // optimize single-valued case
                    if limit == 1 {
                        c.add_single_valued(NumericUtils::double_to_sortable_long(
                            multi_values.next_value()?,
                        ));
                        self.base.tot_count += 1;
                    } else {
                        c.start_multi_valued_doc();
                        let mut previous = 0i64;
                        for i in 0..limit {
                            let val =
                                NumericUtils::double_to_sortable_long(multi_values.next_value()?);
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
            missing_count += self.base.finish_counter(counter);
            self.base.tot_count -= missing_count;
        }
        Ok(())
    }
}

range_facets_delegate!(DoubleRangeFacetCounts, base);
