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

//! Port of `QueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.processors`.

use std::sync::Arc;

use rucene::error::Result;

use crate::flexible::core::config::query_config_handler::QueryConfigHandler;
use crate::flexible::core::nodes::query_node::QueryNodeRef;

/// A [`QueryNodeProcessor`] is an interface for classes that process a
/// QueryNode tree.
///
/// The implementor of this class should perform some operation on a query
/// node tree and return the same or another query node tree.
///
/// It also may carry a [`QueryConfigHandler`] object that contains
/// configuration about the query represented by the query tree or the
/// collection/index where it's intended to be executed.
///
/// In case there is any [`QueryConfigHandler`] associated to the query
/// tree, it must be passed to the processor before processing the query
/// tree.
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.core.processors.QueryNodeProcessor`.
/// A processor is shared as an [`Arc`] (a pipeline and the parser hold the
/// same object), so its methods take `&self` and its state is behind locks.
pub trait QueryNodeProcessor: Send + Sync {
    /// Processes a query node tree. It may return the same or another query
    /// tree. I should never return `null`.
    ///
    /// Equivalent to `process(QueryNode queryTree)`.
    ///
    /// # Errors
    ///
    /// A `QueryNodeException` if something goes wrong during the query node
    /// processing.
    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef>;

    /// Sets the [`QueryConfigHandler`] associated to the query tree.
    ///
    /// Equivalent to `setQueryConfigHandler(QueryConfigHandler
    /// queryConfigHandler)`; `None` for Java's `null`.
    fn set_query_config_handler(&self, query_config_handler: Option<Arc<QueryConfigHandler>>);

    /// Returns the [`QueryConfigHandler`] associated to the query tree if
    /// any, otherwise it returns `null`
    ///
    /// Equivalent to `getQueryConfigHandler()`.
    fn get_query_config_handler(&self) -> Option<Arc<QueryConfigHandler>>;
}
