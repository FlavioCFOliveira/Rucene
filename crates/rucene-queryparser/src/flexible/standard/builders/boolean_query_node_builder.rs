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

//! Port of `BooleanQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneErrorKind, Result};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder;
use rucene::search::{IndexSearcher, Query};

use super::standard_query_builder::{built_query, check_cast, StandardQueryBuilder};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::modifier_query_node::Modifier;
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::query_node_exception::QueryNodeException;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::parser::escape_query_syntax_impl::EscapeQuerySyntaxImpl;
use crate::jdk::object::JavaObject;

/// Builds a `BooleanQuery` object from a `BooleanQueryNode` object. Every
/// children in the `BooleanQueryNode` object must be already tagged using
/// `QueryTreeBuilder.QUERY_TREE_BUILDER_TAGID` with a [`Query`] object.
///
/// It takes in consideration if the children is a `ModifierQueryNode` to
/// define the `BooleanClause`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.BooleanQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct BooleanQueryNodeBuilder;

impl BooleanQueryNodeBuilder {
    /// Equivalent to `new BooleanQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }

    /// Equivalent to the private static `getModifierValue(QueryNode node)`.
    fn get_modifier_value(node: &dyn QueryNode) -> Occur {
        if let Some(m_node) = node.as_modifier_query_node() {
            match m_node.get_modifier() {
                Modifier::ModReq => return Occur::MUST,
                Modifier::ModNot => return Occur::MUST_NOT,
                Modifier::ModNone => return Occur::SHOULD,
            }
        }
        Occur::SHOULD
    }
}

impl StandardQueryBuilder for BooleanQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        check_cast(query_node.as_ref(), &BOOLEAN_QUERY_NODE_CLASS)?;
        let mut b_query = Builder::new();
        if let Some(children) = query_node.get_children() {
            for child in &children {
                if let Some(query) = built_query(child.as_ref())? {
                    if let Err(ex) = b_query.add(query, Self::get_modifier_value(child.as_ref())) {
                        if matches!(
                            ex.kind(),
                            LuceneErrorKind::TooManyClauses(_)
                                | LuceneErrorKind::TooManyNestedClauses(_)
                        ) {
                            let query_string =
                                query_node.to_query_string(&EscapeQuerySyntaxImpl::new())?;
                            return Err(QueryNodeException::with_message_and_cause(
                                Arc::new(MessageImpl::with_args(
                                    QueryParserMessages::TOO_MANY_BOOLEAN_CLAUSES.get(),
                                    vec![
                                        JavaObject::from(IndexSearcher::get_max_clause_count()),
                                        JavaObject::from(query_string),
                                    ],
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
        Ok(Some(Arc::new(b_query.build())))
    }
}
