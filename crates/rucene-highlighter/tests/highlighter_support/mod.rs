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

//! Helpers shared by the test targets of `rucene-highlighter` that port the tests
//! of Lucene 10.5.0 `lucene/highlighter/src/test` (sprint 17, rmp #629): the suite
//! plumbing of the sprint-9 core test ports (`core_support`,
//! `search_support`), reused by path.

#![allow(dead_code, unused_imports)]

#[path = "../../../rucene-test-framework/tests/core_support/mod.rs"]
pub mod core_support;

#[path = "../../../rucene-test-framework/tests/core_search/support.rs"]
pub mod search_support;

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::error::Result;
use rucene_test_framework::analysis::base_token_stream_test_case::BaseTokenStreamTestCase;

/// The declaration of a Java test class `name` (`public class … extends
/// BaseTokenStreamTestCase`) with `methods`.
pub fn btstc_class(name: &str, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// The `createComponents(String)` body of an anonymous analyzer.
pub type CreateComponents = dyn Fn(Option<&str>) -> Result<TokenStreamComponents> + Send + Sync;

/// An anonymous `new Analyzer() { createComponents }` of a test
/// (`GLOBAL_REUSE_STRATEGY`): its Java binary name and body.
pub struct FnAnalyzer {
    state: AnalyzerState,
    name: &'static str,
    create: Box<CreateComponents>,
}

impl fmt::Debug for FnAnalyzer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnAnalyzer")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl FnAnalyzer {
    /// `new Analyzer() { createComponents }`.
    pub fn new(
        name: &'static str,
        create: impl Fn(Option<&str>) -> Result<TokenStreamComponents> + Send + Sync + 'static,
    ) -> FnAnalyzer {
        FnAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            name,
            create: Box::new(create),
        }
    }

    /// The analyzer behind an `Arc`.
    pub fn arc(self) -> Arc<dyn Analyzer> {
        Arc::new(self)
    }
}

impl Analyzer for FnAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        self.name
    }

    fn create_components(&self, field_name: Option<&str>) -> Result<TokenStreamComponents> {
        (self.create)(field_name)
    }
}

/// Runs `body`, Java's `try { body }` whose `finally` the caller runs after.
pub fn attempt<T>(body: impl FnOnce() -> Result<T>) -> Result<T> {
    body()
}
