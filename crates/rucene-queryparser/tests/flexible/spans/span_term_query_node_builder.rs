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

//! Port of `SpanTermQueryNodeBuilder.java`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::Query;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queryparser::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};
use rucene_queryparser::flexible::standard::builders::standard_query_builder::{
    new_term, StandardQueryBuilder,
};

/// This builder creates `SpanTermQuery`s from a `FieldQueryNode` object.
#[derive(Debug, Default)]
pub struct SpanTermQueryNodeBuilder;

impl SpanTermQueryNodeBuilder {
    /// The covariant `SpanTermQuery build(QueryNode node)`.
    pub fn build_span_term(&self, node: &QueryNodeRef) -> Result<SpanTermQuery> {
        // `(FieldQueryNode) node`: the class or one of its subclasses
        let field_query_node = node
            .as_field_query_node()
            .ok_or_else(|| class_cast(node.as_ref(), &FIELD_QUERY_NODE_CLASS))?;
        Ok(SpanTermQuery::new(new_term(
            field_query_node.get_field_as_string(),
            field_query_node.get_text_as_string(),
        )?))
    }
}

impl StandardQueryBuilder for SpanTermQueryNodeBuilder {
    fn build(&self, node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        Ok(Some(Arc::new(self.build_span_term(node)?)))
    }
}
