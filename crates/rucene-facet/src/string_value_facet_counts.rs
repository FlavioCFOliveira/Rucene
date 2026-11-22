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

//! Port of `StringValueFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::sync::{Arc, Mutex, PoisonError};

use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};
use rucene::index::multi_doc_values::MultiSortedSetDocValues;
use rucene::index::multi_reader::reader_util;
use rucene::index::{DocValues, IndexReader, IndexReaderContext, LeafReaderContext, OrdinalMap, SortedSetDocValues};
use rucene::internal::hppc::IntIntHashMap;
use rucene::search::NO_MORE_DOCS;
use rucene::util::{Bits, BytesRef};

use crate::facet_result::FacetResult;
use crate::facet_utils::{unwrap_singleton_sorted, SortedValuesView};
use crate::facets::{validate_top_n, Facets};
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::label_and_value::LabelAndValue;
use crate::sortedset::abstract_sorted_set_doc_value_facet_counts::next_matching_doc;
use crate::sortedset::sorted_set_doc_values_facet_counts::READER_MISMATCH;
use crate::string_doc_values_reader_state::StringDocValuesReaderState;
use crate::top_ord_and_int_queue::{OrdAndInt, TopOrdAndIntQueue};
use crate::top_ord_and_number_queue::OrdAndValue;

/// Compute facet counts from a previously indexed `SortedSetDocValues` or
/// `SortedDocValues` field.
///
/// Equivalent to `org.apache.lucene.facet.StringValueFacetCounts`. The
/// per-segment values Java reads out of the merged
/// `MultiSortedSetDocValues` (`multiValues.values[ord]`) are fresh,
/// unpositioned per-leaf values here, which is what Java's are when counting
/// starts.
pub struct StringValueFacetCounts {
    reader: Arc<dyn IndexReader>,
    field: String,
    ordinal_map: Option<Arc<OrdinalMap>>,
    /// `docValues`, read through `lookupOrd`/`lookupTerm`, which move it.
    doc_values: Mutex<Box<dyn SortedSetDocValues>>,
    dense_counts: Option<Vec<i32>>,
    sparse_counts: Option<IntIntHashMap>,
    initialized: bool,
    cardinality: i32,
    total_doc_count: i32,
}

impl StringValueFacetCounts {
    /// Returns all facet counts for the field, same result as searching on
    /// `MatchAllDocsQuery` but faster.
    ///
    /// Equivalent to `StringValueFacetCounts(StringDocValuesReaderState)`.
    pub fn new(state: &StringDocValuesReaderState) -> Result<Self> {
        Self::with_hits(state, None)
    }

    /// Counts facets across the provided hits.
    ///
    /// Equivalent to `StringValueFacetCounts(StringDocValuesReaderState,
    /// FacetsCollector)`.
    pub fn with_hits(state: &StringDocValuesReaderState, facets_collector: Option<&FacetsCollector>) -> Result<Self> {
        let reader = Arc::clone(&state.reader);
        let field = state.field.clone();
        let ordinal_map = state.ordinal_map.clone();
        let doc_values = Self::get_doc_values(&reader, &field, ordinal_map.as_ref())?;
        let value_count = doc_values.get_value_count()?;
        if value_count > i32::MAX as i64 {
            return Err(LuceneError::illegal_argument(format!(
                "can only handle valueCount < Integer.MAX_VALUE; got {value_count}"
            )));
        }
        let cardinality = value_count as i32;
        let mut this = Self {
            reader,
            field,
            ordinal_map,
            doc_values: Mutex::new(doc_values),
            dense_counts: None,
            sparse_counts: None,
            initialized: false,
            cardinality,
            total_doc_count: 0,
        };

        match facets_collector {
            Some(facets_collector) => {
                if cardinality < 1024 {
                    // count densely for low cardinality
                    this.sparse_counts = None;
                    this.dense_counts = None;
                    this.initialized = false;
                    this.count(facets_collector)?;
                } else {
                    let mut total_hits = 0i32;
                    let mut total_docs = 0i32;
                    for matching_docs in facets_collector.get_matching_docs() {
                        total_hits = total_hits.wrapping_add(matching_docs.total_hits);
                        total_docs = total_docs.wrapping_add(matching_docs.context.leaf_reader().max_doc());
                    }

                    // No counting needed if there are no hits:
                    if total_hits == 0 {
                        this.sparse_counts = None;
                        this.dense_counts = None;
                        this.initialized = true;
                    } else {
                        // If our result set is < 10% of the index, we collect
                        // sparse counts:
                        if total_hits < total_docs / 10 {
                            this.sparse_counts = Some(IntIntHashMap::new());
                            this.dense_counts = None;
                            this.initialized = true;
                        } else {
                            this.sparse_counts = None;
                            this.dense_counts = Some(vec![0; cardinality.max(0) as usize]);
                            this.initialized = true;
                        }
                        this.count(facets_collector)?;
                    }
                }
            }
            None => {
                // Since we're counting all ordinals, count densely:
                this.sparse_counts = None;
                this.dense_counts = Some(vec![0; cardinality.max(0) as usize]);
                this.initialized = true;

                this.count_all()?;
            }
        }
        Ok(this)
    }

    fn lookup_ord(&self, ord: i32) -> Result<String> {
        let mut dv = self.doc_values.lock().unwrap_or_else(PoisonError::into_inner);
        dv.lookup_ord(ord as i64)?.utf8_to_string()
    }

    /// Equivalent to `getDocValues()`.
    fn get_doc_values(
        reader: &Arc<dyn IndexReader>,
        field: &str,
        ordinal_map: Option<&Arc<OrdinalMap>>,
    ) -> Result<Box<dyn SortedSetDocValues>> {
        let leaves = Arc::clone(reader).leaves()?;
        let leaf_count = leaves.len();

        if leaf_count == 0 {
            return Ok(Box::new(DocValues::empty_sorted_set()));
        }

        if leaf_count == 1 {
            return DocValues::get_sorted_set(&*leaves[0].leaf_reader(), field);
        }

        // A good bit of this logic is forked from
        // MultiDocValues#getSortedSetValues
        let mut doc_values = Vec::with_capacity(leaf_count);
        let mut starts = vec![0i32; leaf_count + 1];
        let mut cost = 0i64;
        for (i, context) in leaves.iter().enumerate() {
            let dv = DocValues::get_sorted_set(&*context.leaf_reader(), field)?;
            cost += rucene::search::DocIdSetIterator::cost(&*dv);
            doc_values.push(dv);
            starts[i] = context.doc_base();
        }
        starts[leaf_count] = reader.max_doc();

        Ok(Box::new(MultiSortedSetDocValues::new(
            doc_values,
            starts,
            Arc::clone(ordinal_map.ok_or_else(LuceneError::null_pointer)?),
            cost,
        )))
    }

    /// Equivalent to `count(FacetsCollector)`.
    fn count(&mut self, facets_collector: &FacetsCollector) -> Result<()> {
        let matching_docs = facets_collector.get_matching_docs();

        if matching_docs.is_empty() {
            return Ok(());
        }

        if matching_docs.len() == 1 {
            let hits = &matching_docs[0];

            // Validate state before doing anything else. We only check
            // the first segment since they should all ladder up to the same
            // top-level reader:
            if hits.total_hits == 0 {
                return Ok(());
            }
            self.validate_state(&hits.context)?;

            // Assuming the state is valid, ordinalMap should be null since we
            // have one segment:
            debug_assert!(self.ordinal_map.is_none());

            let values = DocValues::get_sorted_set(&*hits.context.leaf_reader(), &self.field)?;
            self.count_one_segment(values, hits.context.ord(), Some(hits), None)?;
        } else {
            // Validate state before doing anything else. We only check the
            // first segment since they should all ladder up to the same
            // top-level reader:
            self.validate_state(&matching_docs[0].context)?;

            for hits in matching_docs {
                // Assuming the state is valid, ordinalMap should be non-null
                // and docValues should be a MultiSortedSetDocValues since we
                // have more than one segment:
                debug_assert!(self.ordinal_map.is_some());

                if hits.total_hits == 0 {
                    continue;
                }

                let values = DocValues::get_sorted_set(&*hits.context.leaf_reader(), &self.field)?;
                self.count_one_segment(values, hits.context.ord(), Some(hits), None)?;
            }
        }
        Ok(())
    }

    /// Equivalent to `countAll()`.
    fn count_all(&mut self) -> Result<()> {
        let leaves = Arc::clone(&self.reader).leaves()?;
        let num_leaves = leaves.len();
        if num_leaves == 0 {
            return Ok(());
        }

        if num_leaves == 1 {
            // Since we have a single segment, ordinalMap should be null:
            debug_assert!(self.ordinal_map.is_none());

            let context = &leaves[0];
            let leaf_reader = context.leaf_reader();
            let values = DocValues::get_sorted_set(&*leaf_reader, &self.field)?;
            match leaf_reader.get_live_docs() {
                None => self.count_one_segment_nhld(values, context.ord())?,
                Some(live_docs) => self.count_one_segment(values, context.ord(), None, Some(&*live_docs))?,
            }
        } else {
            // Since we have more than one segment, we should have a non-null
            // ordinalMap and docValues should be a MultiSortedSetDocValues
            // instance:
            debug_assert!(self.ordinal_map.is_some());

            for context in &leaves {
                let leaf_reader = context.leaf_reader();
                let values = DocValues::get_sorted_set(&*leaf_reader, &self.field)?;
                match leaf_reader.get_live_docs() {
                    None => self.count_one_segment_nhld(values, context.ord())?,
                    Some(live_docs) => self.count_one_segment(values, context.ord(), None, Some(&*live_docs))?,
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `countOneSegment(SortedSetDocValues, int, MatchingDocs,
    /// Bits)`.
    fn count_one_segment(
        &mut self,
        multi_values: Box<dyn SortedSetDocValues>,
        segment_ord: i32,
        hits: Option<&MatchingDocs>,
        live_docs: Option<&dyn Bits>,
    ) -> Result<()> {
        if !self.initialized {
            // If the counters weren't initialized, we can assume the
            // cardinality is low enough that dense counting will be preferable:
            debug_assert!(self.dense_counts.is_none() && self.sparse_counts.is_none());
            self.dense_counts = Some(vec![0; self.cardinality.max(0) as usize]);
            self.initialized = true;
        }

        let segment_cardinality = multi_values.get_value_count()? as i32;
        // It's slightly more efficient to work against SortedDocValues if the
        // field is actually single-valued (see: LUCENE-5309)
        let mut view = unwrap_singleton_sorted(multi_values)?;

        // Data structure to count. If we have no hits, we use the liveDocs:
        debug_assert!(hits.is_some() || live_docs.is_some());
        let mut bits = match hits {
            Some(hits) => Some(hits.bits.iterator()?),
            None => None,
        };

        macro_rules! next {
            ($values:expr) => {
                next_matching_doc(&mut **$values, bits.as_mut(), live_docs)?
            };
        }

        let ordinal_map = self.ordinal_map.clone();
        match ordinal_map {
            None => {
                // If there's no ordinal map we don't need to map segment
                // ordinals to globals, so counting is very straight-forward:
                match &mut view {
                    SortedValuesView::Single(single_values) => loop {
                        if next!(single_values) == NO_MORE_DOCS {
                            break;
                        }
                        let ord = single_values.ord_value()?;
                        self.increment(ord, 1);
                        self.total_doc_count += 1;
                    },
                    SortedValuesView::Multi(multi_values) => loop {
                        if next!(multi_values) == NO_MORE_DOCS {
                            break;
                        }
                        let mut counted_doc_in_total = false;
                        for _ in 0..multi_values.doc_value_count()? {
                            let term = multi_values.next_ord()? as i32;
                            self.increment(term, 1);
                            if !counted_doc_in_total {
                                self.total_doc_count += 1;
                                counted_doc_in_total = true;
                            }
                        }
                    },
                }
            }
            Some(ordinal_map) => {
                // We need to map segment ordinals to globals. We have two
                // different approaches to this depending on how "dense" the
                // requested hits are:
                let ord_map = ordinal_map.get_global_ords(segment_ord as usize);

                // If the matching hits are "sparse" compared to the segment
                // cardinality, map directly:
                if hits.is_some_and(|h| h.total_hits < segment_cardinality / 10) {
                    match &mut view {
                        SortedValuesView::Single(single_values) => loop {
                            if next!(single_values) == NO_MORE_DOCS {
                                break;
                            }
                            let ord = single_values.ord_value()?;
                            self.increment(ord_map.get(ord as i64) as i32, 1);
                            self.total_doc_count += 1;
                        },
                        SortedValuesView::Multi(multi_values) => loop {
                            if next!(multi_values) == NO_MORE_DOCS {
                                break;
                            }
                            let mut counted_doc_in_total = false;
                            for _ in 0..multi_values.doc_value_count()? {
                                let term = multi_values.next_ord()? as i32;
                                self.increment(ord_map.get(term as i64) as i32, 1);
                                if !counted_doc_in_total {
                                    self.total_doc_count += 1;
                                    counted_doc_in_total = true;
                                }
                            }
                        },
                    }
                } else {
                    // If the matching hits are "dense" compared to the
                    // segment cardinality, it's more efficient to count by
                    // segment ordinal and map to global ordinals once:
                    let mut seg_counts = vec![0i32; segment_cardinality.max(0) as usize];
                    match &mut view {
                        SortedValuesView::Single(single_values) => loop {
                            if next!(single_values) == NO_MORE_DOCS {
                                break;
                            }
                            seg_counts[single_values.ord_value()? as usize] += 1;
                            self.total_doc_count += 1;
                        },
                        SortedValuesView::Multi(multi_values) => loop {
                            if next!(multi_values) == NO_MORE_DOCS {
                                break;
                            }
                            let mut counted_doc_in_total = false;
                            for _ in 0..multi_values.doc_value_count()? {
                                let term = multi_values.next_ord()? as i32;
                                seg_counts[term as usize] += 1;
                                if !counted_doc_in_total {
                                    self.total_doc_count += 1;
                                    counted_doc_in_total = true;
                                }
                            }
                        },
                    }

                    for (ord, &count) in seg_counts.iter().enumerate() {
                        if count != 0 {
                            self.increment(ord_map.get(ord as i64) as i32, count);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `countOneSegmentNHLD(SortedSetDocValues, int)`.
    fn count_one_segment_nhld(&mut self, multi_values: Box<dyn SortedSetDocValues>, segment_ord: i32) -> Result<()> {
        // It's slightly more efficient to work against SortedDocValues if the
        // field is actually single-valued (see: LUCENE-5309)
        let segment_cardinality = multi_values.get_value_count()? as i32;
        let view = unwrap_singleton_sorted(multi_values)?;

        let ordinal_map = self.ordinal_map.clone();
        match ordinal_map {
            None => match view {
                SortedValuesView::Single(mut single_values) => {
                    let mut doc = single_values.next_doc()?;
                    while doc != NO_MORE_DOCS {
                        let ord = single_values.ord_value()?;
                        self.increment(ord, 1);
                        self.total_doc_count += 1;
                        doc = single_values.next_doc()?;
                    }
                }
                SortedValuesView::Multi(mut multi_values) => {
                    let mut doc = multi_values.next_doc()?;
                    while doc != NO_MORE_DOCS {
                        let mut counted_doc_in_total = false;
                        for _ in 0..multi_values.doc_value_count()? {
                            let term = multi_values.next_ord()? as i32;
                            self.increment(term, 1);
                            if !counted_doc_in_total {
                                self.total_doc_count += 1;
                                counted_doc_in_total = true;
                            }
                        }
                        doc = multi_values.next_doc()?;
                    }
                }
            },
            Some(ordinal_map) => {
                // We need to map segment ordinals to globals. Since we're
                // counting all docs (there's no "hits" sparsity here), count
                // by segment ordinal and map once:
                let ord_map = ordinal_map.get_global_ords(segment_ord as usize);
                let mut seg_counts = vec![0i32; segment_cardinality.max(0) as usize];
                match view {
                    SortedValuesView::Single(mut single_values) => {
                        let mut doc = single_values.next_doc()?;
                        while doc != NO_MORE_DOCS {
                            seg_counts[single_values.ord_value()? as usize] += 1;
                            self.total_doc_count += 1;
                            doc = single_values.next_doc()?;
                        }
                    }
                    SortedValuesView::Multi(mut multi_values) => {
                        let mut doc = multi_values.next_doc()?;
                        while doc != NO_MORE_DOCS {
                            let mut counted_doc_in_total = false;
                            for _ in 0..multi_values.doc_value_count()? {
                                let term = multi_values.next_ord()? as i32;
                                seg_counts[term as usize] += 1;
                                if !counted_doc_in_total {
                                    self.total_doc_count += 1;
                                    counted_doc_in_total = true;
                                }
                            }
                            doc = multi_values.next_doc()?;
                        }
                    }
                }

                for (ord, &count) in seg_counts.iter().enumerate() {
                    if count != 0 {
                        self.increment(ord_map.get(ord as i64) as i32, count);
                    }
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `increment(int, int)`.
    fn increment(&mut self, ordinal: i32, amount: i32) {
        if let Some(sparse_counts) = &mut self.sparse_counts {
            sparse_counts.add_to(ordinal, amount);
        } else {
            self.dense_counts.as_mut().expect("INVARIANT: initialized")[ordinal as usize] += amount;
        }
    }

    /// Equivalent to `validateState(LeafReaderContext)`.
    fn validate_state(&self, context: &Arc<LeafReaderContext>) -> Result<()> {
        // LUCENE-5090: make sure the provided reader context "matches"
        // the top-level reader passed to the
        // SortedSetDocValuesReaderState, else cryptic
        // AIOOBE can happen:
        let top = reader_util::get_top_level_context(Arc::clone(context) as Arc<dyn IndexReaderContext>);
        if !std::ptr::addr_eq(Arc::as_ptr(&top.reader()), Arc::as_ptr(&self.reader)) {
            return Err(LuceneError::illegal_state(READER_MISMATCH));
        }
        Ok(())
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

impl Facets for StringValueFacetCounts {
    fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        self.validate_dim_and_path_for_get_children(dim, path)?;

        let mut label_values = Vec::new();

        if let Some(sparse_counts) = &self.sparse_counts {
            for sparse_count in sparse_counts.iter() {
                let count = sparse_count.value;
                let term = self.lookup_ord(sparse_count.key)?;
                label_values.push(LabelAndValue::new(term, NumericValue::Int(count)));
            }
        } else if let Some(dense_counts) = &self.dense_counts {
            for (i, &count) in dense_counts.iter().enumerate() {
                if count != 0 {
                    let term = self.lookup_ord(i as i32)?;
                    label_values.push(LabelAndValue::new(term, NumericValue::Int(count)));
                }
            }
        }

        let child_count = label_values.len() as i32;
        Ok(Some(FacetResult::new(
            self.field.clone(),
            Vec::new(),
            NumericValue::Int(self.total_doc_count),
            label_values,
            child_count,
        )))
    }

    fn get_top_children(&self, top_n: i32, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        validate_top_n(top_n)?;
        self.validate_dim_and_path_for_get_children(dim, path)?;

        let top_n = top_n.min(self.cardinality);
        let mut q: Option<TopOrdAndIntQueue> = None;
        let mut reuse: Option<Box<dyn OrdAndValue>> = None;
        let mut child_count = 0i32; // total number of labels with non-zero count

        let mut insert = |q: &mut Option<TopOrdAndIntQueue>, ord: i32, count: i32| -> Result<()> {
            if q.is_none() {
                // Lazy-init for sparse case:
                *q = Some(TopOrdAndIntQueue::new(top_n)?);
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
            Ok(())
        };

        if let Some(sparse_counts) = &self.sparse_counts {
            for sparse_count in sparse_counts.iter() {
                child_count += 1; // every count in sparseValues should be non-zero
                insert(&mut q, sparse_count.key, sparse_count.value)?;
            }
        } else if let Some(dense_counts) = &self.dense_counts {
            for (i, &count) in dense_counts.iter().enumerate() {
                if count != 0 {
                    child_count += 1;
                    insert(&mut q, i as i32, count)?;
                }
            }
        }

        let result_count = q.as_ref().map_or(0, |q| q.size());
        let mut label_values: Vec<Option<LabelAndValue>> = (0..result_count).map(|_| None).collect();
        for i in (0..result_count).rev() {
            let ord_and_value = q.as_mut().expect("INVARIANT: non-empty").pop().expect("INVARIANT: size");
            let ord_and_value = ord_and_value
                .as_any()
                .downcast_ref::<OrdAndInt>()
                .expect("ClassCastException: not an OrdAndInt");
            let term = self.lookup_ord(ord_and_value.ord)?;
            label_values[i] = Some(LabelAndValue::new(term, NumericValue::Int(ord_and_value.value)));
        }

        Ok(Some(FacetResult::new(
            self.field.clone(),
            Vec::new(),
            NumericValue::Int(self.total_doc_count),
            label_values.into_iter().flatten().collect(),
            child_count,
        )))
    }

    fn get_specific_value(&self, dim: &str, path: &[&str]) -> Result<NumericValue> {
        if dim != self.field {
            return Err(LuceneError::illegal_argument(format!(
                "invalid dim \"{dim}\"; should be \"{}\"",
                self.field
            )));
        }
        if path.len() != 1 {
            return Err(LuceneError::illegal_argument("path must be length=1"));
        }
        let ord = {
            let mut dv = self.doc_values.lock().unwrap_or_else(PoisonError::into_inner);
            dv.lookup_term(&BytesRef::from_char_sequence(path[0]))? as i32
        };
        if ord < 0 {
            return Ok(NumericValue::Int(-1));
        }

        if let Some(sparse_counts) = &self.sparse_counts {
            return Ok(NumericValue::Int(sparse_counts.get(ord)));
        }
        if let Some(dense_counts) = &self.dense_counts {
            return Ok(NumericValue::Int(dense_counts[ord as usize]));
        }

        Ok(NumericValue::Int(0))
    }

    fn get_all_dims(&self, top_n: i32) -> Result<Vec<FacetResult>> {
        validate_top_n(top_n)?;
        Ok(self.get_top_children(top_n, &self.field, &[])?.into_iter().collect())
    }
}
