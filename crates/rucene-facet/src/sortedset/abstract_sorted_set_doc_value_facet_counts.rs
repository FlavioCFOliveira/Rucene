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

//! Port of `AbstractSortedSetDocValueFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.sortedset`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};
use rucene::index::{OrdinalMap, SortedSetDocValues};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::java_collections::java_compare_to;
use rucene::util::{Bits, BytesRef};

use crate::facet_result::FacetResult;
use crate::facet_utils::Intersect2;
use crate::facets::validate_top_n;
use crate::facets_config::{DimConfig, FacetsConfig};
use crate::label_and_value::{number_int_value, LabelAndValue};
use crate::sortedset::sorted_set_doc_values_reader_state::{
    ChildOrdIterator, SortedSetDocValuesReaderState,
};
use crate::top_ord_and_int_queue::{OrdAndInt, TopOrdAndIntQueue};
use crate::top_ord_and_number_queue::OrdAndValue;

/// The counts a subclass keeps: `int[] counts` in
/// `SortedSetDocValuesFacetCounts`, `AtomicIntegerArray counts` in
/// `ConcurrentSortedSetDocValuesFacetCounts`.
pub(crate) enum CountStore {
    Plain(Option<Vec<i32>>),
    Atomic(Arc<[AtomicI32]>),
}

/// Intermediate result to store top children for a given path before
/// resolving labels, etc.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.sortedset.AbstractSortedSetDocValueFacetCounts.TopChildrenForPath`.
pub(crate) struct TopChildrenForPath {
    path_count: i32,
    child_count: i32,
    q: Option<TopOrdAndIntQueue>,
}

/// Equivalent to
/// `org.apache.lucene.facet.sortedset.AbstractSortedSetDocValueFacetCounts.DimValue`.
#[derive(Debug, Clone)]
pub(crate) struct DimValue {
    dim: String,
    value: i32,
}

struct DimValueLessThan;

impl PriorityQueueComparator<DimValue> for DimValueLessThan {
    fn less_than(&self, a: &DimValue, b: &DimValue) -> bool {
        if a.value > b.value {
            false
        } else if a.value < b.value {
            true
        } else {
            java_compare_to(&a.dim, &b.dim) == Ordering::Greater
        }
    }
}

/// Equivalent to the record
/// `org.apache.lucene.facet.sortedset.AbstractSortedSetDocValueFacetCounts.ChildIterationCursor`.
pub(crate) struct ChildIterationCursor {
    path_ord: i32,
    child_iterator: ChildOrdIterator,
}

/// Base class for SSDV faceting implementations.
///
/// Equivalent to `org.apache.lucene.facet.sortedset.AbstractSortedSetDocValueFacetCounts`;
/// its two subclasses' `hasCounts()`/`getCount(int)` read
/// [`CountStore`].
pub(crate) struct AbstractSortedSetDocValueFacetCounts {
    pub(crate) state: Arc<dyn SortedSetDocValuesReaderState>,
    pub(crate) state_config: Arc<FacetsConfig>,
    /// `dv`, read through `lookupOrd`/`lookupTerm`, which move it.
    dv: Mutex<Option<Box<dyn SortedSetDocValues>>>,
    /// The `mapping` of `dv` when it is a `MultiSortedSetDocValues`.
    pub(crate) dv_mapping: Option<Arc<OrdinalMap>>,
    pub(crate) field: String,
    pub(crate) counts: CountStore,
}

impl AbstractSortedSetDocValueFacetCounts {
    /// Equivalent to `AbstractSortedSetDocValueFacetCounts(SortedSetDocValuesReaderState)`.
    pub(crate) fn new(
        state: Arc<dyn SortedSetDocValuesReaderState>,
        counts: CountStore,
    ) -> Result<Self> {
        let field = state.get_field().to_string();
        let state_config = Arc::clone(state.get_facets_config());
        let (dv, dv_mapping) = match state.get_doc_values()? {
            Some((dv, mapping)) => (Some(dv), mapping),
            None => (None, None),
        };
        Ok(Self {
            state,
            state_config,
            dv: Mutex::new(dv),
            dv_mapping,
            field,
            counts,
        })
    }

    fn dv(&self) -> MutexGuard<'_, Option<Box<dyn SortedSetDocValues>>> {
        self.dv.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lookup_ord(&self, ord: i32) -> Result<String> {
        let mut dv = self.dv();
        let dv = dv.as_mut().ok_or_else(LuceneError::null_pointer)?;
        dv.lookup_ord(ord as i64)?.utf8_to_string()
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i64> {
        let mut dv = self.dv();
        let dv = dv.as_mut().ok_or_else(LuceneError::null_pointer)?;
        dv.lookup_term(key)
    }

    /// Were any counts actually computed? (They may not be if there were no
    /// hits, etc.)
    ///
    /// Equivalent to `hasCounts()`.
    pub(crate) fn has_counts(&self) -> bool {
        match &self.counts {
            CountStore::Plain(counts) => counts.is_some(),
            CountStore::Atomic(_) => true,
        }
    }

    /// Retrieve the count for a specified ordinal.
    ///
    /// Equivalent to `getCount(int)`.
    pub(crate) fn get_count(&self, ord: i32) -> i32 {
        match &self.counts {
            CountStore::Plain(counts) => counts
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)
                .expect("NullPointerException: counts")[ord as usize],
            CountStore::Atomic(counts) => counts[ord as usize].load(AtomicOrdering::SeqCst),
        }
    }

    /// Equivalent to `getTopChildren(int, String, String...)`.
    pub(crate) fn get_top_children(
        &self,
        top_n: i32,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<FacetResult>> {
        validate_top_n(top_n)?;
        if !self.has_counts() {
            return Ok(None);
        }
        let top_children_for_path = self.get_top_children_for_path(top_n, dim, path)?;
        self.create_facet_result(top_children_for_path, dim, path)
    }

    /// Equivalent to `getAllChildren(String, String...)`.
    pub(crate) fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        let dim_config = self.state_config.get_dim_config(dim);

        let Some(mut iteration_cursor) = self.prepare_child_iteration(dim, &dim_config, path)?
        else {
            return Ok(None);
        };

        if !self.has_counts() {
            return Ok(None);
        }

        let mut path_count = 0i32;
        let mut label_values = Vec::new();
        while iteration_cursor.child_iterator.has_next() {
            let ord = iteration_cursor.child_iterator.next_int();
            let count = self.get_count(ord);
            if count > 0 {
                path_count = path_count.wrapping_add(count);
                let term = self.lookup_ord(ord)?;
                let parts = FacetsConfig::string_to_path(&term);
                labels_push(&mut label_values, parts, count);
            }
        }

        path_count =
            self.adjust_path_count_if_necessary(&dim_config, iteration_cursor.path_ord, path_count);

        let child_count = label_values.len() as i32;
        Ok(Some(FacetResult::new(
            dim,
            path.iter().map(|p| p.to_string()).collect(),
            NumericValue::Int(path_count),
            label_values,
            child_count,
        )))
    }

    /// Equivalent to `getSpecificValue(String, String...)`.
    pub(crate) fn get_specific_value(&self, dim: &str, path: &[&str]) -> Result<NumericValue> {
        if !self.state_config.get_dim_config(dim).hierarchical() && path.len() != 1 {
            return Err(LuceneError::illegal_argument(format!(
                "{dim} is not configured as hierarchical, path must be length=1"
            )));
        }
        let ord = self.lookup_term(&BytesRef::from_char_sequence(
            &FacetsConfig::path_to_string_dim(dim, path)?,
        ))? as i32;
        if ord < 0 {
            return Ok(NumericValue::Int(-1));
        }

        Ok(NumericValue::Int(if !self.has_counts() {
            0
        } else {
            self.get_count(ord)
        }))
    }

    /// Equivalent to `getAllDims(int)`.
    pub(crate) fn get_all_dims(&self, top_n: i32) -> Result<Vec<FacetResult>> {
        validate_top_n(top_n)?;
        if !self.has_counts() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        for dim in self.state.get_dims() {
            let top_children_for_path = self.get_top_children_for_path(top_n, &dim, &[])?;
            if let Some(facet_result) =
                self.create_facet_result(top_children_for_path, &dim, &[])?
            {
                results.push(facet_result);
            }
        }

        // Sort by highest count:
        results.sort_by(|a, b| {
            let (av, bv) = (number_int_value(&a.value), number_int_value(&b.value));
            if av > bv {
                Ordering::Less
            } else if bv > av {
                Ordering::Greater
            } else {
                java_compare_to(&a.dim, &b.dim)
            }
        });
        Ok(results)
    }

    /// Equivalent to `getTopDims(int, int)`.
    pub(crate) fn get_top_dims(
        &self,
        top_n_dims: i32,
        top_n_children: i32,
    ) -> Result<Vec<FacetResult>> {
        validate_top_n(top_n_dims)?;
        validate_top_n(top_n_children)?;

        if !self.has_counts() {
            return Ok(Vec::new());
        }

        // Creates priority queue to store top dimensions and sort by their
        // aggregated values/hits and string values.
        let mut pq: PriorityQueue<DimValue, DimValueLessThan> =
            PriorityQueue::new(top_n_dims as usize, DimValueLessThan)?;

        // Keep track of intermediate results, if we compute them, so we can
        // reuse them later:
        let mut intermediate_results: Option<HashMap<String, TopChildrenForPath>> = None;

        for dim in self.state.get_dims() {
            let dim_config = self.state_config.get_dim_config(&dim);
            let dim_count;
            if dim_config.hierarchical() {
                // For hierarchical dims, we directly index each level of the
                // ancestry path (i.e., we "rollup" at indexing time), meaning
                // we can directly access accurate dim counts without needing
                // to rollup the descendant paths:
                let dim_ord = self
                    .state
                    .get_dim_tree(&dim)?
                    .ok_or_else(LuceneError::null_pointer)?
                    .dim_start_ord;
                dim_count = self.get_count(dim_ord);
            } else {
                let ord_range = self
                    .state
                    .get_ord_range(&dim)?
                    .ok_or_else(LuceneError::null_pointer)?;
                let dim_ord = ord_range.start();
                if dim_config.multi_valued() {
                    if dim_config.require_dim_count() {
                        // If a dim is configured as multi-valued and requires
                        // dim counts, we index dim counts directly, so we can
                        // access accurate counts without needing to rollup
                        // children:
                        dim_count = self.get_count(dim_ord);
                    } else {
                        // If a dim is configured as multi-valued but _not_
                        // requiring dim counts, we have no way to get accurate
                        // counts. We use -1 to indicate this:
                        dim_count = -1;
                    }
                } else {
                    // If a dim is single-valued, we must aggregate child
                    // counts to get accurate dim counts. We don't index the
                    // dim counts directly:
                    let child_it = ChildOrdIterator::Range(ord_range.iterator());
                    let top_children_for_path =
                        self.compute_top_children(child_it, top_n_children, &dim_config, dim_ord)?;
                    dim_count = top_children_for_path.path_count;
                    intermediate_results
                        .get_or_insert_with(HashMap::new)
                        .insert(dim.clone(), top_children_for_path);
                }
            }

            // Check if we should insert this dim into the queue:
            if dim_count != 0 {
                if pq.size() < top_n_dims as usize {
                    pq.add(DimValue {
                        dim: dim.clone(),
                        value: dim_count,
                    });
                } else {
                    let top = pq.top().expect("INVARIANT: queue is full");
                    if dim_count > top.value
                        || (dim_count == top.value
                            && java_compare_to(&dim, &top.dim) == Ordering::Less)
                    {
                        let bottom_dim = pq.top_mut().expect("INVARIANT: queue is full");
                        bottom_dim.dim = dim.clone();
                        bottom_dim.value = dim_count;
                        pq.update_top();
                    }
                }
            }
        }

        // Now we populate the results:
        let mut result_size = pq.size();
        let mut results: Vec<Option<FacetResult>> = (0..result_size).map(|_| None).collect();

        while pq.size() > 0 {
            let dim_value = pq.pop().expect("INVARIANT: size > 0");
            let mut top_children_for_path = intermediate_results
                .as_mut()
                .and_then(|m| m.remove(&dim_value.dim));
            if top_children_for_path.is_none() {
                top_children_for_path =
                    self.get_top_children_for_path(top_n_children, &dim_value.dim, &[])?;
            }
            let facet_result =
                self.create_facet_result(top_children_for_path, &dim_value.dim, &[])?;
            // should not be null since only dims with non-zero values were
            // considered earlier
            debug_assert!(facet_result.is_some());
            result_size -= 1;
            results[result_size] = facet_result;
        }
        Ok(results.into_iter().flatten().collect())
    }

    /// Equivalent to `prepareChildIteration(String, DimConfig, String...)`.
    fn prepare_child_iteration(
        &self,
        dim: &str,
        dim_config: &DimConfig,
        path: &[&str],
    ) -> Result<Option<ChildIterationCursor>> {
        let path_ord;
        let mut child_iterator;
        if dim_config.hierarchical() {
            let dim_tree = self.state.get_dim_tree(dim)?;
            if !path.is_empty() {
                path_ord = self.lookup_term(&BytesRef::from_char_sequence(
                    &FacetsConfig::path_to_string_dim(dim, path)?,
                ))? as i32;
            } else {
                // If there's no path, this is a little more efficient to just
                // look up the dim:
                path_ord = dim_tree
                    .as_ref()
                    .ok_or_else(LuceneError::null_pointer)?
                    .dim_start_ord;
            }
            if path_ord < 0 {
                // path was never indexed
                return Ok(None);
            }
            child_iterator = ChildOrdIterator::Tree(
                dim_tree
                    .as_ref()
                    .ok_or_else(LuceneError::null_pointer)?
                    .iterator_from(path_ord),
            );
        } else {
            if !path.is_empty() {
                return Err(LuceneError::illegal_argument(
                    "Field is not configured as hierarchical, path should be 0 length",
                ));
            }
            let Some(ord_range) = self.state.get_ord_range(dim)? else {
                // means dimension was never indexed
                return Ok(None);
            };
            path_ord = ord_range.start();
            child_iterator = ChildOrdIterator::Range(ord_range.iterator());
            if dim_config.multi_valued() && dim_config.require_dim_count() {
                // If the dim is multi-valued and requires dim counts, we know
                // we've explicitly indexed the dimension and we need to skip
                // past it so the iterator is positioned on the first child:
                child_iterator.next_int();
            }
        }
        Ok(Some(ChildIterationCursor {
            path_ord,
            child_iterator,
        }))
    }

    /// Determine the top-n children for a specified dimension + path.
    ///
    /// Equivalent to `getTopChildrenForPath(int, String, String...)`.
    fn get_top_children_for_path(
        &self,
        top_n: i32,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<TopChildrenForPath>> {
        let dim_config = self.state_config.get_dim_config(dim);
        let Some(iteration_cursor) = self.prepare_child_iteration(dim, &dim_config, path)? else {
            return Ok(None);
        };

        // Compute the actual results:
        Ok(Some(self.compute_top_children(
            iteration_cursor.child_iterator,
            top_n,
            &dim_config,
            iteration_cursor.path_ord,
        )?))
    }

    /// Determine the top-n children for a specified dimension + path.
    ///
    /// Equivalent to `computeTopChildren(PrimitiveIterator.OfInt, int,
    /// DimConfig, int)`.
    fn compute_top_children(
        &self,
        mut child_ords: ChildOrdIterator,
        top_n: i32,
        dim_config: &DimConfig,
        path_ord: i32,
    ) -> Result<TopChildrenForPath> {
        let mut q: Option<TopOrdAndIntQueue> = None;
        let mut path_count = 0i32;
        let mut child_count = 0i32;

        let mut reuse: Option<Box<dyn OrdAndValue>> = None;
        while child_ords.has_next() {
            let ord = child_ords.next_int();
            let count = self.get_count(ord);
            if count > 0 {
                path_count = path_count.wrapping_add(count);
                child_count += 1;

                // Lazy init for top-N queue:
                if q.is_none() {
                    q = Some(TopOrdAndIntQueue::new(top_n)?);
                }
                let q = q.as_mut().expect("INVARIANT: initialized above");
                let mut r = reuse.take().unwrap_or_else(|| q.new_ord_and_value());
                {
                    let r = r
                        .as_any_mut()
                        .downcast_mut::<OrdAndInt>()
                        .expect("ClassCastException: not an OrdAndInt");
                    r.ord = ord;
                    r.value = count;
                }
                reuse = q.insert_with_overflow(r);
            }
        }

        path_count = self.adjust_path_count_if_necessary(dim_config, path_ord, path_count);

        Ok(TopChildrenForPath {
            path_count,
            child_count,
            q,
        })
    }

    /// Helper method to determine the actual path count to use in a facet
    /// result.
    ///
    /// Equivalent to `adjustPathCountIfNecessary(DimConfig, int, int)`.
    fn adjust_path_count_if_necessary(
        &self,
        dim_config: &DimConfig,
        path_ord: i32,
        computed_count: i32,
    ) -> i32 {
        if dim_config.hierarchical() {
            // For hierarchical dims, we directly index each level of the
            // ancestry path (i.e., we "rollup" at indexing time), meaning we
            // can directly access accurate dim counts without needing to
            // rollup the descendant paths:
            self.get_count(path_ord)
        } else if dim_config.multi_valued() {
            if dim_config.require_dim_count() {
                // If a dim is configured as multi-valued and requires dim
                // counts, we index dim counts directly, so we can access
                // accurate counts without needing to rollup children:
                self.get_count(path_ord)
            } else {
                // If a dim is configured as multi-valued but _not_ requiring
                // dim counts, we have no way to get accurate counts. We use -1
                // to indicate this:
                -1
            }
        } else {
            // If a dim is single-valued, we must aggregate child counts to
            // get accurate dim counts. We don't index the dim counts
            // directly:
            computed_count
        }
    }

    /// Create a FacetResult for the provided dim + path.
    ///
    /// Equivalent to `createFacetResult(TopChildrenForPath, String,
    /// String...)`.
    fn create_facet_result(
        &self,
        top_children_for_path: Option<TopChildrenForPath>,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<FacetResult>> {
        // If the intermediate result is null or there are no children, we
        // return null:
        let Some(top_children_for_path) = top_children_for_path else {
            return Ok(None);
        };
        if top_children_for_path.child_count == 0 {
            return Ok(None);
        }

        let mut q = top_children_for_path
            .q
            .expect("INVARIANT: children were counted");
        let len = q.size();
        let mut label_values: Vec<Option<LabelAndValue>> = (0..len).map(|_| None).collect();
        for i in (0..len).rev() {
            let ord_and_value = q.pop().expect("INVARIANT: queue has len entries");
            let ord_and_value = ord_and_value
                .as_any()
                .downcast_ref::<OrdAndInt>()
                .expect("ClassCastException: not an OrdAndInt");
            let term = self.lookup_ord(ord_and_value.ord)?;
            let parts = FacetsConfig::string_to_path(&term);
            label_values[i] = Some(LabelAndValue::new(
                parts[parts.len() - 1].clone(),
                NumericValue::Int(ord_and_value.value),
            ));
        }

        Ok(Some(FacetResult::new(
            dim,
            path.iter().map(|p| p.to_string()).collect(),
            NumericValue::Int(top_children_for_path.path_count),
            label_values.into_iter().flatten().collect(),
            top_children_for_path.child_count,
        )))
    }
}

fn labels_push(label_values: &mut Vec<LabelAndValue>, parts: Vec<String>, count: i32) {
    label_values.push(LabelAndValue::new(
        parts[parts.len() - 1].clone(),
        NumericValue::Int(count),
    ));
}

/// The doc iterator a counting loop walks: Java's `it`, which is the values
/// themselves, `FacetUtils.liveDocsDISI(valuesIt, liveDocs)`, or
/// `ConjunctionUtils.intersectIterators(List.of(hits.bits().iterator(),
/// valuesIt))`.
pub(crate) fn next_matching_doc(
    values: &mut dyn DocIdSetIterator,
    bits: Option<&mut Box<dyn DocIdSetIterator>>,
    live_docs: Option<&dyn Bits>,
) -> Result<i32> {
    if let Some(bits) = bits {
        return Intersect2::new(&mut **bits, values).next_doc();
    }
    let mut doc = values.next_doc()?;
    if let Some(live_docs) = live_docs {
        while doc != NO_MORE_DOCS && !live_docs.get(doc as usize) {
            doc = values.next_doc()?;
        }
    }
    Ok(doc)
}

/// Implements the `Facets` methods of a subclass by delegating to its base.
macro_rules! ssdv_facets_delegate {
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

            fn get_top_dims(
                &self,
                top_n_dims: i32,
                top_n_children: i32,
            ) -> rucene::error::Result<Vec<crate::facet_result::FacetResult>> {
                self.$base.get_top_dims(top_n_dims, top_n_children)
            }
        }
    };
}
pub(crate) use ssdv_facets_delegate;
