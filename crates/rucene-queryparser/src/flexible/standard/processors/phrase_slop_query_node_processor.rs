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

//! Port of `PhraseSlopQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::error::Result;

use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::nodes::slop_query_node::SlopQueryNode;
use crate::flexible::core::nodes::tokenized_phrase_query_node::TOKENIZED_PHRASE_QUERY_NODE_CLASS;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::nodes::multi_phrase_query_node::MULTI_PHRASE_QUERY_NODE_CLASS;

/// This processor removes invalid `SlopQueryNode` objects in the query node
/// tree. A `SlopQueryNode` is invalid if its child is neither a
/// `TokenizedPhraseQueryNode` nor a `MultiPhraseQueryNode`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.PhraseSlopQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct PhraseSlopQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl PhraseSlopQueryNodeProcessor {
    /// Equivalent to `new PhraseSlopQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for PhraseSlopQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(phrase_slop_node) = node.as_any().downcast_ref::<SlopQueryNode>() {
            let child = phrase_slop_node.get_child()?;
            if !instance_of(child.as_ref(), &TOKENIZED_PHRASE_QUERY_NODE_CLASS)
                && !instance_of(child.as_ref(), &MULTI_PHRASE_QUERY_NODE_CLASS)
            {
                return phrase_slop_node.get_child();
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
