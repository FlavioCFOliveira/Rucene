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

//! Port of `SpansQueryTreeBuilder.java`.

use std::ops::Deref;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::Query;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queryparser::flexible::core::builders::query_builder::QueryBuilder;
use rucene_queryparser::flexible::core::builders::query_tree_builder::QueryTreeBuilder;
use rucene_queryparser::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::core::nodes::query_node::QueryNodeRef;
use rucene_queryparser::flexible::standard::builders::standard_query_builder::{
    query_of, StandardQueryBuilder,
};

use super::span_or_query_node_builder::SpanOrQueryNodeBuilder;
use super::span_query_cast::span_query_of;
use super::span_term_query_node_builder::SpanTermQueryNodeBuilder;

/// Sets up a query tree builder to build a span query tree from a query
/// node tree.
///
/// The defined map is:
/// * every `BooleanQueryNode` instance is delegated to the
///   `SpanOrQueryNodeBuilder`;
/// * every `FieldQueryNode` instance is delegated to the
///   `SpanTermQueryNodeBuilder`.
///
/// It extends `QueryTreeBuilder` (embedded, reached by dereference) and
/// implements `StandardQueryBuilder`.
#[derive(Debug)]
pub struct SpansQueryTreeBuilder {
    base: QueryTreeBuilder,
}

impl SpansQueryTreeBuilder {
    /// `new SpansQueryTreeBuilder()`.
    pub fn new() -> Self {
        let base = QueryTreeBuilder::new();
        base.set_builder_for_class(&BOOLEAN_QUERY_NODE_CLASS, Arc::new(SpanOrQueryNodeBuilder));
        base.set_builder_for_class(&FIELD_QUERY_NODE_CLASS, Arc::new(SpanTermQueryNodeBuilder));
        SpansQueryTreeBuilder { base }
    }

    /// The covariant `SpanQuery build(QueryNode queryTree)`: `(SpanQuery)
    /// super.build(queryTree)`.
    pub fn build_span(&self, query_tree: &QueryNodeRef) -> Result<Option<Arc<dyn SpanQuery>>> {
        let obj = QueryBuilder::build(&self.base, query_tree)?;
        span_query_of(obj.as_ref())
    }
}

impl Deref for SpansQueryTreeBuilder {
    type Target = QueryTreeBuilder;

    fn deref(&self) -> &QueryTreeBuilder {
        &self.base
    }
}

impl StandardQueryBuilder for SpansQueryTreeBuilder {
    fn build(&self, query_tree: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        // the span query, as the `Query` it is
        let obj = QueryBuilder::build(&self.base, query_tree)?;
        span_query_of(obj.as_ref())?;
        obj.as_ref().map(query_of).transpose()
    }

    fn as_query_tree_builder(&self) -> Option<&QueryTreeBuilder> {
        Some(&self.base)
    }
}
