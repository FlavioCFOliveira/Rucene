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

//! Port of `TestSpanExplanations.java` (TestExplanations subclass focusing
//! on span queries) and of its subclass
//! `TestSpanExplanationsOfNonMatches.java`, which verifies non matches.
//!
//! Both classes run the 33 test methods `TestSpanExplanations` declares;
//! each runs on the instance's [`SpanExplanationsCase`], whose `qtest` the
//! subclass overrides.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::score_mode::ScoreMode;
use rucene::search::similarities::ExplanationValue;
use rucene::search::term_query::TermQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::search::base_explanation_test_case::{self, match_these_items, FIELD};
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::base_span_explanation_test_case::{
    self as base, sf, snear, snear3, snear_q, snear_q3, snot, sor, sor3, st,
};
use crate::queries_support::first_leaf;

/// `FIELD_CONTENT` (`private static final`).
const FIELD_CONTENT: &str = "content";

/// The `qtest` a class runs its test methods with.
pub trait SpanExplanationsCase: Send + Sync {
    /// `qtest(Query q, int[] expDocNrs)`: `BaseExplanationTestCase.qtest`
    /// unless overridden.
    fn qtest(&self, q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
        base_explanation_test_case::qtest(q, exp_doc_nrs)
    }
}

/// The instance of a class: the case whose `qtest` it uses.
pub struct CaseInstance(Arc<dyn SpanExplanationsCase>);

/// `TestSpanExplanations` itself.
struct ParentCase;

impl SpanExplanationsCase for ParentCase {}

/// `TestSpanExplanationsOfNonMatches`: subclass of TestSimpleExplanations
/// that verifies non matches.
struct NonMatchesCase;

impl SpanExplanationsCase for NonMatchesCase {
    /// Overrides superclass to ignore matches and focus on non-matches:
    /// `CheckHits.checkNoMatchExplanations(q, FIELD, searcher, expDocNrs)`.
    fn qtest(&self, q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
        let state = base_explanation_test_case::state();
        let searcher = &state
            .as_ref()
            .unwrap_or_else(|| {
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                )
            })
            .searcher;
        CheckHits::check_no_match_explanations(&q, Some(FIELD), searcher, exp_doc_nrs)
    }
}

/// A test method of `TestSpanExplanations`, run on the instance's case.
fn case_test(name: &str, body: fn(&dyn SpanExplanationsCase) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut CaseInstance| body(&*inst.0).unwrap_or_else(|e| rethrow_java(e)))
}

/// `org.apache.lucene.queries.spans.TestSpanExplanations extends
/// BaseSpanExplanationTestCase`.
pub fn class_def() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.queries.spans.TestSpanExplanations")
            .extends(&base::class())
            .constructor(Constructor::no_args(|| CaseInstance(Arc::new(ParentCase))))
            .method(case_test("testST1", test_st1))
            .method(case_test("testST2", test_st2))
            .method(case_test("testST4", test_st4))
            .method(case_test("testST5", test_st5))
            .method(case_test("testSF1", test_sf1))
            .method(case_test("testSF2", test_sf2))
            .method(case_test("testSF4", test_sf4))
            .method(case_test("testSF5", test_sf5))
            .method(case_test("testSF6", test_sf6))
            .method(case_test("testSO1", test_so1))
            .method(case_test("testSO2", test_so2))
            .method(case_test("testSO3", test_so3))
            .method(case_test("testSO4", test_so4))
            .method(case_test("testSNear1", test_snear1))
            .method(case_test("testSNear2", test_snear2))
            .method(case_test("testSNear3", test_snear3))
            .method(case_test("testSNear4", test_snear4))
            .method(case_test("testSNear5", test_snear5))
            .method(case_test("testSNear6", test_snear6))
            .method(case_test("testSNear7", test_snear7))
            .method(case_test("testSNear8", test_snear8))
            .method(case_test("testSNear9", test_snear9))
            .method(case_test("testSNear10", test_snear10))
            .method(case_test("testSNear11", test_snear11))
            .method(case_test("testSNot1", test_snot1))
            .method(case_test("testSNot2", test_snot2))
            .method(case_test("testSNot4", test_snot4))
            .method(case_test("testSNot5", test_snot5))
            .method(case_test("testSNot7", test_snot7))
            .method(case_test("testSNot10", test_snot10))
            .method(case_test(
                "testExplainWithoutScoring",
                test_explain_without_scoring,
            ))
            .method(case_test("test1", test1))
            .method(case_test("test2", test2))
            .build()
    });
    Arc::clone(&CLASS)
}

/// `TestSpanExplanationsOfNonMatches extends TestSpanExplanations`.
pub fn non_matches_class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.queries.spans.TestSpanExplanationsOfNonMatches")
        .extends(&class_def())
        .constructor(Constructor::no_args(|| {
            CaseInstance(Arc::new(NonMatchesCase))
        }))
        .build()
}

fn q(s: Arc<dyn SpanQuery>) -> Arc<dyn Query> {
    s
}

fn boost(query: Arc<dyn Query>, b: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(query, b)?))
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(field, text)))
}

// ---------------------------------------------------------------------------
// SpanTermQuery
// ---------------------------------------------------------------------------

fn test_st1(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = st("w1");
    c.qtest(q(sq), &[0, 1, 2, 3])
}

fn test_st2(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = st("w1");
    c.qtest(boost(q(sq), 1000.0)?, &[0, 1, 2, 3])
}

fn test_st4(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = st("xx");
    c.qtest(q(sq), &[2, 3])
}

fn test_st5(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = st("xx");
    c.qtest(boost(q(sq), 1000.0)?, &[2, 3])
}

// ---------------------------------------------------------------------------
// SpanFirstQuery
// ---------------------------------------------------------------------------

fn test_sf1(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sf("w1", 1);
    c.qtest(q(sq), &[0, 1, 2, 3])
}

fn test_sf2(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sf("w1", 1);
    c.qtest(boost(q(sq), 1000.0)?, &[0, 1, 2, 3])
}

fn test_sf4(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sf("xx", 2);
    c.qtest(q(sq), &[2])
}

fn test_sf5(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sf("yy", 2);
    c.qtest(q(sq), &[])
}

fn test_sf6(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sf("yy", 4);
    c.qtest(boost(q(sq), 1000.0)?, &[2])
}

// ---------------------------------------------------------------------------
// SpanOrQuery
// ---------------------------------------------------------------------------

fn test_so1(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sor("w1", "QQ")?;
    c.qtest(q(sq), &[0, 1, 2, 3])
}

fn test_so2(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sor3("w1", "w3", "zz")?;
    c.qtest(q(sq), &[0, 1, 2, 3])
}

fn test_so3(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sor3("w5", "QQ", "yy")?;
    c.qtest(q(sq), &[0, 2, 3])
}

fn test_so4(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = sor3("w5", "QQ", "yy")?;
    c.qtest(q(sq), &[0, 2, 3])
}

// ---------------------------------------------------------------------------
// SpanNearQuery
// ---------------------------------------------------------------------------

fn test_snear1(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear("w1", "QQ", 100, true)?;
    c.qtest(q(sq), &[])
}

fn test_snear2(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear("w1", "xx", 100, true)?;
    c.qtest(q(sq), &[2, 3])
}

fn test_snear3(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear("w1", "xx", 0, true)?;
    c.qtest(q(sq), &[2])
}

fn test_snear4(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear("w1", "xx", 1, true)?;
    c.qtest(q(sq), &[2, 3])
}

fn test_snear5(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear("xx", "w1", 0, false)?;
    c.qtest(q(sq), &[2])
}

fn test_snear6(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear3("w1", "w2", "QQ", 100, true)?;
    c.qtest(q(sq), &[])
}

fn test_snear7(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear3("w1", "xx", "w2", 100, true)?;
    c.qtest(q(sq), &[2, 3])
}

fn test_snear8(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear3("w1", "xx", "w2", 0, true)?;
    c.qtest(q(sq), &[2])
}

fn test_snear9(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear3("w1", "xx", "w2", 1, true)?;
    c.qtest(q(sq), &[2, 3])
}

fn test_snear10(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear3("xx", "w1", "w2", 0, false)?;
    c.qtest(q(sq), &[2])
}

fn test_snear11(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snear3("w1", "w2", "w3", 1, true)?;
    c.qtest(q(sq), &[0, 1])
}

// ---------------------------------------------------------------------------
// SpanNotQuery
// ---------------------------------------------------------------------------

fn test_snot1(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snot(sf("w1", 10), st("QQ"))?;
    c.qtest(q(sq), &[0, 1, 2, 3])
}

fn test_snot2(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snot(sf("w1", 10), st("QQ"))?;
    c.qtest(boost(q(sq), 1000.0)?, &[0, 1, 2, 3])
}

fn test_snot4(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snot(sf("w1", 10), st("xx"))?;
    c.qtest(q(sq), &[0, 1, 2, 3])
}

fn test_snot5(c: &dyn SpanExplanationsCase) -> Result<()> {
    let sq = snot(sf("w1", 10), st("xx"))?;
    c.qtest(boost(q(sq), 1000.0)?, &[0, 1, 2, 3])
}

fn test_snot7(c: &dyn SpanExplanationsCase) -> Result<()> {
    let f = snear("w1", "w3", 10, true)?;
    let sq = snot(f, st("xx"))?;
    c.qtest(q(sq), &[0, 1, 3])
}

fn test_snot10(c: &dyn SpanExplanationsCase) -> Result<()> {
    let t = st("xx");
    let sq = snot(snear("w1", "w3", 10, true)?, t)?;
    c.qtest(q(sq), &[0, 1, 3])
}

fn test_explain_without_scoring(_c: &dyn SpanExplanationsCase) -> Result<()> {
    let query = SpanNearQuery::new(
        vec![
            Arc::new(SpanTermQuery::new(Term::from_text(FIELD_CONTENT, "dolor"))),
            Arc::new(SpanTermQuery::new(Term::from_text(FIELD_CONTENT, "lorem"))),
        ],
        0,
        true,
    )?;

    let analyzer: Arc<dyn Analyzer> = {
        let state = base_explanation_test_case::state();
        Arc::clone(
            &state
                .as_ref()
                .unwrap_or_else(|| {
                    randomizedtesting::jdk::exception::throw_without_message(
                        "java.lang.NullPointerException",
                    )
                })
                .analyzer,
        ) as Arc<dyn Analyzer>
    };
    let rd = LuceneTestCase::new_directory();
    {
        let writer = IndexWriter::new(
            rd.as_directory(),
            LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer)),
        )?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            FIELD_CONTENT,
            "dolor lorem ipsum",
            Store::YES,
        )));
        writer.add_document(&doc)?;
        writer.close()?;
    }
    {
        let reader = directory_reader::open(rd.as_directory())?;
        let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let span_weight =
            query.create_span_weight(&index_searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
        let ctx = first_leaf(&index_searcher);
        let explanation = span_weight.explain(&ctx, 0)?;
        // assertEquals(0f, explanation.getValue()): Float.equals, so a Float
        // of the same bits
        let same = matches!(explanation.value(), ExplanationValue::Float(v) if v.to_bits() == 0f32.to_bits());
        if !same {
            Assert::fail(&format!("expected:<0.0> but was:<{}>", explanation.value()));
        }
        Assert::assert_equals(
            "match spanNear([content:dolor, content:lorem], 0, true) in 0 without score"
                .to_string(),
            explanation.description().to_string(),
        );
        reader.close()?;
    }
    rd.close()
}

fn test1(c: &dyn SpanExplanationsCase) -> Result<()> {
    let mut qb = BooleanQueryBuilder::new();

    let phrase_query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(1, FIELD, &["w1", "w2"])?);
    qb.add(phrase_query, Occur::MUST)?;
    qb.add(
        q(snear_q(st("w2"), sor("w5", "zz")?, 4, true)?),
        Occur::SHOULD,
    )?;
    qb.add(
        q(snear_q3(sf("w3", 2), st("w2"), st("w3"), 5, true)?),
        Occur::SHOULD,
    )?;

    let mut tb = BooleanQueryBuilder::new();
    tb.add(tq(FIELD, "xx"), Occur::MUST)?;
    tb.add(match_these_items(&[1, 3])?, Occur::FILTER)?;
    let t: Arc<dyn Query> = Arc::new(tb.build());
    qb.add(boost(t, 1000.0)?, Occur::SHOULD)?;

    let t: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(match_these_items(&[0, 2])?));
    qb.add(boost(t, 30.0)?, Occur::SHOULD)?;

    let mut disjuncts: Vec<Arc<dyn Query>> = Vec::new();
    disjuncts.push(q(snear_q(st("w2"), sor("w5", "zz")?, 4, true)?));
    disjuncts.push(tq(FIELD, "QQ"));

    let mut xx_yy_zz = BooleanQueryBuilder::new();
    xx_yy_zz.add(tq(FIELD, "xx"), Occur::SHOULD)?;
    xx_yy_zz.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    xx_yy_zz.add(tq(FIELD, "zz"), Occur::MUST_NOT)?;

    disjuncts.push(Arc::new(xx_yy_zz.build()));

    let mut xx_w1 = BooleanQueryBuilder::new();
    xx_w1.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    xx_w1.add(tq(FIELD, "w1"), Occur::MUST_NOT)?;

    disjuncts.push(Arc::new(xx_w1.build()));

    let disjuncts2: Vec<Arc<dyn Query>> = vec![tq(FIELD, "w1"), tq(FIELD, "w2"), tq(FIELD, "w3")];
    disjuncts.push(Arc::new(DisjunctionMaxQuery::new(disjuncts2, 0.5)?));

    qb.add(
        Arc::new(DisjunctionMaxQuery::new(disjuncts, 0.2)?),
        Occur::SHOULD,
    )?;

    let mut b = BooleanQueryBuilder::new();
    b.set_minimum_number_should_match(2);
    b.add(q(snear("w1", "w2", 1, true)?), Occur::SHOULD)?;
    b.add(q(snear("w2", "w3", 1, true)?), Occur::SHOULD)?;
    b.add(q(snear("w1", "w3", 3, true)?), Occur::SHOULD)?;

    qb.add(Arc::new(b.build()), Occur::SHOULD)?;

    c.qtest(Arc::new(qb.build()), &[0, 1, 2])
}

fn test2(c: &dyn SpanExplanationsCase) -> Result<()> {
    let mut qb = BooleanQueryBuilder::new();

    let phrase_query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(1, FIELD, &["w1", "w2"])?);
    qb.add(phrase_query, Occur::MUST)?;
    qb.add(
        q(snear_q(st("w2"), sor("w5", "zz")?, 4, true)?),
        Occur::SHOULD,
    )?;
    qb.add(
        q(snear_q3(sf("w3", 2), st("w2"), st("w3"), 5, true)?),
        Occur::SHOULD,
    )?;

    let mut tb = BooleanQueryBuilder::new();
    tb.add(tq(FIELD, "xx"), Occur::MUST)?;
    tb.add(match_these_items(&[1, 3])?, Occur::FILTER)?;
    let t: Arc<dyn Query> = Arc::new(tb.build());
    qb.add(boost(t, 1000.0)?, Occur::SHOULD)?;

    let t: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(match_these_items(&[0, 2])?));
    qb.add(boost(t, 20.0)?, Occur::SHOULD)?;

    let mut disjuncts: Vec<Arc<dyn Query>> = Vec::new();
    disjuncts.push(q(snear_q(st("w2"), sor("w5", "zz")?, 4, true)?));
    disjuncts.push(tq(FIELD, "QQ"));

    let mut xx_yy_zz = BooleanQueryBuilder::new();
    xx_yy_zz.add(tq(FIELD, "xx"), Occur::SHOULD)?;
    xx_yy_zz.add(tq(FIELD, "yy"), Occur::SHOULD)?;
    xx_yy_zz.add(tq(FIELD, "zz"), Occur::MUST_NOT)?;

    disjuncts.push(Arc::new(xx_yy_zz.build()));

    let mut xx_w1 = BooleanQueryBuilder::new();
    xx_w1.add(tq(FIELD, "xx"), Occur::MUST_NOT)?;
    xx_w1.add(tq(FIELD, "w1"), Occur::MUST_NOT)?;

    disjuncts.push(Arc::new(xx_w1.build()));

    let dm2: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(
        vec![tq(FIELD, "w1"), tq(FIELD, "w2"), tq(FIELD, "w3")],
        0.5,
    )?);
    disjuncts.push(dm2);

    qb.add(
        Arc::new(DisjunctionMaxQuery::new(disjuncts, 0.2)?),
        Occur::SHOULD,
    )?;

    let mut builder = BooleanQueryBuilder::new();
    builder.set_minimum_number_should_match(2);
    builder.add(q(snear("w1", "w2", 1, true)?), Occur::SHOULD)?;
    builder.add(q(snear("w2", "w3", 1, true)?), Occur::SHOULD)?;
    builder.add(q(snear("w1", "w3", 3, true)?), Occur::SHOULD)?;
    let b: Arc<dyn Query> = Arc::new(builder.build());

    qb.add(boost(b, 0.0)?, Occur::SHOULD)?;

    c.qtest(Arc::new(qb.build()), &[0, 1, 2])
}
