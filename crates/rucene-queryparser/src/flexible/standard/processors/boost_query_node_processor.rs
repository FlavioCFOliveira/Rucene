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

//! Port of `BoostQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::error::Result;

use crate::flexible::core::nodes::boost_query_node::BoostQueryNode;
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::core::util::string_utils::StringUtils;
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;

/// This processor iterates the query node tree looking for every
/// `FieldableNode` that has `ConfigurationKeys.BOOST` in its config. If
/// there is, the boost is applied to that `FieldableNode`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.BoostQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct BoostQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl BoostQueryNodeProcessor {
    /// Equivalent to the implicit `new BoostQueryNodeProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for BoostQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(field_node) = node.as_fieldable_node() {
            let parent = node.get_parent();
            let parent_is_fieldable = parent
                .as_ref()
                .is_some_and(|p| p.as_fieldable_node().is_some());
            if parent.is_none() || !parent_is_fieldable {
                if let Some(config) = self.get_query_config_handler() {
                    let field = field_node.get_field()?;
                    let field_config = config
                        .get_field_config(StringUtils::to_string(field.as_ref()).as_deref())?;
                    if let Some(boost) = field_config.get(ConfigurationKeys::BOOST) {
                        let boosted: QueryNodeRef = BoostQueryNode::new(Some(node), boost)?;
                        return Ok(boosted);
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
