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

//! Port of `SynonymQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder;
use rucene::search::Query;

use super::standard_query_builder::{built_query, check_cast, StandardQueryBuilder};
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::standard::nodes::synonym_query_node::SYNONYM_QUERY_NODE_CLASS;

/// Builder for `SynonymQueryNode`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.SynonymQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct SynonymQueryNodeBuilder;

impl SynonymQueryNodeBuilder {
    /// Sole constructor.
    ///
    /// Equivalent to `new SynonymQueryNodeBuilder()`.
    pub fn new() -> Self {
        Self
    }
}

impl StandardQueryBuilder for SynonymQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        // TODO: use SynonymQuery instead
        check_cast(query_node.as_ref(), &SYNONYM_QUERY_NODE_CLASS)?;
        let mut builder = Builder::new();
        let children = query_node
            .get_children()
            .ok_or_else(LuceneError::null_pointer)?;
        for child in &children {
            if let Some(query) = built_query(child.as_ref())? {
                builder.add(query, Occur::SHOULD)?;
            }
        }
        Ok(Some(Arc::new(builder.build())))
    }
}
