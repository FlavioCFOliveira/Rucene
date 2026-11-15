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

//! Port of `FuzzyQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::Term;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::Query;

use super::standard_query_builder::{cast, multi_term_field, StandardQueryBuilder};
use crate::flexible::core::nodes::fuzzy_query_node::{FuzzyQueryNode, FUZZY_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};

/// Builds a `FuzzyQuery` object from a `FuzzyQueryNode` object.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.FuzzyQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct FuzzyQueryNodeBuilder;

impl FuzzyQueryNodeBuilder {
    /// Equivalent to `new FuzzyQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for FuzzyQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let fuzzy_node: &FuzzyQueryNode = cast(query_node, &FUZZY_QUERY_NODE_CLASS)?;
        let field_node = fuzzy_node
            .as_field_query_node()
            .ok_or_else(LuceneError::null_pointer)?;
        let text = field_node
            .get_text_as_string()
            .ok_or_else(LuceneError::null_pointer)?;
        // `text.codePointCount(0, text.length())`
        let num_edits =
            FuzzyQuery::float_to_edits(fuzzy_node.get_similarity(), text.chars().count() as i32);
        // `new Term(field, text)`, then the `MultiTermQuery` constructor's
        // check of `term.field()`.
        let field = multi_term_field(field_node.get_field_as_string())?;
        let term = Term::from_text(field, &text);
        Ok(Some(Arc::new(FuzzyQuery::with_prefix_length(
            term,
            num_edits,
            fuzzy_node.get_prefix_length(),
        )?)))
    }
}
