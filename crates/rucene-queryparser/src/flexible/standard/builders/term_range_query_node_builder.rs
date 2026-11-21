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

//! Port of `TermRangeQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::Query;

use super::standard_query_builder::{
    cast, multi_term_field, rewrite_method_of, StandardQueryBuilder,
};
use crate::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};
use crate::flexible::core::util::string_utils::StringUtils;
use crate::flexible::standard::nodes::term_range_query_node::{
    TermRangeQueryNode, TERM_RANGE_QUERY_NODE_CLASS,
};

/// Builds a `TermRangeQuery` object from a `TermRangeQueryNode` object.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.TermRangeQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct TermRangeQueryNodeBuilder;

impl TermRangeQueryNodeBuilder {
    /// Equivalent to `new TermRangeQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for TermRangeQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let range_node: &TermRangeQueryNode = cast(query_node, &TERM_RANGE_QUERY_NODE_CLASS)?;
        let range = range_node.abstract_range_query_node();
        let upper_bound = range.get_upper_bound()?;
        let upper = upper_bound
            .as_field_query_node()
            .ok_or_else(|| class_cast(upper_bound.as_ref(), &FIELD_QUERY_NODE_CLASS))?;
        let lower_bound = range.get_lower_bound()?;
        let lower = lower_bound
            .as_field_query_node()
            .ok_or_else(|| class_cast(lower_bound.as_ref(), &FIELD_QUERY_NODE_CLASS))?;

        let field = StringUtils::to_string(range.get_field()?.as_ref());
        let mut lower_text = lower.get_text_as_string();
        let mut upper_text = upper.get_text_as_string();

        if lower_text
            .as_deref()
            .ok_or_else(LuceneError::null_pointer)?
            .is_empty()
        {
            lower_text = None;
        }

        if upper_text
            .as_deref()
            .ok_or_else(LuceneError::null_pointer)?
            .is_empty()
        {
            upper_text = None;
        }

        let method = rewrite_method_of(query_node.as_ref())?;

        Ok(Some(Arc::new(
            TermRangeQuery::new_string_range_with_rewrite_method(
                multi_term_field(field)?,
                lower_text.as_deref(),
                upper_text.as_deref(),
                range.is_lower_inclusive(),
                range.is_upper_inclusive(),
                method,
            )?,
        )))
    }
}
