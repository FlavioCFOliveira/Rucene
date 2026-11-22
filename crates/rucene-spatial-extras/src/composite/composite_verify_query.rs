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

//! The types declared in `CompositeVerifyQuery.java`, of the Java package
//! `org.apache.lucene.spatial.composite`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::{
    ConstantScoreScorer, ConstantScoreWeight, ConstantScoreWeightImpl, DefaultScorerSupplier, IndexSearcher, Query,
    QueryVisitor, ScoreMode, ScorerSupplier, SegmentCacheable, Weight,
};

use crate::search_util::{share, ScorerDisi};
use crate::util::shape_values_predicate::ShapeValuesPredicate;

/// Equivalent to `org.apache.lucene.spatial.composite.CompositeVerifyQuery`
/// (`public class CompositeVerifyQuery extends Query`): a Query that
/// considers an "indexQuery" to have approximate results, and a follow-on
/// ShapeValuesSource is called to verify each hit from
/// `TwoPhaseIterator.matches()`.
#[derive(Debug, Clone)]
pub struct CompositeVerifyQuery {
    index_query: Arc<dyn Query>, // approximation (matches more than needed)
    predicate_value_source: Arc<ShapeValuesPredicate>,
}

impl CompositeVerifyQuery {
    /// `CompositeVerifyQuery(Query indexQuery, ShapeValuesPredicate
    /// predicateValueSource)`.
    pub fn new(index_query: Arc<dyn Query>, predicate_value_source: Arc<ShapeValuesPredicate>) -> Self {
        Self {
            index_query,
            predicate_value_source,
        }
    }
}

/// The anonymous `ConstantScoreWeight` of `createWeight`.
#[derive(Debug)]
struct CompositeVerifyWeight {
    index_query_weight: Arc<dyn Weight>,
    predicate_value_source: Arc<ShapeValuesPredicate>,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for CompositeVerifyWeight {
    fn scorer_supplier(&self, context: &LeafReaderContext, score: f32) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(index_query_scorer) = self.index_query_weight.scorer(context)? else {
            return Ok(None);
        };
        let pred_func_values = self
            .predicate_value_source
            .iterator(context, Box::new(ScorerDisi(share(index_query_scorer))))?;
        let scorer = ConstantScoreScorer::from_two_phase(score, self.score_mode, pred_func_values);
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.predicate_value_source.is_cacheable(ctx)
    }
}

impl Query for CompositeVerifyQuery {
    fn to_query_string(&self, field: &str) -> String {
        // TODO verify this looks good
        format!(
            "CompositeVerifyQuery({}, {})",
            self.index_query.to_query_string(field),
            self.predicate_value_source
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.index_query.rewrite(index_searcher)? {
            Some(rewritten) => Ok(Some(Arc::new(CompositeVerifyQuery::new(
                rewritten,
                Arc::clone(&self.predicate_value_source),
            )))),
            None => Ok(None),
        }
    }

    fn create_weight(&self, searcher: &IndexSearcher, score_mode: ScoreMode, boost: f32) -> Result<Arc<dyn Weight>> {
        // scores aren't unsupported
        let index_query_weight = self.index_query.create_weight(searcher, ScoreMode::COMPLETE_NO_SCORES, boost)?;
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            CompositeVerifyWeight {
                index_query_weight,
                predicate_value_source: Arc::clone(&self.predicate_value_source),
                score_mode,
            },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other.as_any().downcast_ref::<CompositeVerifyQuery>().is_some_and(|o| {
            self.index_query.query_eq(&*o.index_query) && self.predicate_value_source.predicate_eq(&o.predicate_value_source)
        })
    }

    fn query_hash(&self) -> i32 {
        let mut result = self.class_hash();
        result = result.wrapping_mul(31).wrapping_add(self.index_query.query_hash());
        result = result.wrapping_mul(31).wrapping_add(self.predicate_value_source.predicate_hash());
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.spatial.composite.CompositeVerifyQuery"
    }
}
