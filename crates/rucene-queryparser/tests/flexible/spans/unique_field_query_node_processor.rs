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

//! Port of `UniqueFieldQueryNodeProcessor.java`.

use rucene::error::{LuceneError, Result};
use rucene_queryparser::flexible::core::nodes::query_node::QueryNodeRef;
use rucene_queryparser::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use rucene_queryparser::jdk::char_sequence::CharSequence;

use super::spans_query_config_handler::SpansQueryConfigHandler;

/// This processor changes every field name of each `FieldableNode` query
/// node contained in the query tree to the field name defined in the
/// `UniqueFieldAttribute`. So, the `UniqueFieldAttribute` must be defined in
/// the `QueryConfigHandler` object set in this processor, otherwise it throws
/// an exception.
#[derive(Debug, Default)]
pub struct UniqueFieldQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl UniqueFieldQueryNodeProcessor {
    /// `new UniqueFieldQueryNodeProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for UniqueFieldQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(field_node) = node.as_fieldable_node() {
            let Some(query_config) = self.get_query_config_handler() else {
                return Err(LuceneError::illegal_argument(
                    "A config handler is expected by the processor UniqueFieldQueryNodeProcessor!",
                ));
            };
            if !query_config.has(SpansQueryConfigHandler::UNIQUE_FIELD) {
                return Err(LuceneError::illegal_argument(
                    "UniqueFieldAttribute should be defined in the config handler!",
                ));
            }
            let unique_field = query_config.get(SpansQueryConfigHandler::UNIQUE_FIELD);
            field_node.set_field(unique_field.map(CharSequence::from))?;
        }
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
