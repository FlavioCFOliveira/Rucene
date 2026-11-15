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

//! Port of the test class of Lucene 10.5.0 `lucene/sandbox/src/test` that rmp
//! task #483 lists, run as a suite by the ported runner:
//! `org.apache.lucene.sandbox.queries.TestFuzzyLikeThisQuery`.

#![allow(dead_code)]

#[path = "../../rucene-test-framework/tests/core_support/mod.rs"]
mod core_support;

#[path = "../../rucene-test-framework/tests/core_search/support.rs"]
mod search_support;

mod queries {
    pub mod test_fuzzy_like_this_query;
}

#[test]
fn test_fuzzy_like_this_query() {
    core_support::run(queries::test_fuzzy_like_this_query::class_def);
}
