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

//! Port of `OpenRangeQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use rucene::error::Result;

use crate::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::core::util::unescaped_char_sequence::UnescapedCharSequence;
use crate::flexible::standard::nodes::term_range_query_node::TermRangeQueryNode;
use crate::jdk::char_sequence::CharSequence;

/// Processes `TermRangeQuery`s with open ranges.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.OpenRangeQueryNodeProcessor`.
#[derive(Debug, Default)]
pub struct OpenRangeQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

/// Whether a bound's text is not an escaped open range token:
/// `!(text instanceof UnescapedCharSequence) ||
/// !((UnescapedCharSequence) text).wasEscaped(0)`.
fn not_escaped(text: Option<&CharSequence>) -> Result<bool> {
    match text
        .and_then(CharSequence::as_any)
        .and_then(|a| a.downcast_ref::<UnescapedCharSequence>())
    {
        None => Ok(true),
        Some(u) => Ok(!u.was_escaped(0)?),
    }
}

impl OpenRangeQueryNodeProcessor {
    /// Equivalent to `OpenRangeQueryNodeProcessor.OPEN_RANGE_TOKEN`.
    pub const OPEN_RANGE_TOKEN: &'static str = "*";

    /// Equivalent to `new OpenRangeQueryNodeProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for OpenRangeQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(range_node) = node.as_any().downcast_ref::<TermRangeQueryNode>() {
            let range = range_node.abstract_range_query_node();
            let lower_bound = range.get_lower_bound()?;
            let lower_node = lower_bound
                .as_field_query_node()
                .ok_or_else(|| class_cast(lower_bound.as_ref(), &FIELD_QUERY_NODE_CLASS))?;
            let upper_bound = range.get_upper_bound()?;
            let upper_node = upper_bound
                .as_field_query_node()
                .ok_or_else(|| class_cast(upper_bound.as_ref(), &FIELD_QUERY_NODE_CLASS))?;
            let mut lower_text = lower_node.get_text();
            let mut upper_text = upper_node.get_text();

            if upper_node.get_text_as_string().as_deref() == Some(Self::OPEN_RANGE_TOKEN)
                && not_escaped(upper_text.as_ref())?
            {
                upper_text = Some(CharSequence::from(""));
            }

            if lower_node.get_text_as_string().as_deref() == Some(Self::OPEN_RANGE_TOKEN)
                && not_escaped(lower_text.as_ref())?
            {
                lower_text = Some(CharSequence::from(""));
            }

            lower_node.set_text(lower_text);
            upper_node.set_text(upper_text);
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
