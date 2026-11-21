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

//! Port of `TestFunctionScoreQuery.java`.
//!
//! **Test divergence (user decision, rmp #479).** `testBoostsAreAppliedLast`
//! and `testAccessToValueSource` build their score source with
//! `JavascriptCompiler.compile("ln(score + 4)")` bound to
//! `DoubleValuesSource.SCORES` (`TestFunctionScoreQuery.java:213-231,
//! 288-297`). `lucene/expressions` is outside the port; the port uses the
//! test-local [`LnScorePlus4`], a `DoubleValuesSource` that computes
//! `Math.log(score + 4)` exactly and answers `needsScores() == true`. Both
//! methods test `FunctionScoreQuery` (boosts applied last, `getSource`
//! equality), not expressions. Everything else in the class is faithful.

use std::any::Any;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, NumericDocValuesField, Store, TextField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{directory_reader, IndexReader, LeafReaderContext, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::{self, DoubleValuesSource};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::term_query::TermQuery;
use rucene::search::weight::Weight;
use rucene::search::SegmentCacheable;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::function::function_test_setup::{create_index, dir, subclass, INT_FIELD, TEXT_FIELD};
use crate::queries_support::core_support::{random, test};
use crate::queries_support::search_support::static_hook;

/// The static fields `reader` and `searcher`.
struct Statics {
    reader: Arc<dyn IndexReader>,
    searcher: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn statics() -> (Arc<dyn IndexReader>, Arc<IndexSearcher>) {
    let g = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.as_ref().expect("beforeClass ran");
    (Arc::clone(&s.reader), Arc::clone(&s.searcher))
}

fn searcher() -> Arc<IndexSearcher> {
    statics().1
}

/// `TestFunctionScoreQuery`.
pub fn class_def() -> Arc<Class> {
    subclass(
        "org.apache.lucene.queries.function.TestFunctionScoreQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testEqualities", test_equalities),
            test("testSimpleSourceScore", test_simple_source_score),
            test("testScoreModifyingSource", test_score_modifying_source),
            test(
                "testCombiningMultipleQueryScores",
                test_combining_multiple_query_scores,
            ),
            test("testBoostsAreAppliedLast", test_boosts_are_applied_last),
            test("testTruncateNegativeScores", test_truncate_negative_scores),
            test("testNaN", test_nan),
            test("testAccessToValueSource", test_access_to_value_source),
            test("testScoreMode", test_score_mode),
            test("testScoreCalledTwice", test_score_called_twice),
            test("testQueryMatchesCount", test_query_matches_count),
        ],
    )
}

fn before_class() -> Result<()> {
    create_index(true)?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir())? as Arc<dyn IndexReader>;
    let searcher = Arc::new(IndexSearcher::new(Arc::clone(&reader))?);
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics { reader, searcher });
    Ok(())
}

fn after_class() -> Result<()> {
    if let Some(Statics { reader, searcher }) =
        STATICS.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        drop(searcher);
        reader.close()?;
    }
    Ok(())
}

fn tq(text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(TEXT_FIELD, text)))
}

fn fsq(q: Arc<dyn Query>, source: Arc<dyn DoubleValuesSource>) -> FunctionScoreQuery {
    FunctionScoreQuery::new(q, source)
}

fn test_equalities() -> Result<()> {
    let q1 = fsq(tq("a"), double_values_source::constant(1.0));
    let q2 = fsq(tq("b"), double_values_source::constant(1.0));
    let q3 = fsq(tq("b"), double_values_source::constant(2.0));
    let q4 = fsq(tq("b"), double_values_source::constant(2.0));
    QueryUtils::check(&q1);
    QueryUtils::check_unequal(&q1, &q3);
    QueryUtils::check_unequal(&q1, &q2);
    QueryUtils::check_unequal(&q2, &q3);
    QueryUtils::check_equal(&q3, &q4);

    let bq1 = FunctionScoreQuery::boost_by_value(tq("a"), double_values_source::constant(2.0));
    QueryUtils::check(&bq1);
    let bq2 = FunctionScoreQuery::boost_by_value(tq("a"), double_values_source::constant(4.0));
    QueryUtils::check_unequal(&bq1, &bq2);
    let bq3 = FunctionScoreQuery::boost_by_value(tq("b"), double_values_source::constant(4.0));
    QueryUtils::check_unequal(&bq1, &bq3);
    QueryUtils::check_unequal(&bq2, &bq3);
    let bq4 = FunctionScoreQuery::boost_by_value(tq("b"), double_values_source::constant(4.0));
    QueryUtils::check_equal(&bq3, &bq4);

    let qq1 = FunctionScoreQuery::boost_by_query(tq("a"), tq("z"), 0.1);
    QueryUtils::check(&qq1);
    let qq2 = FunctionScoreQuery::boost_by_query(tq("a"), tq("z"), 0.2);
    QueryUtils::check_unequal(&qq1, &qq2);
    let qq3 = FunctionScoreQuery::boost_by_query(tq("b"), tq("z"), 0.1);
    QueryUtils::check_unequal(&qq1, &qq3);
    QueryUtils::check_unequal(&qq2, &qq3);
    let qq4 = FunctionScoreQuery::boost_by_query(tq("a"), tq("zz"), 0.1);
    QueryUtils::check_unequal(&qq1, &qq4);
    QueryUtils::check_unequal(&qq2, &qq4);
    QueryUtils::check_unequal(&qq3, &qq4);
    let qq5 = FunctionScoreQuery::boost_by_query(tq("a"), tq("z"), 0.1);
    QueryUtils::check_equal(&qq1, &qq5);
    Ok(())
}

// FunctionQuery equivalent
fn test_simple_source_score() -> Result<()> {
    let searcher = searcher();
    let q: Arc<dyn Query> = Arc::new(fsq(
        tq("first"),
        double_values_source::from_int_field(INT_FIELD),
    ));
    QueryUtils::check_searcher_wrap(&*random(), &q, Some(&searcher), LuceneTestCase::rarely());
    let expected_docs = [4, 7, 9];
    let docs = searcher.search(q, 4)?;
    Assert::assert_equals(expected_docs.len() as i64, docs.total_hits.value());
    for i in 0..expected_docs.len() {
        Assert::assert_equals(docs.score_docs[i].doc, expected_docs[i]);
    }
    Ok(())
}

// CustomScoreQuery and BoostedQuery equivalent
fn test_score_modifying_source() -> Result<()> {
    let searcher = searcher();
    let mut b = BooleanQueryBuilder::new();
    b.add(tq("first"), Occur::SHOULD)?;
    b.add(tq("text"), Occur::SHOULD)?;
    let bq: Arc<dyn Query> = Arc::new(b.build());
    let plain = searcher.search(Arc::clone(&bq), 1)?;

    let fq: Arc<dyn Query> = Arc::new(FunctionScoreQuery::boost_by_value(
        bq,
        double_values_source::from_int_field("iii"),
    ));

    QueryUtils::check_searcher_wrap(&*random(), &fq, Some(&searcher), LuceneTestCase::rarely());

    let expected_docs = [4, 7, 9, 8, 12];
    let docs = searcher.search(Arc::clone(&fq), 5)?;
    Assert::assert_equals(plain.total_hits.value(), docs.total_hits.value());
    for i in 0..expected_docs.len() {
        Assert::assert_equals(expected_docs[i], docs.score_docs[i].doc);
    }

    let expl = searcher.explain(fq, 4)?;
    Assert::assert_true(expl.to_string().contains("first"));
    Assert::assert_true(expl.to_string().contains("iii"));
    Ok(())
}

// BoostingQuery equivalent
fn test_combining_multiple_query_scores() -> Result<()> {
    let searcher = searcher();
    let q = tq("text");
    let plain = searcher.search(Arc::clone(&q), 1)?;

    let fq: Arc<dyn Query> = Arc::new(FunctionScoreQuery::boost_by_query(
        q,
        tq("rechecking"),
        100.0,
    ));

    QueryUtils::check_searcher_wrap(&*random(), &fq, Some(&searcher), LuceneTestCase::rarely());

    let expected_docs = [6, 1, 0, 2, 8];
    let docs = searcher.search(Arc::clone(&fq), 20)?;
    Assert::assert_equals(plain.total_hits.value(), docs.total_hits.value());
    for i in 0..expected_docs.len() {
        Assert::assert_equals(expected_docs[i], docs.score_docs[i].doc);
    }

    let expl = searcher.explain(fq, 6)?;
    Assert::assert_true(expl.to_string().contains("rechecking"));
    Assert::assert_true(expl.to_string().contains("text"));
    Ok(())
}

/// The score source `JavascriptCompiler.compile("ln(score + 4)")
/// .getDoubleValuesSource(bindings)` with `score` bound to
/// `DoubleValuesSource.SCORES` stands for (test divergence, see the module
/// documentation): `Math.log(score + 4)`, needing scores. Like the
/// `Expression` object Java compiles, it equals only itself.
#[derive(Debug)]
pub struct LnScorePlus4;

/// The values of [`LnScorePlus4`]: the score values, positioned on every
/// document, mapped through `Math.log(score + 4)`.
struct LnScorePlus4Values<'a> {
    scores: Box<dyn DoubleValues + 'a>,
}

impl DoubleValues for LnScorePlus4Values<'_> {
    fn double_value(&mut self) -> Result<f64> {
        Ok((self.scores.double_value()? + 4.0).ln())
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.scores.advance_exact(doc)?;
        Ok(true)
    }
}

impl SegmentCacheable for LnScorePlus4 {
    /// The `score` variable (`DoubleValuesSource.SCORES`) is not cacheable.
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for LnScorePlus4 {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let scores = scores.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });
        Ok(Box::new(LnScorePlus4Values { scores }))
    }

    fn needs_scores(&self) -> bool {
        true
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        std::ptr::addr_eq(self as *const Self, other as *const dyn DoubleValuesSource)
    }

    fn source_hash(&self) -> i32 {
        (self as *const Self as usize) as i32
    }

    fn to_source_string(&self) -> String {
        "expr(ln(score + 4))".to_string()
    }
}

// check boosts with non-distributive score source
fn test_boosts_are_applied_last() -> Result<()> {
    let searcher = searcher();
    let q1: Arc<dyn Query> = Arc::new(fsq(tq("text"), Arc::new(LnScorePlus4)));
    let plain = searcher.search(Arc::clone(&q1), 5)?;

    let boosted: Arc<dyn Query> = Arc::new(BoostQuery::new(q1, 2.0)?);
    let afterboost = searcher.search(boosted, 5)?;
    Assert::assert_equals(plain.total_hits.value(), afterboost.total_hits.value());
    for i in 0..5 {
        Assert::assert_equals(plain.score_docs[i].doc, afterboost.score_docs[i].doc);
        Assert::assert_equals_double_msg(
            None,
            f64::from(plain.score_docs[i].score),
            f64::from(afterboost.score_docs[i].score / 2.0),
            0.0001,
        );
    }
    Ok(())
}

fn test_truncate_negative_scores() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("foo", -2));
    w.add_document(&doc)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let q: Arc<dyn Query> = Arc::new(fsq(
        Arc::new(MatchAllDocsQuery::instance()),
        double_values_source::from_long_field("foo"),
    ));
    QueryUtils::check_searcher(&*random(), &q, Some(&searcher));
    let expl = searcher.explain(q, 0)?;
    Assert::assert_equals_float_msg(None, 0.0, expl.value().double_value() as f32, 0.0);
    Assert::assert_true_msg(
        &expl.to_string(),
        expl.details()[0].description().contains("truncated score"),
    );
    reader.close()?;
    dir.close()
}

fn test_nan() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    doc.add(NumericDocValuesField::new("foo", f64::NAN.to_bits() as i64));
    w.add_document(&doc)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let q: Arc<dyn Query> = Arc::new(fsq(
        Arc::new(MatchAllDocsQuery::instance()),
        double_values_source::from_double_field("foo"),
    ));
    QueryUtils::check_searcher(&*random(), &q, Some(&searcher));
    let expl = searcher.explain(q, 0)?;
    Assert::assert_equals_float_msg(None, 0.0, expl.value().double_value() as f32, 0.0);
    Assert::assert_true_msg(
        &expl.to_string(),
        expl.details()[0]
            .description()
            .contains("NaN is an illegal score"),
    );
    reader.close()?;
    dir.close()
}

// check access to the score source of a functionScoreQuery
fn test_access_to_value_source() -> Result<()> {
    let q1 = fsq(tq("a"), double_values_source::constant(31.0));
    let q2 = FunctionScoreQuery::new(
        Arc::clone(q1.get_wrapped_query()),
        Arc::clone(q1.get_source()),
    );
    QueryUtils::check(&q2);
    QueryUtils::check_equal(&q2, &q1);

    let q3 = fsq(tq("first"), double_values_source::from_int_field(INT_FIELD));
    let q4 = FunctionScoreQuery::new(
        Arc::clone(q3.get_wrapped_query()),
        Arc::clone(q3.get_source()),
    );
    QueryUtils::check_equal(&q3, &q4);

    let q5 = fsq(tq("text"), Arc::new(LnScorePlus4));
    let q6 = FunctionScoreQuery::new(
        Arc::clone(q5.get_wrapped_query()),
        Arc::clone(q5.get_source()),
    );
    QueryUtils::check_equal(&q5, &q6);
    Ok(())
}

fn test_score_mode() -> Result<()> {
    // Value Source doesn't need scores
    assert_inner_score_mode(
        ScoreMode::COMPLETE_NO_SCORES,
        ScoreMode::COMPLETE,
        double_values_source::from_double_field("foo"),
    )?;
    assert_inner_score_mode(
        ScoreMode::COMPLETE_NO_SCORES,
        ScoreMode::COMPLETE_NO_SCORES,
        double_values_source::from_double_field("foo"),
    )?;
    assert_inner_score_mode(
        ScoreMode::COMPLETE_NO_SCORES,
        ScoreMode::TOP_SCORES,
        double_values_source::from_double_field("foo"),
    )?;

    // Value Source needs scores
    assert_inner_score_mode(
        ScoreMode::COMPLETE,
        ScoreMode::COMPLETE,
        double_values_source::scores(),
    )?;
    assert_inner_score_mode(
        ScoreMode::COMPLETE_NO_SCORES,
        ScoreMode::COMPLETE_NO_SCORES,
        double_values_source::scores(),
    )?;
    assert_inner_score_mode(
        ScoreMode::COMPLETE,
        ScoreMode::TOP_SCORES,
        double_values_source::scores(),
    )
}

/// The anonymous `TermQuery` subclass of `assertInnerScoreMode`, whose
/// `createWeight` records the score mode before calling `super`.
#[derive(Debug)]
struct ScoreModeRecordingTermQuery {
    inner: TermQuery,
    score_mode_in_weight: Arc<Mutex<Option<ScoreMode>>>,
}

impl Query for ScoreModeRecordingTermQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(visitor)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        *self
            .score_mode_in_weight
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(score_mode);
        self.inner.create_weight(searcher, score_mode, boost)
    }

    /// `TermQuery.equals`: `sameClassAs(other)` and the same term.
    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<ScoreModeRecordingTermQuery>()
            .is_some_and(|o| self.inner.query_eq(&o.inner))
    }

    fn query_hash(&self) -> i32 {
        self.inner.query_hash()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.function.TestFunctionScoreQuery$1"
    }
}

fn assert_inner_score_mode(
    expected_score_mode: ScoreMode,
    input_score_mode: ScoreMode,
    value_source: Arc<dyn DoubleValuesSource>,
) -> Result<()> {
    let score_mode_in_weight = Arc::new(Mutex::new(None));
    let inner_q: Arc<dyn Query> = Arc::new(ScoreModeRecordingTermQuery {
        inner: TermQuery::new(Term::from_text(TEXT_FIELD, "a")),
        score_mode_in_weight: Arc::clone(&score_mode_in_weight),
    });
    let fq = FunctionScoreQuery::new(inner_q, value_source);
    fq.create_weight(&searcher(), input_score_mode, 1.0)?;
    let actual = *score_mode_in_weight
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    Assert::assert_equals_str(
        None,
        Some(&format!("{expected_score_mode:?}")),
        Some(&actual.map_or_else(|| "null".to_string(), |m| format!("{m:?}"))),
    );
    Ok(())
}

/// The FunctionScoreQuery's Scorer score() is going to be called twice for
/// the same doc.
fn test_score_called_twice() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let conf = LuceneTestCase::new_index_writer_config();
        let index_writer = IndexWriter::new(dir.as_directory(), conf)?;
        let mut doc = Document::new();
        doc.add(TextField::new(
            "ExampleText",
            "periodic function".to_string(),
            Store::NO,
        )?);
        doc.add(TextField::new(
            "ExampleText",
            "plot of the original function".to_string(),
            Store::NO,
        )?);
        index_writer.add_document(&doc)?;
        index_writer.commit()?;
        index_writer.close()?;

        let reader = directory_reader::open(dir.as_directory())?;
        let result = (|| -> Result<()> {
            let q: Arc<dyn Query> =
                Arc::new(TermQuery::new(Term::from_text("ExampleText", "function")));
            let q: Arc<dyn Query> = Arc::new(FunctionScoreQuery::boost_by_query(
                q,
                Arc::new(PhraseQuery::with_slop(
                    1,
                    "ExampleText",
                    &["function", "plot"],
                )?),
                2.0,
            ));
            let q: Arc<dyn Query> = Arc::new(FunctionScoreQuery::boost_by_value(
                q,
                double_values_source::scores(),
            ));
            let s = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            Assert::assert_equals(1i64, s.search(q, 10)?.total_hits.value());
            Ok(())
        })();
        let closed = reader.close();
        result.and(closed)
    })();
    let closed = dir.close();
    result.and(closed)
}

// Weight#count is delegated to the inner weight
fn test_query_matches_count() -> Result<()> {
    let (reader, searcher) = statics();
    let query = tq("first");
    let fq: Arc<dyn Query> = Arc::new(FunctionScoreQuery::boost_by_value(
        query,
        double_values_source::from_int_field("iii"),
    ));
    let search_count = searcher.count(Arc::clone(&fq))?;
    let weight = searcher.create_weight(fq, ScoreMode::COMPLETE, 1.0)?;
    let mut weight_count = 0;
    for leaf_reader_context in reader.leaves()? {
        weight_count += weight.count(&leaf_reader_context)?;
    }
    Assert::assert_equals(search_count, weight_count);
    Ok(())
}
