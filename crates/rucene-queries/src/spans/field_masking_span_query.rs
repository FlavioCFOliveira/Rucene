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

//! Port of `FieldMaskingSpanQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::weight::Weight;
use rucene::util::java_collections::java_string_hash_code;

use crate::spans::span_query::SpanQuery;
use crate::spans::span_weight::SpanWeight;

/// Wrapper to allow span query objects to participate in composite
/// single-field span queries by 'lying' about their search field.
///
/// Equivalent to the final `org.apache.lucene.queries.spans.FieldMaskingSpanQuery`.
///
/// The masked span query functions as normal, but
/// [`SpanQuery::get_field`] simply hands back the value supplied in this
/// type's constructor. This can be used to support queries like
/// [`SpanNearQuery`](crate::spans::span_near_query::SpanNearQuery) or
/// [`SpanOrQuery`](crate::spans::span_or_query::SpanOrQuery) across different
/// fields, which is not ordinarily permitted — for example, for denormalized
/// relational data, a span near query with a slop of 0 can be applied across
/// `studentfirstname:james` and a masked `studentsurname:jones`.
///
/// Note: as [`SpanQuery::get_field`] returns the masked field, scoring will be
/// done using the similarity and collection statistics of the field name
/// supplied, but with the term statistics of the real field. This may lead to
/// exceptions, poor performance, and unexpected scoring behaviour.
#[derive(Debug, Clone)]
pub struct FieldMaskingSpanQuery {
    masked_query: Arc<dyn SpanQuery>,
    field: String,
}

impl FieldMaskingSpanQuery {
    /// Equivalent to `FieldMaskingSpanQuery(SpanQuery, String)`.
    pub fn new(masked_query: Arc<dyn SpanQuery>, masked_field: impl Into<String>) -> Self {
        Self {
            masked_query,
            field: masked_field.into(),
        }
    }

    /// Equivalent to `FieldMaskingSpanQuery.getMaskedQuery()`.
    pub fn get_masked_query(&self) -> &Arc<dyn SpanQuery> {
        &self.masked_query
    }

    /// Equivalent to the private `FieldMaskingSpanQuery.equalsTo(FieldMaskingSpanQuery)`.
    fn equals_to(&self, other: &FieldMaskingSpanQuery) -> bool {
        self.field == other.field
            && self
                .masked_query
                .query_eq(&*other.masked_query as &dyn Query)
    }
}

impl Query for FieldMaskingSpanQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        buffer.push_str("mask(");
        buffer.push_str(&self.masked_query.to_query_string(field));
        buffer.push(')');
        buffer.push_str(" as ");
        buffer.push_str(&self.field);
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            self.masked_query
                .visit(&mut *visitor.get_sub_visitor(Occur::MUST, self));
        }
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
                .downcast_ref::<FieldMaskingSpanQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    /// `classHash() ^ getMaskedQuery().hashCode() ^ getField().hashCode()`.
    fn query_hash(&self) -> i32 {
        self.class_hash() ^ self.masked_query.query_hash() ^ java_string_hash_code(&self.field)
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.FieldMaskingSpanQuery"
    }
}

impl SpanQuery for FieldMaskingSpanQuery {
    fn get_field(&self) -> Option<&str> {
        Some(&self.field)
    }

    // :NOTE: getBoost and setBoost are not proxied to the maskedQuery
    // ...this is done to be more consistent with things like SpanFirstQuery

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        self.masked_query
            .create_span_weight(searcher, score_mode, boost)
    }

    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        if let Some(rewritten) = self.masked_query.rewrite_span(searcher)? {
            return Ok(Some(Arc::new(FieldMaskingSpanQuery::new(
                rewritten,
                self.field.clone(),
            ))));
        }

        Ok(None)
    }
}
