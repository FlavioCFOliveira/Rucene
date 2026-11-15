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

//! Port of `OrQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use super::boolean_query_node::{BooleanQueryNode, BOOLEAN_QUERY_NODE_CLASS};
use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::QueryNodeImpl;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.OrQueryNode`.
pub static OR_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.OrQueryNode",
    Some(&BOOLEAN_QUERY_NODE_CLASS),
    &[],
);

/// A [`OrQueryNode`] represents an OR boolean operation performed on a list
/// of nodes.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.OrQueryNode`,
/// which extends `BooleanQueryNode` (embedded).
#[derive(Debug)]
pub struct OrQueryNode {
    base: BooleanQueryNode,
}

impl OrQueryNode {
    /// Equivalent to `new OrQueryNode(List<QueryNode> clauses)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None` clauses, and
    /// `IllegalArgumentException("OR query must have at least one clause")`
    /// for none.
    pub fn new(clauses: Option<Vec<QueryNodeRef>>) -> Result<Arc<Self>> {
        let empty = clauses.as_ref().is_none_or(Vec::is_empty);
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: BooleanQueryNode::fields(QueryNodeImpl::new(this.clone())),
        });
        node.base.init(clauses)?;
        if empty {
            return Err(LuceneError::illegal_argument(
                "OR query must have at least one clause",
            ));
        }
        Ok(node)
    }
}

impl QueryNode for OrQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &OR_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        self.base.to_java_string_with("or")
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        self.base.to_query_string_with(escape_syntax_parser, " OR ")
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned(this.clone(), clauses),
        }))
    }
}
