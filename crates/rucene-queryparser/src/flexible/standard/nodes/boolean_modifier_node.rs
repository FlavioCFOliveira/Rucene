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

//! Port of `BooleanModifierNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.
use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use crate::flexible::core::nodes::modifier_query_node::{
    Modifier, ModifierQueryNode, MODIFIER_QUERY_NODE_CLASS,
};
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::QueryNodeImpl;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.BooleanModifierNode`.
pub static BOOLEAN_MODIFIER_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.BooleanModifierNode",
    Some(&MODIFIER_QUERY_NODE_CLASS),
    &[],
);

/// A [`BooleanModifierNode`] has the same behaviour as
/// [`ModifierQueryNode`], it only indicates that this modifier was added by
/// `BooleanQuery2ModifierNodeProcessor` and not by the user.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.BooleanModifierNode`,
/// which extends `ModifierQueryNode` (embedded).
#[derive(Debug)]
pub struct BooleanModifierNode {
    base: ModifierQueryNode,
}

impl BooleanModifierNode {
    /// Equivalent to `new BooleanModifierNode(QueryNode node, Modifier
    /// mod)`.
    ///
    /// # Errors
    ///
    /// The exceptions of `ModifierQueryNode(QueryNode, Modifier)`.
    pub fn new(node: Option<QueryNodeRef>, modifier: Modifier) -> Result<Arc<Self>> {
        let node = node.ok_or_else(ModifierQueryNode::null_query)?;
        let this_node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: ModifierQueryNode::fields(QueryNodeImpl::new(this.clone()), modifier),
        });
        this_node.base.init(node, modifier)?;
        Ok(this_node)
    }
}

impl QueryNode for BooleanModifierNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &BOOLEAN_MODIFIER_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        self.base.modifier_to_java_string()
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        self.base.modifier_to_query_string(escape_syntax_parser)
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self
                .base
                .cloned(self.query_node_impl().cloned_state(this.clone(), clauses)),
        }))
    }

    fn as_modifier_query_node(&self) -> Option<&ModifierQueryNode> {
        Some(&self.base)
    }
}
