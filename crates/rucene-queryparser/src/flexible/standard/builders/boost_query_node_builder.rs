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

//! Port of `BoostQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::boost_query::BoostQuery;
use rucene::search::Query;

use super::standard_query_builder::{built_query, cast, StandardQueryBuilder};
use crate::flexible::core::nodes::boost_query_node::{BoostQueryNode, BOOST_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::query_node::QueryNodeRef;

/// This builder basically reads the [`Query`] object set on the
/// `BoostQueryNode` child using `QueryTreeBuilder.QUERY_TREE_BUILDER_TAGID`
/// and applies the boost value defined in the `BoostQueryNode`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.BoostQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct BoostQueryNodeBuilder;

impl BoostQueryNodeBuilder {
    /// Equivalent to `new BoostQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for BoostQueryNodeBuilder {
    /// Equivalent to `build(QueryNode queryNode)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `new BoostQuery(null, boost)` for a
    /// child without a built query, and the `IllegalArgumentException` of an
    /// invalid boost.
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let boost_node: &BoostQueryNode = cast(query_node, &BOOST_QUERY_NODE_CLASS)?;
        let Some(child) = boost_node.get_child() else {
            return Ok(None);
        };
        // `Objects.requireNonNull(query)` of the `BoostQuery` constructor.
        let query = built_query(child.as_ref())?.ok_or_else(LuceneError::null_pointer)?;
        Ok(Some(Arc::new(BoostQuery::new(
            query,
            boost_node.get_value(),
        )?)))
    }
}
