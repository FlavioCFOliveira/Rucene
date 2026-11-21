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

//! Port of `BooleanSingleChildOptimizationQueryNodeProcessor.java`, of the
//! Java package `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::error::Result;

use crate::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::modifier_query_node::Modifier;
use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::nodes::boolean_modifier_node::BOOLEAN_MODIFIER_NODE_CLASS;

/// This processor removes every `BooleanQueryNode` that contains only one
/// child and returns this child. If this child is `ModifierQueryNode` that
/// was defined by the user. A modifier is not defined by the user when it's
/// a `BooleanModifierNode`
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.BooleanSingleChildOptimizationQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct BooleanSingleChildOptimizationQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl BooleanSingleChildOptimizationQueryNodeProcessor {
    /// Equivalent to `new BooleanSingleChildOptimizationQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for BooleanSingleChildOptimizationQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if instance_of(node.as_ref(), &BOOLEAN_QUERY_NODE_CLASS) {
            if let Some(children) = node.get_children() {
                if children.len() == 1 {
                    let child = children[0].clone();
                    if let Some(mod_node) = child.as_modifier_query_node() {
                        if instance_of(child.as_ref(), &BOOLEAN_MODIFIER_NODE_CLASS)
                            || mod_node.get_modifier() == Modifier::ModNone
                        {
                            return Ok(child);
                        }
                    } else {
                        return Ok(child);
                    }
                }
            }
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
