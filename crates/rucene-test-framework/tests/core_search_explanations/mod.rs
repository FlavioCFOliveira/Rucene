//! The test methods of `org.apache.lucene.search.TestSimpleExplanations` and
//! `TestComplexExplanations` (`lucene/core/src/test`), which their
//! subclasses `TestSimpleExplanationsOfNonMatches`,
//! `TestSimpleExplanationsWithFillerDocs` and
//! `TestComplexExplanationsOfNonMatches` inherit.
//!
//! Java's `qtest` and `bqtest` are instance methods the subclasses override;
//! here they are the methods of [`ExplanationsCase`], which each test method
//! receives.

use std::sync::Arc;

use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::similarities::explanation::Explanation;
use rucene::search::synonym_query::Builder as SynonymQueryBuilder;
use rucene::search::{BoostQuery, ConstantScoreQuery, DisjunctionMaxQuery, MatchAllDocsQuery};
use rucene::util::BytesRef;
use rucene_test_framework::search::base_explanation_test_case::{
    self, match_these_items, opt_b, req_b, ta, ALTFIELD, FIELD,
};

/// The overridable instance methods `qtest(Query, int[])` and
/// `bqtest(Query, int[])` of `BaseExplanationTestCase`.
pub trait ExplanationsCase: Send + Sync {
    /// `qtest(Query q, int[] expDocNrs)`.
    fn qtest(&self, q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
        base_explanation_test_case::qtest(q, exp_doc_nrs)
    }

    /// `bqtest(Query q, int[] expDocNrs)`: tests a query using qtest after
    /// wrapping it with both optB and reqB.
    fn bqtest(&self, q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
        self.qtest(req_b(Arc::clone(&q))?, exp_doc_nrs)?;
        self.qtest(opt_b(q)?, exp_doc_nrs)
    }
}

/// `new TermQuery(new Term(field, text))`.
fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    base_explanation_test_case::term_query(field, text)
}

/// `MatchAllDocsQuery.INSTANCE`.
fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

/// `new PhraseQuery(slop, field, terms...)` (`slop` 0 for
/// `new PhraseQuery(field, terms...)`).
fn phrase(slop: i32, field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop(slop, field, terms)?))
}

/// `new BoostQuery(query, boost)`.
fn boost(query: Arc<dyn Query>, boost: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(query, boost)?))
}

/// `new ConstantScoreQuery(query)`.
fn csq(query: Arc<dyn Query>) -> Arc<dyn Query> {
    Arc::new(ConstantScoreQuery::new(query))
}

/// `new DisjunctionMaxQuery(disjuncts, tieBreakerMultiplier)`.
fn dmq(disjuncts: Vec<Arc<dyn Query>>, tie_breaker: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(DisjunctionMaxQuery::new(disjuncts, tie_breaker)?))
}

/// `builder.build()` of a `BooleanQuery.Builder`.
fn bq(builder: &BooleanQueryBuilder) -> Arc<dyn Query> {
    Arc::new(builder.build())
}

/// `Object.hashCode()` equality as Rust states it: the `Hash` of the value.
fn hash_of<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault};
    BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default().hash_one(t)
}

// -----------------------------------------------------------------------------
// TestSimpleExplanations
// -----------------------------------------------------------------------------

/// `testT1()`.
pub fn test_t1(c: &dyn ExplanationsCase) -> Result<()> {
    c.qtest(tq(FIELD, "w1"), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testT2()`.
pub fn test_t2(c: &dyn ExplanationsCase) -> Result<()> {
    let term_query = tq(FIELD, "w1");
    c.qtest(boost(term_query, 100.0)?, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMA1()`.
pub fn test_ma1(c: &dyn ExplanationsCase) -> Result<()> {
    c.qtest(match_all(), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMA2()`.
pub fn test_ma2(c: &dyn ExplanationsCase) -> Result<()> {
    let q = match_all();
    c.qtest(boost(q, 1000.0)?, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testP1()`.
pub fn test_p1(c: &dyn ExplanationsCase) -> Result<()> {
    let phrase_query = phrase(0, FIELD, &["w1", "w2"])?;
    c.qtest(phrase_query, &[0])?;
    Ok(())
}

/// `testP2()`.
pub fn test_p2(c: &dyn ExplanationsCase) -> Result<()> {
    let phrase_query = phrase(0, FIELD, &["w1", "w3"])?;
    c.qtest(phrase_query, &[1, 3])?;
    Ok(())
}

/// `testP3()`.
pub fn test_p3(c: &dyn ExplanationsCase) -> Result<()> {
    let phrase_query = phrase(1, FIELD, &["w1", "w2"])?;
    c.qtest(phrase_query, &[0, 1, 2])?;
    Ok(())
}

/// `testP4()`.
pub fn test_p4(c: &dyn ExplanationsCase) -> Result<()> {
    let phrase_query = phrase(1, FIELD, &["w2", "w3"])?;
    c.qtest(phrase_query, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testP5()`.
pub fn test_p5(c: &dyn ExplanationsCase) -> Result<()> {
    let phrase_query = phrase(1, FIELD, &["w3", "w2"])?;
    c.qtest(phrase_query, &[1, 3])?;
    Ok(())
}

/// `testP6()`.
pub fn test_p6(c: &dyn ExplanationsCase) -> Result<()> {
    let phrase_query = phrase(2, FIELD, &["w3", "w2"])?;
    c.qtest(phrase_query, &[0, 1, 3])?;
    Ok(())
}

/// `testP7()`.
pub fn test_p7(c: &dyn ExplanationsCase) -> Result<()> {
    let phrase_query = phrase(3, FIELD, &["w3", "w2"])?;
    c.qtest(phrase_query, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testCSQ1()`.
pub fn test_csq1(c: &dyn ExplanationsCase) -> Result<()> {
    let q = csq(match_these_items(&[0, 1, 2, 3])?);
    c.qtest(q, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testCSQ2()`.
pub fn test_csq2(c: &dyn ExplanationsCase) -> Result<()> {
    let q = csq(match_these_items(&[1, 3])?);
    c.qtest(q, &[1, 3])?;
    Ok(())
}

/// `testCSQ3()`.
pub fn test_csq3(c: &dyn ExplanationsCase) -> Result<()> {
    let q = csq(match_these_items(&[0, 2])?);
    c.qtest(boost(q, 1000.0)?, &[0, 2])?;
    Ok(())
}

/// `testDMQ1()`.
pub fn test_dmq1(c: &dyn ExplanationsCase) -> Result<()> {
    let q = dmq(vec![tq(FIELD, "w1"), tq(FIELD, "w5")], 0.0)?;
    c.qtest(q, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testDMQ2()`.
pub fn test_dmq2(c: &dyn ExplanationsCase) -> Result<()> {
    let q = dmq(vec![tq(FIELD, "w1"), tq(FIELD, "w5")], 0.5)?;
    c.qtest(q, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testDMQ3()`.
pub fn test_dmq3(c: &dyn ExplanationsCase) -> Result<()> {
    let q = dmq(vec![tq(FIELD, "QQ"), tq(FIELD, "w5")], 0.5)?;
    c.qtest(q, &[0])?;
    Ok(())
}

/// `testDMQ4()`.
pub fn test_dmq4(c: &dyn ExplanationsCase) -> Result<()> {
    let q = dmq(vec![tq(FIELD, "QQ"), tq(FIELD, "xx")], 0.5)?;
    c.qtest(q, &[2, 3])?;
    Ok(())
}

/// `testDMQ5()`.
pub fn test_dmq5(c: &dyn ExplanationsCase) -> Result<()> {
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    boolean_query.add(tq(FIELD, "QQ"), Occur::MUST_NOT)?;
    let q = dmq(vec![bq(&boolean_query), tq(FIELD, "xx")], 0.5)?;
    c.qtest(q, &[2, 3])?;
    Ok(())
}

/// `testDMQ6()`.
pub fn test_dmq6(c: &dyn ExplanationsCase) -> Result<()> {
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tq(FIELD, "yy"), Occur::MUST_NOT)?;
    boolean_query.add(tq(FIELD, "w3"), Occur::SHOULD)?;
    let q = dmq(vec![bq(&boolean_query), tq(FIELD, "xx")], 0.5)?;
    c.qtest(q, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testDMQ7()`.
pub fn test_dmq7(c: &dyn ExplanationsCase) -> Result<()> {
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tq(FIELD, "yy"), Occur::MUST_NOT)?;
    boolean_query.add(tq(FIELD, "w3"), Occur::SHOULD)?;
    let q = dmq(vec![bq(&boolean_query), tq(FIELD, "w2")], 0.5)?;
    c.qtest(q, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testDMQ8()`.
pub fn test_dmq8(c: &dyn ExplanationsCase) -> Result<()> {
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    let boosted_query = tq(FIELD, "w5");
    boolean_query.add(boost(boosted_query, 100.0)?, Occur::SHOULD)?;
    let xx_boosted_query = tq(FIELD, "xx");
    let q = dmq(
        vec![bq(&boolean_query), boost(xx_boosted_query, 100000.0)?],
        0.5,
    )?;
    c.qtest(q, &[0, 2, 3])?;
    Ok(())
}

/// `testDMQ9()`.
pub fn test_dmq9(c: &dyn ExplanationsCase) -> Result<()> {
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    let boosted_query = tq(FIELD, "w5");
    boolean_query.add(boost(boosted_query, 100.0)?, Occur::SHOULD)?;
    let xx_boosted_query = tq(FIELD, "xx");
    let q = dmq(vec![bq(&boolean_query), boost(xx_boosted_query, 0.0)?], 0.5)?;
    c.qtest(q, &[0, 2, 3])?;
    Ok(())
}

/// `testMPQ1()`.
pub fn test_mpq1(c: &dyn ExplanationsCase) -> Result<()> {
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_terms(ta(&["w1"]))?;
    qb.add_terms(ta(&["w2", "w3", "xx"]))?;
    c.qtest(Arc::new(qb.build()), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMPQ2()`.
pub fn test_mpq2(c: &dyn ExplanationsCase) -> Result<()> {
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_terms(ta(&["w1"]))?;
    qb.add_terms(ta(&["w2", "w3"]))?;
    c.qtest(Arc::new(qb.build()), &[0, 1, 3])?;
    Ok(())
}

/// `testMPQ3()`.
pub fn test_mpq3(c: &dyn ExplanationsCase) -> Result<()> {
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_terms(ta(&["w1", "xx"]))?;
    qb.add_terms(ta(&["w2", "w3"]))?;
    c.qtest(Arc::new(qb.build()), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMPQ4()`.
pub fn test_mpq4(c: &dyn ExplanationsCase) -> Result<()> {
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_terms(ta(&["w1"]))?;
    qb.add_terms(ta(&["w2"]))?;
    c.qtest(Arc::new(qb.build()), &[0])?;
    Ok(())
}

/// `testMPQ5()`.
pub fn test_mpq5(c: &dyn ExplanationsCase) -> Result<()> {
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_terms(ta(&["w1"]))?;
    qb.add_terms(ta(&["w2"]))?;
    qb.set_slop(1)?;
    c.qtest(Arc::new(qb.build()), &[0, 1, 2])?;
    Ok(())
}

/// `testMPQ6()`.
pub fn test_mpq6(c: &dyn ExplanationsCase) -> Result<()> {
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_terms(ta(&["w1", "w3"]))?;
    qb.add_terms(ta(&["w2"]))?;
    qb.set_slop(1)?;
    c.qtest(Arc::new(qb.build()), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ1()`.
pub fn test_bq1(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::MUST)?;
    query.add(tq(FIELD, "w2"), Occur::MUST)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ2()`.
pub fn test_bq2(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "yy"), Occur::MUST)?;
    query.add(tq(FIELD, "w3"), Occur::MUST)?;
    c.qtest(bq(&query), &[2, 3])?;
    Ok(())
}

/// `testBQ3()`.
pub fn test_bq3(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    query.add(tq(FIELD, "w3"), Occur::MUST)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ4()`.
pub fn test_bq4(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    inner_query.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::SHOULD)?;
    c.qtest(bq(&outer_query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ5()`.
pub fn test_bq5(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "qq"), Occur::MUST)?;
    inner_query.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::SHOULD)?;
    c.qtest(bq(&outer_query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ6()`.
pub fn test_bq6(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "qq"), Occur::MUST_NOT)?;
    inner_query.add(tq(FIELD, "w5"), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::MUST_NOT)?;
    c.qtest(bq(&outer_query), &[1, 2, 3])?;
    Ok(())
}

/// `testBQ7()`.
pub fn test_bq7(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::MUST)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "qq"), Occur::SHOULD)?;
    let mut child_left = BooleanQueryBuilder::new();
    child_left.add(tq(FIELD, "xx"), Occur::SHOULD)?;
    child_left.add(tq(FIELD, "w2"), Occur::MUST_NOT)?;
    inner_query.add(bq(&child_left), Occur::SHOULD)?;
    let mut child_right = BooleanQueryBuilder::new();
    child_right.add(tq(FIELD, "w3"), Occur::MUST)?;
    child_right.add(tq(FIELD, "w4"), Occur::MUST)?;
    inner_query.add(bq(&child_right), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::MUST)?;
    c.qtest(bq(&outer_query), &[0])?;
    Ok(())
}

/// `testBQ8()`.
pub fn test_bq8(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::MUST)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "qq"), Occur::SHOULD)?;
    let mut child_left = BooleanQueryBuilder::new();
    child_left.add(tq(FIELD, "xx"), Occur::SHOULD)?;
    child_left.add(tq(FIELD, "w2"), Occur::MUST_NOT)?;
    inner_query.add(bq(&child_left), Occur::SHOULD)?;
    let mut child_right = BooleanQueryBuilder::new();
    child_right.add(tq(FIELD, "w3"), Occur::MUST)?;
    child_right.add(tq(FIELD, "w4"), Occur::MUST)?;
    inner_query.add(bq(&child_right), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::SHOULD)?;
    c.qtest(bq(&outer_query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ9()`.
pub fn test_bq9(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::MUST)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "qq"), Occur::SHOULD)?;
    let mut child_left = BooleanQueryBuilder::new();
    child_left.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    child_left.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    inner_query.add(bq(&child_left), Occur::SHOULD)?;
    let mut child_right = BooleanQueryBuilder::new();
    child_right.add(tq(FIELD, "w3"), Occur::MUST)?;
    child_right.add(tq(FIELD, "w4"), Occur::MUST)?;
    inner_query.add(bq(&child_right), Occur::MUST_NOT)?;
    outer_query.add(bq(&inner_query), Occur::SHOULD)?;
    c.qtest(bq(&outer_query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ10()`.
pub fn test_bq10(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::MUST)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "qq"), Occur::SHOULD)?;
    let mut child_left = BooleanQueryBuilder::new();
    child_left.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    child_left.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    inner_query.add(bq(&child_left), Occur::SHOULD)?;
    let mut child_right = BooleanQueryBuilder::new();
    child_right.add(tq(FIELD, "w3"), Occur::MUST)?;
    child_right.add(tq(FIELD, "w4"), Occur::MUST)?;
    inner_query.add(bq(&child_right), Occur::MUST_NOT)?;
    outer_query.add(bq(&inner_query), Occur::MUST)?;
    c.qtest(bq(&outer_query), &[1])?;
    Ok(())
}

/// `testBQ11()`.
pub fn test_bq11(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let boosted_query = tq(FIELD, "w1");
    query.add(boost(boosted_query, 1000.0)?, Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ14()`.
pub fn test_bq14(c: &dyn ExplanationsCase) -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    q.add(tq(FIELD, "QQQQQ"), Occur::SHOULD)?;
    q.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    c.qtest(bq(&q), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ15()`.
pub fn test_bq15(c: &dyn ExplanationsCase) -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    q.add(tq(FIELD, "QQQQQ"), Occur::MUST_NOT)?;
    q.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    c.qtest(bq(&q), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ16()`.
pub fn test_bq16(c: &dyn ExplanationsCase) -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    q.add(tq(FIELD, "QQQQQ"), Occur::SHOULD)?;
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    boolean_query.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    q.add(bq(&boolean_query), Occur::SHOULD)?;
    c.qtest(bq(&q), &[0, 1])?;
    Ok(())
}

/// `testBQ17()`.
pub fn test_bq17(c: &dyn ExplanationsCase) -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    q.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    boolean_query.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    q.add(bq(&boolean_query), Occur::SHOULD)?;
    c.qtest(bq(&q), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ19()`.
pub fn test_bq19(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "yy"), Occur::MUST_NOT)?;
    query.add(tq(FIELD, "w3"), Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1])?;
    Ok(())
}

/// `testBQ20()`.
pub fn test_bq20(c: &dyn ExplanationsCase) -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    q.set_minimum_number_should_match(2);
    q.add(tq(FIELD, "QQQQQ"), Occur::SHOULD)?;
    q.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    q.add(tq(FIELD, "zz"), Occur::SHOULD)?;
    q.add(tq(FIELD, "w5"), Occur::SHOULD)?;
    q.add(tq(FIELD, "w4"), Occur::SHOULD)?;
    c.qtest(bq(&q), &[0, 3])?;
    Ok(())
}

/// `testBQ21()`.
pub fn test_bq21(c: &dyn ExplanationsCase) -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    q.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    q.add(tq(FIELD, "zz"), Occur::SHOULD)?;
    c.qtest(bq(&q), &[1, 2, 3])?;
    Ok(())
}

/// `testBQ23()`.
pub fn test_bq23(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::FILTER)?;
    query.add(tq(FIELD, "w2"), Occur::FILTER)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ24()`.
pub fn test_bq24(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::FILTER)?;
    query.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ25()`.
pub fn test_bq25(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::FILTER)?;
    query.add(tq(FIELD, "w2"), Occur::MUST)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ26()`.
pub fn test_bq26(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::FILTER)?;
    query.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    c.qtest(bq(&query), &[0, 1])?;
    Ok(())
}

/// `testMultiFieldBQ1()`.
pub fn test_multi_field_bq1(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::MUST)?;
    query.add(tq(ALTFIELD, "w2"), Occur::MUST)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMultiFieldBQ2()`.
pub fn test_multi_field_bq2(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "yy"), Occur::MUST)?;
    query.add(tq(ALTFIELD, "w3"), Occur::MUST)?;
    c.qtest(bq(&query), &[2, 3])?;
    Ok(())
}

/// `testMultiFieldBQ3()`.
pub fn test_multi_field_bq3(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    query.add(tq(ALTFIELD, "w3"), Occur::MUST)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMultiFieldBQ4()`.
pub fn test_multi_field_bq4(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    inner_query.add(tq(ALTFIELD, "w2"), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::SHOULD)?;
    c.qtest(bq(&outer_query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMultiFieldBQ5()`.
pub fn test_multi_field_bq5(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(ALTFIELD, "qq"), Occur::MUST)?;
    inner_query.add(tq(ALTFIELD, "w2"), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::SHOULD)?;
    c.qtest(bq(&outer_query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMultiFieldBQ6()`.
pub fn test_multi_field_bq6(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(ALTFIELD, "qq"), Occur::MUST_NOT)?;
    inner_query.add(tq(ALTFIELD, "w5"), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::MUST_NOT)?;
    c.qtest(bq(&outer_query), &[1, 2, 3])?;
    Ok(())
}

/// `testMultiFieldBQ7()`.
pub fn test_multi_field_bq7(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::MUST)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(ALTFIELD, "qq"), Occur::SHOULD)?;
    let mut child_left = BooleanQueryBuilder::new();
    child_left.add(tq(ALTFIELD, "xx"), Occur::SHOULD)?;
    child_left.add(tq(ALTFIELD, "w2"), Occur::MUST_NOT)?;
    inner_query.add(bq(&child_left), Occur::SHOULD)?;
    let mut child_right = BooleanQueryBuilder::new();
    child_right.add(tq(ALTFIELD, "w3"), Occur::MUST)?;
    child_right.add(tq(ALTFIELD, "w4"), Occur::MUST)?;
    inner_query.add(bq(&child_right), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::MUST)?;
    c.qtest(bq(&outer_query), &[0])?;
    Ok(())
}

/// `testMultiFieldBQ8()`.
pub fn test_multi_field_bq8(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(ALTFIELD, "w1"), Occur::MUST)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(FIELD, "qq"), Occur::SHOULD)?;
    let mut child_left = BooleanQueryBuilder::new();
    child_left.add(tq(ALTFIELD, "xx"), Occur::SHOULD)?;
    child_left.add(tq(FIELD, "w2"), Occur::MUST_NOT)?;
    inner_query.add(bq(&child_left), Occur::SHOULD)?;
    let mut child_right = BooleanQueryBuilder::new();
    child_right.add(tq(ALTFIELD, "w3"), Occur::MUST)?;
    child_right.add(tq(FIELD, "w4"), Occur::MUST)?;
    inner_query.add(bq(&child_right), Occur::SHOULD)?;
    outer_query.add(bq(&inner_query), Occur::SHOULD)?;
    c.qtest(bq(&outer_query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMultiFieldBQ9()`.
pub fn test_multi_field_bq9(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::MUST)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(ALTFIELD, "qq"), Occur::SHOULD)?;
    let mut child_left = BooleanQueryBuilder::new();
    child_left.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    child_left.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    inner_query.add(bq(&child_left), Occur::SHOULD)?;
    let mut child_right = BooleanQueryBuilder::new();
    child_right.add(tq(ALTFIELD, "w3"), Occur::MUST)?;
    child_right.add(tq(FIELD, "w4"), Occur::MUST)?;
    inner_query.add(bq(&child_right), Occur::MUST_NOT)?;
    outer_query.add(bq(&inner_query), Occur::SHOULD)?;
    c.qtest(bq(&outer_query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMultiFieldBQ10()`.
pub fn test_multi_field_bq10(c: &dyn ExplanationsCase) -> Result<()> {
    let mut outer_query = BooleanQueryBuilder::new();
    outer_query.add(tq(FIELD, "w1"), Occur::MUST)?;
    let mut inner_query = BooleanQueryBuilder::new();
    inner_query.add(tq(ALTFIELD, "qq"), Occur::SHOULD)?;
    let mut child_left = BooleanQueryBuilder::new();
    child_left.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    child_left.add(tq(ALTFIELD, "w2"), Occur::SHOULD)?;
    inner_query.add(bq(&child_left), Occur::SHOULD)?;
    let mut child_right = BooleanQueryBuilder::new();
    child_right.add(tq(ALTFIELD, "w3"), Occur::MUST)?;
    child_right.add(tq(FIELD, "w4"), Occur::MUST)?;
    inner_query.add(bq(&child_right), Occur::MUST_NOT)?;
    outer_query.add(bq(&inner_query), Occur::MUST)?;
    c.qtest(bq(&outer_query), &[1])?;
    Ok(())
}

/// `testMultiFieldBQofPQ1()`.
pub fn test_multi_field_b_qof_pq1(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    let left_child = phrase(0, FIELD, &["w1", "w2"])?;
    query.add(left_child, Occur::SHOULD)?;
    let right_child = phrase(0, ALTFIELD, &["w1", "w2"])?;
    query.add(right_child, Occur::SHOULD)?;
    c.qtest(bq(&query), &[0])?;
    Ok(())
}

/// `testMultiFieldBQofPQ2()`.
pub fn test_multi_field_b_qof_pq2(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    let left_child = phrase(0, FIELD, &["w1", "w3"])?;
    query.add(left_child, Occur::SHOULD)?;
    let right_child = phrase(0, ALTFIELD, &["w1", "w3"])?;
    query.add(right_child, Occur::SHOULD)?;
    c.qtest(bq(&query), &[1, 3])?;
    Ok(())
}

/// `testMultiFieldBQofPQ3()`.
pub fn test_multi_field_b_qof_pq3(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    let left_child = phrase(1, FIELD, &["w1", "w2"])?;
    query.add(left_child, Occur::SHOULD)?;
    let right_child = phrase(1, ALTFIELD, &["w1", "w2"])?;
    query.add(right_child, Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1, 2])?;
    Ok(())
}

/// `testMultiFieldBQofPQ4()`.
pub fn test_multi_field_b_qof_pq4(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    let left_child = phrase(1, FIELD, &["w2", "w3"])?;
    query.add(left_child, Occur::SHOULD)?;
    let right_child = phrase(1, ALTFIELD, &["w2", "w3"])?;
    query.add(right_child, Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMultiFieldBQofPQ5()`.
pub fn test_multi_field_b_qof_pq5(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    let left_child = phrase(1, FIELD, &["w3", "w2"])?;
    query.add(left_child, Occur::SHOULD)?;
    let right_child = phrase(1, ALTFIELD, &["w3", "w2"])?;
    query.add(right_child, Occur::SHOULD)?;
    c.qtest(bq(&query), &[1, 3])?;
    Ok(())
}

/// `testMultiFieldBQofPQ6()`.
pub fn test_multi_field_b_qof_pq6(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    let left_child = phrase(2, FIELD, &["w3", "w2"])?;
    query.add(left_child, Occur::SHOULD)?;
    let right_child = phrase(2, ALTFIELD, &["w3", "w2"])?;
    query.add(right_child, Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1, 3])?;
    Ok(())
}

/// `testMultiFieldBQofPQ7()`.
pub fn test_multi_field_b_qof_pq7(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    let left_child = phrase(3, FIELD, &["w3", "w2"])?;
    query.add(left_child, Occur::SHOULD)?;
    let right_child = phrase(1, ALTFIELD, &["w3", "w2"])?;
    query.add(right_child, Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testSynonymQuery()`.
pub fn test_synonym_query(c: &dyn ExplanationsCase) -> Result<()> {
    let query = SynonymQueryBuilder::new(Some(FIELD))
        .add_term(&Term::new(FIELD, BytesRef::from_char_sequence("w1")))?
        .add_term(&Term::new(FIELD, BytesRef::from_char_sequence("w2")))?
        .build();
    c.qtest(Arc::new(query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testEquality()` (`@Test`).
pub fn test_equality(_c: &dyn ExplanationsCase) -> Result<()> {
    let e1 = Explanation::matched(1f32, "an explanation", vec![]);
    let e2 = Explanation::matched(
        1f32,
        "an explanation",
        vec![Explanation::matched(1f32, "a subexplanation", vec![])],
    );
    let e25 = Explanation::matched(
        1f32,
        "an explanation",
        vec![Explanation::matched(
            1f32,
            "a subexplanation",
            vec![Explanation::matched(1f32, "a subsubexplanation", vec![])],
        )],
    );
    let e3 = Explanation::matched(1f32, "an explanation", vec![]);
    let e4 = Explanation::matched(2f32, "an explanation", vec![]);
    let e5 = Explanation::no_match("an explanation", vec![]);
    let e6 = Explanation::no_match(
        "an explanation",
        vec![Explanation::matched(1f32, "a subexplanation", vec![])],
    );
    let e7 = Explanation::no_match("an explanation", vec![]);
    let e8 = Explanation::matched(1f32, "another explanation", vec![]);
    Assert::assert_true(e1 == e3);
    Assert::assert_false(e1 == e2);
    Assert::assert_false(e2 == e25);
    Assert::assert_false(e1 == e4);
    Assert::assert_false(e1 == e5);
    Assert::assert_true(e5 == e7);
    Assert::assert_false(e5 == e6);
    Assert::assert_false(e1 == e8);
    Assert::assert_equals(hash_of(&e1), hash_of(&e3));
    Assert::assert_equals(hash_of(&e5), hash_of(&e7));
    Ok(())
}

// -----------------------------------------------------------------------------
// TestComplexExplanations
// -----------------------------------------------------------------------------

/// `testT3()`.
pub fn complex_test_t3(c: &dyn ExplanationsCase) -> Result<()> {
    let query = tq(FIELD, "w1");
    c.bqtest(boost(query, 0.0)?, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testMA3()`.
pub fn complex_test_ma3(c: &dyn ExplanationsCase) -> Result<()> {
    let q = match_all();
    c.bqtest(boost(q, 0.0)?, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testFQ5()`.
pub fn complex_test_fq5(c: &dyn ExplanationsCase) -> Result<()> {
    let query = tq(FIELD, "xx");
    let filtered = bq(BooleanQueryBuilder::new()
        .add(boost(query, 0.0)?, Occur::MUST)?
        .add(match_these_items(&[1, 3])?, Occur::FILTER)?);
    c.bqtest(filtered, &[3])?;
    Ok(())
}

/// `testCSQ4()`.
pub fn complex_test_csq4(c: &dyn ExplanationsCase) -> Result<()> {
    let q = csq(match_these_items(&[3])?);
    c.bqtest(boost(q, 0.0)?, &[3])?;
    Ok(())
}

/// `testDMQ10()`.
pub fn complex_test_dmq10(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    let boosted_query = tq(FIELD, "w5");
    query.add(boost(boosted_query, 100.0)?, Occur::SHOULD)?;
    let xx_boosted_query = tq(FIELD, "xx");
    let q = dmq(vec![bq(&query), boost(xx_boosted_query, 0.0)?], 0.5)?;
    c.bqtest(boost(q, 0.0)?, &[0, 2, 3])?;
    Ok(())
}

/// `testMPQ7()`.
pub fn complex_test_mpq7(c: &dyn ExplanationsCase) -> Result<()> {
    let mut qb = MultiPhraseQueryBuilder::new();
    qb.add_terms(ta(&["w1"]))?;
    qb.add_terms(ta(&["w2"]))?;
    qb.set_slop(1)?;
    c.bqtest(boost(Arc::new(qb.build()), 0.0)?, &[0, 1, 2])?;
    Ok(())
}

/// `testBQ12()`.
pub fn complex_test_bq12(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let boosted_query = tq(FIELD, "w2");
    query.add(boost(boosted_query, 0.0)?, Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ13()`.
pub fn complex_test_bq13(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "w1"), Occur::SHOULD)?;
    let boosted_query = tq(FIELD, "w5");
    query.add(boost(boosted_query, 0.0)?, Occur::MUST_NOT)?;
    c.qtest(bq(&query), &[1, 2, 3])?;
    Ok(())
}

/// `testBQ18()`.
pub fn complex_test_bq18(c: &dyn ExplanationsCase) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    let boosted_query = tq(FIELD, "w1");
    query.add(boost(boosted_query, 0.0)?, Occur::MUST)?;
    query.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    c.qtest(bq(&query), &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ21()`.
pub fn complex_test_bq21(c: &dyn ExplanationsCase) -> Result<()> {
    let mut builder = BooleanQueryBuilder::new();
    builder.add(tq(FIELD, "w1"), Occur::MUST)?;
    builder.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    let query = bq(&builder);
    c.bqtest(boost(query, 0.0)?, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testBQ22()`.
pub fn complex_test_bq22(c: &dyn ExplanationsCase) -> Result<()> {
    let mut builder = BooleanQueryBuilder::new();
    let boosted_query = tq(FIELD, "w1");
    builder.add(boost(boosted_query, 0.0)?, Occur::MUST)?;
    builder.add(tq(FIELD, "w2"), Occur::SHOULD)?;
    let query = bq(&builder);
    c.bqtest(boost(query, 0.0)?, &[0, 1, 2, 3])?;
    Ok(())
}

/// `testExplainFailingOptionalClauses()`.
pub fn complex_test_explain_failing_optional_clauses(_c: &dyn ExplanationsCase) -> Result<()> {
    let state = base_explanation_test_case::state();
    let searcher = &state
        .as_ref()
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        })
        .searcher;
    // minShouldMatch not satisfied, show failing SHOULDs.
    let msm = bq(BooleanQueryBuilder::new()
        .add(tq(FIELD, "w1"), Occur::SHOULD)?
        .add(tq(FIELD, "xx"), Occur::SHOULD)?
        .add(tq(FIELD, "zz"), Occur::SHOULD)?
        .set_minimum_number_should_match(2));
    let msm_expl = searcher.explain(msm, 0)?.to_string();
    Assert::assert_true(
        msm_expl.contains("Failure to match minimum number of optional clauses: 2, matched: 1"),
    );
    Assert::assert_true(msm_expl.contains("no match on optional clause (field:xx)"));
    Assert::assert_true(msm_expl.contains("no match on optional clause (field:zz)"));
    Assert::assert_false(msm_expl.contains("no match on optional clause (field:w1)"));

    // nothing matches in disjunction, show failing SHOULDs.
    let disj = bq(BooleanQueryBuilder::new()
        .add(tq(FIELD, "xx"), Occur::SHOULD)?
        .add(tq(FIELD, "zz"), Occur::SHOULD)?);
    let disj_expl = searcher.explain(disj, 0)?.to_string();
    Assert::assert_true(disj_expl.contains("no match on optional clause (field:xx)"));
    Assert::assert_true(disj_expl.contains("no match on optional clause (field:zz)"));

    // MUST clause fails, failing SHOULDs are not shown.
    let must_fail = bq(BooleanQueryBuilder::new()
        .add(tq(FIELD, "missing"), Occur::MUST)?
        .add(tq(FIELD, "xx"), Occur::SHOULD)?);
    Assert::assert_false(
        searcher
            .explain(must_fail, 0)?
            .to_string()
            .contains("no match on optional clause"),
    );
    Ok(())
}
