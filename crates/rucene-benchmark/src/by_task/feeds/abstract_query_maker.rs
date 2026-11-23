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

//! The types declared in `AbstractQueryMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene::search::query::{query_to_string, Query};

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::utils::config::Config;
use crate::jdk::{self, NEW_LINE};

/// The fields of `AbstractQueryMaker`.
#[derive(Debug, Default)]
struct State {
    qnum: usize,
    queries: Option<Vec<Arc<dyn Query>>>,
    config: Option<Arc<Config>>,
}

/// The protected state of
/// `org.apache.lucene.benchmark.byTask.feeds.AbstractQueryMaker`: `qnum`,
/// `queries` and `config`.
#[derive(Debug, Default)]
pub struct AbstractQueryMakerBase {
    state: Mutex<State>,
}

impl AbstractQueryMakerBase {
    /// A base with no queries.
    pub fn new() -> Self {
        Self::default()
    }

    /// `config`.
    pub fn config(&self) -> Option<Arc<Config>> {
        jdk::lock(&self.state).config.clone()
    }

    /// `queries`.
    pub fn queries(&self) -> Option<Vec<Arc<dyn Query>>> {
        jdk::lock(&self.state).queries.clone()
    }

    /// `nextQnum()`: return next qnum.
    pub fn next_qnum(&self) -> usize {
        let mut s = jdk::lock(&self.state);
        let res = s.qnum;
        let len = s.queries.as_ref().map_or(0, Vec::len);
        s.qnum = (s.qnum + 1) % len;
        res
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.AbstractQueryMaker`
/// (`public abstract class AbstractQueryMaker implements QueryMaker`):
/// abstract base query maker. Each query maker should just implement the
/// [`prepare_queries`](Self::prepare_queries) method.
///
/// Every `AbstractQueryMaker` is a [`QueryMaker`] through the blanket
/// implementation, which forwards to the methods below (the defaults are the
/// base class's; a subclass overrides them here).
pub trait AbstractQueryMaker: Send + Sync + Any {
    /// The embedded base state.
    fn aqm_base(&self) -> &AbstractQueryMakerBase;

    /// `getClass().getName()`, for `toString()`.
    fn java_class_name(&self) -> &'static str;

    /// `prepareQueries()`.
    ///
    /// # Errors
    ///
    /// The exception of building the queries.
    fn prepare_queries(&self) -> Result<Vec<Arc<dyn Query>>>;

    /// `resetInputs()`.
    ///
    /// # Errors
    ///
    /// As [`set_config`](Self::set_config).
    fn reset_inputs(&self) -> Result<()> {
        jdk::lock(&self.aqm_base().state).qnum = 0;
        // re-initialize since properties by round may have changed.
        let config = self.aqm_base().config().expect("setConfig was called");
        AbstractQueryMaker::set_config(self, config)
    }

    /// `setConfig(Config)`.
    ///
    /// # Errors
    ///
    /// The exception of [`prepare_queries`](Self::prepare_queries).
    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        set_config_base(self, config)
    }

    /// `printQueries()`.
    fn print_queries(&self) -> String {
        let newline = NEW_LINE;
        let mut sb = String::new();
        if let Some(queries) = self.aqm_base().queries() {
            for (i, q) in queries.iter().enumerate() {
                sb.push_str(&format!(
                    "{i}. {} - {}",
                    jdk::simple_name(q.java_class_name()),
                    query_to_string(&**q)
                ));
                sb.push_str(newline);
            }
        }
        sb
    }

    /// `makeQuery()`.
    ///
    /// # Errors
    ///
    /// None in the base class.
    fn make_query(&self) -> Result<Arc<dyn Query>> {
        let n = self.aqm_base().next_qnum();
        Ok(Arc::clone(&self.aqm_base().queries().expect("setConfig was called")[n]))
    }

    /// `makeQuery(int size)`.
    ///
    /// # Errors
    ///
    /// `Exception(this + ".makeQuery(int size) is not supported!")`.
    fn make_query_of_size(&self, size: i32) -> Result<Arc<dyn Query>> {
        let _ = size;
        Err(jdk::exception(format!(
            "{}.makeQuery(int size) is not supported!",
            jdk::object_to_string(self.java_class_name(), self)
        )))
    }
}

/// `AbstractQueryMaker.setConfig(Config)` (what a subclass's `super.setConfig`
/// calls): stores the config, then `queries = prepareQueries()`.
///
/// # Errors
///
/// The exception of `prepareQueries()`.
pub fn set_config_base<T: AbstractQueryMaker + ?Sized>(maker: &T, config: Arc<Config>) -> Result<()> {
    jdk::lock(&maker.aqm_base().state).config = Some(config);
    let queries = maker.prepare_queries()?;
    jdk::lock(&maker.aqm_base().state).queries = Some(queries);
    Ok(())
}

impl<T: AbstractQueryMaker> QueryMaker for T {
    fn make_query_of_size(&self, size: i32) -> Result<Arc<dyn Query>> {
        AbstractQueryMaker::make_query_of_size(self, size)
    }

    fn make_query(&self) -> Result<Arc<dyn Query>> {
        AbstractQueryMaker::make_query(self)
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        AbstractQueryMaker::set_config(self, config)
    }

    fn reset_inputs(&self) -> Result<()> {
        AbstractQueryMaker::reset_inputs(self)
    }

    fn print_queries(&self) -> String {
        AbstractQueryMaker::print_queries(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
