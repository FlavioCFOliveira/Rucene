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

//! Port of `SpanPositionRangeQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::weight::Weight;

use crate::spans::filter_spans::AcceptStatus;
use crate::spans::span_position_check_query::{
    span_position_check_create_weight, span_position_check_eq, span_position_check_hash,
    span_position_check_rewrite, span_position_check_visit, SpanPositionCheckQuery,
};
use crate::spans::span_query::SpanQuery;
use crate::spans::span_weight::SpanWeight;
use crate::spans::spans::Spans;

/// Checks to see if the [`get_match`](SpanPositionCheckQuery::get_match) lies
/// between a start and end position.
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanPositionRangeQuery`,
/// which extends `SpanPositionCheckQuery`. See
/// [`SpanFirstQuery`](crate::spans::span_first_query::SpanFirstQuery) for a
/// derivation that is optimized for the case where start position is 0.
#[derive(Debug, Clone)]
pub struct SpanPositionRangeQuery {
    /// The protected `match` field it inherits.
    match_query: Arc<dyn SpanQuery>,
    /// The protected `start` field.
    start: i32,
    /// The protected `end` field.
    end: i32,
}

impl SpanPositionRangeQuery {
    /// Equivalent to `SpanPositionRangeQuery(SpanQuery, int, int)`.
    pub fn new(match_query: Arc<dyn SpanQuery>, start: i32, end: i32) -> Self {
        Self {
            match_query,
            start,
            end,
        }
    }

    /// Returns the minimum position permitted in a match.
    ///
    /// Equivalent to `SpanPositionRangeQuery.getStart()`.
    pub fn get_start(&self) -> i32 {
        self.start
    }

    /// Returns the maximum end position permitted in a match.
    ///
    /// Equivalent to `SpanPositionRangeQuery.getEnd()`.
    pub fn get_end(&self) -> i32 {
        self.end
    }
}

impl Query for SpanPositionRangeQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        buffer.push_str("spanPosRange(");
        buffer.push_str(&self.match_query.to_query_string(field));
        buffer.push_str(&format!(", {}, ", self.start));
        buffer.push_str(&self.end.to_string());
        buffer.push(')');
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        span_position_check_visit(self, visitor);
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

    /// `super.equals(o)`, then `end` and `start`.
    fn query_eq(&self, other: &dyn Query) -> bool {
        if !span_position_check_eq(self, other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<SpanPositionRangeQuery>() else {
            return false;
        };
        self.end == other.end && self.start == other.start
    }

    fn query_hash(&self) -> i32 {
        let mut h = span_position_check_hash(self) ^ self.end;
        h = h.wrapping_mul(127) ^ self.start;
        h
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.SpanPositionRangeQuery"
    }
}

impl SpanQuery for SpanPositionRangeQuery {
    fn get_field(&self) -> Option<&str> {
        self.match_query.get_field()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        span_position_check_create_weight(Arc::new(self.clone()), searcher, score_mode, boost)
    }

    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        span_position_check_rewrite(self, searcher)
    }
}

impl SpanPositionCheckQuery for SpanPositionRangeQuery {
    fn get_match(&self) -> &Arc<dyn SpanQuery> {
        &self.match_query
    }

    fn accept_position(&self, spans: &mut dyn Spans) -> Result<AcceptStatus> {
        debug_assert!(spans.start_position() != spans.end_position());
        let res = if spans.start_position() >= self.end {
            AcceptStatus::NoMoreInCurrentDoc
        } else if spans.start_position() >= self.start && spans.end_position() <= self.end {
            AcceptStatus::Yes
        } else {
            AcceptStatus::No
        };
        Ok(res)
    }

    fn with_match(&self, match_query: Arc<dyn SpanQuery>) -> Arc<dyn SpanPositionCheckQuery> {
        let mut clone = self.clone();
        clone.match_query = match_query;
        Arc::new(clone)
    }
}
