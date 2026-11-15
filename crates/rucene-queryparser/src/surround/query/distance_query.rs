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

//! The port of `DistanceQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::Query;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;

use super::basic_query_factory::BasicQueryFactory;
use super::composed_query::{self, ComposedQuery, ComposedQueryState};
use super::distance_rewrite_query::DistanceRewriteQuery;
use super::distance_sub_query::DistanceSubQuery;
use super::or_query::class_cast;
use super::span_near_clause_factory::SpanNearClauseFactory;
use super::srnd_query::{SrndQuery, SrndQueryState};

/// Factory for NEAR queries.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.DistanceQuery`.
#[derive(Debug, Clone)]
pub struct DistanceQuery {
    composed: ComposedQueryState,
    op_distance: i32,
    ordered: bool,
}

impl DistanceQuery {
    /// Equivalent to `new DistanceQuery(List<SrndQuery>, boolean, int,
    /// String, boolean)`.
    ///
    /// # Errors
    ///
    /// As [`ComposedQueryState::new`].
    pub fn new(
        queries: Vec<Arc<dyn SrndQuery>>,
        infix: bool,
        op_distance: i32,
        op_name: &str,
        ordered: bool,
    ) -> Result<Self> {
        Ok(Self {
            composed: ComposedQueryState::new(queries, infix, op_name)?,
            op_distance, /* the distance indicated in the operator */
            ordered,
        })
    }

    /// Equivalent to `getOpDistance()`.
    pub fn get_op_distance(&self) -> i32 {
        self.op_distance
    }

    /// Equivalent to `subQueriesOrdered()`.
    pub fn sub_queries_ordered(&self) -> bool {
        self.ordered
    }

    /// Equivalent to `getSpanNearQuery(IndexReader, String,
    /// BasicQueryFactory)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the index access, `TooManyBasicQueries`, and
    /// the `ClassCastException` of a sub-query that is not a
    /// `DistanceSubQuery`.
    pub fn get_span_near_query(
        &self,
        reader: &Arc<dyn IndexReader>,
        field_name: &str,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Arc<dyn Query>> {
        let mut span_clauses: Vec<Arc<dyn SpanQuery>> =
            Vec::with_capacity(self.get_nr_sub_queries() as usize);
        let mut sqi = self.get_sub_queries_iterator();
        while let Some(sq) = sqi.next() {
            let mut sncf =
                SpanNearClauseFactory::new(Arc::clone(reader), field_name, Arc::clone(qf));

            sq.as_distance_sub_query()
                .ok_or_else(|| class_cast(sq.as_ref()))?
                .add_span_queries(&mut sncf)?;
            if sncf.size() == 0 {
                /* distance operator requires all sub queries */
                for rest in sqi.by_ref() {
                    /* produce evt. error messages but ignore results */
                    rest.as_distance_sub_query()
                        .ok_or_else(|| class_cast(rest.as_ref()))?
                        .add_span_queries(&mut sncf)?;
                    sncf.clear();
                }
                return Ok(Arc::new(MatchNoDocsQuery::instance()));
            }

            span_clauses.push(sncf.make_span_clause()?);
        }

        Ok(Arc::new(SpanNearQuery::new(
            span_clauses,
            self.get_op_distance() - 1,
            self.sub_queries_ordered(),
        )?))
    }
}

impl ComposedQuery for DistanceQuery {
    fn composed(&self) -> &ComposedQueryState {
        &self.composed
    }
}

impl SrndQuery for DistanceQuery {
    fn srnd_state(&self) -> &SrndQueryState {
        &self.composed.srnd
    }

    fn srnd_to_string(&self) -> String {
        composed_query::to_string(self)
    }

    fn is_fields_sub_query_acceptable(&self) -> bool {
        composed_query::is_fields_sub_query_acceptable(self)
    }

    fn clone_srnd(&self) -> Arc<dyn SrndQuery> {
        Arc::new(self.clone())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queryparser.surround.query.DistanceQuery"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn make_lucene_query_field_no_boost(
        self: Arc<Self>,
        field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(DistanceRewriteQuery::new(self, field_name, qf)?))
    }

    fn as_distance_sub_query(&self) -> Option<&dyn DistanceSubQuery> {
        Some(self)
    }
}

impl DistanceSubQuery for DistanceQuery {
    fn distance_sub_query_not_allowed(&self) -> Option<String> {
        for leq in self.get_sub_queries_iterator() {
            if let Some(dsq) = leq.as_distance_sub_query() {
                let m = dsq.distance_sub_query_not_allowed();
                if m.is_some() {
                    return m;
                }
            } else {
                return Some(format!(
                    "Operator {} does not allow subquery {}",
                    self.get_operator_name(),
                    leq.srnd_to_string()
                ));
            }
        }
        None /* subqueries acceptable */
    }

    fn add_span_queries(&self, sncf: &mut SpanNearClauseFactory) -> Result<()> {
        let snq = self.get_span_near_query(
            sncf.get_index_reader(),
            sncf.get_field_name(),
            sncf.get_basic_query_factory(),
        )?;
        sncf.add_span_query(snq)
    }
}
