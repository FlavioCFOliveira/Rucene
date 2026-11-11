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

//! Port of `org.apache.lucene.tests.search.AssertingWeight`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::LeafReaderContext;
use rucene::search::similarities::Explanation;
use rucene::search::{
    BulkScorer, DefaultBulkScorer, Matches, Query, ScoreMode, Scorer, ScorerSupplier,
    SegmentCacheable, Weight,
};

use crate::search::asserting_bulk_scorer::AssertingBulkScorer;
use crate::search::asserting_matches::AssertingMatches;
use crate::search::asserting_scorer::AssertingScorer;
use crate::util::lucene_test_case::LuceneTestCase;

/// A [`Weight`] whose scorers, bulk scorers and matches are wrapped with
/// assertions, and whose counts are checked.
///
/// Equivalent to the package-private
/// `org.apache.lucene.tests.search.AssertingWeight`, a `FilterWeight`.
pub struct AssertingWeight {
    /// `random`.
    random: Arc<dyn Random>,
    /// `in`.
    inner: Arc<dyn Weight>,
    /// `scoreMode`.
    score_mode: ScoreMode,
    /// `FilterWeight`'s query, `in.getQuery()`.
    query: Arc<dyn Query>,
}

impl fmt::Debug for AssertingWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssertingWeight")
            .field("in", &self.inner)
            .field("score_mode", &self.score_mode)
            .finish_non_exhaustive()
    }
}

impl AssertingWeight {
    /// `new AssertingWeight(random, in, scoreMode)`.
    pub fn new(random: Arc<dyn Random>, inner: Arc<dyn Weight>, score_mode: ScoreMode) -> Self {
        let query = inner.get_query();
        AssertingWeight {
            random,
            inner,
            score_mode,
            query,
        }
    }

    /// `FilterWeight.in`.
    pub fn get_in(&self) -> &Arc<dyn Weight> {
        &self.inner
    }
}

impl SegmentCacheable for AssertingWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl Weight for AssertingWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.inner.explain(context, doc)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let count = self.inner.count(context)?;
        let num_docs = context.leaf_reader().num_docs();
        if count < -1 || count > num_docs {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "count={count}, numDocs={num_docs}"
            ))));
        }
        Ok(count)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        Ok(self
            .inner
            .matches(context, doc)?
            .map(|matches| Arc::new(AssertingMatches::new(matches)) as Arc<dyn Matches>))
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(in_scorer_supplier) = self.inner.scorer_supplier(context)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(AssertingScorerSupplier {
            in_scorer_supplier,
            random: Arc::clone(&self.random),
            score_mode: self.score_mode,
            max_doc: context.leaf_reader().max_doc(),
            get_called: false,
            top_level_scoring_clause: false,
        })))
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

/// The anonymous `ScorerSupplier` of `AssertingWeight.scorerSupplier`.
struct AssertingScorerSupplier {
    in_scorer_supplier: Box<dyn ScorerSupplier>,
    random: Arc<dyn Random>,
    score_mode: ScoreMode,
    /// `context.reader().maxDoc()`.
    max_doc: i32,
    /// `getCalled`.
    get_called: bool,
    /// `topLevelScoringClause`.
    top_level_scoring_clause: bool,
}

impl ScorerSupplier for AssertingScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        java_assert!(!self.get_called);
        self.get_called = true;
        java_assert!(lead_cost >= 0, "{lead_cost}");
        let can_score = self.score_mode.needs_scores();
        let can_set_min_competitive_score =
            self.score_mode == ScoreMode::TOP_SCORES && self.top_level_scoring_clause;
        Ok(AssertingScorer::wrap(
            self.in_scorer_supplier.get(lead_cost)?,
            can_score,
            can_set_min_competitive_score,
        ))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        java_assert!(!self.get_called);
        // We explicitly test both the delegate's bulk scorer, and also the
        // normal scorer. This ensures that normal scorers are sometimes tested
        // with an asserting wrapper.
        let in_scorer = if LuceneTestCase::usually_with(&*self.random) {
            self.get_called = true;
            self.in_scorer_supplier.bulk_scorer()?
        } else {
            // Don't set getCalled = true, since this calls #get under the hood
            // (`ScorerSupplier.bulkScorer()`'s default body).
            let in_scorer: Box<dyn BulkScorer> =
                Box::new(DefaultBulkScorer::new(self.get(i64::MAX)?));
            java_assert!(self.get_called);
            in_scorer
        };
        let random: Arc<dyn Random> = Arc::new(JdkRandom::new(self.random.next_long()));
        Ok(
            AssertingBulkScorer::wrap(random, Some(in_scorer), self.max_doc)
                .expect("INVARIANT: wrapping a bulk scorer yields one"),
        )
    }

    fn cost(&self) -> i64 {
        let cost = self.in_scorer_supplier.cost();
        java_assert_unchecked!(cost >= 0);
        cost
    }

    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        java_assert!(!self.get_called);
        self.top_level_scoring_clause = true;
        self.in_scorer_supplier.set_top_level_scoring_clause()
    }
}
