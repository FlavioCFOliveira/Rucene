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

//! Port of `GroupQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::Query;

use super::standard_query_builder::{built_query, cast, StandardQueryBuilder};
use crate::flexible::core::nodes::group_query_node::{GroupQueryNode, GROUP_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::query_node::QueryNodeRef;

/// Builds no object, it only returns the [`Query`] object set on the
/// `GroupQueryNode` object using a
/// `QueryTreeBuilder.QUERY_TREE_BUILDER_TAGID` tag.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.GroupQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct GroupQueryNodeBuilder;

impl GroupQueryNodeBuilder {
    /// Equivalent to `new GroupQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for GroupQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let group_node: &GroupQueryNode = cast(query_node, &GROUP_QUERY_NODE_CLASS)?;
        built_query(group_node.get_child()?.as_ref())
    }
}
