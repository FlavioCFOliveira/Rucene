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

//! Port of `MinShouldMatchNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::boolean_query::{BooleanQuery, Builder};
use rucene::search::Query;

use super::standard_query_builder::{built_query, cast, query_class_cast, query_object};
use crate::flexible::core::builders::query_builder::QueryBuilder;
use crate::flexible::core::nodes::query_node::{JavaObjectRef, QueryNodeRef};
use crate::flexible::standard::nodes::min_should_match_node::{
    MinShouldMatchNode, MIN_SHOULD_MATCH_NODE_CLASS,
};

/// Builds a `BooleanQuery` from all of the clauses in a given
/// `BooleanQueryNode` group, with a minimum number of clauses that should
/// match.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.MinShouldMatchNodeBuilder`,
/// a `QueryBuilder` whose `build` returns a `Query`.
#[derive(Debug, Default)]
pub struct MinShouldMatchNodeBuilder;

impl MinShouldMatchNodeBuilder {
    /// Equivalent to the implicit `new MinShouldMatchNodeBuilder()`.
    pub fn new() -> Self {
        Self
    }

    /// Equivalent to `build(QueryNode queryNode)`, which returns a `Query`.
    ///
    /// # Errors
    ///
    /// `RuntimeException("Unexpected number of node children: <n>")`, the
    /// `ClassCastException` of a group whose query is not a `BooleanQuery`
    /// (the query of a group of one clause), the `NullPointerException` of a
    /// group without a query, and the `TooManyClauses` of adding the clauses.
    pub fn build_query(&self, query_node: &QueryNodeRef) -> Result<Arc<dyn Query>> {
        let mm_node: &MinShouldMatchNode = cast(query_node, &MIN_SHOULD_MATCH_NODE_CLASS)?;
        let children = query_node
            .get_children()
            .ok_or_else(LuceneError::null_pointer)?;
        if children.len() != 1 {
            return Err(LuceneError::unchecked(
                "java.lang.RuntimeException",
                format!("Unexpected number of node children: {}", children.len()),
            ));
        }
        let group = mm_node
            .group_query_node
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?;
        let q = built_query(group.as_ref())?;
        // `(BooleanQuery) q`, then `booleanQuery.clauses()`
        let q = q.ok_or_else(LuceneError::null_pointer)?;
        let boolean_query = q
            .as_any()
            .downcast_ref::<BooleanQuery>()
            .ok_or_else(|| query_class_cast(q.as_ref(), "org.apache.lucene.search.BooleanQuery"))?;
        let mut builder = Builder::new();
        builder.set_minimum_number_should_match(mm_node.min_should_match);
        for clause in boolean_query.clauses() {
            builder.add_clause(clause.clone())?;
        }
        Ok(Arc::new(builder.build()))
    }
}

impl QueryBuilder for MinShouldMatchNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<JavaObjectRef>> {
        Ok(Some(query_object(self.build_query(query_node)?)))
    }
}
