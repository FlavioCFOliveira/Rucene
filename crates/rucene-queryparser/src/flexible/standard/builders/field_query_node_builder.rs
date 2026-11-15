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

//! Port of `FieldQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::term_query::TermQuery;
use rucene::search::Query;

use super::standard_query_builder::{new_term, StandardQueryBuilder};
use crate::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};

/// Builds a `TermQuery` object from a `FieldQueryNode` object.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.FieldQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct FieldQueryNodeBuilder;

impl FieldQueryNodeBuilder {
    /// Equivalent to `new FieldQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for FieldQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let field_node = query_node
            .as_field_query_node()
            .ok_or_else(|| class_cast(query_node.as_ref(), &FIELD_QUERY_NODE_CLASS))?;
        Ok(Some(Arc::new(TermQuery::new(new_term(
            field_node.get_field_as_string(),
            field_node.get_text_as_string(),
        )?))))
    }
}
