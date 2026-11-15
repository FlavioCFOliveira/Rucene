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

//! Port of `MatchAllDocsQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::match_all_docs_query_node::MatchAllDocsQueryNode;
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};

/// This processor converts every `WildcardQueryNode` that is "*:*" to
/// `MatchAllDocsQueryNode`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.MatchAllDocsQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct MatchAllDocsQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl MatchAllDocsQueryNodeProcessor {
    /// Equivalent to `new MatchAllDocsQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for MatchAllDocsQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(fqn) = node.as_field_query_node() {
            // `fqn.getField().toString()` and `fqn.getText().toString()`
            // dereference a `null` field or text with JDK 25's helpful
            // `NullPointerException` message.
            let field = fqn.get_field().ok_or_else(|| {
                LuceneError::null_pointer_with_message(
                    "Cannot invoke \"java.lang.CharSequence.toString()\" because the return value of \"org.apache.lucene.queryparser.flexible.core.nodes.FieldQueryNode.getField()\" is null",
                )
            })?;
            if field.to_string() == "*" {
                let text = fqn.get_text().ok_or_else(|| {
                    LuceneError::null_pointer_with_message(
                        "Cannot invoke \"java.lang.CharSequence.toString()\" because the return value of \"org.apache.lucene.queryparser.flexible.core.nodes.FieldQueryNode.getText()\" is null",
                    )
                })?;
                if text.to_string() == "*" {
                    let match_all: QueryNodeRef = MatchAllDocsQueryNode::new();
                    return Ok(match_all);
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
