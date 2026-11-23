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

//! The types declared in `IntersectsRPTVerifyQuery.java`, of the Java
//! package `org.apache.lucene.spatial.composite`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::{
    ConstantScoreScorer, ConstantScoreWeight, ConstantScoreWeightImpl, DefaultScorerSupplier,
    IndexSearcher, Query, QueryVisitor, ScoreMode, ScorerSupplier, SegmentCacheable, Weight,
};
use rucene::util::doc_id_set::DocIdSetBuilder;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::abstract_prefix_tree_query::{BaseTermsEnumTraverser, TreeDocIdSet};
use crate::prefix::abstract_visiting_prefix_tree_query::{
    AbstractVisitingPrefixTreeQuery, VisitorHooks, VisitorTemplate,
};
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use crate::util::shape_values_predicate::{PredicateMatcher, ShapeValuesPredicate};

/// Equivalent to `org.apache.lucene.spatial.composite.IntersectsRPTVerifyQuery`
/// (`public class IntersectsRPTVerifyQuery extends Query`): a spatial
/// Intersects predicate that distinguishes an approximated match from an
/// exact match based on which cells are within the query shape. It
/// exposes a `TwoPhaseIterator` that will verify a match with
/// `ShapeValuesPredicate` for the approximated matches.
#[derive(Debug, Clone)]
pub struct IntersectsRPTVerifyQuery {
    intersects_diff_query: IntersectsDifferentiatingQuery,
    predicate_value_source: Arc<ShapeValuesPredicate>,
}

impl IntersectsRPTVerifyQuery {
    /// `IntersectsRPTVerifyQuery(Shape queryShape, String fieldName,
    /// SpatialPrefixTree grid, int detailLevel, int prefixGridScanLevel,
    /// ShapeValuesPredicate predicateValueSource)`.
    pub fn new(
        query_shape: Arc<dyn Shape>,
        field_name: &str,
        grid: Arc<dyn SpatialPrefixTree>,
        detail_level: i32,
        prefix_grid_scan_level: i32,
        predicate_value_source: Arc<ShapeValuesPredicate>,
    ) -> Self {
        Self {
            predicate_value_source,
            intersects_diff_query: IntersectsDifferentiatingQuery::new(
                query_shape,
                field_name,
                grid,
                detail_level,
                prefix_grid_scan_level,
            ),
        }
    }
}

/// The anonymous `ConstantScoreWeight` of `createWeight`.
#[derive(Debug)]
struct IntersectsRPTVerifyWeight {
    intersects_diff_query: IntersectsDifferentiatingQuery,
    predicate_value_source: Arc<ShapeValuesPredicate>,
    score_mode: ScoreMode,
}

/// The anonymous `TwoPhaseIterator` of `scorerSupplier`.
struct VerifyTwoPhase {
    approx_disi: Box<dyn DocIdSetIterator>,
    exact_iterator: Option<Box<dyn DocIdSetIterator>>,
    /// `predFuncValues`, whose approximation is `approxDISI`.
    pred_func_values: PredicateMatcher,
}

impl TwoPhaseIterator for VerifyTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.approx_disi
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &*self.approx_disi
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.approx_disi.doc_id();
        if let Some(exact_iterator) = self.exact_iterator.as_mut() {
            if exact_iterator.doc_id() < doc {
                exact_iterator.advance(doc)?;
            }
            if exact_iterator.doc_id() == doc {
                return Ok(true);
            }
        }
        self.pred_func_values.matches(doc)
    }

    fn match_cost(&self) -> f32 {
        100.0 // TODO: use cost of exactIterator.advance() and
              // predFuncValues.cost()
    }
}

impl ConstantScoreWeightImpl for IntersectsRPTVerifyWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let result = self.intersects_diff_query.compute(context)?;
        let Some(approx_doc_id_set) = &result.approx_doc_id_set else {
            return Ok(None);
        };
        let approx_disi = approx_doc_id_set.iterator()?;
        let exact_iterator = match &result.exact_doc_id_set {
            Some(exact_doc_id_set) => {
                // If both sets are the same, there's nothing to verify; we needn't return a
                // TwoPhaseIterator (Java builds a plain scorer here, then replaces it below)
                Some(exact_doc_id_set.iterator()?)
            }
            None => None,
        };

        let two_phase_iterator = VerifyTwoPhase {
            approx_disi,
            exact_iterator,
            pred_func_values: self
                .predicate_value_source
                .matcher(self.predicate_value_source.values(context)?),
        };

        let scorer = ConstantScoreScorer::from_two_phase(
            score,
            self.score_mode,
            Box::new(two_phase_iterator),
        );
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.predicate_value_source.is_cacheable(ctx)
    }
}

impl Query for IntersectsRPTVerifyQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!("IntersectsVerified(fieldName={field})")
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            IntersectsRPTVerifyWeight {
                intersects_diff_query: self.intersects_diff_query.clone(),
                predicate_value_source: Arc::clone(&self.predicate_value_source),
                score_mode,
            },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<IntersectsRPTVerifyQuery>()
            .is_some_and(|o| {
                self.intersects_diff_query
                    .query_eq(&o.intersects_diff_query)
                    && self
                        .predicate_value_source
                        .predicate_eq(&o.predicate_value_source)
            })
    }

    fn query_hash(&self) -> i32 {
        let mut result = self.class_hash();
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.intersects_diff_query.query_hash());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.predicate_value_source.predicate_hash());
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.spatial.composite.IntersectsRPTVerifyQuery"
    }
}

/// Equivalent to `org.apache.lucene.spatial.composite.IntersectsRPTVerifyQuery.IntersectsDifferentiatingQuery`
/// (`private static class IntersectsDifferentiatingQuery extends
/// AbstractVisitingPrefixTreeQuery`): this may be a "Query" but we don't use
/// it as-such; the caller calls the constructor and then
/// [`compute`](Self::compute) and examines the results which consists of
/// two parts -- the approximated results, and a subset of exact matches.
/// The difference needs to be verified.
#[derive(Debug, Clone)]
pub(crate) struct IntersectsDifferentiatingQuery {
    avptq: AbstractVisitingPrefixTreeQuery,
}

impl IntersectsDifferentiatingQuery {
    /// `IntersectsDifferentiatingQuery(Shape queryShape, String fieldName,
    /// SpatialPrefixTree grid, int detailLevel, int prefixGridScanLevel)`.
    fn new(
        query_shape: Arc<dyn Shape>,
        field_name: &str,
        grid: Arc<dyn SpatialPrefixTree>,
        detail_level: i32,
        prefix_grid_scan_level: i32,
    ) -> Self {
        Self {
            avptq: AbstractVisitingPrefixTreeQuery::new(
                query_shape,
                field_name,
                grid,
                detail_level,
                prefix_grid_scan_level,
            ),
        }
    }

    /// `compute(LeafReaderContext context)`.
    fn compute(&self, context: &LeafReaderContext) -> Result<IntersectsDifferentiatingVisitor> {
        let mut template = VisitorTemplate::new(
            &self.avptq,
            context,
            IntersectsDifferentiatingVisitor::default(),
        )?;
        template.get_doc_id_set()?; // computes
        Ok(template.hooks)
    }

    /// `getDocIdSet(LeafReaderContext context)`: not used this way.
    ///
    /// # Errors
    ///
    /// Always `IllegalStateException`.
    #[allow(dead_code)] // Java's override that is never called this way.
    pub(crate) fn get_doc_id_set(
        &self,
        _context: &LeafReaderContext,
    ) -> Result<Option<TreeDocIdSet>> {
        Err(LuceneError::illegal_state(String::new()))
    }

    /// Equivalent to `IntersectsDifferentiatingQuery.toString(String)`: not
    /// used this way.
    ///
    /// # Panics
    ///
    /// Always, with Java's `IllegalStateException`.
    #[allow(dead_code)] // Java's override that is never called this way.
    pub(crate) fn to_query_string(&self, _field: &str) -> String {
        panic!("java.lang.IllegalStateException")
    }

    /// `equals(Object o)`: `AbstractPrefixTreeQuery`'s.
    fn query_eq(&self, other: &IntersectsDifferentiatingQuery) -> bool {
        self.avptq.base.equals_to(&other.avptq.base)
    }

    /// `hashCode()`: `AbstractPrefixTreeQuery`'s, over this class's
    /// `classHash()`.
    fn query_hash(&self) -> i32 {
        let class_hash = crate::jdk::string_hash(
            "org.apache.lucene.spatial.composite.IntersectsRPTVerifyQuery$IntersectsDifferentiatingQuery",
        );
        self.avptq.base.hash_code(class_hash)
    }
}

/// Equivalent to `org.apache.lucene.spatial.composite.IntersectsRPTVerifyQuery.IntersectsDifferentiatingQuery.IntersectsDifferentiatingVisitor`
/// (`class IntersectsDifferentiatingVisitor extends VisitorTemplate`).
// TODO consider if IntersectsPrefixTreeQuery should simply do this and provide both sets
#[derive(Default)]
pub(crate) struct IntersectsDifferentiatingVisitor {
    approx_builder: Option<DocIdSetBuilder>,
    exact_builder: Option<DocIdSetBuilder>,
    approx_is_empty: bool,
    exact_is_empty: bool,
    exact_doc_id_set: Option<Arc<dyn DocIdSet + Send + Sync>>,
    approx_doc_id_set: Option<Arc<dyn DocIdSet + Send + Sync>>,
}

impl VisitorHooks for IntersectsDifferentiatingVisitor {
    fn start(&mut self, t: &mut BaseTermsEnumTraverser) -> Result<()> {
        let terms = t
            .terms
            .as_ref()
            .expect("terms != null when termsEnum != null");
        self.approx_builder = Some(DocIdSetBuilder::from_terms(t.max_doc, &**terms)?);
        self.exact_builder = Some(DocIdSetBuilder::from_terms(t.max_doc, &**terms)?);
        self.approx_is_empty = true;
        self.exact_is_empty = true;
        Ok(())
    }

    fn finish(&mut self, _t: &mut BaseTermsEnumTraverser) -> Result<Option<TreeDocIdSet>> {
        let exact_builder = self.exact_builder.take().expect("set by start()");
        if self.exact_is_empty {
            self.exact_doc_id_set = None;
        } else {
            self.exact_doc_id_set = Some(Arc::from(exact_builder.build()?));
        }
        if self.approx_is_empty {
            self.approx_doc_id_set = self.exact_doc_id_set.clone(); // optimization
        } else {
            let mut approx_builder = self.approx_builder.take().expect("set by start()");
            if let Some(exact_doc_id_set) = &self.exact_doc_id_set {
                approx_builder.add(&mut *exact_doc_id_set.iterator()?)?;
            }
            self.approx_doc_id_set = Some(Arc::from(approx_builder.build()?));
        }
        Ok(None) // unused in this weird re-use of AVPTQ
    }

    fn visit_prefix(
        &mut self,
        t: &mut BaseTermsEnumTraverser,
        q: &AbstractVisitingPrefixTreeQuery,
        cell: &CellRef,
    ) -> Result<bool> {
        if cell.get_shape_rel() == Some(SpatialRelation::Within) {
            self.exact_is_empty = false;
            t.collect_docs_builder(self.exact_builder.as_mut().expect("set by start()"))?; // note: we'll add exact to approx on finish()
            return Ok(false);
        } else if cell.get_level() == q.base.detail_level {
            self.approx_is_empty = false;
            t.collect_docs_builder(self.approx_builder.as_mut().expect("set by start()"))?;
            return Ok(false);
        }
        Ok(true)
    }

    fn visit_leaf(
        &mut self,
        t: &mut BaseTermsEnumTraverser,
        _q: &AbstractVisitingPrefixTreeQuery,
        cell: &CellRef,
    ) -> Result<()> {
        if cell.get_shape_rel() == Some(SpatialRelation::Within) {
            self.exact_is_empty = false;
            t.collect_docs_builder(self.exact_builder.as_mut().expect("set by start()"))
        // note: we'll add exact to approx on finish()
        } else {
            self.approx_is_empty = false;
            t.collect_docs_builder(self.approx_builder.as_mut().expect("set by start()"))
        }
    }
}
