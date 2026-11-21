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

//! Port of `AssertingSpanQuery.java` of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/spans/`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::weight::Weight;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_weight::SpanWeight;

use super::asserting_span_weight::AssertingSpanWeight;

/// Wraps a span query with asserts.
///
/// Equivalent to `org.apache.lucene.queries.spans.AssertingSpanQuery`.
#[derive(Debug, Clone)]
pub struct AssertingSpanQuery {
    /// `in`.
    in_query: Arc<dyn SpanQuery>,
}

impl AssertingSpanQuery {
    /// `AssertingSpanQuery(SpanQuery in)`.
    pub fn new(in_query: Arc<dyn SpanQuery>) -> Self {
        Self { in_query }
    }

    /// The wrapped query.
    pub fn in_query(&self) -> &Arc<dyn SpanQuery> {
        &self.in_query
    }
}

impl Query for AssertingSpanQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "AssertingSpanQuery({})",
            self.in_query.to_query_string(field)
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.in_query.visit(visitor);
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
        Ok(self.create_span_weight(searcher, score_mode, boost)?)
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(self.rewrite_span(searcher)?.map(|q| q as Arc<dyn Query>))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<AssertingSpanQuery>()
                .is_some_and(|other| self.in_query.query_eq(&*other.in_query as &dyn Query))
    }

    fn query_hash(&self) -> i32 {
        self.in_query.query_hash()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.AssertingSpanQuery"
    }
}

impl SpanQuery for AssertingSpanQuery {
    fn get_field(&self) -> Option<&str> {
        self.in_query.get_field()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        let weight = self
            .in_query
            .create_span_weight(searcher, score_mode, boost)?;
        Ok(AssertingSpanWeight::new(
            searcher,
            weight,
            Arc::clone(&self.in_query),
        )?)
    }

    /// `Query q = in.rewrite(indexSearcher); if (q == in) return
    /// super.rewrite(indexSearcher); else if (q instanceof SpanQuery) return
    /// new AssertingSpanQuery((SpanQuery) q); else return q;`: a span query
    /// rewrites to a span query (`SpanQuery::rewrite_span`), and
    /// `Query.rewrite` returns `this`.
    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        match self.in_query.rewrite_span(searcher)? {
            None => Ok(None),
            Some(q) => Ok(Some(Arc::new(AssertingSpanQuery::new(q)))),
        }
    }
}
