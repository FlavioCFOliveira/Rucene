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

//! Port of `IntervalQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::{Arc, Mutex, PoisonError};

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};

use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::core::query_node_exception::QueryNodeException;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::interval_query_node::IntervalQueryNode;
use crate::jdk::object::JavaObject;

/// This processor makes sure that `ConfigurationKeys.ANALYZER` is defined in
/// the `QueryConfigHandler` and injects this analyzer into
/// `IntervalQueryNode`s.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.IntervalQueryNodeProcessor`.
#[derive(Default)]
pub struct IntervalQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
    analyzer: Mutex<Option<Arc<dyn Analyzer>>>,
}

impl std::fmt::Debug for IntervalQueryNodeProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntervalQueryNodeProcessor")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl IntervalQueryNodeProcessor {
    /// Equivalent to the implicit `new IntervalQueryNodeProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }

    fn analyzer(&self) -> Option<Arc<dyn Analyzer>> {
        self.analyzer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl QueryNodeProcessorImplOps for IntervalQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        let analyzer = self
            .get_query_config_handler()
            .ok_or_else(LuceneError::null_pointer)?
            .get(ConfigurationKeys::ANALYZER);
        *self.analyzer.lock().unwrap_or_else(PoisonError::into_inner) = analyzer;
        QueryNodeProcessorImpl::process(self, query_tree)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(interval_query_node) = node.as_any().downcast_ref::<IntervalQueryNode>() {
            let Some(analyzer) = self.analyzer() else {
                return Err(QueryNodeException::new(Arc::new(MessageImpl::with_args(
                    QueryParserMessages::ANALYZER_REQUIRED.get(),
                    vec![JavaObject::from(interval_query_node.to_java_string()?)],
                )))
                .error());
            };
            interval_query_node.set_analyzer(Some(analyzer))?;
        }
        Ok(node)
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
