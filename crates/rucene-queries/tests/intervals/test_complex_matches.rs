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

//! Port of `TestComplexMatches.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::intervals::interval_query::IntervalQuery;
use rucene_test_framework::search::matches_test_base::{
    self, MatchesTestBase, MatchesTestInstance, TermMatch, FIELD_WITH_OFFSETS,
};

use crate::intervals::support::{ordered, term, wildcard};
use crate::queries_support::search_support::ftest;

/// `getDocuments()`.
struct Docs;

impl MatchesTestBase for Docs {
    fn get_documents(&self) -> Vec<String> {
        vec![
            "compare computer science".to_string(),
            "a b a b a".to_string(),
        ]
    }
}

/// `TestComplexMatches`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.queries.intervals.TestComplexMatches")
        .extends(&matches_test_base::class())
        .constructor(Constructor::no_args(|| MatchesTestInstance::new(Docs)))
        .method(ftest("testWildcards", test_wildcards))
        .method(ftest("testRepeatedIterators", test_repeated_iterators))
        .build()
}

fn test_wildcards(inst: &mut MatchesTestInstance) -> Result<()> {
    let state = inst.state.as_ref().expect("setUp() ran");
    let q: Arc<dyn Query> = Arc::new(IntervalQuery::new(
        FIELD_WITH_OFFSETS,
        ordered(vec![wildcard("comp*"), term("science")]),
    ));
    state.check_matches(q, FIELD_WITH_OFFSETS, &[vec![0, 1, 2, 8, 24]])
}

fn test_repeated_iterators(inst: &mut MatchesTestInstance) -> Result<()> {
    let state = inst.state.as_ref().expect("setUp() ran");
    let q: Arc<dyn Query> = Arc::new(IntervalQuery::new(
        FIELD_WITH_OFFSETS,
        ordered(vec![term("a"), term("b"), term("a"), term("b"), term("a")]),
    ));
    state.check_term_matches(
        q,
        FIELD_WITH_OFFSETS,
        &[
            vec![],
            vec![vec![
                TermMatch::new(0, 0, 1),
                TermMatch::new(1, 2, 3),
                TermMatch::new(2, 4, 5),
                TermMatch::new(3, 6, 7),
                TermMatch::new(4, 8, 9),
            ]],
        ],
    )
}
