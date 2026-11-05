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

//! Port of `org.apache.lucene.tests.search.AssertingQuery`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::Result;
use rucene::search::{IndexSearcher, Query, QueryVisitor, ScoreMode, Weight};

use crate::search::asserting_weight::AssertingWeight;

/// Assertion-enabled query: its weight is an [`AssertingWeight`].
///
/// Equivalent to `org.apache.lucene.tests.search.AssertingQuery`.
pub struct AssertingQuery {
    /// `random`.
    random: Arc<dyn Random>,
    /// `in`.
    inner: Arc<dyn Query>,
}

impl fmt::Debug for AssertingQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssertingQuery")
            .field("in", &self.inner)
            .finish_non_exhaustive()
    }
}

impl AssertingQuery {
    /// `new AssertingQuery(random, in)`.
    pub fn new(random: Arc<dyn Random>, inner: Arc<dyn Query>) -> AssertingQuery {
        AssertingQuery { random, inner }
    }

    /// `AssertingQuery.wrap(random, query)`: `query` itself when it already is
    /// an `AssertingQuery`.
    pub fn wrap(random: Arc<dyn Random>, query: Arc<dyn Query>) -> Arc<dyn Query> {
        if query.as_any().is::<AssertingQuery>() {
            query
        } else {
            Arc::new(AssertingQuery::new(random, query))
        }
    }

    /// `getRandom()`.
    pub fn get_random(&self) -> &Arc<dyn Random> {
        &self.random
    }

    /// `getIn()`.
    pub fn get_in(&self) -> &Arc<dyn Query> {
        &self.inner
    }
}

impl Query for AssertingQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(visitor);
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
        let non_negative = boost >= 0.0;
        java_assert!(non_negative);
        // Java evaluates the arguments in order: the random first.
        let random: Arc<dyn Random> = Arc::new(JdkRandom::new(self.random.next_long()));
        let inner = self.inner.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(AssertingWeight::new(random, inner, score_mode)))
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.inner.rewrite(searcher)? {
            // `rewritten == in`: `super.rewrite(indexSearcher)`, this query.
            None => Ok(None),
            Some(rewritten) => {
                let random: Arc<dyn Random> = Arc::new(JdkRandom::new(self.random.next_long()));
                Ok(Some(AssertingQuery::wrap(random, rewritten)))
            }
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<AssertingQuery>()
                .is_some_and(|other| self.inner.query_eq(&*other.inner))
    }

    /// `-in.hashCode()`.
    fn query_hash(&self) -> u64 {
        self.inner.query_hash().wrapping_neg()
    }
}
