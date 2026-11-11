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

//! Port of `org.apache.lucene.tests.search.RandomApproximationQuery`.
//!
//! # Layout
//!
//! Java's `RandomApproximationScorer` builds its two-phase view over the
//! wrapped scorer's iterator, and returns
//! `TwoPhaseIterator.asDocIdSetIterator(twoPhaseView)` as its own iterator,
//! all three sharing the same objects. Rust's [`Scorer::iterator`] and
//! [`Scorer::two_phase_iterator`] return borrows, so the views are nested
//! fields: the scorer owns the iterator view, which owns the two-phase view,
//! which owns the approximation, which owns the wrapped scorer.
//!
//! **Divergence.** Rust's [`DocIdSetIterator::cost`] takes `&self` while a
//! scorer lends its iterator only through `&mut self`, so the approximation
//! over a scorer reads the iterator's cost once, when it is built.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::LeafReaderContext;
use rucene::search::similarities::Explanation;
use rucene::search::{
    DefaultScorerSupplier, DocIdSetIterator, IndexSearcher, Matches, Query, QueryVisitor, Scorable,
    ScoreMode, Scorer, ScorerSupplier, SegmentCacheable, TwoPhaseIterator, Weight, NO_MORE_DOCS,
};

/// A [`Query`] that adds random approximations to its scorers.
///
/// Equivalent to `org.apache.lucene.tests.search.RandomApproximationQuery`.
#[derive(Clone)]
pub struct RandomApproximationQuery {
    /// `query`.
    query: Arc<dyn Query>,
    /// `random`.
    random: Arc<dyn Random>,
}

impl fmt::Debug for RandomApproximationQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RandomApproximationQuery")
            .field("query", &self.query)
            .finish_non_exhaustive()
    }
}

impl RandomApproximationQuery {
    /// `new RandomApproximationQuery(query, random)`.
    pub fn new(query: Arc<dyn Query>, random: Arc<dyn Random>) -> Self {
        RandomApproximationQuery { query, random }
    }
}

impl Query for RandomApproximationQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.query.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.query.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(self.query.rewrite(searcher)?.map(|rewritten| {
            Arc::new(RandomApproximationQuery::new(
                rewritten,
                Arc::clone(&self.random),
            )) as Arc<dyn Query>
        }))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<RandomApproximationQuery>()
                .is_some_and(|other| self.query.query_eq(&*other.query))
    }

    /// `31 * classHash() + query.hashCode()`.
    fn query_hash(&self) -> u64 {
        self.class_hash()
            .wrapping_mul(31)
            .wrapping_add(self.query.query_hash())
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let weight = self.query.create_weight(searcher, score_mode, boost)?;
        let random: Arc<dyn Random> = Arc::new(JdkRandom::new(self.random.next_long()));
        Ok(Arc::new(RandomApproximationWeight {
            query: weight.get_query(),
            inner: weight,
            random,
        }))
    }
}

/// `RandomApproximationQuery.RandomApproximationWeight`, a `FilterWeight`.
struct RandomApproximationWeight {
    query: Arc<dyn Query>,
    inner: Arc<dyn Weight>,
    random: Arc<dyn Random>,
}

impl fmt::Debug for RandomApproximationWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RandomApproximationWeight")
            .field("in", &self.inner)
            .finish_non_exhaustive()
    }
}

impl SegmentCacheable for RandomApproximationWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl Weight for RandomApproximationWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.inner.explain(context, doc)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.inner.matches(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(mut scorer_supplier) = self.inner.scorer_supplier(context)? else {
            return Ok(None);
        };
        let sub_scorer = scorer_supplier.get(i64::MAX)?;
        let random: Arc<dyn Random> = Arc::new(JdkRandom::new(self.random.next_long()));
        let scorer = RandomApproximationScorer::new(sub_scorer, random);
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn count(&self, _context: &LeafReaderContext) -> Result<i32> {
        Ok(-1)
    }
}

/// The iterator a [`RandomApproximation`] approximates: Java's
/// `DocIdSetIterator disi`.
pub trait ApproximatedIterator {
    /// `disi`, mutably.
    fn disi(&mut self) -> &mut dyn DocIdSetIterator;

    /// `disi.docID()`.
    fn disi_doc_id(&self) -> i32;

    /// `disi.cost()`.
    fn disi_cost(&self) -> i64;
}

impl ApproximatedIterator for Box<dyn DocIdSetIterator> {
    fn disi(&mut self) -> &mut dyn DocIdSetIterator {
        &mut **self
    }

    fn disi_doc_id(&self) -> i32 {
        self.doc_id()
    }

    fn disi_cost(&self) -> i64 {
        self.cost()
    }
}

/// A scorer's iterator (`scorer.iterator()`), with its cost read once.
struct ScorerIterator {
    scorer: Box<dyn Scorer>,
    cost: i64,
}

impl ApproximatedIterator for ScorerIterator {
    fn disi(&mut self) -> &mut dyn DocIdSetIterator {
        self.scorer.iterator()
    }

    fn disi_doc_id(&self) -> i32 {
        self.scorer.doc_id()
    }

    fn disi_cost(&self) -> i64 {
        self.cost
    }
}

/// `RandomApproximationQuery.RandomApproximation`: an approximation that
/// returns random documents between the target and the next match.
pub struct RandomApproximation<D: ApproximatedIterator> {
    random: Arc<dyn Random>,
    disi: D,
    /// `AbstractDocIdSetIterator.doc`.
    doc: i32,
}

impl<D: ApproximatedIterator> DocIdSetIterator for RandomApproximation<D> {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if self.disi.disi_doc_id() < target {
            self.disi.disi().advance(target)?;
        }
        if self.disi.disi_doc_id() == NO_MORE_DOCS {
            self.doc = NO_MORE_DOCS;
            return Ok(NO_MORE_DOCS);
        }
        self.doc =
            RandomNumbers::random_int_between(&*self.random, target, self.disi.disi_doc_id());
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.disi.disi_cost()
    }
}

/// A wrapper around a [`DocIdSetIterator`] that matches the same documents,
/// but introduces false positives that need to be verified via
/// [`TwoPhaseIterator::matches`].
///
/// Equivalent to `RandomApproximationQuery.RandomTwoPhaseView`.
pub struct RandomTwoPhaseView<D: ApproximatedIterator> {
    approximation: RandomApproximation<D>,
    /// `lastDoc`.
    last_doc: i32,
    /// `randomMatchCost`.
    random_match_cost: f32,
}

impl RandomTwoPhaseView<Box<dyn DocIdSetIterator>> {
    /// `new RandomTwoPhaseView(random, disi)`.
    pub fn new(random: Arc<dyn Random>, disi: Box<dyn DocIdSetIterator>) -> Self {
        RandomTwoPhaseView::over(random, disi)
    }
}

impl<D: ApproximatedIterator> RandomTwoPhaseView<D> {
    /// The constructor's body over any [`ApproximatedIterator`].
    fn over(random: Arc<dyn Random>, disi: D) -> Self {
        let random_match_cost = random.next_float() * 200.0; // between 0 and 200
        RandomTwoPhaseView {
            approximation: RandomApproximation {
                random,
                disi,
                doc: -1,
            },
            last_doc: -1,
            random_match_cost,
        }
    }
}

impl<D: ApproximatedIterator> TwoPhaseIterator for RandomTwoPhaseView<D> {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.approximation.doc;
        if doc == -1 || doc == NO_MORE_DOCS {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "matches() should not be called on doc ID {doc}"
            ))));
        }
        if self.last_doc == doc {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "matches() has been called twice on doc ID {doc}"
            ))));
        }
        self.last_doc = doc;
        Ok(doc == self.approximation.disi.disi_doc_id())
    }

    fn match_cost(&self) -> f32 {
        self.random_match_cost
    }

    fn doc_id_run_end(&mut self) -> Result<i32> {
        if self.approximation.doc == self.approximation.disi.disi_doc_id() {
            return self.approximation.disi.disi().doc_id_run_end();
        }
        // `super.docIDRunEnd()`: `approximation().docID()`.
        Ok(self.approximation.doc)
    }
}

/// `TwoPhaseIterator.asDocIdSetIterator(twoPhaseView)`: Java's
/// `TwoPhaseIteratorAsDocIdSetIterator`, a `FilterDocIdSetIterator` over the
/// approximation that confirms each document with `matches()`.
struct AsDocIdSetIterator {
    two_phase: RandomTwoPhaseView<ScorerIterator>,
}

impl AsDocIdSetIterator {
    /// `doNext(doc)`.
    fn do_next(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            } else if self.two_phase.matches()? {
                return Ok(doc);
            }
            doc = self.two_phase.approximation.next_doc()?;
        }
    }
}

impl DocIdSetIterator for AsDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.two_phase.approximation.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.two_phase.approximation.next_doc()?;
        self.do_next(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.two_phase.approximation.advance(target)?;
        self.do_next(doc)
    }

    fn cost(&self) -> i64 {
        self.two_phase.approximation.cost()
    }
}

/// `RandomApproximationQuery.RandomApproximationScorer`.
struct RandomApproximationScorer {
    iterator: AsDocIdSetIterator,
}

impl RandomApproximationScorer {
    fn new(mut scorer: Box<dyn Scorer>, random: Arc<dyn Random>) -> Self {
        let cost = scorer.iterator().cost();
        RandomApproximationScorer {
            iterator: AsDocIdSetIterator {
                two_phase: RandomTwoPhaseView::over(random, ScorerIterator { scorer, cost }),
            },
        }
    }

    fn scorer(&mut self) -> &mut dyn Scorer {
        &mut *self.iterator.two_phase.approximation.disi.scorer
    }

    fn scorer_ref(&self) -> &dyn Scorer {
        &*self.iterator.two_phase.approximation.disi.scorer
    }
}

impl Scorable for RandomApproximationScorer {
    fn score(&mut self) -> Result<f32> {
        self.scorer().score()
    }
}

impl Scorer for RandomApproximationScorer {
    fn doc_id(&self) -> i32 {
        self.iterator.two_phase.approximation.doc
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.iterator
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(&mut self.iterator.two_phase)
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        let mut target = target;
        let scorer_doc = self.scorer_ref().doc_id();
        if scorer_doc > target && self.iterator.two_phase.approximation.doc != scorer_doc {
            // The random approximation can return doc ids that are not present
            // in the underlying scorer. These additional doc ids are always
            // *before* the next matching doc so we cannot use them to shallow
            // advance the main scorer which is already ahead.
            target = scorer_doc;
        }
        self.scorer().advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.scorer().get_max_score(up_to)
    }
}
