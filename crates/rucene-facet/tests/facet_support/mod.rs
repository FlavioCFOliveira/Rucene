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

//! Helpers shared by the ports of the tests of Lucene 10.5.0
//! `lucene/facet/src/test` (sprint 17, rmp #630): the suite plumbing of the
//! sprint-9 core test ports (`core_support`, `search_support`), reused by
//! path, and the class `org.apache.lucene.facet.FacetTestCase`.

#![allow(dead_code, unused_imports)]

#[path = "../../../rucene-test-framework/tests/core_support/mod.rs"]
pub mod core_support;

#[path = "../../../rucene-test-framework/tests/core_search/support.rs"]
pub mod search_support;

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene_test_framework::util::LuceneTestCase;

/// The class `org.apache.lucene.facet.FacetTestCase` (`public abstract
/// class FacetTestCase extends LuceneTestCase`). Its helper methods are
/// ported with the test classes that call them.
fn facet_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.facet.FacetTestCase")
            .extends(&LuceneTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test class `name` extending `FacetTestCase`, with static `methods`.
pub fn facet_class(name: &str, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&facet_test_case_class())
        .constructor(randomizedtesting::jdk::reflect::Constructor::no_args(|| ()));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}
