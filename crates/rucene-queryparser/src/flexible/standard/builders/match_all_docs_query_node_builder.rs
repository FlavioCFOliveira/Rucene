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

//! Port of `MatchAllDocsQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::Query;

use super::standard_query_builder::StandardQueryBuilder;
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::match_all_docs_query_node::MATCH_ALL_DOCS_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::query_node_exception::QueryNodeException;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::parser::escape_query_syntax_impl::EscapeQuerySyntaxImpl;
use crate::jdk::object::JavaObject;

/// Builds a `MatchAllDocsQuery` object from a `MatchAllDocsQueryNode`
/// object.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.MatchAllDocsQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct MatchAllDocsQueryNodeBuilder;

impl MatchAllDocsQueryNodeBuilder {
    /// Equivalent to `new MatchAllDocsQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for MatchAllDocsQueryNodeBuilder {
    /// Equivalent to `build(QueryNode queryNode)`: `MatchAllDocsQuery.INSTANCE`.
    ///
    /// # Errors
    ///
    /// The `QueryNodeException` `LUCENE_QUERY_CONVERSION_ERROR` of a node
    /// that is not a `MatchAllDocsQueryNode`.
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        // validates node
        if !instance_of(query_node.as_ref(), &MATCH_ALL_DOCS_QUERY_NODE_CLASS) {
            let query_string = query_node.to_query_string(&EscapeQuerySyntaxImpl::new())?;
            return Err(QueryNodeException::new(Arc::new(MessageImpl::with_args(
                QueryParserMessages::LUCENE_QUERY_CONVERSION_ERROR.get(),
                vec![
                    JavaObject::from(query_string),
                    JavaObject::from(query_node.java_class().get_name()),
                ],
            )))
            .error());
        }
        Ok(Some(Arc::new(MatchAllDocsQuery::instance())))
    }
}
