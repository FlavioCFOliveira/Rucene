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

//! Port of `DummyQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::Query;

use super::standard_query_builder::StandardQueryBuilder;
use crate::flexible::core::nodes::query_node::QueryNodeRef;

/// This builder does nothing. Commonly used for `QueryNode` objects that are
/// built by its parent's builder.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.DummyQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct DummyQueryNodeBuilder;

impl DummyQueryNodeBuilder {
    /// Constructs a [`DummyQueryNodeBuilder`] object.
    ///
    /// Equivalent to `new DummyQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for DummyQueryNodeBuilder {
    /// Always return `null`.
    ///
    /// Equivalent to `build(QueryNode queryNode)`, which returns a `null`
    /// `TermQuery`.
    fn build(&self, _query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        Ok(None)
    }
}
