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

//! Port of `AnyQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneErrorKind, Result};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder;
use rucene::search::Query;

use super::standard_query_builder::{built_query, cast, StandardQueryBuilder};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::any_query_node::{AnyQueryNode, ANY_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::query_node_exception::QueryNodeException;
use crate::flexible::messages::message_impl::MessageImpl;

/// Builds a BooleanQuery of SHOULD clauses, possibly with some minimum number
/// to match.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.AnyQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct AnyQueryNodeBuilder;

impl AnyQueryNodeBuilder {
    /// Equivalent to `new AnyQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for AnyQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let and_node: &AnyQueryNode = cast(query_node, &ANY_QUERY_NODE_CLASS)?;
        let mut b_query = Builder::new();
        if let Some(children) = query_node.get_children() {
            for child in &children {
                if let Some(query) = built_query(child.as_ref())? {
                    if let Err(ex) = b_query.add(query, Occur::SHOULD) {
                        if matches!(
                            ex.kind(),
                            LuceneErrorKind::TooManyClauses(_)
                                | LuceneErrorKind::TooManyNestedClauses(_)
                        ) {
                            return Err(QueryNodeException::with_message_and_cause(
                                Arc::new(MessageImpl::new(
                                    /*
                                     * IQQQ.Q0028E_TOO_MANY_BOOLEAN_CLAUSES,
                                     * BooleanQuery.getMaxClauseCount()
                                     */
                                    QueryParserMessages::EMPTY_MESSAGE.get(),
                                )),
                                ex,
                            )
                            .error());
                        }
                        return Err(ex);
                    }
                }
            }
        }
        b_query.set_minimum_number_should_match(and_node.get_minimum_matching_elements());
        Ok(Some(Arc::new(b_query.build())))
    }
}
