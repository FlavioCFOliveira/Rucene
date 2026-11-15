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

//! The port of `DistanceSubQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use rucene::error::Result;

use super::span_near_clause_factory::SpanNearClauseFactory;

/// Interface for queries that can be nested as subqueries into a span near.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.DistanceSubQuery`.
pub trait DistanceSubQuery {
    /// When `distance_sub_query_not_allowed()` returns non null, the reason
    /// why the subquery is not allowed as a distance subquery is returned.
    /// When `distance_sub_query_not_allowed()` returns `None`,
    /// `add_span_queries` can be used in the creation of the span near
    /// clause for the subquery.
    ///
    /// Equivalent to `distanceSubQueryNotAllowed()`.
    fn distance_sub_query_not_allowed(&self) -> Option<String>;

    /// Equivalent to `addSpanQueries(SpanNearClauseFactory)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the index access and `TooManyBasicQueries`.
    fn add_span_queries(&self, sncf: &mut SpanNearClauseFactory) -> Result<()>;
}
