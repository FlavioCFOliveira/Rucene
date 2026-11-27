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

//! Port of `MatchingFacetSetsCounts.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;
use std::sync::Arc;

use rucene::document::{IntPoint, NumericValue};
use rucene::error::{LuceneError, Result};
use rucene::index::{BinaryDocValues, DocValues};
use rucene::search::{DocIdSetIterator, Query, NO_MORE_DOCS};
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::java_collections::java_compare_to;

use crate::facet_counts_with_filter_query::FacetCountsWithFilterQuery;
use crate::facet_result::FacetResult;
use crate::facets::{validate_top_n, Facets};
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::facetset::facet_set_decoder::FacetSetDecoder;
use crate::facetset::facet_set_matcher::FacetSetMatcher;
use crate::label_and_value::LabelAndValue;

/// Holds a single entry.
///
/// Equivalent to `org.apache.lucene.facet.facetset.MatchingFacetSetsCounts.Entry`.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    label: String,
    count: i32,
}

/// Equivalent to `compare(int, int, String, String)`.
fn compare(count1: i32, count2: i32, label1: &str, label2: &str) -> Ordering {
    let mut cmp = count1.cmp(&count2);
    if cmp == Ordering::Equal {
        cmp = java_compare_to(label2, label1);
    }
    cmp
}

struct EntryLessThan;

impl PriorityQueueComparator<Entry> for EntryLessThan {
    fn less_than(&self, a: &Entry, b: &Entry) -> bool {
        compare(a.count, b.count, &a.label, &b.label) == Ordering::Less
    }
}

/// The binary doc values Java passes to `createIterator` and keeps reading;
/// shared between the conjunction and the counting loop.
struct SharedBinary(Rc<RefCell<Box<dyn BinaryDocValues>>>);

impl DocIdSetIterator for SharedBinary {
    fn doc_id(&self) -> i32 {
        self.0.borrow().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.0.borrow_mut().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.borrow_mut().advance(target)
    }

    fn cost(&self) -> i64 {
        self.0.borrow().cost()
    }
}

/// Returns the counts for each given `FacetSet`.
///
/// Equivalent to `org.apache.lucene.facet.facetset.MatchingFacetSetsCounts`.
pub struct MatchingFacetSetsCounts {
    filter: FacetCountsWithFilterQuery,
    facet_set_matchers: Vec<Arc<dyn FacetSetMatcher>>,
    counts: Vec<i32>,
    field: String,
    facet_set_decoder: Arc<dyn FacetSetDecoder>,
    tot_count: i32,
}

impl MatchingFacetSetsCounts {
    /// Constructs a new instance of matching facet set counts which
    /// calculates the countBytes for each given facet set matcher.
    ///
    /// Equivalent to `MatchingFacetSetsCounts(String, FacetsCollector,
    /// FacetSetDecoder, FacetSetMatcher...)`.
    pub fn new(
        field: &str,
        hits: &FacetsCollector,
        facet_set_decoder: Arc<dyn FacetSetDecoder>,
        facet_set_matchers: Vec<Arc<dyn FacetSetMatcher>>,
    ) -> Result<Self> {
        Self::with_fast_match(field, hits, facet_set_decoder, None, facet_set_matchers)
    }

    /// Constructs a new instance with a fast-match query.
    ///
    /// Equivalent to `MatchingFacetSetsCounts(String, FacetsCollector,
    /// FacetSetDecoder, Query, FacetSetMatcher...)`.
    pub fn with_fast_match(
        field: &str,
        hits: &FacetsCollector,
        facet_set_decoder: Arc<dyn FacetSetDecoder>,
        fast_match_query: Option<Arc<dyn Query>>,
        facet_set_matchers: Vec<Arc<dyn FacetSetMatcher>>,
    ) -> Result<Self> {
        if facet_set_matchers.is_empty() {
            return Err(LuceneError::illegal_argument(
                "facetSetMatchers cannot be null or empty",
            ));
        }
        if Self::are_facet_set_matcher_dimensions_inconsistent(&facet_set_matchers) {
            return Err(LuceneError::illegal_argument(
                "All facet set matchers must be the same dimensionality",
            ));
        }
        let mut this = Self {
            filter: FacetCountsWithFilterQuery::new(fast_match_query),
            counts: vec![0; facet_set_matchers.len()],
            facet_set_matchers,
            field: field.to_string(),
            facet_set_decoder,
            tot_count: 0,
        };
        this.tot_count = this.count(field, hits.get_matching_docs()?)?;
        Ok(this)
    }

    /// Counts from the provided field.
    ///
    /// Equivalent to `count(String, List)`.
    fn count(&mut self, field: &str, matching_docs: &[MatchingDocs]) -> Result<i32> {
        let mut tot_count = 0;
        for hits in matching_docs {
            let binary_doc_values = Rc::new(RefCell::new(DocValues::get_binary(
                &*hits.context.leaf_reader(),
                field,
            )?));

            let Some(mut it) = self.filter.create_iterator(
                hits,
                vec![Some(Box::new(SharedBinary(Rc::clone(&binary_doc_values)))
                    as Box<dyn DocIdSetIterator>)],
            )?
            else {
                continue;
            };

            let mut dim_values: Vec<i64> = Vec::new(); // dimension values buffer
            let mut expected_num_dims = -1;
            let mut doc = it.next_doc()?;
            while doc != NO_MORE_DOCS {
                let mut should_count_doc = false;
                let bytes_ref = binary_doc_values.borrow().binary_value()?;
                let num_dims = IntPoint::decode_dimension(&bytes_ref.bytes, 0);
                if expected_num_dims == -1 {
                    expected_num_dims = num_dims;
                    dim_values = vec![0; num_dims.max(0) as usize];
                } else {
                    // Verify that the number of indexed dimensions for all
                    // matching documents is the same (since we cannot verify
                    // that at indexing time).
                    debug_assert!(
                        num_dims == expected_num_dims,
                        "Expected ({expected_num_dims}) dimensions, found ({num_dims}) for doc ({doc})"
                    );
                }

                let mut start = 4usize;
                while start < bytes_ref.length {
                    start += self
                        .facet_set_decoder
                        .decode(&bytes_ref, start, &mut dim_values);
                    for (j, matcher) in self.facet_set_matchers.iter().enumerate() {
                        // for each facet set matcher
                        if matcher.matches(&dim_values) {
                            self.counts[j] += 1;
                            should_count_doc = true;
                        }
                    }
                }
                if should_count_doc {
                    tot_count += 1;
                }
                doc = it.next_doc()?;
            }
        }
        Ok(tot_count)
    }

    /// Equivalent to `areFacetSetMatcherDimensionsInconsistent(FacetSetMatcher[])`.
    fn are_facet_set_matcher_dimensions_inconsistent(
        facet_set_matchers: &[Arc<dyn FacetSetMatcher>],
    ) -> bool {
        let dims = facet_set_matchers[0].dims();
        facet_set_matchers.iter().any(|m| m.dims() != dims)
    }
}

impl Facets for MatchingFacetSetsCounts {
    /// This instance, for Java's `instanceof` tests and casts.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        if self.field != dim {
            return Err(LuceneError::illegal_argument(format!(
                "invalid dim \"{dim}\"; should be \"{}\"",
                self.field
            )));
        }
        if !path.is_empty() {
            return Err(LuceneError::illegal_argument("path.length should be 0"));
        }
        let label_values: Vec<LabelAndValue> = self
            .counts
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                LabelAndValue::new(self.facet_set_matchers[i].label(), NumericValue::Int(c))
            })
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

    fn get_top_children(
        &self,
        top_n: i32,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<FacetResult>> {
        validate_top_n(top_n)?;

        let top_n = top_n.min(self.counts.len() as i32);
        let mut pq: PriorityQueue<Entry, EntryLessThan> =
            PriorityQueue::with_sentinels(top_n.max(0) as usize, EntryLessThan, || {
                Some(Entry {
                    label: String::new(),
                    count: 0,
                })
            })?;

        let mut child_count = 0;
        for (i, &count) in self.counts.iter().enumerate() {
            if count > 0 {
                child_count += 1;
                let label = self.facet_set_matchers[i].label();
                let reuse = pq
                    .top()
                    .expect("INVARIANT: the queue is pre-filled with sentinels");
                if compare(reuse.count, count, &reuse.label, label) == Ordering::Less {
                    let reuse = pq.top_mut().expect("INVARIANT: pre-filled");
                    reuse.label = label.to_string();
                    reuse.count = count;
                    pq.update_top();
                }
            }
        }

        // Pop off any sentinel values in the case that we had fewer child
        // labels with non-zero counts than the requested top-n:
        while (child_count as usize) < pq.size() {
            pq.pop();
        }

        let mut label_values: Vec<Option<LabelAndValue>> = (0..(top_n.min(child_count) as usize))
            .map(|_| None)
            .collect();
        for i in (0..pq.size()).rev() {
            let e = pq.pop().expect("INVARIANT: size > i");
            label_values[i] = Some(LabelAndValue::new(e.label, NumericValue::Int(e.count)));
        }

        Ok(Some(FacetResult::new(
            dim,
            path.iter().map(|p| p.to_string()).collect(),
            NumericValue::Int(self.tot_count),
            label_values.into_iter().flatten().collect(),
            child_count,
        )))
    }

    fn get_specific_value(&self, _dim: &str, _path: &[&str]) -> Result<NumericValue> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn get_all_dims(&self, top_n: i32) -> Result<Vec<FacetResult>> {
        validate_top_n(top_n)?;
        Ok(self
            .get_top_children(top_n, &self.field, &[])?
            .into_iter()
            .collect())
    }
}
