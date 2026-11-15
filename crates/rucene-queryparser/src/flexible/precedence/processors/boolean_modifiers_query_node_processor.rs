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

//! Port of `BooleanModifiersQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.precedence.processors`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::and_query_node::AND_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::modifier_query_node::{Modifier, ModifierQueryNode};
use crate::flexible::core::nodes::or_query_node::OR_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::config::standard_query_config_handler::{
    ConfigurationKeys, Operator,
};

/// This processor is used to apply the correct `ModifierQueryNode` to
/// `BooleanQueryNode`s children.
///
/// It walks through the query node tree looking for `BooleanQueryNode`s. If
/// an `AndQueryNode` is found, every child, which is not a
/// `ModifierQueryNode` or the `ModifierQueryNode` is `Modifier.MOD_NONE`,
/// becomes a `Modifier.MOD_REQ`. For any other `BooleanQueryNode` which is
/// not an `OrQueryNode`, it checks the default operator is `Operator.AND`,
/// if it is, the same operation when an `AndQueryNode` is found is applied
/// to it.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.precedence.processors.BooleanModifiersQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct BooleanModifiersQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
    children_buffer: Mutex<Vec<QueryNodeRef>>,
    using_and: AtomicBool,
}

impl BooleanModifiersQueryNodeProcessor {
    /// Equivalent to `new BooleanModifiersQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }

    fn buffer(&self) -> std::sync::MutexGuard<'_, Vec<QueryNodeRef>> {
        self.children_buffer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The children of `node` with the `MOD_REQ` modifier applied, set back
    /// on it through the buffer.
    fn apply_required(&self, node: &QueryNodeRef) -> Result<()> {
        self.buffer().clear();
        let children = node.get_children().ok_or_else(LuceneError::null_pointer)?;
        for child in children {
            let applied = Self::apply_modifier(child, Modifier::ModReq)?;
            self.buffer().push(applied);
        }
        let buffer = self.buffer().clone();
        node.set(buffer)
    }

    /// Equivalent to the private `applyModifier(QueryNode node,
    /// ModifierQueryNode.Modifier mod)`.
    fn apply_modifier(node: QueryNodeRef, mod_: Modifier) -> Result<QueryNodeRef> {
        // check if modifier is not already defined and is default
        match node.as_modifier_query_node() {
            None => {
                let modified: QueryNodeRef = ModifierQueryNode::new(Some(node), mod_)?;
                Ok(modified)
            }
            Some(mod_node) => {
                if mod_node.get_modifier() == Modifier::ModNone {
                    let modified: QueryNodeRef =
                        ModifierQueryNode::new(Some(mod_node.get_child()?), mod_)?;
                    return Ok(modified);
                }
                Ok(node)
            }
        }
    }
}

impl QueryNodeProcessorImplOps for BooleanModifiersQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

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
        QueryNodeProcessorImpl::process(self, query_tree)
    }

    // Java's two branches have the same body.
    #[allow(clippy::if_same_then_else)]
    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if instance_of(node.as_ref(), &AND_QUERY_NODE_CLASS) {
            self.apply_required(&node)?;
        } else if self.using_and.load(Ordering::SeqCst)
            && instance_of(node.as_ref(), &BOOLEAN_QUERY_NODE_CLASS)
            && !instance_of(node.as_ref(), &OR_QUERY_NODE_CLASS)
        {
            self.apply_required(&node)?;
        }
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
