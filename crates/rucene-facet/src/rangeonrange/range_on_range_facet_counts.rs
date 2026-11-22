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

//! Port of `RangeOnRangeFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.rangeonrange`.

use std::cmp::Ordering;
use std::sync::Arc;

use rucene::document::range_fields::{BinaryRangeDocValues, RangeQueryType};
use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};
use rucene::index::DocValues;
use rucene::search::{Query, NO_MORE_DOCS};
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::java_collections::java_compare_to;

use crate::facet_counts_with_filter_query::FacetCountsWithFilterQuery;
use crate::facet_result::FacetResult;
use crate::facets::validate_top_n;
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::label_and_value::LabelAndValue;

/// Holds a single entry.
///
/// Equivalent to `org.apache.lucene.facet.rangeonrange.RangeOnRangeFacetCounts.Entry`.
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
/// Equivalent to `org.apache.lucene.facet.rangeonrange.RangeOnRangeFacetCounts`.
pub(crate) struct RangeOnRangeFacetCounts {
    filter: FacetCountsWithFilterQuery,
    /// Ranges passed to constructor.
    labels: Vec<String>,
    /// Counts, initialized in by subclass.
    counts: Vec<i32>,
    /// Our field name.
    field: String,
    /// Total number of hits.
    tot_count: i32,
}

impl RangeOnRangeFacetCounts {
    /// Create `RangeFacetCounts`.
    ///
    /// Equivalent to `RangeOnRangeFacetCounts(String, FacetsCollector,
    /// RangeFieldQuery.QueryType, Query, int, byte[][], String[])`.
    pub(crate) fn new(
        field: &str,
        hits: &FacetsCollector,
        query_type: RangeQueryType,
        fast_match_query: Option<Arc<dyn Query>>,
        num_encoded_value_bytes: usize,
        encoded_ranges: Vec<Vec<u8>>,
        labels: Vec<String>,
    ) -> Result<Self> {
        debug_assert!(encoded_ranges.len() == labels.len());
        debug_assert!(encoded_ranges[0].len() % (2 * num_encoded_value_bytes) == 0);
        let mut this = Self {
            filter: FacetCountsWithFilterQuery::new(fast_match_query),
            labels,
            counts: vec![0; encoded_ranges.len()],
            field: field.to_string(),
            tot_count: 0,
        };
        this.count(field, hits.get_matching_docs(), &encoded_ranges, num_encoded_value_bytes, query_type)?;
        Ok(this)
    }

    /// Counts from the provided field.
    ///
    /// Equivalent to `count(String, List, byte[][], int,
    /// RangeFieldQuery.QueryType)`.
    pub(crate) fn count(
        &mut self,
        field: &str,
        matching_docs: &[MatchingDocs],
        encoded_ranges: &[Vec<u8>],
        num_encoded_value_bytes: usize,
        query_type: RangeQueryType,
    ) -> Result<()> {
        // TODO: We currently just exhaustively check the ranges in each
        // document with every range in the ranges array. We might be able to
        // do something more efficient here by grouping the ranges array into
        // a space partitioning data structure of some sort.

        let dims = encoded_ranges[0].len() / (2 * num_encoded_value_bytes);
        // `ArrayUtil.getUnsignedComparator(numEncodedValueBytes)`: core's
        // `RangeFieldQuery.QueryType.matches` compares the encoded bytes
        // unsigned itself.

        let mut missing_count = 0;

        for hits in matching_docs {
            let mut binary_range_doc_values = BinaryRangeDocValues::new(
                DocValues::get_binary(&*hits.context.leaf_reader(), field)?,
                dims,
                num_encoded_value_bytes,
            );

            let Some(mut it) = self.filter.create_iterator(hits, Vec::new())? else {
                continue;
            };

            self.tot_count += hits.total_hits;
            let mut doc = it.next_doc()?;
            while doc != NO_MORE_DOCS {
                use rucene::index::DocValuesIterator;
                if binary_range_doc_values.advance_exact(doc)? {
                    let mut has_valid_range = false;
                    for (range, encoded_range) in encoded_ranges.iter().enumerate() {
                        let packed_range = binary_range_doc_values.packed_value();
                        debug_assert!(encoded_range.len() == packed_range.len());
                        if query_type.matches(encoded_range, packed_range, dims, num_encoded_value_bytes) {
                            self.counts[range] += 1;
                            has_valid_range = true;
                        }
                    }
                    if !has_valid_range {
                        missing_count += 1;
                    }
                } else {
                    missing_count += 1;
                }
                doc = it.next_doc()?;
            }
        }
        self.tot_count -= missing_count;
        Ok(())
    }

    /// Equivalent to `getAllChildren(String, String...)`.
    pub(crate) fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        self.validate_dim_and_path_for_get_children(dim, path)?;
        let label_values: Vec<LabelAndValue> = self
            .counts
            .iter()
            .enumerate()
            .map(|(i, &c)| LabelAndValue::new(self.labels[i].clone(), NumericValue::Int(c)))
            .collect();
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
    pub(crate) fn get_top_children(&self, top_n: i32, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        validate_top_n(top_n)?;
        self.validate_dim_and_path_for_get_children(dim, path)?;

        let size = top_n.min(self.counts.len() as i32);
        let mut pq: PriorityQueue<Entry, EntryLessThan> = PriorityQueue::new(size.max(0) as usize, EntryLessThan)?;

        let mut child_count = 0;
        let mut e: Option<Entry> = None;
        for (i, &count) in self.counts.iter().enumerate() {
            if count != 0 {
                child_count += 1;
                let mut entry = e.take().unwrap_or_default();
                entry.label = self.labels[i].clone();
                entry.count = count;
                e = pq.insert_with_overflow(entry);
            }
        }

        let mut results: Vec<Option<LabelAndValue>> = (0..pq.size()).map(|_| None).collect();
        while pq.size() != 0 {
            let entry = pq.pop().expect("INVARIANT: size > 0");
            results[pq.size()] = Some(LabelAndValue::new(entry.label, NumericValue::Int(entry.count)));
        }
        Ok(Some(FacetResult::new(
            dim,
            path.iter().map(|p| p.to_string()).collect(),
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
        Ok(self.get_top_children(top_n, &self.field, &[])?.into_iter().collect())
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

/// Implements `Facets` of a subclass by delegating to its base.
macro_rules! range_on_range_facets_delegate {
    ($ty:ty, $base:ident) => {
        impl crate::facets::Facets for $ty {
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
    };
}
pub(crate) use range_on_range_facets_delegate;