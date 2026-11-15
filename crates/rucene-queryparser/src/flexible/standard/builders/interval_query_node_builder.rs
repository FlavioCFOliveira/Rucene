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

//! Port of `IntervalQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::Query;

use super::standard_query_builder::{cast, StandardQueryBuilder};
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::standard::nodes::interval_query_node::{
    IntervalQueryNode, INTERVAL_QUERY_NODE_CLASS,
};

/// Builds a [`Query`] from an `IntervalQueryNode`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.IntervalQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct IntervalQueryNodeBuilder;

impl IntervalQueryNodeBuilder {
    /// Equivalent to the implicit `new IntervalQueryNodeBuilder()`.
    pub fn new() -> Self {
        Self
    }
}

impl StandardQueryBuilder for IntervalQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let node: &IntervalQueryNode = cast(query_node, &INTERVAL_QUERY_NODE_CLASS)?;
        Ok(Some(node.get_query()?))
    }
}
