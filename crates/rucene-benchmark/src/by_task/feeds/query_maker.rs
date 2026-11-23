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

//! The types declared in `QueryMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::query::Query;

use crate::by_task::utils::config::Config;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.QueryMaker`
/// (`public interface QueryMaker`): create queries for the test.
///
/// A query maker is shared by the tasks of one class across threads, so its
/// methods take `&self`.
pub trait QueryMaker: Send + Sync + Any {
    /// `makeQuery(int size)`: create the next query, of the given size.
    ///
    /// # Errors
    ///
    /// An `Exception` if cannot make the query, or if size > 0 was specified
    /// but this feature is not supported.
    fn make_query_of_size(&self, size: i32) -> Result<Arc<dyn Query>>;

    /// `makeQuery()`: create the next query.
    ///
    /// # Errors
    ///
    /// An `Exception` if cannot make the query.
    fn make_query(&self) -> Result<Arc<dyn Query>>;

    /// `setConfig(Config config)`: set the properties.
    ///
    /// # Errors
    ///
    /// The exception of preparing the queries.
    fn set_config(&self, config: Arc<Config>) -> Result<()>;

    /// `resetInputs()`: reset inputs so that the test run would behave,
    /// input wise, as if it just started.
    ///
    /// # Errors
    ///
    /// The exception of preparing the queries again.
    fn reset_inputs(&self) -> Result<()>;

    /// `printQueries()`: print the queries.
    fn print_queries(&self) -> String;

    /// This maker as `Any`.
    fn as_any(&self) -> &dyn Any;
}
