//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSimpleSearchEquivalence.java`.
//!
//! Basic equivalence tests for core queries

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::synonym_query::Builder as SynonymQueryBuilder;
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQueryBuilder, BoostQuery, DisjunctionMaxQuery, Occur, Query};
use rucene_test_framework::search::asserting_query::AssertingQuery;
use rucene_test_framework::search::search_equivalence_test_base::{
    assert_same_scores, assert_same_set, assert_subset_of, random_term,
};

use crate::core_search::test_approximation_search_equivalence::{eq_class, eq_test};
use crate::core_support::{assert_equals, random};

// TODO: we could go a little crazy for a lot of these,
// but these are just simple minimal cases in case something
// goes horribly wrong. Put more intense tests elsewhere.

/// `TestSimpleSearchEquivalence`.
pub fn class_def() -> Arc<Class> {
    eq_class(
        "org.apache.lucene.search.TestSimpleSearchEquivalence",
        vec![
            eq_test("testTermVersusBooleanOr", test_term_versus_boolean_or),
            eq_test(
                "testTermVersusBooleanReqOpt",
                test_term_versus_boolean_req_opt,
            ),
            eq_test(
                "testBooleanReqExclVersusTerm",
                test_boolean_req_excl_versus_term,
            ),
            eq_test(
                "testBooleanAndVersusBooleanOr",
                test_boolean_and_versus_boolean_or,
            ),
            eq_test(
                "testDisjunctionSumVersusDisjunctionMax",
                test_disjunction_sum_versus_disjunction_max,
            ),
            eq_test(
                "testExactPhraseVersusBooleanAnd",
                test_exact_phrase_versus_boolean_and,
            ),
            eq_test(
                "testExactPhraseVersusBooleanAndWithHoles",
                test_exact_phrase_versus_boolean_and_with_holes,
            ),
            eq_test(
                "testPhraseVersusSloppyPhrase",
                test_phrase_versus_sloppy_phrase,
            ),
            eq_test(
                "testPhraseVersusSloppyPhraseWithHoles",
                test_phrase_versus_sloppy_phrase_with_holes,
            ),
            eq_test(
                "testExactPhraseVersusMultiPhrase",
                test_exact_phrase_versus_multi_phrase,
            ),
            eq_test(
                "testExactPhraseVersusMultiPhraseWithHoles",
                test_exact_phrase_versus_multi_phrase_with_holes,
            ),
            eq_test(
                "testSloppyPhraseVersusBooleanAnd",
                test_sloppy_phrase_versus_boolean_and,
            ),
            eq_test(
                "testPhraseRelativePositions",
                test_phrase_relative_positions,
            ),
            eq_test(
                "testSloppyPhraseRelativePositions",
                test_sloppy_phrase_relative_positions,
            ),
            eq_test(
                "testBoostQuerySimplification",
                test_boost_query_simplification,
            ),
            eq_test(
                "testBooleanBoostPropagation",
                test_boolean_boost_propagation,
            ),
            eq_test("testBooleanOrVsSynonym", test_boolean_or_vs_synonym),
        ],
    )
}

fn tq(t: &Term) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(t.clone()))
}

/// `new BooleanQuery.Builder().add(a, oa).add(b, ob).build()`.
fn bq2(a: Arc<dyn Query>, oa: Occur, b: Arc<dyn Query>, ob: Occur) -> Result<Arc<dyn Query>> {
    let mut q = BooleanQueryBuilder::new();
    q.add(a, oa)?;
    q.add(b, ob)?;
    Ok(Arc::new(q.build()))
}

/// `new PhraseQuery(slop, t1.field(), t1.bytes(), t2.bytes())`.
fn phrase(slop: i32, t1: &Term, t2: &Term) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop_bytes(
        slop,
        t1.field(),
        &[t1.bytes().clone(), t2.bytes().clone()],
    )?))
}

/// A ⊆ (A B)
///
/// Port of `TestSimpleSearchEquivalence.testTermVersusBooleanOr`.
fn test_term_versus_boolean_or() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = tq(&t1);
    let q2 = bq2(tq(&t1), Occur::SHOULD, tq(&t2), Occur::SHOULD)?;
    assert_subset_of(q1, q2)
}

/// A ⊆ (+A B)
///
/// Port of `TestSimpleSearchEquivalence.testTermVersusBooleanReqOpt`.
fn test_term_versus_boolean_req_opt() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = tq(&t1);
    let q2 = bq2(tq(&t1), Occur::MUST, tq(&t2), Occur::SHOULD)?;
    assert_subset_of(q1, q2)
}

/// (A -B) ⊆ A
///
/// Port of `TestSimpleSearchEquivalence.testBooleanReqExclVersusTerm`.
fn test_boolean_req_excl_versus_term() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = bq2(tq(&t1), Occur::MUST, tq(&t2), Occur::MUST_NOT)?;
    let q2 = tq(&t1);
    assert_subset_of(q1, q2)
}

/// (+A +B) ⊆ (A B)
///
/// Port of `TestSimpleSearchEquivalence.testBooleanAndVersusBooleanOr`.
fn test_boolean_and_versus_boolean_or() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = bq2(tq(&t1), Occur::SHOULD, tq(&t2), Occur::SHOULD)?;
    let q2 = bq2(tq(&t1), Occur::SHOULD, tq(&t2), Occur::SHOULD)?;
    assert_subset_of(q1, q2)
}

/// (A B) = (A | B)
///
/// Port of `TestSimpleSearchEquivalence.testDisjunctionSumVersusDisjunctionMax`.
fn test_disjunction_sum_versus_disjunction_max() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = bq2(tq(&t1), Occur::SHOULD, tq(&t2), Occur::SHOULD)?;
    let q2: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(vec![tq(&t1), tq(&t2)], 0.5)?);
    assert_same_set(q1, q2)
}

/// "A B" ⊆ (+A +B)
///
/// Port of `TestSimpleSearchEquivalence.testExactPhraseVersusBooleanAnd`.
fn test_exact_phrase_versus_boolean_and() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = phrase(0, &t1, &t2)?;
    let q2 = bq2(tq(&t1), Occur::MUST, tq(&t2), Occur::MUST)?;
    assert_subset_of(q1, q2)
}

/// same as above, with posincs
///
/// Port of `TestSimpleSearchEquivalence.testExactPhraseVersusBooleanAndWithHoles`.
fn test_exact_phrase_versus_boolean_and_with_holes() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(t1.clone(), 0)?;
    builder.add_at(t2.clone(), 2)?;
    let q1: Arc<dyn Query> = Arc::new(builder.build()?);
    let q2 = bq2(tq(&t1), Occur::MUST, tq(&t2), Occur::MUST)?;
    assert_subset_of(q1, q2)
}

/// "A B" ⊆ "A B"~1
///
/// Port of `TestSimpleSearchEquivalence.testPhraseVersusSloppyPhrase`.
fn test_phrase_versus_sloppy_phrase() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = phrase(0, &t1, &t2)?;
    let q2 = phrase(1, &t1, &t2)?;
    assert_subset_of(q1, q2)
}

/// same as above, with posincs
///
/// Port of `TestSimpleSearchEquivalence.testPhraseVersusSloppyPhraseWithHoles`.
fn test_phrase_versus_sloppy_phrase_with_holes() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(t1, 0)?;
    builder.add_at(t2, 2)?;
    let q1: Arc<dyn Query> = Arc::new(builder.build()?);
    builder.set_slop(2);
    let q2: Arc<dyn Query> = Arc::new(builder.build()?);
    assert_subset_of(q1, q2)
}

/// "A B" ⊆ "A (B C)"
///
/// Port of `TestSimpleSearchEquivalence.testExactPhraseVersusMultiPhrase`.
fn test_exact_phrase_versus_multi_phrase() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = phrase(0, &t1, &t2)?;
    let t3 = random_term();
    let mut q2b = MultiPhraseQueryBuilder::new();
    q2b.add(t1)?;
    q2b.add_terms(vec![t2, t3])?;
    assert_subset_of(q1, Arc::new(q2b.build()))
}

/// same as above, with posincs
///
/// Port of `TestSimpleSearchEquivalence.testExactPhraseVersusMultiPhraseWithHoles`.
fn test_exact_phrase_versus_multi_phrase_with_holes() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(t1.clone(), 0)?;
    builder.add_at(t2.clone(), 2)?;
    let q1: Arc<dyn Query> = Arc::new(builder.build()?);
    let t3 = random_term();
    let mut q2b = MultiPhraseQueryBuilder::new();
    q2b.add(t1)?;
    q2b.add_at(vec![t2, t3], 2)?;
    assert_subset_of(q1, Arc::new(q2b.build()))
}

/// "A B"~∞ = +A +B if A != B
///
/// Port of `TestSimpleSearchEquivalence.testSloppyPhraseVersusBooleanAnd`.
fn test_sloppy_phrase_versus_boolean_and() -> Result<()> {
    let t1 = random_term();
    // semantics differ from SpanNear: SloppyPhrase handles repeats,
    // so we must ensure t1 != t2
    let mut t2 = random_term();
    while t1 == t2 {
        t2 = random_term();
    }
    let q1 = phrase(i32::MAX, &t1, &t2)?;
    let q2 = bq2(tq(&t1), Occur::MUST, tq(&t2), Occur::MUST)?;
    assert_same_set(q1, q2)
}

/// Phrase positions are relative.
///
/// Port of `TestSimpleSearchEquivalence.testPhraseRelativePositions`.
fn test_phrase_relative_positions() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = phrase(0, &t1, &t2)?;
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(t1, 10000)?;
    builder.add_at(t2, 10001)?;
    let q2: Arc<dyn Query> = Arc::new(builder.build()?);
    assert_same_scores(q1, q2)
}

/// Sloppy-phrase positions are relative.
///
/// Port of `TestSimpleSearchEquivalence.testSloppyPhraseRelativePositions`.
fn test_sloppy_phrase_relative_positions() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let q1 = phrase(2, &t1, &t2)?;
    let mut builder = PhraseQueryBuilder::new();
    builder.add_at(t1, 10000)?;
    builder.add_at(t2, 10001)?;
    builder.set_slop(2);
    let q2: Arc<dyn Query> = Arc::new(builder.build()?);
    assert_same_scores(q1, q2)
}

/// Port of `TestSimpleSearchEquivalence.testBoostQuerySimplification`.
fn test_boost_query_simplification() -> Result<()> {
    let b1 = random().next_float() * 10.0;
    let b2 = random().next_float() * 10.0;
    let term = random_term();

    let q1: Arc<dyn Query> = Arc::new(BoostQuery::new(
        Arc::new(BoostQuery::new(tq(&term), b2)?),
        b1,
    )?);
    // Use AssertingQuery to prevent BoostQuery from merging inner and outer boosts
    let q2: Arc<dyn Query> = Arc::new(BoostQuery::new(
        Arc::new(AssertingQuery::new(
            random(),
            Arc::new(BoostQuery::new(tq(&term), b2)?),
        )),
        b1,
    )?);

    assert_same_scores(q1, q2)
}

/// Port of `TestSimpleSearchEquivalence.testBooleanBoostPropagation`.
fn test_boolean_boost_propagation() -> Result<()> {
    let boost1 = random().next_float();
    let tq: Arc<dyn Query> = Arc::new(BoostQuery::new(tq(&random_term()), boost1)?);

    let boost2 = random().next_float();
    // Applying boost2 over the term or boolean query should have the same effect
    let q1: Arc<dyn Query> = Arc::new(BoostQuery::new(Arc::clone(&tq), boost2)?);
    let q2 = bq2(Arc::clone(&tq), Occur::MUST, Arc::clone(&tq), Occur::FILTER)?;
    let q2: Arc<dyn Query> = Arc::new(BoostQuery::new(q2, boost2)?);

    assert_same_scores(q1, q2)
}

/// Port of `TestSimpleSearchEquivalence.testBooleanOrVsSynonym`.
fn test_boolean_or_vs_synonym() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    assert_equals(t1.field(), t2.field());
    let mut q1 = SynonymQueryBuilder::new(t1.field());
    q1.add_term(&t1)?;
    q1.add_term(&t2)?;
    let q1: Arc<dyn Query> = Arc::new(q1.build());
    let q2 = bq2(tq(&t1), Occur::SHOULD, tq(&t2), Occur::SHOULD)?;
    assert_same_set(q1, q2)
}
