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

//! Port of `WildcardQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::Query;
use rucene::util::automaton::DEFAULT_DETERMINIZE_WORK_LIMIT;

use super::standard_query_builder::{multi_term_term, rewrite_method_of, StandardQueryBuilder};
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};
use crate::flexible::standard::nodes::wildcard_query_node::WILDCARD_QUERY_NODE_CLASS;

/// Builds a `WildcardQuery` object from a `WildcardQueryNode` object.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.WildcardQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct WildcardQueryNodeBuilder;

impl WildcardQueryNodeBuilder {
    /// Equivalent to `new WildcardQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for WildcardQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        // `(WildcardQueryNode) queryNode`: a `WildcardQueryNode` or a
        // `PrefixWildcardQueryNode`, whose `FieldQueryNode` part holds the
        // field and text.
        let wildcard_node = query_node
            .as_field_query_node()
            .filter(|_| {
                crate::flexible::core::nodes::query_node::instance_of(
                    query_node.as_ref(),
                    &WILDCARD_QUERY_NODE_CLASS,
                )
            })
            .ok_or_else(|| class_cast(query_node.as_ref(), &WILDCARD_QUERY_NODE_CLASS))?;
        let method = rewrite_method_of(query_node.as_ref())?;
        let term = multi_term_term(
            wildcard_node.get_field_as_string(),
            wildcard_node.get_text_as_string(),
        )?;
        Ok(Some(Arc::new(WildcardQuery::with_rewrite_method(
            term,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
            method,
        )?)))
    }
}
