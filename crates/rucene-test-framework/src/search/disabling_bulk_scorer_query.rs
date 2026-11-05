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

//! Port of `org.apache.lucene.tests.search.DisablingBulkScorerQuery`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::similarities::Explanation;
use rucene::search::{
    BulkScorer, DefaultBulkScorer, IndexSearcher, Matches, Query, QueryVisitor, ScoreMode, Scorer,
    ScorerSupplier, SegmentCacheable, Weight,
};

/// A query that wraps another query and ensures that its scorer supplier's
/// bulk scorer is a [`DefaultBulkScorer`] over its scorer.
///
/// Equivalent to `org.apache.lucene.tests.search.DisablingBulkScorerQuery`.
#[derive(Debug, Clone)]
pub struct DisablingBulkScorerQuery {
    /// `query`.
    query: Arc<dyn Query>,
}

impl DisablingBulkScorerQuery {
    /// `new DisablingBulkScorerQuery(query)`.
    pub fn new(query: Arc<dyn Query>) -> Self {
        DisablingBulkScorerQuery { query }
    }
}

impl Query for DisablingBulkScorerQuery {
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
        Ok(self
            .query
            .rewrite(searcher)?
            .map(|rewritten| Arc::new(DisablingBulkScorerQuery::new(rewritten)) as Arc<dyn Query>))
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let inner = self.query.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(DisablingWeight {
            query: inner.get_query(),
            inner,
        }))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<DisablingBulkScorerQuery>()
                .is_some_and(|other| self.query.query_eq(&*other.query))
    }

    /// `31 * classHash() + query.hashCode()`.
    fn query_hash(&self) -> u64 {
        self.class_hash()
            .wrapping_mul(31)
            .wrapping_add(self.query.query_hash())
    }
}

/// The anonymous `FilterWeight` of `createWeight`.
#[derive(Debug)]
struct DisablingWeight {
    query: Arc<dyn Query>,
    inner: Arc<dyn Weight>,
}

impl SegmentCacheable for DisablingWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl Weight for DisablingWeight {
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

    /// Java wraps the inner supplier even when it is `null` (no match in the
    /// leaf): the supplier is returned, and its first `get` or `cost` fails
    /// with a `NullPointerException`; reproduced.
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let scorer_supplier = self.inner.scorer_supplier(context)?;
        Ok(Some(Box::new(DisablingScorerSupplier { scorer_supplier })))
    }
}

/// The anonymous `ScorerSupplier` of `scorerSupplier`.
struct DisablingScorerSupplier {
    /// The captured `scorerSupplier`, `null` when the wrapped weight matches
    /// nothing in the leaf.
    scorer_supplier: Option<Box<dyn ScorerSupplier>>,
}

/// The JDK's helpful message for a call on the `null` captured supplier.
fn npe_message(method: &str) -> String {
    format!(
        "Cannot invoke \"org.apache.lucene.search.ScorerSupplier.{method}\" because \"this.val$scorerSupplier\" is null"
    )
}

impl ScorerSupplier for DisablingScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        match self.scorer_supplier.as_mut() {
            Some(scorer_supplier) => scorer_supplier.get(lead_cost),
            None => Err(LuceneError::unchecked(
                "java.lang.NullPointerException",
                npe_message("get(long)"),
            )),
        }
    }

    fn cost(&self) -> i64 {
        match self.scorer_supplier.as_ref() {
            Some(scorer_supplier) => scorer_supplier.cost(),
            None => Throwable::new("java.lang.NullPointerException", npe_message("cost()")).throw(),
        }
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        Ok(Box::new(DefaultBulkScorer::new(self.get(i64::MAX)?)))
    }
}
