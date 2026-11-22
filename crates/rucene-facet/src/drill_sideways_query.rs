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

//! Port of `DrillSidewaysQuery.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::doc_id_set_iterator;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::{
    BulkScorer, Collector, CollectorManager, Explanation, IndexSearcher, Query, QueryVisitor,
    ScoreMode, Scorer, ScorerSupplier, Weight,
};
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::drill_sideways_scorer::{CollectorSlot, DocsAndCost, DrillSidewaysScorer};
use crate::facets_collector::retain_leaf_context;

/// The sideways collectors created per leaf, one list per leaf (Java's
/// `Collections.synchronizedList`).
pub(crate) type ManagedCollectors<K> = Arc<Mutex<Vec<Vec<CollectorSlot<K>>>>>;

/// Only purpose is to punch through and return a `DrillSidewaysScorer`.
///
/// Equivalent to `org.apache.lucene.facet.DrillSidewaysQuery`.
pub(crate) struct DrillSidewaysQuery<M: CollectorManager + Send + Sync + 'static>
where
    M::Collector: Send + 'static,
{
    pub(crate) base_query: Arc<dyn Query>,
    pub(crate) drill_sideways_collector_managers: Arc<Vec<M>>,
    pub(crate) managed_drill_sideways_collectors: ManagedCollectors<M::Collector>,
    pub(crate) drill_down_queries: Vec<Arc<dyn Query>>,
    pub(crate) score_sub_docs_at_once: bool,
}

impl<M: CollectorManager + Send + Sync + 'static> Clone for DrillSidewaysQuery<M>
where
    M::Collector: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            base_query: Arc::clone(&self.base_query),
            drill_sideways_collector_managers: Arc::clone(&self.drill_sideways_collector_managers),
            managed_drill_sideways_collectors: Arc::clone(&self.managed_drill_sideways_collectors),
            drill_down_queries: self.drill_down_queries.clone(),
            score_sub_docs_at_once: self.score_sub_docs_at_once,
        }
    }
}

impl<M: CollectorManager + Send + Sync + 'static> fmt::Debug for DrillSidewaysQuery<M>
where
    M::Collector: Send + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DrillSidewaysQuery")
    }
}

impl<M: CollectorManager + Send + Sync + 'static> DrillSidewaysQuery<M>
where
    M::Collector: Send + 'static,
{
    /// Construct a new `DrillSidewaysQuery` that will create new
    /// `FacetsCollector`s for each `LeafReaderContext` using the provided
    /// `FacetsCollectorManager`s.
    ///
    /// Equivalent to `DrillSidewaysQuery(Query, List, Query[], boolean)`.
    pub(crate) fn new(
        base_query: Arc<dyn Query>,
        drill_sideways_collector_managers: Arc<Vec<M>>,
        drill_down_queries: Vec<Arc<dyn Query>>,
        score_sub_docs_at_once: bool,
    ) -> Self {
        Self {
            base_query,
            drill_sideways_collector_managers,
            managed_drill_sideways_collectors: Arc::new(Mutex::new(Vec::new())),
            drill_down_queries,
            score_sub_docs_at_once,
        }
    }

    /// Equivalent to `equalsTo(DrillSidewaysQuery)`.
    fn equals_to(&self, other: &DrillSidewaysQuery<M>) -> bool {
        self.base_query.query_eq(&*other.base_query)
            && self.drill_down_queries.len() == other.drill_down_queries.len()
            && self
                .drill_down_queries
                .iter()
                .zip(&other.drill_down_queries)
                .all(|(a, b)| a.query_eq(&**b))
            && Arc::ptr_eq(
                &self.drill_sideways_collector_managers,
                &other.drill_sideways_collector_managers,
            )
    }
}

impl<M: CollectorManager + Send + Sync + 'static> Query for DrillSidewaysQuery<M>
where
    M::Collector: Send + 'static,
{
    fn to_query_string(&self, _field: &str) -> String {
        "DrillSidewaysQuery".to_string()
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let mut new_query = Arc::clone(&self.base_query);
        loop {
            match new_query.rewrite(index_searcher)? {
                None => break,
                Some(rewritten_query) => {
                    if Arc::ptr_eq(&rewritten_query, &new_query) {
                        break;
                    }
                    new_query = rewritten_query;
                }
            }
        }
        if Arc::ptr_eq(&new_query, &self.base_query) {
            Ok(None)
        } else {
            let mut q = self.clone();
            q.base_query = new_query;
            Ok(Some(Arc::new(q)))
        }
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let base_weight = self.base_query.create_weight(searcher, score_mode, boost)?;
        let mut drill_downs = Vec::with_capacity(self.drill_down_queries.len());
        for query in &self.drill_down_queries {
            drill_downs.push(searcher.create_weight(
                searcher.rewrite(Arc::clone(query))?,
                ScoreMode::COMPLETE_NO_SCORES,
                1.0,
            )?);
        }
        Ok(Arc::new(DrillSidewaysWeight {
            query: self.clone(),
            base_weight,
            drill_downs,
            score_mode,
        }))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<DrillSidewaysQuery<M>>()
                .is_some_and(|o| self.equals_to(o))
    }

    fn query_hash(&self) -> i32 {
        let prime = 31i32;
        let mut result = self.class_hash();
        result = prime.wrapping_mul(result).wrapping_add(self.base_query.query_hash());
        let mut array_hash = 1i32;
        for q in &self.drill_down_queries {
            array_hash = array_hash.wrapping_mul(31).wrapping_add(q.query_hash());
        }
        result = prime.wrapping_mul(result).wrapping_add(array_hash);
        result = prime.wrapping_mul(result).wrapping_add(java_identity_hash_code_stand_in(
            "java.util.List<CollectorManager>",
        ));
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.DrillSidewaysQuery"
    }
}

/// The anonymous `Weight` of `createWeight`.
struct DrillSidewaysWeight<M: CollectorManager + Send + Sync + 'static>
where
    M::Collector: Send + 'static,
{
    query: DrillSidewaysQuery<M>,
    base_weight: Arc<dyn Weight>,
    drill_downs: Vec<Arc<dyn Weight>>,
    score_mode: ScoreMode,
}

impl<M: CollectorManager + Send + Sync + 'static> fmt::Debug for DrillSidewaysWeight<M>
where
    M::Collector: Send + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DrillSidewaysWeight")
    }
}

impl<M: CollectorManager + Send + Sync + 'static> SegmentCacheable for DrillSidewaysWeight<M>
where
    M::Collector: Send + 'static,
{
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        // We can never cache DSQ instances. It's critical that the
        // BulkScorer produced by this Weight runs through the "normal"
        // execution path so that it has access to an "acceptDocs" instance
        // that accurately reflects deleted docs. During caching, "acceptDocs"
        // is null so that caching over-matches (since the final BulkScorer
        // would account for deleted docs). The problem is that this BulkScorer
        // has a side-effect of populating the "sideways" FacetsCollectors, so
        // it will use deleted docs in its sideways counting if caching kicks
        // in. See LUCENE-10060:
        false
    }
}

impl<M: CollectorManager + Send + Sync + 'static> Weight for DrillSidewaysWeight<M>
where
    M::Collector: Send + 'static,
{
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::new(self.query.clone())
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.base_weight.explain(context, doc)
    }

    fn scorer_supplier(&self, context: &LeafReaderContext) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let base_scorer_supplier = self.base_weight.scorer_supplier(context)?;

        let drill_down_count = self.drill_downs.len();
        let mut sideways_collectors: Vec<CollectorSlot<M::Collector>> = Vec::with_capacity(drill_down_count);

        let mut dims: Vec<DocsAndCost<M::Collector>> = Vec::with_capacity(drill_down_count);

        let mut null_count = 0;
        for dim in 0..drill_down_count {
            let scorer: Box<dyn Scorer> = match self.drill_downs[dim].scorer(context)? {
                Some(scorer) => scorer,
                None => {
                    null_count += 1;
                    Box::new(ConstantScoreScorer::from_iterator(
                        0.0,
                        self.score_mode,
                        Box::new(doc_id_set_iterator::empty()),
                    ))
                }
            };

            let sideways_collector = Arc::new(Mutex::new(
                self.query.drill_sideways_collector_managers[dim].new_collector()?,
            ));
            sideways_collectors.push(Arc::clone(&sideways_collector));

            dims.push(DocsAndCost::new(scorer, sideways_collector));
        }
        self.query
            .managed_drill_sideways_collectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sideways_collectors);

        // If more than one dim has no matches, then there are no hits nor
        // drill-sideways counts. Or, if we have only one dim and that dim has
        // no matches, same thing.
        let Some(base_scorer_supplier) = base_scorer_supplier.filter(|_| null_count <= 1) else {
            // Java obtained each dimension's leaf collector in this method;
            // it finishes them here.
            for dim in &dims {
                let mut collector = dim.collector().lock().unwrap_or_else(PoisonError::into_inner);
                let mut leaf = collector
                    .get_leaf_collector(context)
                    .map_err(|e| e.into_lucene_error())?;
                leaf.finish()?;
            }
            return Ok(None);
        };

        // Sort drill-downs by most restrictive first (stable, as
        // `Arrays.sort` of objects):
        dims.sort_by_key(|d| d.cost);

        Ok(Some(Box::new(DrillSidewaysScorerSupplier {
            context: retain_leaf_context(context),
            base_scorer_supplier: Some(base_scorer_supplier),
            dims: Some(dims),
            score_sub_docs_at_once: self.query.score_sub_docs_at_once,
        })))
    }
}

/// The anonymous `ScorerSupplier` of `scorerSupplier(LeafReaderContext)`.
struct DrillSidewaysScorerSupplier<K: Collector + Send + 'static> {
    context: Arc<LeafReaderContext>,
    base_scorer_supplier: Option<Box<dyn ScorerSupplier>>,
    dims: Option<Vec<DocsAndCost<K>>>,
    score_sub_docs_at_once: bool,
}

impl<K: Collector + Send + 'static> ScorerSupplier for DrillSidewaysScorerSupplier<K> {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        // We only return a BulkScorer:
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        let base = self
            .base_scorer_supplier
            .as_mut()
            .ok_or_else(LuceneError::null_pointer)?
            .get(i64::MAX)?;
        Ok(Box::new(DrillSidewaysScorer::new(
            Arc::clone(&self.context),
            base,
            self.dims.take().ok_or_else(LuceneError::null_pointer)?,
            self.score_sub_docs_at_once,
        )))
    }

    /// Java throws `UnsupportedOperationException`; nothing reaches it on the
    /// bulk-scoring path this supplier serves.
    fn cost(&self) -> i64 {
        panic!("java.lang.UnsupportedOperationException")
    }
}
