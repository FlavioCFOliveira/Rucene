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

//! The port of `DistanceRewriteQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::Query;

use super::basic_query_factory::BasicQueryFactory;
use super::distance_query::DistanceQuery;
use super::rewrite_query::RewriteQuery;
use super::srnd_query::SrndQuery;

/// Rewrites a `DistanceQuery` into a span near query.
///
/// Equivalent to the package-private
/// `org.apache.lucene.queryparser.surround.query.DistanceRewriteQuery`.
#[derive(Debug)]
pub(crate) struct DistanceRewriteQuery {
    base: RewriteQuery,
    distance_query: Arc<DistanceQuery>,
}

impl DistanceRewriteQuery {
    /// Equivalent to the constructor `DistanceRewriteQuery(DistanceQuery,
    /// String, BasicQueryFactory)`.
    ///
    /// # Errors
    ///
    /// As [`RewriteQuery::new`].
    pub(crate) fn new(
        srnd_query: Arc<DistanceQuery>,
        field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Self> {
        let srnd: Arc<dyn SrndQuery> = Arc::clone(&srnd_query) as Arc<dyn SrndQuery>;
        Ok(Self {
            base: RewriteQuery::new(srnd, field_name, qf)?,
            distance_query: srnd_query,
        })
    }
}

impl Query for DistanceRewriteQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.base.to_string(self.java_class_name(), field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        // TODO implement this
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        self.distance_query
            .get_span_near_query(
                index_searcher.get_index_reader(),
                &self.base.field_name,
                &self.base.qf,
            )
            .map(Some)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<Self>()
                .is_some_and(|other| self.base.equals_to(&other.base))
    }

    fn query_hash(&self) -> i32 {
        self.base.hash_code(self.class_hash())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queryparser.surround.query.DistanceRewriteQuery"
    }
}
