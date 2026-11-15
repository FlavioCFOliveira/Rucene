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

//! Port of `SpanOrQueryNodeBuilder.java`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::Query;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queryparser::flexible::core::builders::query_tree_builder::QueryTreeBuilder;
use rucene_queryparser::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::core::nodes::query_node::QueryNodeRef;
use rucene_queryparser::flexible::standard::builders::standard_query_builder::{
    check_cast, StandardQueryBuilder,
};

use super::span_query_cast::span_query_of;

/// This builder creates `SpanOrQuery`s from a `BooleanQueryNode`.
///
/// It assumes that the `BooleanQueryNode` instance has at least one child.
#[derive(Debug, Default)]
pub struct SpanOrQueryNodeBuilder;

impl SpanOrQueryNodeBuilder {
    /// The covariant `SpanOrQuery build(QueryNode node)`.
    pub fn build_span_or(&self, node: &QueryNodeRef) -> Result<SpanOrQuery> {
        // validates node
        check_cast(node.as_ref(), &BOOLEAN_QUERY_NODE_CLASS)?;
        let children = node.get_children().ok_or_else(LuceneError::null_pointer)?;
        let mut span_queries = Vec::with_capacity(children.len());
        for child in &children {
            // `(SpanQuery) child.getTag(QueryTreeBuilder.QUERY_TREE_BUILDER_TAGID)`:
            // a `null` element is a `NullPointerException` of the
            // `SpanOrQuery` constructor.
            let tag = child.get_tag(QueryTreeBuilder::QUERY_TREE_BUILDER_TAGID);
            span_queries.push(span_query_of(tag.as_ref())?.ok_or_else(LuceneError::null_pointer)?);
        }
        SpanOrQuery::new(span_queries)
    }
}

impl StandardQueryBuilder for SpanOrQueryNodeBuilder {
    fn build(&self, node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        Ok(Some(Arc::new(self.build_span_or(node)?)))
    }
}
