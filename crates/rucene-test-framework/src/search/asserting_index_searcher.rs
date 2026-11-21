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

//! Port of `org.apache.lucene.tests.search.AssertingIndexSearcher`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::Result;
use rucene::index::{IndexReader, IndexReaderContext};
use rucene::search::{
    Collector, Executor, IndexSearcher, IndexSearcherOverrides, LeafReaderContextPartition, Query,
    ScoreMode, Weight,
};

use crate::search::asserting_collector::{AssertingCollector, CollectorRef};
use crate::search::asserting_weight::AssertingWeight;
use crate::search::query_utils::QueryUtils;

/// Helper class that adds some extra checks to ensure correct usage of
/// [`IndexSearcher`] and [`Weight`].
///
/// Equivalent to `org.apache.lucene.tests.search.AssertingIndexSearcher`, a
/// subclass of `IndexSearcher`. Its constructors return an [`IndexSearcher`]
/// with [`AssertingIndexSearcherOverrides`] installed (see
/// [`IndexSearcherOverrides`]).
pub struct AssertingIndexSearcher;

impl AssertingIndexSearcher {
    /// `new AssertingIndexSearcher(random, r)`.
    ///
    /// # Errors
    ///
    /// As [`IndexSearcher::new`].
    #[allow(clippy::new_ret_no_self)] // Java's constructor; the searcher is an `IndexSearcher`
    pub fn new(random: &dyn Random, r: Arc<dyn IndexReader>) -> Result<IndexSearcher> {
        Ok(Self::install(random, IndexSearcher::new(r)?))
    }

    /// `new AssertingIndexSearcher(random, context)`.
    ///
    /// # Errors
    ///
    /// As [`IndexSearcher::from_context`].
    pub fn from_context(
        random: &dyn Random,
        context: Arc<dyn IndexReaderContext>,
    ) -> Result<IndexSearcher> {
        Ok(Self::install(
            random,
            IndexSearcher::from_context(context, None)?,
        ))
    }

    /// `new AssertingIndexSearcher(random, r, ex)`.
    ///
    /// # Errors
    ///
    /// As [`IndexSearcher::with_executor`].
    pub fn with_executor(
        random: &dyn Random,
        r: Arc<dyn IndexReader>,
        ex: Option<Arc<dyn Executor>>,
    ) -> Result<IndexSearcher> {
        Ok(Self::install(random, IndexSearcher::with_executor(r, ex)?))
    }

    /// `new AssertingIndexSearcher(random, context, ex)`.
    ///
    /// # Errors
    ///
    /// As [`IndexSearcher::from_context`].
    pub fn from_context_with_executor(
        random: &dyn Random,
        context: Arc<dyn IndexReaderContext>,
        ex: Option<Arc<dyn Executor>>,
    ) -> Result<IndexSearcher> {
        Ok(Self::install(
            random,
            IndexSearcher::from_context(context, ex)?,
        ))
    }

    /// The body every constructor runs after `super(…)`:
    /// `this.random = new Random(random.nextLong())`.
    fn install(random: &dyn Random, mut searcher: IndexSearcher) -> IndexSearcher {
        searcher.set_overrides(Arc::new(AssertingIndexSearcherOverrides::new(random)));
        searcher
    }
}

/// The overridden methods of `AssertingIndexSearcher`.
pub struct AssertingIndexSearcherOverrides {
    /// `random`.
    random: Arc<dyn Random>,
}

impl fmt::Debug for AssertingIndexSearcherOverrides {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingIndexSearcherOverrides")
    }
}

impl AssertingIndexSearcherOverrides {
    /// `this.random = new Random(random.nextLong())`.
    pub fn new(random: &dyn Random) -> AssertingIndexSearcherOverrides {
        AssertingIndexSearcherOverrides {
            random: Arc::new(JdkRandom::new(random.next_long())),
        }
    }

    /// The searcher's own random (Java's package-private `random` field).
    pub fn random(&self) -> &Arc<dyn Random> {
        &self.random
    }
}

impl IndexSearcherOverrides for AssertingIndexSearcherOverrides {
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        query: Arc<dyn Query>,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        // this adds assertions to the inner weights/scorers too
        Ok(Arc::new(AssertingWeight::new(
            Arc::clone(&self.random),
            searcher.super_create_weight(query, score_mode, boost)?,
            score_mode,
        )))
    }

    fn rewrite(
        &self,
        searcher: &IndexSearcher,
        original: Arc<dyn Query>,
    ) -> Result<Arc<dyn Query>> {
        // TODO: use the more sophisticated QueryUtils.check sometimes!
        QueryUtils::check(&*original);
        let rewritten = searcher.super_rewrite(original)?;
        QueryUtils::check(&*rewritten);
        Ok(rewritten)
    }

    fn search_partitions(
        &self,
        searcher: &IndexSearcher,
        partitions: &[LeafReaderContextPartition],
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
    ) -> Result<()> {
        java_assert!(weight
            .as_any()
            .is_some_and(|any| any.is::<AssertingWeight>()));
        let mut asserting_collector = AssertingCollector::wrap(CollectorRef(collector));
        searcher.super_search_partitions(partitions, weight, &mut asserting_collector)?;
        java_assert!(asserting_collector.has_finished_collecting_previous_leaf());
        Ok(())
    }

    fn fmt(&self, searcher: &IndexSearcher, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingIndexSearcher(")?;
        searcher.super_fmt(f)?;
        f.write_str(")")
    }
}
