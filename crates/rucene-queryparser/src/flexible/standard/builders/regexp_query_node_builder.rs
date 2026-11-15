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

//! Port of `RegexpQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::Term;
use rucene::search::regexp_query::{DefaultProvider, RegexpQuery};
use rucene::search::Query;
use rucene::util::automaton::{RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT};

use super::standard_query_builder::{cast, rewrite_method_of, StandardQueryBuilder};
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::standard::nodes::regexp_query_node::{
    RegexpQueryNode, REGEXP_QUERY_NODE_CLASS,
};

/// Builds a `RegexpQuery` object from a `RegexpQueryNode` object.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.RegexpQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct RegexpQueryNodeBuilder;

impl RegexpQueryNodeBuilder {
    /// Equivalent to `new RegexpQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for RegexpQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let regexp_node: &RegexpQueryNode = cast(query_node, &REGEXP_QUERY_NODE_CLASS)?;
        let method = rewrite_method_of(query_node.as_ref())?;
        // TODO: make the maxStates configurable w/ a reasonable default (QueryParserBase uses 10000)
        let field = regexp_node.get_field_as_string()?;
        let term = Term::new(field, regexp_node.text_to_bytes_ref()?);
        Ok(Some(Arc::new(RegexpQuery::with_everything(
            term,
            RegExp::ALL,
            0,
            &DefaultProvider,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
            method,
            true,
        )?)))
    }
}
