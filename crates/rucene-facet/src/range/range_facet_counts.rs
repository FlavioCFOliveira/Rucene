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

//! Port of `RangeFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.range`.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};
use rucene::index::DocValues;
use rucene::search::{Query, NO_MORE_DOCS};
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::java_collections::java_compare_to;
use rucene::util::NumericUtils;

use crate::facet_counts_with_filter_query::FacetCountsWithFilterQuery;
use crate::facet_result::FacetResult;
use crate::facet_utils::{unwrap_singleton_numeric, NumericValuesView};
use crate::facets::validate_top_n;
use crate::facets_collector::MatchingDocs;
use crate::label_and_value::LabelAndValue;
use crate::range::double_range::DoubleRange;
use crate::range::long_range::LongRange;
use crate::range::long_range_counter::{self, LongRangeCounter};

/// The `Range[]` of a `RangeFacetCounts`: the `LongRange[]` of
/// `LongRangeFacetCounts` or the `DoubleRange[]` of
/// `DoubleRangeFacetCounts`, whose `getLongRanges()` and `mapDocValue`
/// overrides differ.
#[derive(Debug, Clone)]
pub(crate) enum Ranges {
    Long(Vec<LongRange>),
    Double(Vec<DoubleRange>),
}

impl Ranges {
    fn len(&self) -> usize {
        match self {
            Ranges::Long(r) => r.len(),
            Ranges::Double(r) => r.len(),
        }
    }

    fn label(&self, i: usize) -> &str {
        match self {
            Ranges::Long(r) => &r[i].range.label,
            Ranges::Double(r) => &r[i].range.label,
        }
    }
}

/// Holds a single entry.
///
/// Equivalent to `org.apache.lucene.facet.range.RangeFacetCounts.Entry`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Entry {
    count: i32,
    label: String,
}

struct EntryLessThan;

impl PriorityQueueComparator<Entry> for EntryLessThan {
    fn less_than(&self, a: &Entry, b: &Entry) -> bool {
        let mut cmp = a.count.cmp(&b.count);
        if cmp == Ordering::Equal {
            cmp = java_compare_to(&b.label, &a.label);
        }
        cmp == Ordering::Less
    }
}

/// Base class for range faceting.
///
/// Equivalent to `org.apache.lucene.facet.range.RangeFacetCounts`.
pub(crate) struct RangeFacetCounts {
    pub(crate) filter: FacetCountsWithFilterQuery,
    /// Ranges passed to constructor.
    pub(crate) ranges: Ranges,
    /// Counts.
    pub(crate) counts: Option<Vec<i32>>,
    /// Our field name.
    pub(crate) field: String,
    /// Total number of hits.
    pub(crate) tot_count: i32,
}

impl RangeFacetCounts {
    /// Create `RangeFacetCounts`.
    ///
    /// Equivalent to `RangeFacetCounts(String, Range[], Query)`.
    pub(crate) fn new(
        field: &str,
        ranges: Ranges,
        fast_match_query: Option<Arc<dyn Query>>,
    ) -> Self {
        Self {
            filter: FacetCountsWithFilterQuery::new(fast_match_query),
            ranges,
            counts: None,
            field: field.to_string(),
            tot_count: 0,
        }
    }

    /// Equivalent to `getLongRanges()` and its overrides.
    fn get_long_ranges(&self) -> Result<Vec<LongRange>> {
        match &self.ranges {
            Ranges::Long(r) => Ok(r.clone()),
            Ranges::Double(ranges) => {
                // We must now translate to long based ranges for counting:
                let mut long_ranges = Vec::with_capacity(ranges.len());
                for range in ranges {
                    long_ranges.push(LongRange::new(
                        &range.range.label,
                        NumericUtils::double_to_sortable_long(range.min),
                        true,
                        NumericUtils::double_to_sortable_long(range.max),
                        true,
                    )?);
                }
                Ok(long_ranges)
            }
        }
    }

    /// Equivalent to `mapDocValue(long)` and its override.
    pub(crate) fn map_doc_value(&self, l: i64) -> i64 {
        match &self.ranges {
            Ranges::Long(_) => l,
            Ranges::Double(_) => NumericUtils::sortable_double_bits(l),
        }
    }

    /// Create a `LongRangeCounter`; the counts go back to
    /// [`finish_counter`](Self::finish_counter).
    ///
    /// Equivalent to `setupCounter()`.
    pub(crate) fn setup_counter(&mut self) -> Result<Box<dyn LongRangeCounter>> {
        debug_assert!(self.counts.is_none());
        let counts = vec![0i32; self.ranges.len()];
        Ok(long_range_counter::create(&self.get_long_ranges()?, counts))
    }

    /// `counter.finish()` and the count buffer back into `counts`.
    pub(crate) fn finish_counter(&mut self, mut counter: Box<dyn LongRangeCounter>) -> i32 {
        let missing = counter.finish();
        self.counts = Some(counter.into_counts());
        missing
    }

    /// Counts from the provided field.
    ///
    /// Equivalent to `count(String, List)`.
    pub(crate) fn count_field(
        &mut self,
        field: &str,
        matching_docs: &[MatchingDocs],
    ) -> Result<()> {
        // load doc values for all segments up front and keep track of whether
        // or not we found any that were actually multi-valued. this allows us
        // to optimize the case where all segments contain single-values.
        let mut multi_valued_doc_vals: Option<Vec<Option<NumericValuesView>>> = None;
        let mut found_multi_valued = false;
        for (i, hits) in matching_docs.iter().enumerate() {
            if hits.total_hits == 0 {
                continue;
            }

            let multi_values = DocValues::get_sorted_numeric(&*hits.context.leaf_reader(), field)?;
            let vals = multi_valued_doc_vals
                .get_or_insert_with(|| (0..matching_docs.len()).map(|_| None).collect());
            let view = unwrap_singleton_numeric(multi_values)?;
            if !found_multi_valued && matches!(view, NumericValuesView::Multi(_)) {
                found_multi_valued = true;
            }
            vals[i] = Some(view);
        }

        let Some(mut doc_vals) = multi_valued_doc_vals else {
            // no hits or no doc values in all segments. nothing to count:
            return Ok(());
        };

        // If we only found single-valued segments, we can optimize the
        // counting logic. Java keeps either the single-valued or the
        // multi-valued view of every segment; when a segment is multi-valued
        // every segment is read through `SortedNumericDocValues`, which a
        // single-valued segment's singleton view answers identically.
        let mut counter = self.setup_counter()?;
        let mut missing_count = 0;

        if !found_multi_valued {
            // All segments contain single values, so we can count them
            // directly with the single value logic:
            for (i, hits) in matching_docs.iter().enumerate() {
                let Some(mut it) = self.filter.create_iterator(hits, Vec::new())? else {
                    continue;
                };
                // `singleValuedDocVals[i]`, `null` for a segment without hits.
                let mut single_values = match doc_vals[i].as_mut() {
                    Some(NumericValuesView::Single(single_values)) => Some(single_values),
                    _ => None,
                };
                self.tot_count += hits.total_hits;
                let mut doc = it.next_doc()?;
                while doc != NO_MORE_DOCS {
                    let single_values = single_values
                        .as_mut()
                        .ok_or_else(LuceneError::null_pointer)?;
                    if single_values.advance_exact(doc)? {
                        let v = self.map_doc_value(single_values.long_value()?);
                        counter.add_single_valued(v);
                    } else {
                        missing_count += 1;
                    }
                    doc = it.next_doc()?;
                }
            }
        } else {
            for (i, hits) in matching_docs.iter().enumerate() {
                let Some(mut it) = self.filter.create_iterator(hits, Vec::new())? else {
                    continue;
                };
                let mut view = doc_vals[i].as_mut();
                let mut doc = it.next_doc()?;
                while doc != NO_MORE_DOCS {
                    // `multiValuedDocVals[i]`, `null` for a segment without hits.
                    let view = view.as_mut().ok_or_else(LuceneError::null_pointer)?;
                    let (has, limit) = match view {
                        NumericValuesView::Single(s) => (s.advance_exact(doc)?, 1),
                        NumericValuesView::Multi(m) => {
                            let has = m.advance_exact(doc)?;
                            (has, if has { m.doc_value_count()? } else { 0 })
                        }
                    };
                    if has {
                        let mut next_value = || -> Result<i64> {
                            match view {
                                NumericValuesView::Single(s) => s.long_value(),
                                NumericValuesView::Multi(m) => m.next_value(),
                            }
                        };
                        if limit == 1 {
                            let v = next_value()?;
                            counter.add_single_valued(self.map_doc_value(v));
                            self.tot_count += 1;
                        } else {
                            counter.start_multi_valued_doc();
                            let mut previous = 0i64;
                            for j in 0..limit {
                                let val = self.map_doc_value(next_value()?);
                                if j == 0 || val != previous {
                                    counter.add_multi_valued(val);
                                    previous = val;
                                }
                            }
                            if counter.end_multi_valued_doc() {
                                self.tot_count += 1;
                            }
                        }
                    }
                    doc = it.next_doc()?;
                }
            }
        }

        missing_count += self.finish_counter(counter);
        self.tot_count -= missing_count;
        Ok(())
    }

    /// Equivalent to `getAllChildren(String, String...)`.
    pub(crate) fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        self.validate_dim_and_path_for_get_children(dim, path)?;
        let mut label_values = Vec::with_capacity(self.ranges.len());
        for i in 0..self.ranges.len() {
            let count = self.counts.as_ref().map_or(0, |c| c[i]);
            label_values.push(LabelAndValue::new(
                self.ranges.label(i),
                NumericValue::Int(count),
            ));
        }
        let child_count = label_values.len() as i32;
        Ok(Some(FacetResult::new(
            dim,
            path.iter().map(|p| p.to_string()).collect(),
            NumericValue::Int(self.tot_count),
            label_values,
            child_count,
        )))
    }

    /// Equivalent to `getTopChildren(int, String, String...)`.
    pub(crate) fn get_top_children(
        &self,
        top_n: i32,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<FacetResult>> {
        validate_top_n(top_n)?;
        self.validate_dim_and_path_for_get_children(dim, path)?;

        let path_vec: Vec<String> = path.iter().map(|p| p.to_string()).collect();
        let Some(counts) = &self.counts else {
            debug_assert!(self.tot_count == 0);
            return Ok(Some(FacetResult::new(
                dim,
                path_vec,
                NumericValue::Int(self.tot_count),
                Vec::new(),
                0,
            )));
        };

        let size = top_n.min(counts.len() as i32);
        let mut pq: PriorityQueue<Entry, EntryLessThan> =
            PriorityQueue::new(size.max(0) as usize, EntryLessThan)?;

        let mut child_count = 0;
        let mut e: Option<Entry> = None;
        for (i, &count) in counts.iter().enumerate() {
            if count != 0 {
                child_count += 1;
                let mut entry = e.take().unwrap_or_default();
                entry.label = self.ranges.label(i).to_string();
                entry.count = count;
                e = pq.insert_with_overflow(entry);
            }
        }

        let mut results: Vec<Option<LabelAndValue>> = (0..pq.size()).map(|_| None).collect();
        while pq.size() != 0 {
            let entry = pq.pop().expect("INVARIANT: size > 0");
            results[pq.size()] = Some(LabelAndValue::new(
                entry.label,
                NumericValue::Int(entry.count),
            ));
        }
        Ok(Some(FacetResult::new(
            dim,
            path_vec,
            NumericValue::Int(self.tot_count),
            results.into_iter().flatten().collect(),
            child_count,
        )))
    }

    /// Equivalent to `getSpecificValue(String, String...)`.
    pub(crate) fn get_specific_value(&self, _dim: &str, _path: &[&str]) -> Result<NumericValue> {
        // TODO: should we impl this?
        Err(LuceneError::unsupported_operation(String::new()))
    }

    /// Equivalent to `getAllDims(int)`.
    pub(crate) fn get_all_dims(&self, top_n: i32) -> Result<Vec<FacetResult>> {
        validate_top_n(top_n)?;
        Ok(self
            .get_top_children(top_n, &self.field, &[])?
            .into_iter()
            .collect())
    }

    /// Equivalent to `validateDimAndPathForGetChildren(String, String...)`.
    fn validate_dim_and_path_for_get_children(&self, dim: &str, path: &[&str]) -> Result<()> {
        if dim != self.field {
            return Err(LuceneError::illegal_argument(format!(
                "invalid dim \"{dim}\"; should be \"{}\"",
                self.field
            )));
        }
        if !path.is_empty() {
            return Err(LuceneError::illegal_argument("path.length should be 0"));
        }
        Ok(())
    }
}

impl fmt::Display for RangeFacetCounts {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "RangeFacetCounts totCount={}:", self.tot_count)?;
        for i in 0..self.ranges.len() {
            writeln!(
                f,
                "  {} -> count={}",
                self.ranges.label(i),
                self.counts.as_ref().map_or(0, |c| c[i])
            )?;
        }
        Ok(())
    }
}

/// Implements `Facets` and `Display` of a subclass by delegating to its base.
macro_rules! range_facets_delegate {
    ($ty:ty, $base:ident) => {
        impl crate::facets::Facets for $ty {
            /// This instance, for Java's `instanceof` tests and casts.
            fn as_any(&self) -> Option<&dyn std::any::Any> {
                Some(self)
            }

            fn get_all_children(
                &self,
                dim: &str,
                path: &[&str],
            ) -> rucene::error::Result<Option<crate::facet_result::FacetResult>> {
                self.$base.get_all_children(dim, path)
            }

            fn get_top_children(
                &self,
                top_n: i32,
                dim: &str,
                path: &[&str],
            ) -> rucene::error::Result<Option<crate::facet_result::FacetResult>> {
                self.$base.get_top_children(top_n, dim, path)
            }

            fn get_specific_value(
                &self,
                dim: &str,
                path: &[&str],
            ) -> rucene::error::Result<rucene::document::NumericValue> {
                self.$base.get_specific_value(dim, path)
            }

            fn get_all_dims(
                &self,
                top_n: i32,
            ) -> rucene::error::Result<Vec<crate::facet_result::FacetResult>> {
                self.$base.get_all_dims(top_n)
            }
        }

        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.$base, f)
            }
        }
    };
}
pub(crate) use range_facets_delegate;
