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

//! Port of `FastTaxonomyFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::search::NO_MORE_DOCS;

use crate::facet_utils::{unwrap_singleton_numeric, Intersect2, NumericValuesView};
use crate::facets_collector::{FacetsCollector, MatchingDocs};
use crate::facets_config::{FacetsConfig, DEFAULT_INDEX_FIELD_NAME};
use crate::taxonomy::taxonomy_facets::{taxonomy_facets_delegate, TaxonomyFacets, ValueStore};
use crate::taxonomy::taxonomy_reader::TaxonomyReader;

/// Computes facets counts, assuming the default encoding into DocValues was
/// used.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.FastTaxonomyFacetCounts`.
pub struct FastTaxonomyFacetCounts {
    base: TaxonomyFacets,
}

impl FastTaxonomyFacetCounts {
    /// Create `FastTaxonomyFacetCounts`, which also counts all facet labels.
    ///
    /// Equivalent to `FastTaxonomyFacetCounts(TaxonomyReader, FacetsConfig,
    /// FacetsCollector)`.
    pub fn new(
        taxo_reader: Arc<dyn TaxonomyReader>,
        config: Arc<FacetsConfig>,
        fc: &FacetsCollector,
    ) -> Result<Self> {
        Self::with_index_field_name(DEFAULT_INDEX_FIELD_NAME, taxo_reader, config, fc)
    }

    /// Create `FastTaxonomyFacetCounts`, using the specified
    /// `index_field_name` for ordinals.
    ///
    /// Equivalent to `FastTaxonomyFacetCounts(String, TaxonomyReader,
    /// FacetsConfig, FacetsCollector)`.
    pub fn with_index_field_name(
        index_field_name: &str,
        taxo_reader: Arc<dyn TaxonomyReader>,
        config: Arc<FacetsConfig>,
        fc: &FacetsCollector,
    ) -> Result<Self> {
        let base = TaxonomyFacets::new(index_field_name, taxo_reader, config, Some(fc), ValueStore::Counts)?;
        let mut this = Self { base };
        this.count(fc.get_matching_docs())?;
        Ok(this)
    }

    /// Create `FastTaxonomyFacetCounts`, using the specified
    /// `index_field_name` for ordinals, and counting all non-deleted
    /// documents in the index.
    ///
    /// Equivalent to `FastTaxonomyFacetCounts(String, IndexReader,
    /// TaxonomyReader, FacetsConfig)`.
    pub fn count_all_docs(
        index_field_name: &str,
        reader: &Arc<dyn IndexReader>,
        taxo_reader: Arc<dyn TaxonomyReader>,
        config: Arc<FacetsConfig>,
    ) -> Result<Self> {
        let base = TaxonomyFacets::new(index_field_name, taxo_reader, config, None, ValueStore::Counts)?;
        let mut this = Self { base };
        this.count_all(reader)?;
        Ok(this)
    }

    /// Equivalent to `count(List)`.
    fn count(&mut self, matching_docs: &[MatchingDocs]) -> Result<()> {
        for hits in matching_docs {
            if hits.total_hits == 0 {
                continue;
            }
            let Some(multi_valued) = hits
                .context
                .leaf_reader()
                .get_sorted_numeric_doc_values(&self.base.index_field_name)?
            else {
                continue;
            };
            self.base.initialize_value_counters()?;

            let mut bits_it = hits.bits.iterator()?;
            // `ConjunctionUtils.intersectIterators(Arrays.asList(hits.bits().iterator(), valuesIt))`;
            // the two-iterator conjunction is rebuilt for each step so that the
            // doc values stay readable between steps (it keeps no state).
            match unwrap_singleton_numeric(multi_valued)? {
                NumericValuesView::Single(mut single_valued) => loop {
                    let doc = Intersect2::new(&mut *bits_it, &mut *single_valued).next_doc()?;
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let ord = single_valued.long_value()? as i32;
                    self.increment(ord);
                },
                NumericValuesView::Multi(mut multi_valued) => loop {
                    let doc = Intersect2::new(&mut *bits_it, &mut *multi_valued).next_doc()?;
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    for _ in 0..multi_valued.doc_value_count()? {
                        let ord = multi_valued.next_value()? as i32;
                        self.increment(ord);
                    }
                },
            }
        }

        self.base.rollup()
    }

    fn increment(&mut self, ord: i32) {
        if let Some(counts) = &mut self.base.counts {
            counts[ord as usize] += 1;
        } else {
            self.base
                .sparse_counts
                .as_mut()
                .expect("INVARIANT: initialized")
                .add_to(ord, 1);
        }
    }

    /// Equivalent to `countAll(IndexReader)`.
    fn count_all(&mut self, reader: &Arc<dyn IndexReader>) -> Result<()> {
        for context in Arc::clone(reader).leaves()? {
            let leaf_reader = context.leaf_reader();
            let Some(multi_valued) =
                leaf_reader.get_sorted_numeric_doc_values(&self.base.index_field_name)?
            else {
                continue;
            };
            self.base.initialize_value_counters()?;
            debug_assert!(self.base.counts.is_some());
            let counts = self.base.counts.as_mut().expect("INVARIANT: dense when counting all");

            let live_docs = leaf_reader.get_live_docs();

            match unwrap_singleton_numeric(multi_valued)? {
                NumericValuesView::Single(mut single_valued) => {
                    let mut doc = single_valued.next_doc()?;
                    while doc != NO_MORE_DOCS {
                        if live_docs.as_ref().is_none_or(|l| l.get(doc as usize)) {
                            counts[single_valued.long_value()? as i32 as usize] += 1;
                        }
                        doc = single_valued.next_doc()?;
                    }
                }
                NumericValuesView::Multi(mut multi_valued) => {
                    let mut doc = multi_valued.next_doc()?;
                    while doc != NO_MORE_DOCS {
                        if live_docs.as_ref().is_none_or(|l| l.get(doc as usize)) {
                            for _ in 0..multi_valued.doc_value_count()? {
                                counts[multi_valued.next_value()? as i32 as usize] += 1;
                            }
                        }
                        doc = multi_valued.next_doc()?;
                    }
                }
            }
        }

        self.base.rollup()
    }
}

taxonomy_facets_delegate!(FastTaxonomyFacetCounts, base);
