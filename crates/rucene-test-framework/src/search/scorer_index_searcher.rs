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

//! Port of `org.apache.lucene.tests.search.ScorerIndexSearcher`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::similarities::Explanation;
use rucene::search::{
    BulkScorer, Collector, DefaultBulkScorer, Executor, IndexSearcher, IndexSearcherOverrides,
    Matches, Query, Scorer, ScorerSupplier, SegmentCacheable, Weight,
};

/// An [`IndexSearcher`] that always uses the [`Scorer`] API, never a
/// specialised [`BulkScorer`].
///
/// Equivalent to `org.apache.lucene.tests.search.ScorerIndexSearcher`, a
/// subclass of `IndexSearcher`; its constructors return an [`IndexSearcher`]
/// with [`ScorerIndexSearcherOverrides`] installed.
pub struct ScorerIndexSearcher;

impl ScorerIndexSearcher {
    /// `new ScorerIndexSearcher(r, executor)`.
    ///
    /// # Errors
    ///
    /// As [`IndexSearcher::with_executor`].
    pub fn with_executor(
        r: Arc<dyn IndexReader>,
        executor: Option<Arc<dyn Executor>>,
    ) -> Result<IndexSearcher> {
        let mut searcher = IndexSearcher::with_executor(r, executor)?;
        searcher.set_overrides(Arc::new(ScorerIndexSearcherOverrides));
        Ok(searcher)
    }

    /// `new ScorerIndexSearcher(r)`.
    ///
    /// # Errors
    ///
    /// As [`IndexSearcher::new`].
    #[allow(clippy::new_ret_no_self)] // Java's constructor; the searcher is an `IndexSearcher`
    pub fn new(r: Arc<dyn IndexReader>) -> Result<IndexSearcher> {
        Self::with_executor(r, None)
    }
}

/// The overridden `searchLeaf` of `ScorerIndexSearcher`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScorerIndexSearcherOverrides;

impl IndexSearcherOverrides for ScorerIndexSearcherOverrides {
    fn search_leaf(
        &self,
        searcher: &IndexSearcher,
        ctx: &Arc<LeafReaderContext>,
        min_doc_id: i32,
        max_doc_id: i32,
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
    ) -> Result<()> {
        let filter_weight: Arc<dyn Weight> = Arc::new(ScorerOnlyWeight {
            query: weight.get_query(),
            inner: Arc::clone(weight),
        });
        searcher.super_search_leaf(ctx, min_doc_id, max_doc_id, &filter_weight, collector)
    }
}

/// The anonymous `FilterWeight` of `searchLeaf`.
#[derive(Debug)]
struct ScorerOnlyWeight {
    query: Arc<dyn Query>,
    inner: Arc<dyn Weight>,
}

impl SegmentCacheable for ScorerOnlyWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl Weight for ScorerOnlyWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.inner.explain(context, doc)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.inner.matches(context, doc)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.inner.count(context)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(self
            .inner
            .scorer_supplier(context)?
            .map(|inner| Box::new(ScorerOnlySupplier { inner }) as Box<dyn ScorerSupplier>))
    }
}

/// The anonymous `ScorerSupplier` of `scorerSupplier`: it does not forward
/// `setTopLevelScoringClause`, as Java's does not override it.
struct ScorerOnlySupplier {
    inner: Box<dyn ScorerSupplier>,
}

impl ScorerSupplier for ScorerOnlySupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        self.inner.get(lead_cost)
    }

    /// Don't delegate to `in` to make sure we get a `DefaultBulkScorer`.
    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        Ok(Box::new(DefaultBulkScorer::new(self.get(i64::MAX)?)))
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}
