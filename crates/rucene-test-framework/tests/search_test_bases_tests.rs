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

//! Unit tests of the three test-framework bases the query-module tests
//! extend (rmp #474): `BaseExplanationTestCase`, `SearchEquivalenceTestBase`
//! and `MatchesTestBase`.
//!
//! Each test runs, through the ported `randomizedtesting` runner, a minimal
//! subclass with two methods over ported core queries (`TermQuery`,
//! `BooleanQuery`): one whose expectation is right and must pass, and one
//! whose expectation is wrong (the positive control) and must fail with
//! `java.lang.AssertionError`. A base whose checks could not fail would pass
//! the second method too, and the test would fail.
//!
//! These classes are not Lucene's: Lucene has no unit test of its test
//! bases.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Failure, JUnitCore, Request, RunListener};
use rucene::index::terms::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::util::BytesRef;
use rucene_test_framework::search::base_explanation_test_case::{
    self, bqtest, qtest, ExplanationTestInstance, FIELD,
};
use rucene_test_framework::search::matches_test_base::{
    self, MatchesTestBase, MatchesTestInstance, FIELD_WITH_OFFSETS,
};
use rucene_test_framework::search::search_equivalence_test_base::{
    self, assert_same_scores, assert_subset_of, EquivalenceTestInstance,
};
use rucene_test_framework::util::java_exceptions::rethrow_java;

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// The failures a run reported: `(method, exception class, message)`.
#[derive(Default)]
struct Failures(Mutex<Vec<(String, String, String)>>);

impl RunListener for Failures {
    fn test_failure(&self, f: &Failure) {
        let t = f.exception();
        self.0.lock().unwrap().push((
            f.description()
                .method_name()
                .unwrap_or_else(|| "null".into()),
            t.class_name().to_string(),
            t.message().unwrap_or("null").to_string(),
        ));
    }
}

/// Runs the suite `class` and asserts that it ran two tests and that only
/// `control` failed, with a `java.lang.AssertionError`.
fn assert_only_control_fails(class: Arc<Class>, control: &str) {
    let _guard = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    rucene_test_framework::util::lucene_test_case::enable_error_backtraces();
    let listener = Arc::new(Failures::default());
    let mut core = JUnitCore::new();
    core.add_listener(Arc::clone(&listener) as Arc<dyn RunListener>);
    let result = core.run(&Request::a_class(&class));
    let failures = listener.0.lock().unwrap().clone();
    for (m, c, msg) in &failures {
        println!("{}: FAIL {m}: {c}: {msg}", class.name());
    }
    assert_eq!(result.run_count(), 2, "{}: tests run", class.name());
    assert_eq!(
        failures.len(),
        1,
        "{}: exactly the positive control fails: {failures:?}",
        class.name()
    );
    let (method, exception, _) = &failures[0];
    assert_eq!(method, control, "{}: the failing method", class.name());
    assert_eq!(
        exception,
        "java.lang.AssertionError",
        "{}: the control fails an assertion",
        class.name()
    );
}

/// `new TermQuery(new Term(field, text))`.
fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::new(
        field.to_string(),
        BytesRef::from_char_sequence(text),
    )))
}

/// A `BooleanQuery` of `clauses`.
fn boolean_query(clauses: &[(Arc<dyn Query>, Occur)]) -> Arc<dyn Query> {
    let mut b = Builder::new();
    for (q, occur) in clauses {
        b.add(Arc::clone(q), *occur)
            .unwrap_or_else(|e| rethrow_java(e));
    }
    Arc::new(b.build())
}

/// A test method whose body returns a Lucene result.
fn method<F: std::any::Any + Send>(
    name: &str,
    body: fn(&mut F) -> rucene::error::Result<()>,
) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut F| {
            if let Err(e) = body(inst) {
                rethrow_java(e)
            }
        })
}

// -----------------------------------------------------------------------------
// BaseExplanationTestCase
// -----------------------------------------------------------------------------

/// The docs `BaseExplanationTestCase.docFields` indexes: `w1` and `w2` are in
/// all four, `xx` in docs 2 and 3, `zz` in docs 1 and 3.
fn explanation_class() -> Arc<Class> {
    Class::builder("org.apache.rucene.tests.TestBaseExplanationTestCaseUnit")
        .extends(&base_explanation_test_case::class())
        .constructor(Constructor::no_args(ExplanationTestInstance::default))
        .method(method(
            "testTermAndBooleanQueries",
            |_: &mut ExplanationTestInstance| {
                qtest(term_query(FIELD, "w1"), &[0, 1, 2, 3])?;
                qtest(term_query(FIELD, "xx"), &[2, 3])?;
                bqtest(
                    boolean_query(&[
                        (term_query(FIELD, "w2"), Occur::MUST),
                        (term_query(FIELD, "zz"), Occur::SHOULD),
                    ]),
                    &[0, 1, 2, 3],
                )
            },
        ))
        .method(method(
            "testWrongExpectation",
            |_: &mut ExplanationTestInstance| qtest(term_query(FIELD, "xx"), &[0, 2]),
        ))
        .build()
}

#[test]
fn base_explanation_test_case_checks_hits_and_explanations() {
    assert_only_control_fails(explanation_class(), "testWrongExpectation");
}

// -----------------------------------------------------------------------------
// SearchEquivalenceTestBase
// -----------------------------------------------------------------------------

/// A single character term of the index that matches at least one live
/// document (the stop word is never indexed).
fn indexed_term() -> rucene::error::Result<Term> {
    let state = search_equivalence_test_base::state();
    let state = state.as_ref().expect("beforeClass() ran");
    for c in 'a'..='z' {
        let term = Term::new("field", BytesRef::from_char_sequence(&c.to_string()));
        if state.s1.count(Arc::new(TermQuery::new(term.clone())))? > 0 {
            return Ok(term);
        }
    }
    panic!("the index holds no term");
}

fn equivalence_class() -> Arc<Class> {
    Class::builder("org.apache.rucene.tests.TestSearchEquivalenceTestBaseUnit")
        .extends(&search_equivalence_test_base::class())
        .constructor(Constructor::no_args(EquivalenceTestInstance::default))
        .method(method(
            "testEquivalence",
            |_: &mut EquivalenceTestInstance| {
                let t1 = search_equivalence_test_base::random_term();
                let t2 = search_equivalence_test_base::random_term();
                let q1: Arc<dyn Query> = Arc::new(TermQuery::new(t1.clone()));
                // a single SHOULD clause matches and scores as its clause
                let q2 = boolean_query(&[(Arc::clone(&q1), Occur::SHOULD)]);
                assert_same_scores(Arc::clone(&q1), q2)?;
                // a conjunction is a subset of each of its clauses
                let both = boolean_query(&[
                    (Arc::clone(&q1), Occur::MUST),
                    (Arc::new(TermQuery::new(t2)), Occur::MUST),
                ]);
                assert_subset_of(both, q1)
            },
        ))
        .method(method(
            "testWrongExpectation",
            |_: &mut EquivalenceTestInstance| {
                let t = indexed_term()?;
                let q: Arc<dyn Query> = Arc::new(TermQuery::new(t));
                // matches nothing, so `q` is no subset of it
                let none = boolean_query(&[
                    (Arc::clone(&q), Occur::MUST),
                    (Arc::clone(&q), Occur::MUST_NOT),
                ]);
                assert_subset_of(q, none)
            },
        ))
        .build()
}

#[test]
fn search_equivalence_test_base_checks_subsets_and_scores() {
    assert_only_control_fails(equivalence_class(), "testWrongExpectation");
}

// -----------------------------------------------------------------------------
// MatchesTestBase
// -----------------------------------------------------------------------------

/// The documents of the subclass (`getDocuments()`).
struct UnitDocs;

impl MatchesTestBase for UnitDocs {
    fn get_documents(&self) -> Vec<String> {
        [
            "w1 w2 w3 w4 w5",
            "w1 w3 w2 w3 zz",
            "w1 xx w2 yy w3",
            "w1 w3 xx w2 yy w3 zz",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}

fn matches_class() -> Arc<Class> {
    Class::builder("org.apache.rucene.tests.TestMatchesTestBaseUnit")
        .extends(&matches_test_base::class())
        .constructor(Constructor::no_args(|| MatchesTestInstance::new(UnitDocs)))
        .method(method(
            "testTermAndBooleanMatches",
            |inst: &mut MatchesTestInstance| {
                let state = inst.state.as_ref().expect("setUp() ran");
                // `w1` at position 0, offsets 0-2, in every document
                state.check_matches(
                    term_query(FIELD_WITH_OFFSETS, "w1"),
                    FIELD_WITH_OFFSETS,
                    &[
                        vec![0, 0, 0, 0, 2],
                        vec![1, 0, 0, 0, 2],
                        vec![2, 0, 0, 0, 2],
                        vec![3, 0, 0, 0, 2],
                    ],
                )?;
                // +xx zz: no match in docs 0 and 1; xx in doc 2; xx then zz in doc 3
                let q = boolean_query(&[
                    (term_query(FIELD_WITH_OFFSETS, "xx"), Occur::MUST),
                    (term_query(FIELD_WITH_OFFSETS, "zz"), Occur::SHOULD),
                ]);
                state.check_matches(
                    Arc::clone(&q),
                    FIELD_WITH_OFFSETS,
                    &[
                        vec![0],
                        vec![1],
                        vec![2, 1, 1, 3, 5],
                        vec![3, 2, 2, 6, 8, 6, 6, 18, 20],
                    ],
                )?;
                state.check_label_count(q, FIELD_WITH_OFFSETS, &[0, 0, 1, 2])
            },
        ))
        .method(method(
            "testWrongExpectation",
            |inst: &mut MatchesTestInstance| {
                let state = inst.state.as_ref().expect("setUp() ran");
                state.check_matches(
                    term_query(FIELD_WITH_OFFSETS, "w1"),
                    FIELD_WITH_OFFSETS,
                    &[vec![0, 1, 1, 0, 2]],
                )
            },
        ))
        .build()
}

#[test]
fn matches_test_base_checks_positions_offsets_and_labels() {
    assert_only_control_fails(matches_class(), "testWrongExpectation");
}
