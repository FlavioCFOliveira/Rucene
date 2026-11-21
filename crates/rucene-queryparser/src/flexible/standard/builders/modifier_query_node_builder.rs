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

//! Port of `ModifierQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::Query;

use super::standard_query_builder::{built_query, StandardQueryBuilder};
use crate::flexible::core::nodes::modifier_query_node::MODIFIER_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};

/// Builds no object, it only returns the [`Query`] object set on the
/// `ModifierQueryNode` object using a
/// `QueryTreeBuilder.QUERY_TREE_BUILDER_TAGID` tag.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.ModifierQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct ModifierQueryNodeBuilder;

impl ModifierQueryNodeBuilder {
    /// Equivalent to `new ModifierQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for ModifierQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let modifier_node = query_node
            .as_modifier_query_node()
            .ok_or_else(|| class_cast(query_node.as_ref(), &MODIFIER_QUERY_NODE_CLASS))?;
        built_query(modifier_node.get_child()?.as_ref())
    }
}
