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

//! Port of `DefaultPhraseSlopQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use rucene::error::Result;

use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::nodes::slop_query_node::{SlopQueryNode, SLOP_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::tokenized_phrase_query_node::TOKENIZED_PHRASE_QUERY_NODE_CLASS;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::multi_phrase_query_node::MULTI_PHRASE_QUERY_NODE_CLASS;

/// This processor verifies if `ConfigurationKeys.PHRASE_SLOP` is defined in
/// the `QueryConfigHandler`. If it is, it looks for every
/// `TokenizedPhraseQueryNode` and `MultiPhraseQueryNode` that does not have
/// its parent as a `SlopQueryNode` and applies the slop factor.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.DefaultPhraseSlopQueryNodeProcessor`.
#[derive(Debug)]
pub struct DefaultPhraseSlopQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
    process_children: AtomicBool,
    default_phrase_slop: AtomicI32,
}

impl Default for DefaultPhraseSlopQueryNodeProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultPhraseSlopQueryNodeProcessor {
    /// Equivalent to `new DefaultPhraseSlopQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self {
            base: QueryNodeProcessorImpl::new(),
            process_children: AtomicBool::new(true),
            default_phrase_slop: AtomicI32::new(0),
        }
    }
}

impl QueryNodeProcessorImplOps for DefaultPhraseSlopQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(query_config) = self.get_query_config_handler() {
            if let Some(default_phrase_slop) = query_config.get(ConfigurationKeys::PHRASE_SLOP) {
                self.default_phrase_slop
                    .store(default_phrase_slop, Ordering::SeqCst);
                return QueryNodeProcessorImpl::process(self, query_tree);
            }
        }
        Ok(query_tree)
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if instance_of(node.as_ref(), &TOKENIZED_PHRASE_QUERY_NODE_CLASS)
            || instance_of(node.as_ref(), &MULTI_PHRASE_QUERY_NODE_CLASS)
        {
            let slop: QueryNodeRef =
                SlopQueryNode::new(Some(node), self.default_phrase_slop.load(Ordering::SeqCst))?;
            return Ok(slop);
        }
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if instance_of(node.as_ref(), &SLOP_QUERY_NODE_CLASS) {
            self.process_children.store(false, Ordering::SeqCst);
        }
        Ok(node)
    }

    fn process_children(&self, query_tree: &QueryNodeRef) -> Result<()> {
        if self.process_children.load(Ordering::SeqCst) {
            QueryNodeProcessorImpl::process_children(self, query_tree)
        } else {
            self.process_children.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
