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

//! Port of `FacetsCollectorManager.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::multi_collector_manager::{ErasedCollectorManager, MultiCollectorManager};
use rucene::search::top_field_collector::TopFieldCollector;
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::top_field_docs::TopFieldDocs;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::total_hit_count_collector::TotalHitCountCollectorManager;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::{
    CollectorManager, FieldDoc, IndexSearcher, Query, ScoreDoc, Sort, TopDocs,
};
use rucene::util::doc_id_set::DocIdSetBuilder;

use crate::facets_collector::{FacetsCollector, MatchingDocs, SharedDocIdSet};

/// A `CollectorManager` implementation which produces `FacetsCollector` and
/// produces a merged `FacetsCollector`.
///
/// Equivalent to `org.apache.lucene.facet.FacetsCollectorManager`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FacetsCollectorManager {
    keep_scores: bool,
}

impl FacetsCollectorManager {
    /// Create a new manager that does not keep scores.
    ///
    /// Equivalent to `FacetsCollectorManager()`.
    pub fn new() -> Self {
        Self::with_keep_scores(false)
    }

    /// Create a new manager that keeps scores if `keep_scores` is true.
    ///
    /// Equivalent to `FacetsCollectorManager(boolean)`.
    pub fn with_keep_scores(keep_scores: bool) -> Self {
        Self { keep_scores }
    }

    /// Equivalent to `reduceMatchingDocs(Collection)`: the matching docs of
    /// the same context (by identity) are merged.
    pub(crate) fn reduce_matching_docs(facets_collectors: &[&FacetsCollector]) -> Result<Vec<MatchingDocs>> {
        let mut matching_docs_map: Vec<MatchingDocs> = Vec::new();
        for facets_collector in facets_collectors {
            for matching_docs in facets_collector.get_matching_docs() {
                match matching_docs_map
                    .iter()
                    .position(|m| m.context_identity == matching_docs.context_identity)
                {
                    None => matching_docs_map.push(matching_docs.clone()),
                    Some(i) => {
                        let merged = Self::merge(&matching_docs_map[i], matching_docs)?;
                        matching_docs_map[i] = merged;
                    }
                }
            }
        }
        Ok(matching_docs_map)
    }

    /// Equivalent to `merge(MatchingDocs, MatchingDocs)`.
    fn merge(matching_docs1: &MatchingDocs, matching_docs2: &MatchingDocs) -> Result<MatchingDocs> {
        debug_assert!(matching_docs1.context_identity == matching_docs2.context_identity);
        let scores: Vec<f32> = match (matching_docs1.scores(), matching_docs2.scores()) {
            (None, None) => Vec::new(),
            (Some(s1), None) => s1.to_vec(),
            (None, Some(s2)) => s2.to_vec(),
            (Some(s1), Some(s2)) => {
                let length = s1.len().max(s2.len());
                let mut scores = vec![0.0f32; length];
                for (i, score) in scores.iter_mut().enumerate() {
                    let first_score = if i < s1.len() { s1[i] } else { 0.0 };
                    let second_score = if i < s2.len() { s2[i] } else { 0.0 };
                    debug_assert!(!(first_score > 0.0 && second_score > 0.0));
                    *score = crate::taxonomy::association_aggregation_function::java_math_max_f32(
                        first_score,
                        second_score,
                    );
                }
                scores
            }
        };
        let mut doc_id_set_builder =
            DocIdSetBuilder::new(matching_docs1.context.leaf_reader().max_doc());
        doc_id_set_builder.add(&mut *matching_docs1.bits.iterator()?)?;
        doc_id_set_builder.add(&mut *matching_docs2.bits.iterator()?)?;
        let total_hits = matching_docs1.total_hits + matching_docs2.total_hits;
        let bits: SharedDocIdSet = Arc::from(doc_id_set_builder.build()?);
        let mut merged = MatchingDocs::new(
            Arc::clone(&matching_docs1.context),
            bits,
            total_hits,
            Some(scores),
        );
        merged.context_identity = matching_docs1.context_identity;
        Ok(merged)
    }

    /// Utility method, to search and also collect all hits into the provided
    /// `FacetsCollectorManager`.
    ///
    /// Equivalent to `search(IndexSearcher, Query, int, FacetsCollectorManager)`.
    pub fn search(
        searcher: &IndexSearcher,
        q: Arc<dyn Query>,
        n: i32,
        fcm: &FacetsCollectorManager,
    ) -> Result<FacetsResult> {
        Self::do_search(searcher, None, q, n, None, false, fcm)
    }

    /// Utility method, to search and also collect all hits into the provided
    /// `FacetsCollectorManager`, sorted by `sort`.
    ///
    /// Equivalent to `search(IndexSearcher, Query, int, Sort,
    /// FacetsCollectorManager)`; Java's `null` sort is rejected by the type.
    pub fn search_with_sort(
        searcher: &IndexSearcher,
        q: Arc<dyn Query>,
        n: i32,
        sort: &Sort,
        fcm: &FacetsCollectorManager,
    ) -> Result<FacetsResult> {
        Self::do_search(searcher, None, q, n, Some(sort), false, fcm)
    }

    /// Equivalent to `search(IndexSearcher, Query, int, Sort, boolean,
    /// FacetsCollectorManager)`.
    pub fn search_with_sort_and_scores(
        searcher: &IndexSearcher,
        q: Arc<dyn Query>,
        n: i32,
        sort: &Sort,
        do_doc_scores: bool,
        fcm: &FacetsCollectorManager,
    ) -> Result<FacetsResult> {
        Self::do_search(searcher, None, q, n, Some(sort), do_doc_scores, fcm)
    }

    /// Equivalent to `searchAfter(IndexSearcher, ScoreDoc, Query, int,
    /// FacetsCollectorManager)`.
    pub fn search_after(
        searcher: &IndexSearcher,
        after: Option<ScoreDoc>,
        q: Arc<dyn Query>,
        n: i32,
        fcm: &FacetsCollectorManager,
    ) -> Result<FacetsResult> {
        Self::do_search(
            searcher,
            after.map(After::Score),
            q,
            n,
            None,
            false,
            fcm,
        )
    }

    /// Equivalent to `searchAfter(IndexSearcher, ScoreDoc, Query, int, Sort,
    /// FacetsCollectorManager)`. Java requires `after` to be a `FieldDoc`; the
    /// type does here.
    pub fn search_after_with_sort(
        searcher: &IndexSearcher,
        after: Option<FieldDoc>,
        q: Arc<dyn Query>,
        n: i32,
        sort: &Sort,
        fcm: &FacetsCollectorManager,
    ) -> Result<FacetsResult> {
        Self::do_search(
            searcher,
            after.map(After::Field),
            q,
            n,
            Some(sort),
            false,
            fcm,
        )
    }

    /// Equivalent to `searchAfter(IndexSearcher, ScoreDoc, Query, int, Sort,
    /// boolean, FacetsCollectorManager)`.
    pub fn search_after_with_sort_and_scores(
        searcher: &IndexSearcher,
        after: Option<FieldDoc>,
        q: Arc<dyn Query>,
        n: i32,
        sort: &Sort,
        do_doc_scores: bool,
        fcm: &FacetsCollectorManager,
    ) -> Result<FacetsResult> {
        Self::do_search(
            searcher,
            after.map(After::Field),
            q,
            n,
            Some(sort),
            do_doc_scores,
            fcm,
        )
    }

    /// Equivalent to `doSearch(IndexSearcher, ScoreDoc, Query, int, Sort,
    /// boolean, FacetsCollectorManager)`.
    fn do_search(
        searcher: &IndexSearcher,
        after: Option<After>,
        q: Arc<dyn Query>,
        n: i32,
        sort: Option<&Sort>,
        do_doc_scores: bool,
        fcm: &FacetsCollectorManager,
    ) -> Result<FacetsResult> {
        let mut limit = searcher.get_index_reader().max_doc();
        if limit == 0 {
            limit = 1;
        }
        let n = n.min(limit);

        if let Some(after) = &after {
            let after_doc = after.doc();
            if after_doc >= limit {
                return Err(LuceneError::illegal_argument(format!(
                    "after.doc exceeds the number of documents in the reader: after.doc={after_doc} limit={limit}"
                )));
            }
        }

        let top_docs: FacetsTopDocs;
        let facets_collector: FacetsCollector;
        if n == 0 {
            let hit_count_collector_manager =
                TotalHitCountCollectorManager::new(searcher.get_slices()?);
            let multi_collector_manager = MultiCollectorManager::new(vec![
                Some(Box::new(hit_count_collector_manager) as Box<dyn ErasedCollectorManager>),
                Some(Box::new(*fcm)),
            ])?;
            let mut result = searcher.search_with_manager(q, &multi_collector_manager)?;
            let fc = result.pop().expect("INVARIANT: two managers");
            let count = result.pop().expect("INVARIANT: two managers");
            let count = *count.downcast::<i32>().expect("INVARIANT: hit count");
            top_docs = FacetsTopDocs::TopDocs(TopDocs::new(
                TotalHits::new(count as i64, TotalHitsRelation::EQUAL_TO)?,
                Vec::new(),
            ));
            facets_collector = *fc
                .downcast::<FacetsCollector>()
                .expect("INVARIANT: facets collector");
        } else {
            let multi_collector_manager;
            if let Some(sort) = sort {
                let after = match after {
                    None => None,
                    Some(After::Field(field_doc)) => Some(field_doc),
                    Some(After::Score(score_doc)) => {
                        return Err(LuceneError::illegal_argument(format!(
                            "after must be a FieldDoc; got {score_doc}"
                        )));
                    }
                };
                let top_field_collector_manager =
                    TopFieldCollectorManager::new(sort.clone(), n, after, i32::MAX)?;
                multi_collector_manager = MultiCollectorManager::new(vec![
                    Some(Box::new(top_field_collector_manager) as Box<dyn ErasedCollectorManager>),
                    Some(Box::new(*fcm)),
                ])?;
            } else {
                let after = match after {
                    None => None,
                    Some(After::Score(score_doc)) => Some(score_doc),
                    Some(After::Field(field_doc)) => Some(field_doc.score_doc),
                };
                let top_score_doc_collector_manager =
                    TopScoreDocCollectorManager::new(n, after, i32::MAX)?;
                multi_collector_manager = MultiCollectorManager::new(vec![
                    Some(Box::new(top_score_doc_collector_manager)
                        as Box<dyn ErasedCollectorManager>),
                    Some(Box::new(*fcm)),
                ])?;
            }
            let mut result = searcher.search_with_manager(Arc::clone(&q), &multi_collector_manager)?;
            let fc = result.pop().expect("INVARIANT: two managers");
            let docs = result.pop().expect("INVARIANT: two managers");
            let mut docs = if sort.is_some() {
                FacetsTopDocs::TopFieldDocs(*docs.downcast::<TopFieldDocs>().expect("INVARIANT: top field docs"))
            } else {
                FacetsTopDocs::TopDocs(*docs.downcast::<TopDocs>().expect("INVARIANT: top docs"))
            };
            if do_doc_scores {
                docs.populate_scores(searcher, q)?;
            }
            top_docs = docs;
            facets_collector = *fc
                .downcast::<FacetsCollector>()
                .expect("INVARIANT: facets collector");
        }
        Ok(FacetsResult {
            top_docs,
            facets_collector,
        })
    }
}

/// Java's `ScoreDoc after`, which is a `FieldDoc` when sorting.
enum After {
    Score(ScoreDoc),
    Field(FieldDoc),
}

impl After {
    fn doc(&self) -> i32 {
        match self {
            After::Score(d) => d.doc,
            After::Field(d) => d.score_doc.doc,
        }
    }
}

/// The `TopDocs` of a facets search: a `TopFieldDocs` when the search was
/// sorted, as Java's `TopDocs` reference then is.
#[derive(Debug, Clone)]
pub enum FacetsTopDocs {
    /// Hits sorted by score.
    TopDocs(TopDocs),
    /// Hits sorted by a `Sort`.
    TopFieldDocs(TopFieldDocs),
}

impl FacetsTopDocs {
    /// `TopFieldCollector.populateScores(topDocs.scoreDocs, searcher, q)`.
    pub(crate) fn populate_scores(&mut self, searcher: &IndexSearcher, q: Arc<dyn Query>) -> Result<()> {
        match self {
            FacetsTopDocs::TopDocs(top_docs) => {
                TopFieldCollector::populate_scores(&mut top_docs.score_docs, searcher, q)
            }
            FacetsTopDocs::TopFieldDocs(top_docs) => {
                let mut score_docs: Vec<ScoreDoc> =
                    top_docs.score_docs.iter().map(|d| d.score_doc.clone()).collect();
                TopFieldCollector::populate_scores(&mut score_docs, searcher, q)?;
                for (field_doc, score_doc) in top_docs.score_docs.iter_mut().zip(score_docs) {
                    field_doc.score_doc = score_doc;
                }
                Ok(())
            }
        }
    }

    /// The total hits of either kind.
    pub fn total_hits(&self) -> &TotalHits {
        match self {
            FacetsTopDocs::TopDocs(d) => &d.total_hits,
            FacetsTopDocs::TopFieldDocs(d) => &d.total_hits,
        }
    }
}

/// Holds results of a search run via static utility methods exposed by this
/// class.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.FacetsCollectorManager.FacetsResult`.
pub struct FacetsResult {
    /// Top docs.
    pub top_docs: FacetsTopDocs,
    /// Facets collector.
    pub facets_collector: FacetsCollector,
}

impl FacetsResult {
    /// Equivalent to `topDocs()`.
    pub fn top_docs(&self) -> &FacetsTopDocs {
        &self.top_docs
    }

    /// Equivalent to `facetsCollector()`.
    pub fn facets_collector(&self) -> &FacetsCollector {
        &self.facets_collector
    }
}

/// Reduces `FacetsCollector`s into one.
///
/// Equivalent to the private
/// `org.apache.lucene.facet.FacetsCollectorManager.ReducedFacetsCollector`,
/// a `FacetsCollector` whose matching docs are those of the reduced
/// collectors.
pub(crate) struct ReducedFacetsCollector;

impl ReducedFacetsCollector {
    /// Equivalent to `ReducedFacetsCollector(Collection, boolean)`.
    pub(crate) fn create(facets_collectors: &[&FacetsCollector], keep_scores: bool) -> Result<FacetsCollector> {
        let mut collector = FacetsCollector::with_keep_scores(keep_scores);
        collector
            .get_matching_docs_mut()
            .extend(FacetsCollectorManager::reduce_matching_docs(facets_collectors)?);
        Ok(collector)
    }
}

impl CollectorManager for FacetsCollectorManager {
    type Collector = FacetsCollector;
    type Output = FacetsCollector;

    fn new_collector(&self) -> Result<FacetsCollector> {
        Ok(FacetsCollector::with_keep_scores(self.keep_scores))
    }

    fn reduce(&self, collectors: Vec<FacetsCollector>) -> Result<FacetsCollector> {
        if collectors.is_empty() {
            return Ok(FacetsCollector::new());
        }
        if collectors.len() == 1 {
            return Ok(collectors.into_iter().next().expect("INVARIANT: one collector"));
        }
        debug_assert!(collectors.iter().all(|fc| fc.get_keep_scores() == self.keep_scores));
        let refs: Vec<&FacetsCollector> = collectors.iter().collect();
        ReducedFacetsCollector::create(&refs, self.keep_scores)
    }
}