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

//! Port of `TermRangeQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::abstract_range_query_node::{AbstractRangeQueryNode, ABSTRACT_RANGE_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::fieldable_node::FieldableNode;
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::QueryNodeImpl;
use crate::flexible::core::nodes::range_query_node::RangeQueryNode;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.TermRangeQueryNode`.
pub static TERM_RANGE_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.TermRangeQueryNode",
    Some(&ABSTRACT_RANGE_QUERY_NODE_CLASS),
    &[],
);

/// This query node represents a range query composed by `FieldQueryNode`
/// bounds, which means the bound values are strings.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.TermRangeQueryNode`,
/// which extends `AbstractRangeQueryNode<FieldQueryNode>` (embedded); the
/// bounds are `FieldQueryNode`s (or nodes of its subclasses), as Java's
/// parameter types require.
#[derive(Debug)]
pub struct TermRangeQueryNode {
    base: AbstractRangeQueryNode,
}

impl TermRangeQueryNode {
    /// Constructs a [`TermRangeQueryNode`] object using the given
    /// `FieldQueryNode` as its bounds.
    ///
    /// Equivalent to `new TermRangeQueryNode(FieldQueryNode lower,
    /// FieldQueryNode upper, boolean lowerInclusive, boolean
    /// upperInclusive)`.
    ///
    /// # Errors
    ///
    /// The exceptions of [`AbstractRangeQueryNode::set_bounds`].
    pub fn new(
        lower: Option<QueryNodeRef>,
        upper: Option<QueryNodeRef>,
        lower_inclusive: bool,
        upper_inclusive: bool,
    ) -> Result<Arc<Self>> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: AbstractRangeQueryNode::fields(QueryNodeImpl::new(this.clone())),
        });
        node.base.init();
        node.base
            .set_bounds(lower, upper, lower_inclusive, upper_inclusive)?;
        Ok(node)
    }

    /// The `AbstractRangeQueryNode` this node extends.
    pub fn abstract_range_query_node(&self) -> &AbstractRangeQueryNode {
        &self.base
    }
}

impl QueryNode for TermRangeQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &TERM_RANGE_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        self.base.range_to_query_string(escape_syntax_parser)
    }

    fn to_java_string(&self) -> Result<String> {
        self.base.range_to_java_string(&TERM_RANGE_QUERY_NODE_CLASS)
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self
                .base
                .cloned(self.query_node_impl().cloned_state(this.clone(), clauses)),
        }))
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(&self.base)
    }

    fn as_range_query_node(&self) -> Option<&dyn RangeQueryNode> {
        Some(&self.base)
    }

    fn as_abstract_range_query_node(&self) -> Option<&AbstractRangeQueryNode> {
        Some(&self.base)
    }
}
