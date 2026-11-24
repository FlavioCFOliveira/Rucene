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

//! Port of `SortedSetDocValuesFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.sortedset`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::multi_reader::reader_util;
use rucene::index::{DocValues, IndexReaderContext, LeafReader, OrdinalMap};
use rucene::search::NO_MORE_DOCS;
use rucene::util::Bits;

use crate::facet_utils::{unwrap_singleton_sorted, SortedValuesView};
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::sortedset::abstract_sorted_set_doc_value_facet_counts::{
    next_matching_doc, ssdv_facets_delegate, AbstractSortedSetDocValueFacetCounts, CountStore,
};
use crate::sortedset::sorted_set_doc_values_reader_state::SortedSetDocValuesReaderState;

/// Compute facets counts from previously indexed
/// `SortedSetDocValuesFacetField`, without require a separate taxonomy index.
///
/// Equivalent to `org.apache.lucene.facet.sortedset.SortedSetDocValuesFacetCounts`.
pub struct SortedSetDocValuesFacetCounts {
    base: AbstractSortedSetDocValueFacetCounts,
}

/// The message both count paths throw on a mismatched reader.
pub(crate) const READER_MISMATCH: &str = "the SortedSetDocValuesReaderState provided to this class does not match the reader being searched; you must create a new SortedSetDocValuesReaderState every time you open a new IndexReader";

/// `ReaderUtil.getTopLevelContext(hits.context()).reader() != reader`.
pub(crate) fn check_top_level_reader(
    hits: &MatchingDocs,
    state: &dyn SortedSetDocValuesReaderState,
) -> Result<()> {
    let top = reader_util::get_top_level_context(
        Arc::clone(&hits.context) as Arc<dyn IndexReaderContext>
    );
    if !std::ptr::addr_eq(Arc::as_ptr(&top.reader()), Arc::as_ptr(state.get_reader())) {
        return Err(LuceneError::illegal_state(READER_MISMATCH));
    }
    Ok(())
}

impl SortedSetDocValuesFacetCounts {
    /// Returns all facet counts for the field, same result as searching on
    /// `MatchAllDocsQuery` but faster.
    ///
    /// Equivalent to `SortedSetDocValuesFacetCounts(SortedSetDocValuesReaderState)`.
    pub fn new(state: Arc<dyn SortedSetDocValuesReaderState>) -> Result<Self> {
        Self::with_hits(state, None)
    }

    /// Counts all facet dimensions across the provided hits.
    ///
    /// Equivalent to `SortedSetDocValuesFacetCounts(SortedSetDocValuesReaderState,
    /// FacetsCollector)`.
    pub fn with_hits(
        state: Arc<dyn SortedSetDocValuesReaderState>,
        hits: Option<&FacetsCollector>,
    ) -> Result<Self> {
        let base = AbstractSortedSetDocValueFacetCounts::new(state, CountStore::Plain(None))?;
        let mut this = Self { base };
        match hits {
            None => this.count_all()?,
            Some(hits) => this.count(hits.get_matching_docs()?)?,
        }
        Ok(this)
    }

    /// Equivalent to `initializeCounts()`.
    fn initialize_counts(&mut self) {
        if let CountStore::Plain(counts) = &mut self.base.counts {
            if counts.is_none() {
                *counts = Some(vec![0; self.base.state.get_size().max(0) as usize]);
            }
        }
    }

    fn counts(&mut self) -> &mut Vec<i32> {
        match &mut self.base.counts {
            CountStore::Plain(Some(counts)) => counts,
            _ => unreachable!("counts initialized"),
        }
    }

    /// Equivalent to `countOneSegmentNHLD(OrdinalMap, LeafReader, int)`.
    fn count_one_segment_nhld(
        &mut self,
        ordinal_map: Option<&Arc<OrdinalMap>>,
        reader: &dyn LeafReader,
        seg_ord: i32,
    ) -> Result<()> {
        let multi_values = DocValues::get_sorted_set(reader, &self.base.field)?;

        // It's slightly more efficient to work against SortedDocValues if the
        // field is actually single-valued (see: LUCENE-5309)
        self.initialize_counts();
        let value_count = multi_values.get_value_count()?;
        let view = unwrap_singleton_sorted(multi_values)?;

        // TODO: yet another option is to count all segs first, only in seg-ord
        // space, and then do a merge-sort-PQ in the end to only "resolve to
        // global" those seg ords that can compete, if we know we just need
        // top N?

        if let Some(ordinal_map) = ordinal_map {
            // First count in seg-ord space:
            let ord_map = ordinal_map.get_global_ords(seg_ord as usize);
            let num_seg_ords = value_count as i32;
            let mut seg_counts = vec![0i32; num_seg_ords.max(0) as usize];
            match view {
                SortedValuesView::Single(mut single_values) => {
                    let mut doc = single_values.next_doc()?;
                    while doc != NO_MORE_DOCS {
                        seg_counts[single_values.ord_value()? as usize] += 1;
                        doc = single_values.next_doc()?;
                    }
                }
                SortedValuesView::Multi(mut multi_values) => {
                    let mut doc = multi_values.next_doc()?;
                    while doc != NO_MORE_DOCS {
                        for _ in 0..multi_values.doc_value_count()? {
                            let term = multi_values.next_ord()? as i32;
                            seg_counts[term as usize] += 1;
                        }
                        doc = multi_values.next_doc()?;
                    }
                }
            }

            // Then, migrate to global ords:
            let counts = self.counts();
            for (ord, &count) in seg_counts.iter().enumerate() {
                if count != 0 {
                    // ordinalMap.getGlobalOrd(segOrd, ord));
                    counts[ord_map.get(ord as i64) as i32 as usize] += count;
                }
            }
        } else {
            // No ord mapping (e.g., single segment index):
            // just aggregate directly into counts:
            match view {
                SortedValuesView::Single(mut single_values) => {
                    let mut doc = single_values.next_doc()?;
                    while doc != NO_MORE_DOCS {
                        let ord = single_values.ord_value()?;
                        self.counts()[ord as usize] += 1;
                        doc = single_values.next_doc()?;
                    }
                }
                SortedValuesView::Multi(mut multi_values) => {
                    let mut doc = multi_values.next_doc()?;
                    while doc != NO_MORE_DOCS {
                        for _ in 0..multi_values.doc_value_count()? {
                            let term = multi_values.next_ord()? as i32;
                            self.counts()[term as usize] += 1;
                        }
                        doc = multi_values.next_doc()?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `countOneSegment(OrdinalMap, LeafReader, int,
    /// MatchingDocs, Bits)`.
    fn count_one_segment(
        &mut self,
        ordinal_map: Option<&Arc<OrdinalMap>>,
        reader: &dyn LeafReader,
        seg_ord: i32,
        hits: Option<&MatchingDocs>,
        live_docs: Option<&dyn Bits>,
    ) -> Result<()> {
        if let Some(hits) = hits {
            if hits.total_hits == 0 {
                return Ok(());
            }
        }

        let multi_values = DocValues::get_sorted_set(reader, &self.base.field)?;

        // It's slightly more efficient to work against SortedDocValues if the
        // field is actually single-valued (see: LUCENE-5309)
        self.initialize_counts();
        let value_count = multi_values.get_value_count()?;
        let mut view = unwrap_singleton_sorted(multi_values)?;

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

        if let Some(ordinal_map) = ordinal_map {
            let ord_map = ordinal_map.get_global_ords(seg_ord as usize);

            let num_seg_ords = value_count as i32;

            if hits.is_some_and(|h| h.total_hits < num_seg_ords / 10) {
                // Remap every ord to global ord as we iterate:
                match &mut view {
                    SortedValuesView::Single(single_values) => loop {
                        if next!(single_values) == NO_MORE_DOCS {
                            break;
                        }
                        let ord = single_values.ord_value()?;
                        let global = ord_map.get(ord as i64) as i32;
                        self.counts()[global as usize] += 1;
                    },
                    SortedValuesView::Multi(multi_values) => loop {
                        if next!(multi_values) == NO_MORE_DOCS {
                            break;
                        }
                        for _ in 0..multi_values.doc_value_count()? {
                            let term = multi_values.next_ord()? as i32;
                            let global = ord_map.get(term as i64) as i32;
                            self.counts()[global as usize] += 1;
                        }
                    },
                }
            } else {
                // First count in seg-ord space:
                let mut seg_counts = vec![0i32; num_seg_ords.max(0) as usize];
                match &mut view {
                    SortedValuesView::Single(single_values) => loop {
                        if next!(single_values) == NO_MORE_DOCS {
                            break;
                        }
                        seg_counts[single_values.ord_value()? as usize] += 1;
                    },
                    SortedValuesView::Multi(multi_values) => loop {
                        if next!(multi_values) == NO_MORE_DOCS {
                            break;
                        }
                        for _ in 0..multi_values.doc_value_count()? {
                            let term = multi_values.next_ord()? as i32;
                            seg_counts[term as usize] += 1;
                        }
                    },
                }

                // Then, migrate to global ords:
                let counts = self.counts();
                for (ord, &count) in seg_counts.iter().enumerate() {
                    if count != 0 {
                        // ordinalMap.getGlobalOrd(segOrd, ord));
                        counts[ord_map.get(ord as i64) as i32 as usize] += count;
                    }
                }
            }
        } else {
            // No ord mapping (e.g., single segment index):
            // just aggregate directly into counts:
            match &mut view {
                SortedValuesView::Single(single_values) => loop {
                    if next!(single_values) == NO_MORE_DOCS {
                        break;
                    }
                    let ord = single_values.ord_value()?;
                    self.counts()[ord as usize] += 1;
                },
                SortedValuesView::Multi(multi_values) => loop {
                    if next!(multi_values) == NO_MORE_DOCS {
                        break;
                    }
                    for _ in 0..multi_values.doc_value_count()? {
                        let term = multi_values.next_ord()? as i32;
                        self.counts()[term as usize] += 1;
                    }
                },
            }
        }
        Ok(())
    }

    /// Does all the "real work" of tallying up the counts.
    ///
    /// Equivalent to `count(List)`.
    fn count(&mut self, matching_docs: &[MatchingDocs]) -> Result<()> {
        let ordinal_map = if self.base.dv_mapping.is_some() && matching_docs.len() > 1 {
            self.base.dv_mapping.clone()
        } else {
            None
        };

        let state = Arc::clone(&self.base.state);
        for hits in matching_docs {
            // LUCENE-5090: make sure the provided reader context "matches" the
            // top-level reader passed to the SortedSetDocValuesReaderState,
            // else cryptic AIOOBE can happen:
            check_top_level_reader(hits, &*state)?;

            let reader = hits.context.leaf_reader();
            self.count_one_segment(
                ordinal_map.as_ref(),
                &*reader,
                hits.context.ord(),
                Some(hits),
                None,
            )?;
        }
        Ok(())
    }

    /// Does all the "real work" of tallying up the counts.
    ///
    /// Equivalent to `countAll()`.
    fn count_all(&mut self) -> Result<()> {
        let ordinal_map = self.base.dv_mapping.clone();
        let reader = Arc::clone(self.base.state.get_reader());
        for context in reader.leaves()? {
            let leaf_reader = context.leaf_reader();
            let live_docs = leaf_reader.get_live_docs();
            match live_docs {
                None => {
                    self.count_one_segment_nhld(ordinal_map.as_ref(), &*leaf_reader, context.ord())?
                }
                Some(live_docs) => self.count_one_segment(
                    ordinal_map.as_ref(),
                    &*leaf_reader,
                    context.ord(),
                    None,
                    Some(&*live_docs),
                )?,
            }
        }
        Ok(())
    }
}

ssdv_facets_delegate!(SortedSetDocValuesFacetCounts, base);
