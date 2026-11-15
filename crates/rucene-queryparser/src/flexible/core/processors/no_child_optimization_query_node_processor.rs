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

//! Port of `NoChildOptimizationQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.processors`.

use rucene::error::Result;

use super::query_node_processor_impl::{QueryNodeProcessorImpl, QueryNodeProcessorImplOps};
use crate::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::boost_query_node::BOOST_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::deleted_query_node::DELETED_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::match_no_docs_query_node::MatchNoDocsQueryNode;
use crate::flexible::core::nodes::modifier_query_node::MODIFIER_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::nodes::tokenized_phrase_query_node::TOKENIZED_PHRASE_QUERY_NODE_CLASS;

/// A [`NoChildOptimizationQueryNodeProcessor`] removes every
/// BooleanQueryNode, BoostQueryNode, TokenizedPhraseQueryNode or
/// ModifierQueryNode that do not have a valid children.
///
/// Example: When the children of these nodes are removed for any reason
/// then the nodes may become invalid.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.processors.NoChildOptimizationQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct NoChildOptimizationQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl NoChildOptimizationQueryNodeProcessor {
    /// Equivalent to `new NoChildOptimizationQueryNodeProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for NoChildOptimizationQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if instance_of(node.as_ref(), &BOOLEAN_QUERY_NODE_CLASS)
            || instance_of(node.as_ref(), &BOOST_QUERY_NODE_CLASS)
            || instance_of(node.as_ref(), &TOKENIZED_PHRASE_QUERY_NODE_CLASS)
            || instance_of(node.as_ref(), &MODIFIER_QUERY_NODE_CLASS)
        {
            if let Some(children) = node.get_children() {
                if !children.is_empty() {
                    for child in &children {
                        if !instance_of(child.as_ref(), &DELETED_QUERY_NODE_CLASS) {
                            return Ok(node);
                        }
                    }
                }
            }
            let replacement: QueryNodeRef = MatchNoDocsQueryNode::new();
            return Ok(replacement);
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
