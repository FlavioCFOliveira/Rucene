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

//! Port of `PrefixWildcardQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::Query;

use super::standard_query_builder::{
    cast, multi_term_term, rewrite_method_of, StandardQueryBuilder,
};
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::standard::nodes::prefix_wildcard_query_node::{
    PrefixWildcardQueryNode, PREFIX_WILDCARD_QUERY_NODE_CLASS,
};
use crate::jdk::char_sequence::CharSequence;

/// Builds a `PrefixQuery` object from a `PrefixWildcardQueryNode` object.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.PrefixWildcardQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct PrefixWildcardQueryNodeBuilder;

impl PrefixWildcardQueryNodeBuilder {
    /// Equivalent to `new PrefixWildcardQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

/// `text.subSequence(0, text.length() - 1).toString()`.
///
/// # Errors
///
/// For an empty text, the exception of `subSequence(0, -1)`: the
/// `StringIndexOutOfBoundsException` of a `String`, the
/// `ArrayIndexOutOfBoundsException` of `UnescapedCharSequence`'s array copy.
fn without_last(text: &CharSequence) -> Result<String> {
    let length = text.length();
    if length == 0 {
        return Err(if text.is_string() {
            LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
                "begin 0, end -1, length 0".to_string(),
            ))
        } else {
            LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                "arraycopy: length -1 is negative",
            )
        });
    }
    Ok(text.sub_sequence(0, length - 1)?.to_string())
}

impl StandardQueryBuilder for PrefixWildcardQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let wildcard_node: &PrefixWildcardQueryNode =
            cast(query_node, &PREFIX_WILDCARD_QUERY_NODE_CLASS)?;
        let method = rewrite_method_of(query_node.as_ref())?;
        let field_node = wildcard_node
            .as_field_query_node()
            .ok_or_else(LuceneError::null_pointer)?;
        let text = field_node
            .get_text()
            .ok_or_else(LuceneError::null_pointer)?;
        let text = without_last(&text)?;
        let term = multi_term_term(field_node.get_field_as_string(), Some(text))?;
        Ok(Some(Arc::new(PrefixQuery::with_rewrite_method(
            term, method,
        )?)))
    }
}
