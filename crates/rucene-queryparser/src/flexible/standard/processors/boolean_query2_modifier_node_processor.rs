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

//! Port of `BooleanQuery2ModifierNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::{LuceneError, Result};

use crate::flexible::core::config::query_config_handler::QueryConfigHandler;
use crate::flexible::core::nodes::and_query_node::AND_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::modifier_query_node::{Modifier, ModifierQueryNode};
use crate::flexible::core::nodes::query_node::{instance_of, QueryNode, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor::QueryNodeProcessor;
use crate::flexible::standard::config::standard_query_config_handler::{
    ConfigurationKeys, Operator,
};
use crate::flexible::standard::nodes::boolean_modifier_node::BooleanModifierNode;

/// This processor is used to apply the correct `ModifierQueryNode` to
/// `BooleanQueryNode`s children. This is a variant of
/// `BooleanModifiersQueryNodeProcessor` which ignores precedence.
///
/// The `StandardSyntaxParser` knows the rules of precedence, but lucene
/// does not. e.g. `(A AND B OR C AND D)` ist treated like `(+A +B +C +D)`.
///
/// This processor walks through the query node tree looking for
/// `BooleanQueryNode`s. If an `AndQueryNode` is found, every child, which
/// is not a `ModifierQueryNode` or the `ModifierQueryNode` is
/// `Modifier.MOD_NONE`, becomes a `Modifier.MOD_REQ`. For default
/// `BooleanQueryNode`, it checks the default operator is `Operator.AND`, if
/// it is, the same operation when an `AndQueryNode` is found is applied to
/// it. Each `BooleanQueryNode` which direct parent is also a
/// `BooleanQueryNode` is removed (to ignore the rules of precedence).
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.BooleanQuery2ModifierNodeProcessor`,
/// a `QueryNodeProcessor` that does not extend `QueryNodeProcessorImpl`.
#[derive(Debug)]
pub struct BooleanQuery2ModifierNodeProcessor {
    /// The package-private field `queryConfigHandler`.
    query_config_handler: Mutex<Option<Arc<QueryConfigHandler>>>,
    children_buffer: Mutex<Vec<QueryNodeRef>>,
    using_and: AtomicBool,
}

impl Default for BooleanQuery2ModifierNodeProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl BooleanQuery2ModifierNodeProcessor {
    /// Equivalent to the package-private `TAG_REMOVE`.
    pub(crate) const TAG_REMOVE: &'static str = "remove";
    /// Equivalent to the package-private `TAG_MODIFIER`.
    pub(crate) const TAG_MODIFIER: &'static str = "wrapWithModifier";
    /// Equivalent to the package-private `TAG_BOOLEAN_ROOT`.
    pub(crate) const TAG_BOOLEAN_ROOT: &'static str = "booleanRoot";

    /// Equivalent to `new BooleanQuery2ModifierNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self {
            query_config_handler: Mutex::new(None),
            children_buffer: Mutex::new(Vec::new()),
            using_and: AtomicBool::new(false),
        }
    }

    /// Equivalent to the protected `processChildren(QueryNode queryTree)`:
    /// each child is processed, and the results are dropped.
    ///
    /// # Errors
    ///
    /// What the processing throws.
    pub fn process_children(&self, query_tree: &QueryNodeRef) -> Result<()> {
        if let Some(children) = query_tree.get_children() {
            if !children.is_empty() {
                for child in children {
                    let _child = self.process_iteration(child)?;
                }
            }
        }
        Ok(())
    }

    /// Equivalent to the private `processIteration(QueryNode queryTree)`.
    fn process_iteration(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        let query_tree = self.pre_process_node(query_tree)?;
        self.process_children(&query_tree)?;
        self.post_process_node(query_tree)
    }

    /// Equivalent to the protected
    /// `fillChildrenBufferAndApplyModifiery(QueryNode parent)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a node without children list, the
    /// `ClassCastException` of a modifier tag that is not a `Modifier`, and
    /// the exceptions of building a modifier node.
    pub fn fill_children_buffer_and_apply_modifiery(&self, parent: &QueryNodeRef) -> Result<()> {
        let children = parent
            .get_children()
            .ok_or_else(LuceneError::null_pointer)?;
        for node in children {
            if node.contains_tag(Self::TAG_REMOVE) {
                self.fill_children_buffer_and_apply_modifiery(&node)?;
            } else if node.contains_tag(Self::TAG_MODIFIER) {
                let tag = node.get_tag(Self::TAG_MODIFIER);
                let modifier = match tag.as_ref().map(|t| t.downcast_ref::<Modifier>()) {
                    Some(Some(m)) => *m,
                    _ => {
                        return Err(LuceneError::unchecked(
                            "java.lang.ClassCastException",
                            "cannot be cast to class org.apache.lucene.queryparser.flexible.core.nodes.ModifierQueryNode$Modifier",
                        ))
                    }
                };
                let applied = self.apply_modifier(node, modifier)?;
                self.buffer().push(applied);
            } else {
                self.buffer().push(node);
            }
        }
        Ok(())
    }

    fn buffer(&self) -> std::sync::MutexGuard<'_, Vec<QueryNodeRef>> {
        self.children_buffer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Equivalent to the protected `postProcessNode(QueryNode node)`.
    ///
    /// # Errors
    ///
    /// The exceptions of filling the buffer and of `set`.
    pub fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if node.contains_tag(Self::TAG_BOOLEAN_ROOT) {
            self.buffer().clear();
            self.fill_children_buffer_and_apply_modifiery(&node)?;
            let children = self.buffer().clone();
            node.set(children)?;
        }
        Ok(node)
    }

    /// Equivalent to the protected `preProcessNode(QueryNode node)`.
    ///
    /// # Errors
    ///
    /// None in practice; the signature follows Java's.
    pub fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        let parent = node.get_parent();
        let parent_is_boolean = parent
            .as_ref()
            .is_some_and(|p| instance_of(p.as_ref(), &BOOLEAN_QUERY_NODE_CLASS));
        if instance_of(node.as_ref(), &BOOLEAN_QUERY_NODE_CLASS) {
            if parent_is_boolean {
                node.set_tag(Self::TAG_REMOVE, Arc::new(true)); // no precedence
            } else {
                node.set_tag(Self::TAG_BOOLEAN_ROOT, Arc::new(true));
            }
        } else if parent_is_boolean {
            let parent_is_and = parent
                .as_ref()
                .is_some_and(|p| instance_of(p.as_ref(), &AND_QUERY_NODE_CLASS));
            if parent_is_and
                || (self.using_and.load(Ordering::SeqCst)
                    && self.is_default_boolean_query_node(parent.as_deref()))
            {
                self.tag_modifier_but_do_not_override(&node, Modifier::ModReq);
            }
        }
        Ok(node)
    }

    /// Equivalent to the protected `isDefaultBooleanQueryNode(QueryNode
    /// toTest)`: a node whose class is exactly `BooleanQueryNode`.
    pub fn is_default_boolean_query_node(&self, to_test: Option<&dyn QueryNode>) -> bool {
        to_test.is_some_and(|n| *n.java_class() == BOOLEAN_QUERY_NODE_CLASS)
    }

    /// Equivalent to the private `applyModifier(QueryNode node, Modifier
    /// mod)`.
    fn apply_modifier(&self, node: QueryNodeRef, mod_: Modifier) -> Result<QueryNodeRef> {
        // check if modifier is not already defined and is default
        match node.as_modifier_query_node() {
            None => {
                let wrapped: QueryNodeRef = BooleanModifierNode::new(Some(node), mod_)?;
                Ok(wrapped)
            }
            Some(mod_node) => {
                if mod_node.get_modifier() == Modifier::ModNone {
                    let replaced: QueryNodeRef =
                        ModifierQueryNode::new(Some(mod_node.get_child()?), mod_)?;
                    return Ok(replaced);
                }
                Ok(node)
            }
        }
    }

    /// Equivalent to the protected `tagModifierButDoNotOverride(QueryNode
    /// node, Modifier mod)`.
    pub fn tag_modifier_but_do_not_override(&self, node: &QueryNodeRef, mod_: Modifier) {
        if let Some(mod_node) = node.as_modifier_query_node() {
            if mod_node.get_modifier() == Modifier::ModNone {
                node.set_tag(Self::TAG_MODIFIER, Arc::new(mod_));
            }
        } else {
            node.set_tag(Self::TAG_MODIFIER, Arc::new(Modifier::ModReq));
        }
    }
}

impl QueryNodeProcessor for BooleanQuery2ModifierNodeProcessor {
    /// Equivalent to `process(QueryNode queryTree)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` without `DEFAULT_OPERATOR`, the
    /// `NullPointerException` of a missing configuration handler, and what
    /// the processing throws.
    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        let op = self
            .get_query_config_handler()
            .ok_or_else(LuceneError::null_pointer)?
            .get(ConfigurationKeys::DEFAULT_OPERATOR);
        let Some(op) = op else {
            return Err(LuceneError::illegal_argument(
                "StandardQueryConfigHandler.ConfigurationKeys.DEFAULT_OPERATOR should be set on the QueryConfigHandler",
            ));
        };
        self.using_and.store(Operator::And == op, Ordering::SeqCst);
        self.process_iteration(query_tree)
    }

    fn set_query_config_handler(&self, query_config_handler: Option<Arc<QueryConfigHandler>>) {
        *self
            .query_config_handler
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = query_config_handler;
    }

    fn get_query_config_handler(&self) -> Option<Arc<QueryConfigHandler>> {
        self.query_config_handler
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
