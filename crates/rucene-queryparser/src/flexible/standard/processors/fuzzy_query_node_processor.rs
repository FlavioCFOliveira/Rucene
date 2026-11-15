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

//! Port of `FuzzyQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::fuzzy_query_node::FuzzyQueryNode;
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::jdk::char_sequence::CharSequence;

/// This processor iterates the query node tree looking for every
/// `FuzzyQueryNode`, when this kind of node is found, it checks on the
/// query configuration for `ConfigurationKeys.FUZZY_CONFIG`, gets the fuzzy
/// prefix length and default similarity from it and set to the fuzzy node.
/// For more information about fuzzy prefix length check: `FuzzyQuery`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.FuzzyQueryNodeProcessor`.
///
/// **Divergence.** Java's `Analyzer.normalize` accepts a `null` field; the
/// core `Analyzer` takes a field name, so a node without a field (only when
/// the configured `MULTI_FIELDS` is empty) throws a `NullPointerException`
/// when there is an analyzer.
#[derive(Debug, Default)]
pub struct FuzzyQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl FuzzyQueryNodeProcessor {
    /// Equivalent to the implicit `new FuzzyQueryNodeProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for FuzzyQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(fuzzy_node) = node.as_any().downcast_ref::<FuzzyQueryNode>() {
            let field_node = fuzzy_node
                .as_field_query_node()
                .ok_or_else(LuceneError::null_pointer)?;
            let config = self.get_query_config_handler();
            let analyzer = config
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?
                .get(ConfigurationKeys::ANALYZER);
            if let Some(analyzer) = analyzer {
                // because we call utf8ToString, this will only work with the default
                // TermToBytesRefAttribute
                let text = field_node.get_text_as_string();
                let field = field_node
                    .get_field_as_string()
                    .ok_or_else(LuceneError::null_pointer)?;
                let text = analyzer
                    .normalize(
                        &field,
                        text.as_deref().ok_or_else(LuceneError::null_pointer)?,
                    )?
                    .utf8_to_string()?;
                field_node.set_text(Some(CharSequence::from(text)));
            }
            let config = config.ok_or_else(LuceneError::null_pointer)?;
            if let Some(fuzzy_config) = config.get(ConfigurationKeys::FUZZY_CONFIG) {
                fuzzy_node.set_prefix_length(fuzzy_config.get_prefix_length());
                if fuzzy_node.get_similarity() < 0.0 {
                    fuzzy_node.set_similarity(fuzzy_config.get_min_similarity());
                }
            } else if fuzzy_node.get_similarity() < 0.0 {
                return Err(LuceneError::illegal_argument(
                    "No FUZZY_CONFIG set in the config",
                ));
            }
        }
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
