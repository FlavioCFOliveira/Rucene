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

//! Port of `RegexpQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::regexp_query_node::RegexpQueryNode;
use crate::jdk::char_sequence::CharSequence;

/// Processor for Regexp queries.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.RegexpQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct RegexpQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl RegexpQueryNodeProcessor {
    /// Equivalent to the implicit `new RegexpQueryNodeProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for RegexpQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(regexp_node) = node.as_any().downcast_ref::<RegexpQueryNode>() {
            let analyzer = self
                .get_query_config_handler()
                .ok_or_else(LuceneError::null_pointer)?
                .get(ConfigurationKeys::ANALYZER);
            if let Some(analyzer) = analyzer {
                let text = regexp_node
                    .get_text()
                    .ok_or_else(LuceneError::null_pointer)?
                    .to_string();
                // because we call utf8ToString, this will only work with the default
                // TermToBytesRefAttribute
                let text = analyzer
                    .normalize(&regexp_node.get_field_as_string()?, &text)?
                    .utf8_to_string()?;
                regexp_node.set_text(Some(CharSequence::from(text)));
            }
        }
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
