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

//! Port of `AllowLeadingWildcardProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::core::query_node_exception::QueryNodeException;
use crate::flexible::core::util::unescaped_char_sequence::UnescapedCharSequence;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::wildcard_query_node::WILDCARD_QUERY_NODE_CLASS;
use crate::flexible::standard::parser::escape_query_syntax_impl::EscapeQuerySyntaxImpl;
use crate::jdk::object::JavaObject;

/// This processor verifies if `ConfigurationKeys.ALLOW_LEADING_WILDCARD` is
/// defined in the `QueryConfigHandler`. If it is and leading wildcard is not
/// allowed, it looks for every `WildcardQueryNode` contained in the query
/// node tree and throws an exception if any of them has a leading wildcard
/// ('*' or '?').
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.AllowLeadingWildcardProcessor`.
#[derive(Debug, Default)]
pub struct AllowLeadingWildcardProcessor {
    base: QueryNodeProcessorImpl,
}

impl AllowLeadingWildcardProcessor {
    /// Equivalent to `new AllowLeadingWildcardProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for AllowLeadingWildcardProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        let allows_leading_wildcard = self
            .get_query_config_handler()
            .ok_or_else(LuceneError::null_pointer)?
            .get(ConfigurationKeys::ALLOW_LEADING_WILDCARD);
        if let Some(allows_leading_wildcard) = allows_leading_wildcard {
            if !allows_leading_wildcard {
                return QueryNodeProcessorImpl::process(self, query_tree);
            }
        }
        Ok(query_tree)
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if instance_of(node.as_ref(), &WILDCARD_QUERY_NODE_CLASS) {
            if let Some(wildcard_node) = node.as_field_query_node() {
                let text = wildcard_node
                    .get_text()
                    .ok_or_else(LuceneError::null_pointer)?;
                if !text.is_empty() {
                    // Validate if the wildcard was escaped
                    if UnescapedCharSequence::was_escaped_in(&text, 0)? {
                        return Ok(node);
                    }
                    let first = text.char_at(0)?;
                    if first == u16::from(b'*') || first == u16::from(b'?') {
                        let query_string = node.to_query_string(&EscapeQuerySyntaxImpl::new())?;
                        return Err(QueryNodeException::new(Arc::new(MessageImpl::with_args(
                            QueryParserMessages::LEADING_WILDCARD_NOT_ALLOWED.get(),
                            vec![JavaObject::from(query_string)],
                        )))
                        .error());
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
