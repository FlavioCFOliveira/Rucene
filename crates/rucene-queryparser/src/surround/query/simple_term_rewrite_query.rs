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

//! The port of `SimpleTermRewriteQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::{Occur, Query};

use super::basic_query_factory::BasicQueryFactory;
use super::rewrite_query::RewriteQuery;
use super::simple_term::{MatchingTermVisitor, SimpleTerm};
use super::srnd_boolean_query::SrndBooleanQuery;
use super::srnd_query::SrndQuery;

/// Rewrites a `SimpleTerm` into the term queries of its matching terms.
///
/// Equivalent to the package-private
/// `org.apache.lucene.queryparser.surround.query.SimpleTermRewriteQuery`.
#[derive(Debug)]
pub(crate) struct SimpleTermRewriteQuery {
    base: RewriteQuery,
    simple_term: Arc<dyn SimpleTerm>,
}

impl SimpleTermRewriteQuery {
    /// Equivalent to the constructor `SimpleTermRewriteQuery(SimpleTerm,
    /// String, BasicQueryFactory)`.
    ///
    /// # Errors
    ///
    /// As [`RewriteQuery::new`].
    pub(crate) fn new<T: SimpleTerm + 'static>(
        srnd_query: Arc<T>,
        field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Self> {
        let srnd: Arc<dyn SrndQuery> = Arc::clone(&srnd_query) as Arc<dyn SrndQuery>;
        Ok(Self {
            base: RewriteQuery::new(srnd, field_name, qf)?,
            simple_term: srnd_query,
        })
    }
}

impl Query for SimpleTermRewriteQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.base.to_string(self.java_class_name(), field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        // TODO: implement this
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        struct Collect<'a> {
            lucene_sub_queries: Vec<Arc<dyn Query>>,
            qf: &'a BasicQueryFactory,
        }
        impl MatchingTermVisitor for Collect<'_> {
            fn visit_matching_term(&mut self, term: Term) -> Result<()> {
                self.lucene_sub_queries
                    .push(Arc::new(self.qf.new_term_query(term)?));
                Ok(())
            }
        }
        let mut collect = Collect {
            lucene_sub_queries: Vec::new(),
            qf: &self.base.qf,
        };
        self.simple_term.visit_matching_terms(
            index_searcher.get_index_reader(),
            &self.base.field_name,
            &mut collect,
        )?;
        let lucene_sub_queries = collect.lucene_sub_queries;
        Ok(Some(match lucene_sub_queries.len() {
            0 => Arc::new(MatchNoDocsQuery::instance()),
            1 => Arc::clone(&lucene_sub_queries[0]),
            _ => SrndBooleanQuery::make_boolean_query(
                /* luceneSubQueries all have default weight */
                &lucene_sub_queries,
                Occur::SHOULD,
            )?, /* OR the subquery terms */
        }))
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
        "org.apache.lucene.queryparser.surround.query.SimpleTermRewriteQuery"
    }
}
