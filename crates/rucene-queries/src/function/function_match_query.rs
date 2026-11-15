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

//! Port of `FunctionMatchQuery.java`, of the Java package
//! `org.apache.lucene.queries.function`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::doc_id_set_iterator::{self, AllDocIdSetIterator, DocIdSetIterator};
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::function::value_source::{objects_equals_double, objects_hash, same_object};

/// The default match cost of the two-phase iterator.
///
/// Equivalent to the package-private `FunctionMatchQuery.DEFAULT_MATCH_COST`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub const DEFAULT_MATCH_COST: f32 = 100.0;

/// The `java.util.function.DoublePredicate` a [`FunctionMatchQuery`] tests
/// values with.
pub type DoublePredicate = Arc<dyn Fn(f64) -> bool + Send + Sync>;

/// A query that retrieves all documents with a [`DoubleValues`] value matching
/// a predicate.
///
/// Equivalent to the final class
/// `org.apache.lucene.queries.function.FunctionMatchQuery`. This query works
/// by a linear scan of the index, and is best used in conjunction with other
/// queries that can restrict the number of documents visited.
///
/// **Divergence.** Java compares and hashes the predicate as an object, by
/// identity for a lambda (`FunctionMatchQuery.java:117-128`). Equality here is
/// the identity of the shared predicate ([`Arc::ptr_eq`]); its hash is the
/// stand-in of [`java_identity_hash_code_stand_in`], since an identity hash is
/// unreproducible in Java itself.
#[derive(Clone)]
pub struct FunctionMatchQuery {
    source: Arc<dyn DoubleValuesSource>,
    filter: DoublePredicate,
    /// Not used in equals/hashCode.
    match_cost: f32,
}

impl fmt::Debug for FunctionMatchQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FunctionMatchQuery")
            .field("source", &self.source)
            .field("match_cost", &self.match_cost)
            .finish_non_exhaustive()
    }
}

impl FunctionMatchQuery {
    /// Creates a query with the default two-phase match cost,
    /// `DEFAULT_MATCH_COST` (`100`).
    ///
    /// Equivalent to `FunctionMatchQuery(DoubleValuesSource, DoublePredicate)`.
    pub fn new(source: Arc<dyn DoubleValuesSource>, filter: DoublePredicate) -> Self {
        Self::with_match_cost(source, filter, DEFAULT_MATCH_COST)
    }

    /// Creates a query whose two-phase iterator reports `match_cost`.
    ///
    /// Equivalent to
    /// `FunctionMatchQuery(DoubleValuesSource, DoublePredicate, float)`.
    pub fn with_match_cost(
        source: Arc<dyn DoubleValuesSource>,
        filter: DoublePredicate,
        match_cost: f32,
    ) -> Self {
        Self {
            source,
            filter,
            match_cost,
        }
    }
}

impl Query for FunctionMatchQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!("FunctionMatchQuery({})", self.source.to_source_string())
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let vs = Arc::clone(&self.source).rewrite(searcher)?;
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            FunctionMatchWeight {
                vs,
                source: Arc::clone(&self.source),
                filter: Arc::clone(&self.filter),
                match_cost: self.match_cost,
                score_mode,
            },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<FunctionMatchQuery>()
                .is_some_and(|that| {
                    objects_equals_double(&self.source, &that.source)
                        && Arc::ptr_eq(&self.filter, &that.filter)
                })
    }

    /// `Objects.hash(source, filter)`.
    fn query_hash(&self) -> i32 {
        objects_hash(&[
            self.source.source_hash(),
            java_identity_hash_code_stand_in("java.util.function.DoublePredicate"),
        ])
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.function.FunctionMatchQuery"
    }
}

/// The anonymous `ConstantScoreWeight` of `FunctionMatchQuery.createWeight`.
struct FunctionMatchWeight {
    vs: Arc<dyn DoubleValuesSource>,
    source: Arc<dyn DoubleValuesSource>,
    filter: DoublePredicate,
    match_cost: f32,
    score_mode: ScoreMode,
}

impl fmt::Debug for FunctionMatchWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FunctionMatchWeight")
            .field("vs", &self.vs)
            .finish_non_exhaustive()
    }
}

impl ConstantScoreWeightImpl for FunctionMatchWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let values = Arc::clone(&self.vs).get_values(context, None)?;
        let approximation = doc_id_set_iterator::all(context.leaf_reader().max_doc())?;
        let two_phase = FunctionMatchTwoPhase {
            approximation,
            values,
            filter: Arc::clone(&self.filter),
            match_cost: self.match_cost,
        };
        let scorer =
            ConstantScoreScorer::from_two_phase(score, self.score_mode, Box::new(two_phase));
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.source.is_cacheable(ctx)
    }
}

/// The anonymous `TwoPhaseIterator` of the weight's `scorerSupplier`.
struct FunctionMatchTwoPhase {
    approximation: AllDocIdSetIterator,
    values: Box<dyn DoubleValues>,
    filter: DoublePredicate,
    match_cost: f32,
}

impl TwoPhaseIterator for FunctionMatchTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.approximation.doc_id();
        Ok(self.values.advance_exact(doc)? && (self.filter)(self.values.double_value()?))
    }

    fn match_cost(&self) -> f32 {
        self.match_cost // TODO maybe DoubleValuesSource should have a matchCost?
    }
}
