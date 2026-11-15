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

//! Port of `RemoveDeletedQueryNodesProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.processors`.

use rucene::error::Result;

use super::query_node_processor_impl::{QueryNodeProcessorImpl, QueryNodeProcessorImplOps};
use crate::flexible::core::nodes::deleted_query_node::{
    DeletedQueryNode, DELETED_QUERY_NODE_CLASS,
};
use crate::flexible::core::nodes::match_no_docs_query_node::{
    MatchNoDocsQueryNode, MATCH_NO_DOCS_QUERY_NODE_CLASS,
};
use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};

/// A [`RemoveDeletedQueryNodesProcessor`] removes every instance of
/// `DeletedQueryNode` from a query node tree. If the resulting root node is
/// a `DeletedQueryNode`, `MatchNoDocsQueryNode` is returned.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.processors.RemoveDeletedQueryNodesProcessor`.
#[derive(Debug, Default)]
pub struct RemoveDeletedQueryNodesProcessor {
    base: QueryNodeProcessorImpl,
}

impl RemoveDeletedQueryNodesProcessor {
    /// Equivalent to `new RemoveDeletedQueryNodesProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for RemoveDeletedQueryNodesProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        let query_tree = QueryNodeProcessorImpl::process(self, query_tree)?;
        if instance_of(query_tree.as_ref(), &DELETED_QUERY_NODE_CLASS)
            && !instance_of(query_tree.as_ref(), &MATCH_NO_DOCS_QUERY_NODE_CLASS)
        {
            let replacement: QueryNodeRef = MatchNoDocsQueryNode::new();
            return Ok(replacement);
        }
        Ok(query_tree)
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if !node.is_leaf() {
            let children = node.get_children();
            let remove_boolean = match &children {
                None => true,
                Some(children) if children.is_empty() => true,
                Some(children) => children
                    .iter()
                    .all(|c| instance_of(c.as_ref(), &DELETED_QUERY_NODE_CLASS)),
            };
            if remove_boolean {
                let replacement: QueryNodeRef = DeletedQueryNode::new();
                return Ok(replacement);
            }
        }
        Ok(node)
    }

    fn set_children_order(&self, mut children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        children.retain(|c| !instance_of(c.as_ref(), &DELETED_QUERY_NODE_CLASS));
        Ok(children)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }
}
