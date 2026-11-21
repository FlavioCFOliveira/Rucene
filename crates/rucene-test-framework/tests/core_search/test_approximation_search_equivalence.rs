//! Port of `lucene/core/src/test/org/apache/lucene/search/TestApproximationSearchEquivalence.java`.
//!
//! Basic equivalence tests for approximations.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQueryBuilder, ConstantScoreQuery, Occur, Query};
use rucene_test_framework::search::random_approximation_query::RandomApproximationQuery;
use rucene_test_framework::search::search_equivalence_test_base::{
    self, assert_same_scores, random_term, EquivalenceTestInstance,
};
use rucene_test_framework::util::java_exceptions::rethrow_java;

use crate::core_support::random;

/// A method of a `SearchEquivalenceTestBase` subclass.
pub fn eq_test(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |_inst: &mut EquivalenceTestInstance| {
            if let Err(e) = body() {
                rethrow_java(e)
            }
        },
    )
}

/// `public class name extends SearchEquivalenceTestBase` with `methods`.
pub fn eq_class(name: &str, methods: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&search_equivalence_test_base::class())
        .constructor(Constructor::no_args(EquivalenceTestInstance::default));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// `TestApproximationSearchEquivalence`.
pub fn class_def() -> Arc<Class> {
    eq_class(
        "org.apache.lucene.search.TestApproximationSearchEquivalence",
        vec![
            eq_test("testConjunction", test_conjunction),
            eq_test("testNestedConjunction", test_nested_conjunction),
            eq_test("testDisjunction", test_disjunction),
            eq_test("testNestedDisjunction", test_nested_disjunction),
            eq_test(
                "testDisjunctionInConjunction",
                test_disjunction_in_conjunction,
            ),
            eq_test(
                "testConjunctionInDisjunction",
                test_conjunction_in_disjunction,
            ),
            eq_test("testConstantScore", test_constant_score),
            eq_test("testExclusion", test_exclusion),
            eq_test("testNestedExclusion", test_nested_exclusion),
            eq_test("testReqOpt", test_req_opt),
        ],
    )
}

fn tq(t: &Term) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(t.clone()))
}

/// `new RandomApproximationQuery(q, random())`.
fn raq(q: &Arc<dyn Query>) -> Arc<dyn Query> {
    Arc::new(RandomApproximationQuery::new(Arc::clone(q), random()))
}

fn bq(builder: &BooleanQueryBuilder) -> Arc<dyn Query> {
    Arc::new(builder.build())
}

/// `Term t1 = randomTerm(); Term t2; do { t2 = randomTerm(); } while
/// (t1.equals(t2)); Term t3 = randomTerm();`.
fn three_terms() -> (Term, Term, Term) {
    let t1 = random_term();
    let mut t2 = random_term();
    while t1 == t2 {
        t2 = random_term();
    }
    let t3 = random_term();
    (t1, t2, t3)
}

/// `bq1(q1 o1, q2 o2)` wrapped `bq2(bq1 outer, q3 outer)` against the same
/// with both inner clauses approximated.
fn nested(o_inner: Occur, o_outer: Occur) -> Result<()> {
    let (t1, t2, t3) = three_terms();
    let q1 = tq(&t1);
    let q2 = tq(&t2);
    let q3 = tq(&t3);

    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(Arc::clone(&q1), o_inner)?;
    bq1.add(Arc::clone(&q2), o_inner)?;

    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(bq(&bq1), o_outer)?;
    bq2.add(Arc::clone(&q3), o_outer)?;

    let mut bq3 = BooleanQueryBuilder::new();
    bq3.add(raq(&q1), o_inner)?;
    bq3.add(raq(&q2), o_inner)?;

    let mut bq4 = BooleanQueryBuilder::new();
    bq4.add(bq(&bq3), o_outer)?;
    bq4.add(q3, o_outer)?;

    assert_same_scores(bq(&bq2), bq(&bq4))
}

/// `bq1(q1 o1, q2 o2)` against the same over approximations.
fn flat(o1: Occur, o2: Occur) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = tq(&t1);
    let q2 = tq(&t2);

    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(Arc::clone(&q1), o1)?;
    bq1.add(Arc::clone(&q2), o2)?;

    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(raq(&q1), o1)?;
    bq2.add(raq(&q2), o2)?;

    assert_same_scores(bq(&bq1), bq(&bq2))
}

fn test_conjunction() -> Result<()> {
    flat(Occur::MUST, Occur::MUST)
}

fn test_nested_conjunction() -> Result<()> {
    nested(Occur::MUST, Occur::MUST)
}

fn test_disjunction() -> Result<()> {
    flat(Occur::SHOULD, Occur::SHOULD)
}

fn test_nested_disjunction() -> Result<()> {
    nested(Occur::SHOULD, Occur::SHOULD)
}

fn test_disjunction_in_conjunction() -> Result<()> {
    nested(Occur::SHOULD, Occur::MUST)
}

fn test_conjunction_in_disjunction() -> Result<()> {
    nested(Occur::MUST, Occur::SHOULD)
}

fn test_constant_score() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = tq(&t1);
    let q2 = tq(&t2);

    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(
        Arc::new(ConstantScoreQuery::new(Arc::clone(&q1))),
        Occur::MUST,
    )?;
    bq1.add(
        Arc::new(ConstantScoreQuery::new(Arc::clone(&q2))),
        Occur::MUST,
    )?;

    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(Arc::new(ConstantScoreQuery::new(raq(&q1))), Occur::MUST)?;
    bq2.add(Arc::new(ConstantScoreQuery::new(raq(&q2))), Occur::MUST)?;

    assert_same_scores(bq(&bq1), bq(&bq2))
}

fn test_exclusion() -> Result<()> {
    flat(Occur::MUST, Occur::MUST_NOT)
}

fn test_nested_exclusion() -> Result<()> {
    let (t1, t2, t3) = three_terms();
    let q1 = tq(&t1);
    let q2 = tq(&t2);
    let q3 = tq(&t3);

    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(Arc::clone(&q1), Occur::MUST)?;
    bq1.add(Arc::clone(&q2), Occur::MUST_NOT)?;

    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(bq(&bq1), Occur::MUST)?;
    bq2.add(Arc::clone(&q3), Occur::MUST)?;

    // Both req and excl have approximations
    let mut bq3 = BooleanQueryBuilder::new();
    bq3.add(raq(&q1), Occur::MUST)?;
    bq3.add(raq(&q2), Occur::MUST_NOT)?;

    let mut bq4 = BooleanQueryBuilder::new();
    bq4.add(bq(&bq3), Occur::MUST)?;
    bq4.add(Arc::clone(&q3), Occur::MUST)?;

    assert_same_scores(bq(&bq2), bq(&bq4))?;

    // Only req has an approximation
    let mut bq3 = BooleanQueryBuilder::new();
    bq3.add(raq(&q1), Occur::MUST)?;
    bq3.add(Arc::clone(&q2), Occur::MUST_NOT)?;

    let mut bq4 = BooleanQueryBuilder::new();
    bq4.add(bq(&bq3), Occur::MUST)?;
    bq4.add(Arc::clone(&q3), Occur::MUST)?;

    assert_same_scores(bq(&bq2), bq(&bq4))?;

    // Only excl has an approximation
    let mut bq3 = BooleanQueryBuilder::new();
    bq3.add(Arc::clone(&q1), Occur::MUST)?;
    bq3.add(raq(&q2), Occur::MUST_NOT)?;

    let mut bq4 = BooleanQueryBuilder::new();
    bq4.add(bq(&bq3), Occur::MUST)?;
    bq4.add(q3, Occur::MUST)?;

    assert_same_scores(bq(&bq2), bq(&bq4))
}

fn test_req_opt() -> Result<()> {
    let (t1, t2, t3) = three_terms();
    let q1 = tq(&t1);
    let q2 = tq(&t2);
    let q3 = tq(&t3);

    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(Arc::clone(&q1), Occur::MUST)?;
    bq1.add(Arc::clone(&q2), Occur::SHOULD)?;

    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(bq(&bq1), Occur::MUST)?;
    bq2.add(Arc::clone(&q3), Occur::MUST)?;

    let mut bq3 = BooleanQueryBuilder::new();
    bq3.add(raq(&q1), Occur::MUST)?;
    bq3.add(raq(&q2), Occur::SHOULD)?;

    let mut bq4 = BooleanQueryBuilder::new();
    bq4.add(bq(&bq3), Occur::MUST)?;
    bq4.add(q3, Occur::MUST)?;

    assert_same_scores(bq(&bq2), bq(&bq4))
}
