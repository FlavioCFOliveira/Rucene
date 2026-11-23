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

//! Port of `ConcurrentSortedSetDocValuesFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.sortedset`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{DocValues, LeafReader, OrdinalMap};
use rucene::search::task_executor::{Executor, Future};
use rucene::search::NO_MORE_DOCS;

use crate::facet_utils::{unwrap_singleton_sorted, SortedValuesView};
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::sortedset::abstract_sorted_set_doc_value_facet_counts::{
    next_matching_doc, ssdv_facets_delegate, AbstractSortedSetDocValueFacetCounts, CountStore,
};
use crate::sortedset::sorted_set_doc_values_facet_counts::check_top_level_reader;
use crate::sortedset::sorted_set_doc_values_reader_state::SortedSetDocValuesReaderState;

/// Like `SortedSetDocValuesFacetCounts`, but aggregates counts concurrently
/// across segments.
///
/// Equivalent to `org.apache.lucene.facet.sortedset.ConcurrentSortedSetDocValuesFacetCounts`.
pub struct ConcurrentSortedSetDocValuesFacetCounts {
    base: AbstractSortedSetDocValueFacetCounts,
    exec: Arc<dyn Executor>,
}

impl ConcurrentSortedSetDocValuesFacetCounts {
    /// Returns all facet counts, same result as searching on
    /// `MatchAllDocsQuery` but faster.
    ///
    /// Equivalent to `ConcurrentSortedSetDocValuesFacetCounts(SortedSetDocValuesReaderState,
    /// ExecutorService)`.
    pub fn new(state: Arc<dyn SortedSetDocValuesReaderState>, exec: Arc<dyn Executor>) -> Result<Self> {
        Self::with_hits(state, None, exec)
    }

    /// Counts all facet dimensions across the provided hits.
    ///
    /// Equivalent to `ConcurrentSortedSetDocValuesFacetCounts(SortedSetDocValuesReaderState,
    /// FacetsCollector, ExecutorService)`.
    pub fn with_hits(
        state: Arc<dyn SortedSetDocValuesReaderState>,
        hits: Option<&FacetsCollector>,
        exec: Arc<dyn Executor>,
    ) -> Result<Self> {
        let size = state.get_size().max(0) as usize;
        let counts: Arc<[AtomicI32]> = (0..size).map(|_| AtomicI32::new(0)).collect();
        let base = AbstractSortedSetDocValueFacetCounts::new(state, CountStore::Atomic(counts))?;
        let this = Self { base, exec };
        match hits {
            None => this.count_all()?,
            Some(hits) => this.count(hits.get_matching_docs()?)?,
        }
        Ok(this)
    }

    fn counts(&self) -> Arc<[AtomicI32]> {
        match &self.base.counts {
            CountStore::Atomic(counts) => Arc::clone(counts),
            CountStore::Plain(_) => unreachable!("atomic counts"),
        }
    }

    fn submit(&self, task: CountOneSegment) -> Result<Future> {
        self.exec
            .submit(Box::new(move || {
                task.call()?;
                Ok(Box::new(()) as rucene::search::task_executor::TaskValue)
            }))
            .map_err(|e| LuceneError::unchecked("java.util.concurrent.RejectedExecutionException", e.message))
    }

    /// Equivalent to `count(List)`.
    fn count(&self, matching_docs: &[MatchingDocs]) -> Result<()> {
        let ordinal_map = if self.base.dv_mapping.is_some() && matching_docs.len() > 1 {
            self.base.dv_mapping.clone()
        } else {
            None
        };

        let mut results = Vec::new();
        for hits in matching_docs {
            // LUCENE-5090: make sure the provided reader context "matches" the
            // top-level reader passed to the SortedSetDocValuesReaderState,
            // else cryptic AIOOBE can happen:
            check_top_level_reader(hits, &*self.base.state)?;

            results.push(self.submit(CountOneSegment::new(
                hits.context.leaf_reader(),
                Some(hits.clone()),
                ordinal_map.clone(),
                hits.context.ord(),
                self.base.field.clone(),
                self.counts(),
            ))?);
        }

        for result in results {
            // `IOUtils.rethrowAlways(ee.getCause())`.
            result.get()?;
        }
        Ok(())
    }

    /// Does all the "real work" of tallying up the counts.
    ///
    /// Equivalent to `countAll()`.
    fn count_all(&self) -> Result<()> {
        let ordinal_map = self.base.dv_mapping.clone();

        let mut results = Vec::new();
        for context in Arc::clone(self.base.state.get_reader()).leaves()? {
            results.push(self.submit(CountOneSegment::new(
                context.leaf_reader(),
                None,
                ordinal_map.clone(),
                context.ord(),
                self.base.field.clone(),
                self.counts(),
            ))?);
        }

        for result in results {
            result.get()?;
        }
        Ok(())
    }
}

ssdv_facets_delegate!(ConcurrentSortedSetDocValuesFacetCounts, base);

/// Equivalent to the inner class
/// `org.apache.lucene.facet.sortedset.ConcurrentSortedSetDocValuesFacetCounts.CountOneSegment`,
/// which reads its enclosing instance's `field` and `counts`.
pub(crate) struct CountOneSegment {
    leaf_reader: Arc<dyn LeafReader>,
    hits: Option<MatchingDocs>,
    ordinal_map: Option<Arc<OrdinalMap>>,
    seg_ord: i32,
    field: String,
    counts: Arc<[AtomicI32]>,
}

impl CountOneSegment {
    /// Equivalent to `CountOneSegment(LeafReader, MatchingDocs, OrdinalMap,
    /// int)`.
    fn new(
        leaf_reader: Arc<dyn LeafReader>,
        hits: Option<MatchingDocs>,
        ordinal_map: Option<Arc<OrdinalMap>>,
        seg_ord: i32,
        field: String,
        counts: Arc<[AtomicI32]>,
    ) -> Self {
        Self {
            leaf_reader,
            hits,
            ordinal_map,
            seg_ord,
            field,
            counts,
        }
    }

    fn increment(&self, ord: i32) {
        self.counts[ord as usize].fetch_add(1, Ordering::SeqCst);
    }

    /// Equivalent to `call()`.
    fn call(&self) -> Result<()> {
        if let Some(hits) = &self.hits {
            if hits.total_hits == 0 {
                return Ok(());
            }
        }

        let multi_values = DocValues::get_sorted_set(&*self.leaf_reader, &self.field)?;

        // It's slightly more efficient to work against SortedDocValues if the
        // field is actually single-valued (see: LUCENE-5309)
        let value_count = multi_values.get_value_count()?;
        let mut view = unwrap_singleton_sorted(multi_values)?;

        let mut bits = match &self.hits {
            Some(hits) => Some(hits.bits.iterator()?),
            None => None,
        };
        let live_docs = if self.hits.is_none() {
            self.leaf_reader.get_live_docs()
        } else {
            None
        };

        macro_rules! next {
            ($values:expr) => {
                next_matching_doc(&mut **$values, bits.as_mut(), live_docs.as_deref())?
            };
        }

        if let Some(ordinal_map) = &self.ordinal_map {
            let ord_map = ordinal_map.get_global_ords(self.seg_ord as usize);

            let num_seg_ords = value_count as i32;

            if self.hits.as_ref().is_some_and(|h| h.total_hits < num_seg_ords / 10) {
                // Remap every ord to global ord as we iterate:
                match &mut view {
                    SortedValuesView::Single(single_values) => loop {
                        if next!(single_values) == NO_MORE_DOCS {
                            break;
                        }
                        self.increment(ord_map.get(single_values.ord_value()? as i64) as i32);
                    },
                    SortedValuesView::Multi(multi_values) => loop {
                        if next!(multi_values) == NO_MORE_DOCS {
                            break;
                        }
                        for _ in 0..multi_values.doc_value_count()? {
                            let term = multi_values.next_ord()? as i32;
                            self.increment(ord_map.get(term as i64) as i32);
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
                for (ord, &count) in seg_counts.iter().enumerate() {
                    if count != 0 {
                        self.counts[ord_map.get(ord as i64) as i32 as usize]
                            .fetch_add(count, Ordering::SeqCst);
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
                    self.increment(single_values.ord_value()?);
                },
                SortedValuesView::Multi(multi_values) => loop {
                    if next!(multi_values) == NO_MORE_DOCS {
                        break;
                    }
                    for _ in 0..multi_values.doc_value_count()? {
                        let term = multi_values.next_ord()? as i32;
                        self.increment(term);
                    }
                },
            }
        }
        Ok(())
    }
}
