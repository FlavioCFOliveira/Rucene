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

//! Port of `PhraseQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::phrase_query::Builder;
use rucene::search::Query;

use super::standard_query_builder::{
    built_query, check_cast, query_class_cast, StandardQueryBuilder,
};
use crate::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};
use crate::flexible::core::nodes::tokenized_phrase_query_node::TOKENIZED_PHRASE_QUERY_NODE_CLASS;

/// Builds a `PhraseQuery` object from a `TokenizedPhraseQueryNode` object.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.PhraseQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct PhraseQueryNodeBuilder;

impl PhraseQueryNodeBuilder {
    /// Equivalent to `new PhraseQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for PhraseQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        check_cast(query_node.as_ref(), &TOKENIZED_PHRASE_QUERY_NODE_CLASS)?;
        let mut builder = Builder::new();
        if let Some(children) = query_node.get_children() {
            for child in &children {
                let term_query = built_query(child.as_ref())?;
                let term_query = match &term_query {
                    None => None,
                    Some(q) => Some(q.as_term_query().ok_or_else(|| {
                        query_class_cast(q.as_ref(), "org.apache.lucene.search.TermQuery")
                    })?),
                };
                let term_node = child
                    .as_field_query_node()
                    .ok_or_else(|| class_cast(child.as_ref(), &FIELD_QUERY_NODE_CLASS))?;
                let term = term_query
                    .ok_or_else(LuceneError::null_pointer)?
                    .get_term()
                    .clone();
                builder.add_at(term, term_node.get_position_increment())?;
            }
        }
        Ok(Some(Arc::new(builder.build()?)))
    }
}
