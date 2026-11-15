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

//! Port of `TestSpanMatches.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::query::Query;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::search::matches_test_base::{
    self, MatchesTestBase, MatchesTestInstance, TermMatch, FIELD_WITH_OFFSETS,
};

use crate::queries_support::search_support::ftest;

/// `getDocuments()`.
struct SpanMatchesDocs;

impl MatchesTestBase for SpanMatchesDocs {
    fn get_documents(&self) -> Vec<String> {
        [
            "w1 w2 w3 w4 w5",
            "w1 w3 w2 w3 zz",
            "w1 xx w2 yy w4",
            "w1 w2 w1 w4 w2 w3",
            "a phrase sentence with many phrase sentence iterations of a phrase sentence",
            "nothing matches this document",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}

/// `TestSpanMatches extends MatchesTestBase`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.queries.spans.TestSpanMatches")
        .extends(&matches_test_base::class())
        .constructor(Constructor::no_args(|| {
            MatchesTestInstance::new(SpanMatchesDocs)
        }))
        .method(ftest("testSpanQuery", test_span_query))
        .build()
}

fn stq(text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(Term::from_text(
        FIELD_WITH_OFFSETS,
        text,
    )))
}

fn test_span_query(inst: &mut MatchesTestInstance) -> Result<()> {
    let state = inst.state.as_ref().expect("setUp ran");
    let mut subq = SpanNearQuery::new_ordered_near_query(FIELD_WITH_OFFSETS);
    subq.add_clause(stq("with"))?.add_clause(stq("many"))?;
    let subq: Arc<dyn SpanQuery> = Arc::new(subq.build()?);
    let mut q = SpanNearQuery::new_ordered_near_query(FIELD_WITH_OFFSETS);
    q.add_clause(stq("sentence"))?
        .add_clause(Arc::new(SpanOrQuery::new(vec![subq, stq("iterations")])?))?;
    let q: Arc<dyn Query> = Arc::new(q.build()?);
    state.check_matches(
        Arc::clone(&q),
        FIELD_WITH_OFFSETS,
        &[
            vec![0],
            vec![1],
            vec![2],
            vec![3],
            vec![4, 2, 4, 9, 27, 6, 7, 35, 54],
        ],
    )?;
    state.check_label_count(Arc::clone(&q), FIELD_WITH_OFFSETS, &[0, 0, 0, 0, 1])?;
    state.check_term_matches(
        q,
        FIELD_WITH_OFFSETS,
        &[
            vec![],
            vec![],
            vec![],
            vec![],
            vec![
                vec![
                    TermMatch::new(2, 9, 17),
                    TermMatch::new(3, 18, 22),
                    TermMatch::new(4, 23, 27),
                ],
                vec![TermMatch::new(6, 35, 43), TermMatch::new(7, 44, 54)],
            ],
        ],
    )
}
