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

//! Port of `QueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.builders`.

use rucene::error::Result;

use super::query_tree_builder::QueryTreeBuilder;
use crate::flexible::core::nodes::query_node::{JavaObjectRef, QueryNodeRef};

/// This interface is used by implementors classes that builds some kind of
/// object from a query tree.
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.core.builders.QueryBuilder`.
pub trait QueryBuilder: Send + Sync {
    /// Builds some kind of object from a query tree.
    ///
    /// Equivalent to `build(QueryNode queryNode)`: the object built (`None`
    /// for Java's `null`).
    ///
    /// # Errors
    ///
    /// A `QueryNodeException` if something wrong happens.
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<JavaObjectRef>>;

    /// The builder as a `QueryTreeBuilder` (`instanceof QueryTreeBuilder`):
    /// a `QueryTreeBuilder` or a subclass of it.
    fn as_query_tree_builder(&self) -> Option<&QueryTreeBuilder> {
        None
    }
}
