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

//! Port of `DrillSideways.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::multi_collector::MultiCollector;
use rucene::search::multi_collector_manager::{AnyCollector, MultiCollectorHandle};
use rucene::search::task_executor::{Executor, TaskValue};
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::top_field_docs::TopFieldDocs;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::{CollectorManager, FieldDoc, IndexSearcher, Query, ScoreDoc, Sort, TopDocs};

use crate::drill_down_query::DrillDownQuery;
use crate::drill_sideways_query::DrillSidewaysQuery;
use crate::facets::Facets;
use crate::facets_collector::FacetsCollector;
use crate::facets_collector_manager::{FacetsCollectorManager, FacetsTopDocs};
use crate::facets_config::FacetsConfig;
use crate::multi_facets::MultiFacets;
use crate::sortedset::sorted_set_doc_values_facet_counts::SortedSetDocValuesFacetCounts;
use crate::sortedset::sorted_set_doc_values_reader_state::SortedSetDocValuesReaderState;
use crate::taxonomy::fast_taxonomy_facet_counts::FastTaxonomyFacetCounts;
use crate::taxonomy::taxonomy_reader::TaxonomyReader;

/// The protected methods of `DrillSideways` a subclass overrides:
/// `createDrillDownFacetsCollectorManager`,
/// `createDrillSidewaysFacetsCollectorManager`, `buildFacetsResult` and
/// `scoreSubDocsAtOnce`. The defaults are the base class's bodies.
pub trait DrillSidewaysOverrides: Send + Sync {
    /// Subclass can override to customize drill down facets collector;
    /// `None` for Java's `null` (no drill down facets collector).
    ///
    /// Equivalent to `createDrillDownFacetsCollectorManager()`.
    fn create_drill_down_facets_collector_manager(&self) -> Option<FacetsCollectorManager> {
        Some(FacetsCollectorManager::new())
    }

    /// Subclass can override to customize drill sideways facets collector.
    ///
    /// Equivalent to `createDrillSidewaysFacetsCollectorManager()`.
    fn create_drill_sideways_facets_collector_manager(&self) -> FacetsCollectorManager {
        FacetsCollectorManager::new()
    }

    /// Subclass can override to customize per-dim Facets impl.
    ///
    /// Equivalent to `buildFacetsResult(FacetsCollector, FacetsCollector[],
    /// String[])`.
    fn build_facets_result(
        &self,
        drill_sideways: &DrillSideways,
        drill_downs: Option<&FacetsCollector>,
        drill_sideways_collectors: Option<&[FacetsCollector]>,
        drill_sideways_dims: Option<&[String]>,
    ) -> Result<Option<Arc<dyn Facets>>> {
        drill_sideways.default_build_facets_result(drill_downs, drill_sideways_collectors, drill_sideways_dims)
    }

    /// Override this and return true if your collector (e.g.,
    /// `ToParentBlockJoinCollector`) expects all sub-scorers to be positioned
    /// on the document being collected.
    ///
    /// Equivalent to `scoreSubDocsAtOnce()`.
    fn score_sub_docs_at_once(&self) -> bool {
        false
    }
}

/// The base class's own overrides.
struct DefaultOverrides;

impl DrillSidewaysOverrides for DefaultOverrides {}

/// Computes drill down and sideways counts for the provided
/// `DrillDownQuery`.
///
/// Equivalent to `org.apache.lucene.facet.DrillSideways`.
pub struct DrillSideways {
    /// `IndexSearcher` passed to constructor.
    pub searcher: Arc<IndexSearcher>,
    /// `TaxonomyReader` passed to constructor.
    pub taxo_reader: Option<Arc<dyn TaxonomyReader>>,
    /// `SortedSetDocValuesReaderState` passed to constructor; can be `None`.
    pub state: Option<Arc<dyn SortedSetDocValuesReaderState>>,
    /// `FacetsConfig` passed to constructor.
    pub config: Arc<FacetsConfig>,
    /// Executor for concurrent searches.
    executor: Option<Arc<dyn Executor>>,
    overrides: Arc<dyn DrillSidewaysOverrides>,
}

/// Result of a drill sideways search, including the `Facets` and `TopDocs`.
///
/// Equivalent to `org.apache.lucene.facet.DrillSideways.DrillSidewaysResult`.
pub struct DrillSidewaysResult {
    /// Combined drill down and sideways results.
    pub facets: Option<Arc<dyn Facets>>,
    /// Hits.
    pub hits: Option<FacetsTopDocs>,
    /// Collector to compute Facets on drill down dimensions.
    pub drill_down_facets_collector: Option<FacetsCollector>,
    /// Drill sideways facet collectors, one per dimension.
    pub drill_sideways_facets_collector: Option<Vec<FacetsCollector>>,
    /// Drill sideways dimension names, in the same order as the collectors.
    pub drill_sideways_dims: Option<Vec<String>>,
}

impl DrillSidewaysResult {
    /// Sole constructor.
    ///
    /// Equivalent to `DrillSidewaysResult(Facets, TopDocs, FacetsCollector,
    /// FacetsCollector[], String[])`.
    pub fn new(
        facets: Option<Arc<dyn Facets>>,
        hits: Option<FacetsTopDocs>,
        drill_down_facets_collector: Option<FacetsCollector>,
        drill_sideways_facets_collector: Option<Vec<FacetsCollector>>,
        drill_sideways_dims: Option<Vec<String>>,
    ) -> Self {
        Self {
            facets,
            hits,
            drill_down_facets_collector,
            drill_sideways_facets_collector,
            drill_sideways_dims,
        }
    }
}

/// Result of a concurrent drill sideways search, including the `Facets` and
/// the collector result.
///
/// Equivalent to `org.apache.lucene.facet.DrillSideways.ConcurrentDrillSidewaysResult`,
/// a `DrillSidewaysResult` subclass.
pub struct ConcurrentDrillSidewaysResult<R> {
    /// The `DrillSidewaysResult` fields.
    pub base: DrillSidewaysResult,
    /// Result of the hit collector manager.
    pub collector_result: R,
}

/// Result of a drill sideways search.
///
/// Equivalent to the record `org.apache.lucene.facet.DrillSideways.Result`.
pub struct DrillSidewaysResultRecord<T, R> {
    /// Result of the drill down collector manager.
    pub drill_down_result: T,
    /// Results of the drill sideways collector managers, `None` without
    /// dimensions.
    pub drill_sideways_results: Option<Vec<R>>,
}

/// A callable that searches with a collector manager.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.DrillSideways.CallableCollector`.
pub(crate) struct CallableCollector<M: CollectorManager> {
    searcher: Arc<IndexSearcher>,
    query: Option<Arc<dyn Query>>,
    collector_manager: Arc<M>,
}

impl<M> CallableCollector<M>
where
    M: CollectorManager + Send + Sync + 'static,
    M::Collector: Send + 'static,
    M::Output: Send + 'static,
{
    /// Equivalent to `call()`.
    fn call(self) -> Result<M::Output> {
        let query = self.query.ok_or_else(LuceneError::null_pointer)?;
        self.searcher.search_with_manager(query, &*self.collector_manager)
    }
}

impl DrillSideways {
    /// Create a new `DrillSideways` instance.
    ///
    /// Equivalent to `DrillSideways(IndexSearcher, FacetsConfig,
    /// TaxonomyReader)`.
    pub fn new(searcher: Arc<IndexSearcher>, config: Arc<FacetsConfig>, taxo_reader: Arc<dyn TaxonomyReader>) -> Self {
        Self::with_all(searcher, config, Some(taxo_reader), None, None)
    }

    /// Create a new `DrillSideways` instance, assuming the categories were
    /// indexed with `SortedSetDocValuesFacetField`.
    ///
    /// Equivalent to `DrillSideways(IndexSearcher, FacetsConfig,
    /// SortedSetDocValuesReaderState)`.
    pub fn with_state(
        searcher: Arc<IndexSearcher>,
        config: Arc<FacetsConfig>,
        state: Arc<dyn SortedSetDocValuesReaderState>,
    ) -> Self {
        Self::with_all(searcher, config, None, Some(state), None)
    }

    /// Create a new `DrillSideways` instance, where some dimensions were
    /// indexed with `SortedSetDocValuesFacetField` and others were indexed
    /// with `FacetField`.
    ///
    /// Equivalent to `DrillSideways(IndexSearcher, FacetsConfig,
    /// TaxonomyReader, SortedSetDocValuesReaderState)`.
    pub fn with_taxonomy_and_state(
        searcher: Arc<IndexSearcher>,
        config: Arc<FacetsConfig>,
        taxo_reader: Option<Arc<dyn TaxonomyReader>>,
        state: Option<Arc<dyn SortedSetDocValuesReaderState>>,
    ) -> Self {
        Self::with_all(searcher, config, taxo_reader, state, None)
    }

    /// Create a new `DrillSideways` instance, where some dimensions were
    /// indexed with `SortedSetDocValuesFacetField` and others were indexed
    /// with `FacetField`. Use this constructor to use the concurrent
    /// implementation.
    ///
    /// Equivalent to `DrillSideways(IndexSearcher, FacetsConfig,
    /// TaxonomyReader, SortedSetDocValuesReaderState, ExecutorService)`.
    pub fn with_all(
        searcher: Arc<IndexSearcher>,
        config: Arc<FacetsConfig>,
        taxo_reader: Option<Arc<dyn TaxonomyReader>>,
        state: Option<Arc<dyn SortedSetDocValuesReaderState>>,
        executor: Option<Arc<dyn Executor>>,
    ) -> Self {
        Self::with_overrides(searcher, config, taxo_reader, state, executor, Arc::new(DefaultOverrides))
    }

    /// The constructor of a subclass that overrides the protected methods
    /// listed by [`DrillSidewaysOverrides`].
    pub fn with_overrides(
        searcher: Arc<IndexSearcher>,
        config: Arc<FacetsConfig>,
        taxo_reader: Option<Arc<dyn TaxonomyReader>>,
        state: Option<Arc<dyn SortedSetDocValuesReaderState>>,
        executor: Option<Arc<dyn Executor>>,
        overrides: Arc<dyn DrillSidewaysOverrides>,
    ) -> Self {
        Self {
            searcher,
            taxo_reader,
            state,
            config,
            executor,
            overrides,
        }
    }

    /// The base class's `buildFacetsResult`.
    pub fn default_build_facets_result(
        &self,
        drill_downs: Option<&FacetsCollector>,
        drill_sideways: Option<&[FacetsCollector]>,
        drill_sideways_dims: Option<&[String]>,
    ) -> Result<Option<Arc<dyn Facets>>> {
        let mut drill_down_facets: Option<Arc<dyn Facets>> = None;
        let mut drill_sideways_facets: Vec<(String, Arc<dyn Facets>)> = Vec::new();

        if let Some(taxo_reader) = &self.taxo_reader {
            if let Some(drill_downs) = drill_downs {
                drill_down_facets = Some(Arc::new(FastTaxonomyFacetCounts::new(
                    Arc::clone(taxo_reader),
                    Arc::clone(&self.config),
                    drill_downs,
                )?));
            }
            if let (Some(drill_sideways), Some(dims)) = (drill_sideways, drill_sideways_dims) {
                for (i, collector) in drill_sideways.iter().enumerate() {
                    drill_sideways_facets.push((
                        dims[i].clone(),
                        Arc::new(FastTaxonomyFacetCounts::new(
                            Arc::clone(taxo_reader),
                            Arc::clone(&self.config),
                            collector,
                        )?),
                    ));
                }
            }
        } else {
            let state = || self.state.as_ref().ok_or_else(LuceneError::null_pointer);
            if let Some(drill_downs) = drill_downs {
                drill_down_facets = Some(Arc::new(SortedSetDocValuesFacetCounts::with_hits(
                    Arc::clone(state()?),
                    Some(drill_downs),
                )?));
            }
            if let (Some(drill_sideways), Some(dims)) = (drill_sideways, drill_sideways_dims) {
                for (i, collector) in drill_sideways.iter().enumerate() {
                    drill_sideways_facets.push((
                        dims[i].clone(),
                        Arc::new(SortedSetDocValuesFacetCounts::with_hits(
                            Arc::clone(state()?),
                            Some(collector),
                        )?),
                    ));
                }
            }
        }

        if drill_sideways_facets.is_empty() {
            Ok(drill_down_facets)
        } else {
            // `new HashMap<>()` of the dims: the dims are distinct.
            Ok(Some(Arc::new(MultiFacets::with_default(drill_sideways_facets, drill_down_facets))))
        }
    }

    /// Search, sorting by `Sort`, and computing drill down and sideways
    /// counts.
    ///
    /// Equivalent to `search(DrillDownQuery, Query, FieldDoc, int, Sort,
    /// boolean)`.
    pub fn search_sorted(
        &self,
        query: DrillDownQuery,
        filter: Option<Arc<dyn Query>>,
        after: Option<FieldDoc>,
        top_n: i32,
        sort: Option<&Sort>,
        do_doc_scores: bool,
    ) -> Result<DrillSidewaysResult> {
        let query = match filter {
            Some(filter) => DrillDownQuery::with_filter(Arc::clone(&self.config), filter, &query)?,
            None => query,
        };
        match sort {
            Some(sort) => {
                let mut limit = self.searcher.get_index_reader().max_doc();
                if limit == 0 {
                    limit = 1; // the collector does not alow numHits = 0
                }
                let f_top_n = top_n.min(limit);
                let collector_manager = TopFieldCollectorManager::new(sort.clone(), f_top_n, after, i32::MAX)?;
                let query = Arc::new(query);
                let r = self.search_with_hit_manager(&query, collector_manager)?;

                let mut top_docs: TopFieldDocs = r.collector_result;
                if do_doc_scores {
                    let mut docs = FacetsTopDocs::TopFieldDocs(top_docs);
                    docs.populate_scores(&self.searcher, Arc::clone(&query) as Arc<dyn Query>)?;
                    let FacetsTopDocs::TopFieldDocs(d) = docs else {
                        unreachable!("built above");
                    };
                    top_docs = d;
                }
                Ok(DrillSidewaysResult::new(
                    r.base.facets,
                    Some(FacetsTopDocs::TopFieldDocs(top_docs)),
                    r.base.drill_down_facets_collector,
                    r.base.drill_sideways_facets_collector,
                    r.base.drill_sideways_dims,
                ))
            }
            None => self.search_after(after.map(|f| f.score_doc), query, top_n),
        }
    }

    /// Search, sorting by score, and computing drill down and sideways
    /// counts.
    ///
    /// Equivalent to `search(DrillDownQuery, int)`.
    pub fn search(&self, query: DrillDownQuery, top_n: i32) -> Result<DrillSidewaysResult> {
        self.search_after(None, query, top_n)
    }

    /// Search, sorting by score, and computing drill down and sideways
    /// counts.
    ///
    /// Equivalent to `search(ScoreDoc, DrillDownQuery, int)`.
    pub fn search_after(&self, after: Option<ScoreDoc>, query: DrillDownQuery, top_n: i32) -> Result<DrillSidewaysResult> {
        let mut limit = self.searcher.get_index_reader().max_doc();
        if limit == 0 {
            limit = 1; // the collector does not alow numHits = 0
        }
        let f_top_n = top_n.min(limit);
        let collector_manager = TopScoreDocCollectorManager::new(f_top_n, after, i32::MAX)?;
        let r = self.search_with_hit_manager(&Arc::new(query), collector_manager)?;
        let hits: TopDocs = r.collector_result;
        Ok(DrillSidewaysResult::new(
            r.base.facets,
            Some(FacetsTopDocs::TopDocs(hits)),
            r.base.drill_down_facets_collector,
            r.base.drill_sideways_facets_collector,
            r.base.drill_sideways_dims,
        ))
    }

    /// Override this and return true if your collector expects all
    /// sub-scorers to be positioned on the document being collected.
    ///
    /// Equivalent to `scoreSubDocsAtOnce()`.
    pub fn score_sub_docs_at_once(&self) -> bool {
        self.overrides.score_sub_docs_at_once()
    }

    /// Equivalent to `getDrillDownQuery(DrillDownQuery, Query[], String)`.
    fn get_drill_down_query(
        &self,
        query: &DrillDownQuery,
        queries: &[Arc<dyn Query>],
        excluded_dimension: &str,
    ) -> Result<Option<DrillDownQuery>> {
        let mut ddl = DrillDownQuery::with_base_query(Arc::clone(&self.config), query.get_base_query().cloned());
        for (dim, pos) in query.get_dims() {
            if dim != excluded_dimension {
                ddl.add_query(dim, Arc::clone(&queries[*pos as usize]))?;
            }
        }
        Ok(if ddl.get_dims().len() == queries.len() {
            None
        } else {
            Some(ddl)
        })
    }

    /// Runs a search, using a `CollectorManager` to gather and merge search
    /// results.
    ///
    /// Equivalent to `search(DrillDownQuery, CollectorManager)`.
    pub fn search_with_hit_manager<H>(
        &self,
        query: &Arc<DrillDownQuery>,
        hit_collector_manager: H,
    ) -> Result<ConcurrentDrillSidewaysResult<H::Output>>
    where
        H: CollectorManager + Send + Sync + 'static,
        H::Collector: Send + 'static,
        H::Output: Send + 'static,
    {
        let drill_down_facets_collector_manager = self.overrides.create_drill_down_facets_collector_manager();

        let num_dims = query.get_dims().len();
        let drill_sideways_collector_managers: Option<Vec<FacetsCollectorManager>> = if num_dims != 0 {
            Some(
                (0..num_dims)
                    .map(|_| self.overrides.create_drill_sideways_facets_collector_manager())
                    .collect(),
            )
        } else {
            None
        };

        let facets_collector_result: Option<FacetsCollector>;
        let hit_collector_result: H::Output;
        let drill_sideways_results: Option<Vec<FacetsCollector>>;
        match drill_down_facets_collector_manager {
            Some(fcm) => {
                // `new MultiCollectorManager(drillDownFacetsCollectorManager,
                // hitCollectorManager)`.
                let main_collector_manager = SendMultiCollectorManager::new(vec![
                    Box::new(fcm) as Box<dyn SendErasedManager>,
                    Box::new(hit_collector_manager),
                ]);
                let result = self.run(query, main_collector_manager, drill_sideways_collector_managers)?;
                let mut drill_down_result = result.drill_down_result;
                let hit = drill_down_result.pop().ok_or_else(LuceneError::null_pointer)?;
                let fc = drill_down_result.pop().ok_or_else(LuceneError::null_pointer)?;
                facets_collector_result = Some(
                    *fc.downcast::<FacetsCollector>()
                        .map_err(|_| LuceneError::illegal_state("ClassCastException"))?,
                );
                hit_collector_result = *hit
                    .downcast::<H::Output>()
                    .map_err(|_| LuceneError::illegal_state("ClassCastException"))?;
                drill_sideways_results = result.drill_sideways_results;
            }
            None => {
                let result = self.run(query, hit_collector_manager, drill_sideways_collector_managers)?;
                facets_collector_result = None;
                hit_collector_result = result.drill_down_result;
                drill_sideways_results = result.drill_sideways_results;
            }
        }

        // drill-sideways facets collectors and dims
        let (drill_sideways_dims, drill_sideways_collectors) = if num_dims != 0 {
            let dims: Vec<String> = query.get_dims().iter().map(|(d, _)| d.clone()).collect();
            let collectors = drill_sideways_results.ok_or_else(LuceneError::null_pointer)?;
            debug_assert!(collectors.len() == num_dims);
            (Some(dims), Some(collectors))
        } else {
            (None, None)
        };

        let facets = self.overrides.build_facets_result(
            self,
            facets_collector_result.as_ref(),
            drill_sideways_collectors.as_deref(),
            drill_sideways_dims.as_deref(),
        )?;
        Ok(ConcurrentDrillSidewaysResult {
            base: DrillSidewaysResult::new(
                facets,
                None,
                facets_collector_result,
                drill_sideways_collectors,
                drill_sideways_dims,
            ),
            collector_result: hit_collector_result,
        })
    }

    /// `executor != null ? searchConcurrently(...) : searchSequentially(...)`.
    fn run<C, M>(
        &self,
        query: &Arc<DrillDownQuery>,
        drill_down_collector_manager: C,
        drill_sideways_collector_managers: Option<Vec<M>>,
    ) -> Result<DrillSidewaysResultRecord<C::Output, M::Output>>
    where
        C: CollectorManager + Send + Sync + 'static,
        C::Collector: Send + 'static,
        C::Output: Send + 'static,
        M: CollectorManager + Send + Sync + 'static,
        M::Collector: Send + 'static,
        M::Output: Send + 'static,
    {
        if self.executor.is_some() {
            self.search_concurrently(query, drill_down_collector_manager, drill_sideways_collector_managers)
        } else {
            self.search_sequentially(query, drill_down_collector_manager, drill_sideways_collector_managers)
        }
    }

    /// Runs a search, using a `CollectorManager` to gather and merge search
    /// results.
    ///
    /// Equivalent to `search(DrillDownQuery, CollectorManager, List)`.
    pub fn search_with_managers<C, M>(
        &self,
        query: &Arc<DrillDownQuery>,
        drill_down_collector_manager: Option<C>,
        drill_sideways_collector_managers: Option<Vec<M>>,
    ) -> Result<DrillSidewaysResultRecord<C::Output, M::Output>>
    where
        C: CollectorManager + Send + Sync + 'static,
        C::Collector: Send + 'static,
        C::Output: Send + 'static,
        M: CollectorManager + Send + Sync + 'static,
        M::Collector: Send + 'static,
        M::Output: Send + 'static,
    {
        let Some(drill_down_collector_manager) = drill_down_collector_manager else {
            return Err(LuceneError::illegal_argument(
                "This search method requires client to provide drill down collector manager",
            ));
        };
        match &drill_sideways_collector_managers {
            None => {
                if !query.get_dims().is_empty() {
                    return Err(LuceneError::illegal_argument(
                        "The query requires not null drillSidewaysCollectorManagers",
                    ));
                }
            }
            Some(managers) => {
                if managers.len() != query.get_dims().len() {
                    return Err(LuceneError::illegal_argument(
                        "drillSidewaysCollectorManagers size must be equal to number of dimensions in the query.",
                    ));
                }
            }
        }
        if self.executor.is_some() {
            self.search_concurrently(query, drill_down_collector_manager, drill_sideways_collector_managers)
        } else {
            self.search_sequentially(query, drill_down_collector_manager, drill_sideways_collector_managers)
        }
    }

    /// Equivalent to `searchSequentially(DrillDownQuery, CollectorManager,
    /// List)`.
    fn search_sequentially<C, M>(
        &self,
        query: &Arc<DrillDownQuery>,
        drill_down_collector_manager: C,
        drill_sideways_collector_managers: Option<Vec<M>>,
    ) -> Result<DrillSidewaysResultRecord<C::Output, M::Output>>
    where
        C: CollectorManager + Send + Sync + 'static,
        C::Collector: Send + 'static,
        M: CollectorManager + Send + Sync + 'static,
        M::Collector: Send + 'static,
    {
        let drill_down_dims = query.get_dims();

        if drill_down_dims.is_empty() {
            // There are no drill-down dims, so there is no drill-sideways to
            // compute:
            return Ok(DrillSidewaysResultRecord {
                drill_down_result: self
                    .searcher
                    .search_with_manager(Arc::clone(query) as Arc<dyn Query>, &drill_down_collector_manager)?,
                drill_sideways_results: None,
            });
        }

        let base_query: Arc<dyn Query> = query
            .get_base_query()
            .cloned()
            .unwrap_or_else(|| Arc::new(MatchAllDocsQuery::instance()));
        let drill_down_queries = query.get_drill_down_queries();

        let managers = Arc::new(
            drill_sideways_collector_managers
                .expect("Case without drill sideways dimensions is handled above"),
        );
        let dsq = DrillSidewaysQuery::new(
            base_query,
            Arc::clone(&managers),
            drill_down_queries,
            self.score_sub_docs_at_once(),
        );
        let managed = Arc::clone(&dsq.managed_drill_sideways_collectors);

        let collector_result = self
            .searcher
            .search_with_manager(Arc::new(dsq), &drill_down_collector_manager)?;

        let managed = std::mem::take(&mut *managed.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let num_slices = managed.len();
        let mut per_dim: Vec<Vec<M::Collector>> = (0..drill_down_dims.len()).map(|_| Vec::with_capacity(num_slices)).collect();
        for slice in managed {
            for (dim, slot) in slice.into_iter().enumerate() {
                let collector = Arc::try_unwrap(slot)
                    .map_err(|_| LuceneError::illegal_state("a sideways collector is still in use"))?
                    .into_inner()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                per_dim[dim].push(collector);
            }
        }
        let mut drill_sideways_results = Vec::with_capacity(drill_down_dims.len());
        for (dim, collectors_for_dim) in per_dim.into_iter().enumerate() {
            drill_sideways_results.push(managers[dim].reduce(collectors_for_dim)?);
        }

        Ok(DrillSidewaysResultRecord {
            drill_down_result: collector_result,
            drill_sideways_results: Some(drill_sideways_results),
        })
    }

    /// Equivalent to `searchConcurrently(DrillDownQuery, CollectorManager,
    /// List)`; `ExecutorService.invokeAll` is the submission of every task
    /// followed by waiting on each.
    fn search_concurrently<C, M>(
        &self,
        query: &Arc<DrillDownQuery>,
        drill_down_collector_manager: C,
        drill_sideways_collector_managers: Option<Vec<M>>,
    ) -> Result<DrillSidewaysResultRecord<C::Output, M::Output>>
    where
        C: CollectorManager + Send + Sync + 'static,
        C::Collector: Send + 'static,
        C::Output: Send + 'static,
        M: CollectorManager + Send + Sync + 'static,
        M::Collector: Send + 'static,
        M::Output: Send + 'static,
    {
        let executor = self.executor.as_ref().ok_or_else(LuceneError::null_pointer)?;
        let drill_down_dims = query.get_dims();
        let drill_down_callable_collector = CallableCollector {
            searcher: Arc::clone(&self.searcher),
            query: Some(Arc::clone(query) as Arc<dyn Query>),
            collector_manager: Arc::new(drill_down_collector_manager),
        };
        let mut drill_sideways_callable_collectors = Vec::with_capacity(drill_down_dims.len());
        let filters = query.get_drill_down_queries();
        let managers = drill_sideways_collector_managers.unwrap_or_default();
        let mut managers = managers.into_iter();
        for (dim, _) in drill_down_dims {
            drill_sideways_callable_collectors.push(CallableCollector {
                searcher: Arc::clone(&self.searcher),
                query: self
                    .get_drill_down_query(query, &filters, dim)?
                    .map(|q| Arc::new(q) as Arc<dyn Query>),
                collector_manager: Arc::new(managers.next().ok_or_else(LuceneError::null_pointer)?),
            });
        }

        let rejected = |e: rucene::search::task_executor::RejectedExecutionException| {
            LuceneError::unchecked("java.util.concurrent.RejectedExecutionException", e.message)
        };
        let drill_down_future = executor
            .submit(Box::new(move || {
                Ok(Box::new(drill_down_callable_collector.call()?) as TaskValue)
            }))
            .map_err(rejected)?;
        let mut drill_sideways_futures = Vec::with_capacity(drill_sideways_callable_collectors.len());
        for callable in drill_sideways_callable_collectors {
            drill_sideways_futures.push(
                executor
                    .submit(Box::new(move || Ok(Box::new(callable.call()?) as TaskValue)))
                    .map_err(rejected)?,
            );
        }

        // `ExecutionException` is rethrown as a `RuntimeException`.
        let collector_result = *drill_down_future
            .get()
            .map_err(LuceneError::runtime_exception)?
            .downcast::<C::Output>()
            .map_err(|_| LuceneError::illegal_state("ClassCastException"))?;
        let mut drill_sideways_results = Vec::with_capacity(drill_sideways_futures.len());
        for future in drill_sideways_futures {
            drill_sideways_results.push(
                *future
                    .get()
                    .map_err(LuceneError::runtime_exception)?
                    .downcast::<M::Output>()
                    .map_err(|_| LuceneError::illegal_state("ClassCastException"))?,
            );
        }

        Ok(DrillSidewaysResultRecord {
            drill_down_result: collector_result,
            drill_sideways_results: Some(drill_sideways_results),
        })
    }
}

/// A collector manager whose reduced result is sendable, erased: one
/// sub-manager of [`SendMultiCollectorManager`].
trait SendErasedManager: Send + Sync {
    fn new_any(&self) -> Result<AnyCollector>;
    fn reduce_any(&self, collectors: Vec<Box<dyn Any + Send>>) -> Result<Box<dyn Any + Send>>;
}

impl<M> SendErasedManager for M
where
    M: CollectorManager + Send + Sync,
    M::Collector: Send + 'static,
    M::Output: Send + 'static,
{
    fn new_any(&self) -> Result<AnyCollector> {
        Ok(AnyCollector::new(CollectorManager::new_collector(self)?))
    }

    fn reduce_any(&self, collectors: Vec<Box<dyn Any + Send>>) -> Result<Box<dyn Any + Send>> {
        let typed: Vec<M::Collector> = collectors
            .into_iter()
            .map(|c| *c.downcast::<M::Collector>().expect("INVARIANT: collectors of this manager"))
            .collect();
        Ok(Box::new(CollectorManager::reduce(self, typed)?))
    }
}

/// `MultiCollectorManager` over managers whose results are sendable, so that
/// the drill-down search may run on the executor.
///
/// Equivalent to the `MultiCollectorManager(drillDownFacetsCollectorManager,
/// hitCollectorManager)` of `search(DrillDownQuery, CollectorManager)`: the
/// same `MultiCollector` collection, its results `Object[]`.
struct SendMultiCollectorManager {
    collector_managers: Vec<Box<dyn SendErasedManager>>,
}

impl SendMultiCollectorManager {
    fn new(collector_managers: Vec<Box<dyn SendErasedManager>>) -> Self {
        Self { collector_managers }
    }
}

impl CollectorManager for SendMultiCollectorManager {
    type Collector = MultiCollectorHandle;
    type Output = Vec<Box<dyn Any + Send>>;

    fn new_collector(&self) -> Result<Self::Collector> {
        let mut collectors = Vec::with_capacity(self.collector_managers.len());
        for manager in &self.collector_managers {
            collectors.push(manager.new_any()?);
        }
        if collectors.len() == 1 {
            Ok(MultiCollectorHandle::Single(collectors.pop().expect("INVARIANT: one")))
        } else {
            Ok(MultiCollectorHandle::Multi(MultiCollector::new(collectors)))
        }
    }

    fn reduce(&self, collectors: Vec<Self::Collector>) -> Result<Self::Output> {
        let mut per_manager: Vec<Vec<Box<dyn Any + Send>>> = (0..self.collector_managers.len())
            .map(|_| Vec::with_capacity(collectors.len()))
            .collect();
        for collector in collectors {
            for (index, sub) in collector.into_collectors().into_iter().enumerate() {
                per_manager[index].push(sub.into_any());
            }
        }
        let mut results = Vec::with_capacity(self.collector_managers.len());
        for (manager, reducible) in self.collector_managers.iter().zip(per_manager) {
            results.push(manager.reduce_any(reducible)?);
        }
        Ok(results)
    }
}
