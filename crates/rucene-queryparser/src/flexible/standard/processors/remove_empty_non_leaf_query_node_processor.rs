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

//! Port of `RemoveEmptyNonLeafQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::{Mutex, PoisonError};

use rucene::error::Result;

use crate::flexible::core::nodes::match_no_docs_query_node::MatchNoDocsQueryNode;
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};

/// This processor removes every `QueryNode` that is not a leaf and has not
/// children. If after processing the entire tree the root node is not a
/// leaf and has no children, a `MatchNoDocsQueryNode` object is returned.
///
/// This processor is used at the end of a pipeline to avoid invalid query
/// node tree structures like a `GroupQueryNode` or `ModifierQueryNode` with
/// no children.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.RemoveEmptyNonLeafQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct RemoveEmptyNonLeafQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
    children_buffer: Mutex<Vec<QueryNodeRef>>,
}

impl RemoveEmptyNonLeafQueryNodeProcessor {
    /// Equivalent to `new RemoveEmptyNonLeafQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }

    fn buffer_add(&self, child: QueryNodeRef) {
        self.children_buffer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(child);
    }
}

impl QueryNodeProcessorImplOps for RemoveEmptyNonLeafQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        let query_tree = QueryNodeProcessorImpl::process(self, query_tree)?;
        if !query_tree.is_leaf() {
            let empty = query_tree
                .get_children()
                .is_none_or(|children| children.is_empty());
            if empty {
                let replacement: QueryNodeRef = MatchNoDocsQueryNode::new();
                return Ok(replacement);
            }
        }
        Ok(query_tree)
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn set_children_order(&self, mut children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        for child in &children {
            if !child.is_leaf() {
                let grand_children = child.get_children();
                if grand_children.is_some_and(|g| !g.is_empty()) {
                    self.buffer_add(child.clone());
                }
            } else {
                self.buffer_add(child.clone());
            }
        }
        children.clear();
        // finally
        let buffer = std::mem::take(
            &mut *self
                .children_buffer
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        children.extend(buffer);
        Ok(children)
    }
}
