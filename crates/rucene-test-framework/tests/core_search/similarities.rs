//! Port of the test classes of
//! `lucene/core/src/test/org/apache/lucene/search/similarities/` that
//! declare test methods of their own: `TestAxiomaticSimilarity`,
//! `TestBM25Similarity`, `TestBooleanSimilarity`, `TestClassicSimilarity`,
//! `TestRawTFSimilarity`, `TestSimilarity2` and `TestSimilarityBase`.
//!
//! The four `BaseSimilarityTestCase` subclasses among them are bound here
//! once, with their own methods and the inherited ones; the subclasses
//! without methods of their own stay bound in `base_similarity_suites.rs`.

use std::fmt::Display;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::indexing_chain::FieldInvertState;
use rucene::index::IndexOptions;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::similarities::{
    basic_sim_scorer, log2, new_tfidf_scorer, AfterEffect, AfterEffectB, AfterEffectL,
    AxiomaticF1EXP, AxiomaticF1LOG, AxiomaticF2EXP, AxiomaticF2LOG, AxiomaticF3EXP, AxiomaticF3LOG,
    BM25Similarity, BasicModel, BasicModelG, BasicModelIF, BasicModelIn, BasicModelIne, BasicStats,
    BooleanSimilarity, ClassicSimilarity, CollectionStatistics, DFISimilarity, DFRSimilarity,
    Distribution, DistributionLL, DistributionSPL, Explanation, IBSimilarity, Independence,
    IndependenceChiSquared, IndependenceSaturated, IndependenceStandardized, LMDirichletSimilarity,
    LMJelinekMercerSimilarity, Lambda, LambdaDF, LambdaTTF, NoNormalization, Normalization,
    NormalizationH1, NormalizationH2, NormalizationH3, NormalizationZ, RawTFSimilarity, Similarity,
    SimilarityBase, TermStatistics,
};
use rucene::search::{BoostQuery, DisjunctionMaxQuery, Query};
use rucene::util::{BytesRef, Version};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::similarities::base_similarity_test_case::{
    self, BaseSimilarityTestCase, SimilarityTestInstance,
};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, object_to_string, set_up, tear_down, tq};
use crate::core_support::{assert_equals, assert_equals_msg, class, iae_message, random, test};

const MIN_VALUE: f32 = f32::from_bits(1);
const INT_MAX: f32 = i32::MAX as f32;

// -----------------------------------------------------------------------------
// BaseSimilarityTestCase subclasses
// -----------------------------------------------------------------------------

/// A `BaseSimilarityTestCase` subclass whose `getSimilarity` is `make`.
struct SimilarityBinding {
    make: fn(&dyn Random) -> Result<Arc<dyn Similarity>>,
}

impl BaseSimilarityTestCase for SimilarityBinding {
    fn get_similarity(&self, random: &dyn Random) -> Result<Arc<dyn Similarity>> {
        (self.make)(random)
    }
}

/// A method of a `BaseSimilarityTestCase` subclass: its body reads the
/// subclass's instance fields from the class's fixture static.
fn sim_method(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |_inst: &mut SimilarityTestInstance| {
            if let Err(e) = body() {
                rethrow_java(e)
            }
        },
    )
}

/// `public class name extends BaseSimilarityTestCase` with `methods`.
fn sim_class(
    name: &str,
    make: fn(&dyn Random) -> Result<Arc<dyn Similarity>>,
    methods: Vec<Method>,
) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&base_similarity_test_case::class())
        .constructor(Constructor::no_args(move || {
            SimilarityTestInstance::new(SimilarityBinding { make })
        }));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// The `directory`, `indexReader` and `indexSearcher` fields
/// `TestClassicSimilarity` and `TestRawTFSimilarity` declare.
struct SearcherState {
    directory: TestDirectory,
    index_reader: Arc<dyn DirectoryReader>,
    index_searcher: IndexSearcher,
}

/// Runs `body` over the running instance's searcher state.
fn with_state<T>(
    state: &Mutex<Option<SearcherState>>,
    body: impl FnOnce(&mut SearcherState) -> Result<T>,
) -> Result<T> {
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    body(guard.as_mut().expect("setUp ran"))
}

/// `IOUtils.close(indexReader, directory)`.
fn close_state(state: &Mutex<Option<SearcherState>>) -> Result<()> {
    let taken = state.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(SearcherState {
        mut directory,
        index_reader,
        index_searcher,
    }) = taken
    {
        drop(index_searcher);
        let closed_reader = index_reader.close();
        drop(index_reader);
        let closed_directory = directory.close();
        closed_reader.and(closed_directory)?;
    }
    Ok(())
}

/// `setUp()` with `super.setUp()` first; `tearDown()` with
/// `super.tearDown()` last.
fn hooks(set_up_body: fn() -> Result<()>, tear_down_body: fn() -> Result<()>) -> Vec<Method> {
    vec![
        Method::new("setUp").throws(&["java.lang.Exception"]).body(
            move |_inst: &mut SimilarityTestInstance| {
                LuceneTestCase::super_set_up();
                set_up_body().unwrap_or_else(|e| rethrow_java(e))
            },
        ),
        Method::new("tearDown")
            .throws(&["java.lang.Exception"])
            .body(move |_inst: &mut SimilarityTestInstance| {
                tear_down_body().unwrap_or_else(|e| rethrow_java(e));
                LuceneTestCase::super_tear_down();
            }),
    ]
}

// -----------------------------------------------------------------------------
// TestBM25Similarity
// -----------------------------------------------------------------------------

/// `TestBM25Similarity.getSimilarity`.
fn bm25(random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    // term frequency normalization parameter k1
    let k1 = match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        // upper bounds on individual term's score is 43.262806 * (k1 + 1) * boost
        // we just limit the test to "reasonable" k1 values but don't enforce this anywhere.
        2 => INT_MAX,
        // random value
        _ => INT_MAX * random.next_float(),
    };
    // length normalization parameter b [0 .. 1]
    let b = match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        2 => 1.0,
        // random value
        _ => random.next_float(),
    };
    Ok(Arc::new(BM25Similarity::with_parameters(k1, b)?))
}

/// `TestBM25Similarity`.
pub fn bm25_class_def() -> Arc<Class> {
    sim_class(
        "org.apache.lucene.search.similarities.TestBM25Similarity",
        bm25,
        vec![
            sim_method("testIllegalK1", test_illegal_k1),
            sim_method("testIllegalB", test_illegal_b),
        ],
    )
}

fn test_illegal_k1() -> Result<()> {
    let expected = iae_message(|| BM25Similarity::with_parameters(f32::INFINITY, 0.75));
    Assert::assert_true(expected.contains("illegal k1 value"));
    let expected = iae_message(|| BM25Similarity::with_parameters(-1.0, 0.75));
    Assert::assert_true(expected.contains("illegal k1 value"));
    let expected = iae_message(|| BM25Similarity::with_parameters(f32::NAN, 0.75));
    Assert::assert_true(expected.contains("illegal k1 value"));
    Ok(())
}

fn test_illegal_b() -> Result<()> {
    let expected = iae_message(|| BM25Similarity::with_parameters(1.2, 2.0));
    Assert::assert_true(expected.contains("illegal b value"));
    let expected = iae_message(|| BM25Similarity::with_parameters(1.2, -1.0));
    Assert::assert_true(expected.contains("illegal b value"));
    let expected = iae_message(|| BM25Similarity::with_parameters(1.2, f32::INFINITY));
    Assert::assert_true(expected.contains("illegal b value"));
    let expected = iae_message(|| BM25Similarity::with_parameters(1.2, f32::NAN));
    Assert::assert_true(expected.contains("illegal b value"));
    Ok(())
}

// -----------------------------------------------------------------------------
// TestBooleanSimilarity
// -----------------------------------------------------------------------------

/// `TestBooleanSimilarity.getSimilarity`.
fn boolean_similarity(_random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    Ok(Arc::new(BooleanSimilarity::new()))
}

/// `TestBooleanSimilarity`.
pub fn boolean_class_def() -> Arc<Class> {
    sim_class(
        "org.apache.lucene.search.similarities.TestBooleanSimilarity",
        boolean_similarity,
        vec![
            sim_method(
                "testTermScoreIsEqualToBoost",
                test_term_score_is_equal_to_boost,
            ),
            sim_method(
                "testPhraseScoreIsEqualToBoost",
                test_phrase_score_is_equal_to_boost,
            ),
            sim_method("testSameNormsAsBM25", test_boolean_same_norms_as_bm25),
        ],
    )
}

fn test_term_score_is_equal_to_boost() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("foo", "baz".into(), Store::NO)?));
    w.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
    w.add_document(&doc)?;

    let reader = w.get_reader()?;
    w.close()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(BooleanSimilarity::new()));
    let top_docs = searcher.search(tq("foo", "bar"), 2)?;
    assert_equals(&2, &top_docs.total_hits.value());
    Assert::assert_equals_float_msg(None, 1.0, top_docs.score_docs[0].score, 0.0);
    Assert::assert_equals_float_msg(None, 1.0, top_docs.score_docs[1].score, 0.0);

    let top_docs = searcher.search(tq("foo", "baz"), 1)?;
    assert_equals(&1, &top_docs.total_hits.value());
    Assert::assert_equals_float_msg(None, 1.0, top_docs.score_docs[0].score, 0.0);

    let top_docs = searcher.search(Arc::new(BoostQuery::new(tq("foo", "baz"), 3.0)?), 1)?;
    assert_equals(&1, &top_docs.total_hits.value());
    Assert::assert_equals_float_msg(None, 3.0, top_docs.score_docs[0].score, 0.0);

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_phrase_score_is_equal_to_boost() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut config = LuceneTestCase::new_index_writer_config();
    config.set_similarity(Some(Arc::new(BooleanSimilarity::new())))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "foo",
        "bar baz quux".into(),
        Store::NO,
    )?));
    w.add_document(&doc)?;

    let reader = w.get_reader()?;
    w.close()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(BooleanSimilarity::new()));

    let query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(2, "foo", &["bar", "quux"])?);
    let top_docs = searcher.search(Arc::clone(&query), 2)?;
    assert_equals(&1, &top_docs.total_hits.value());
    Assert::assert_equals_float_msg(None, 1.0, top_docs.score_docs[0].score, 0.0);

    let top_docs = searcher.search(Arc::new(BoostQuery::new(query, 7.0)?), 2)?;
    assert_equals(&1, &top_docs.total_hits.value());
    Assert::assert_equals_float_msg(None, 7.0, top_docs.score_docs[0].score, 0.0);

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `new FieldInvertState(Version.LATEST.major, "foo",
/// IndexOptions.DOCS_AND_FREQS, position, length, numOverlaps, 100,
/// maxTermFrequency, uniqueTermCount)`.
fn spoof_state(length: i32, position: i32, num_overlaps: i32) -> FieldInvertState {
    let max_term_frequency = 1;
    let unique_term_count = 1;
    FieldInvertState::with_statistics(
        Version::LATEST.major as i32,
        "foo".to_string(),
        IndexOptions::DOCS_AND_FREQS,
        position,
        length,
        num_overlaps,
        100,
        max_term_frequency,
        unique_term_count,
    )
}

fn test_boolean_same_norms_as_bm25() -> Result<()> {
    let sim1 = BooleanSimilarity::new();
    let sim2 = BM25Similarity::new();
    for _iter in 0..100 {
        let length = TestUtil::next_int(&*random(), 1, 100);
        let position = random().next_int_bound(length);
        let num_overlaps = random().next_int_bound(length);
        let state = spoof_state(length, position, num_overlaps);
        assert_equals(&sim2.compute_norm(&state)?, &sim1.compute_norm(&state)?);
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// TestClassicSimilarity
// -----------------------------------------------------------------------------

static CLASSIC_STATE: Mutex<Option<SearcherState>> = Mutex::new(None);

/// `TestClassicSimilarity.getSimilarity`.
fn classic_similarity(_random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    Ok(Arc::new(ClassicSimilarity::new()))
}

/// `TestClassicSimilarity`.
pub fn classic_class_def() -> Arc<Class> {
    let mut methods = hooks(classic_set_up, || close_state(&CLASSIC_STATE));
    methods.extend([
        sim_method("testHit", test_hit),
        sim_method("testMiss", test_miss),
        sim_method("testEmpty", test_empty),
        sim_method("testBQHit", test_bq_hit),
        sim_method("testBQHitOrMiss", test_bq_hit_or_miss),
        sim_method("testBQHitOrEmpty", test_bq_hit_or_empty),
        sim_method("testDMQHit", test_dmq_hit),
        sim_method("testDMQHitOrMiss", test_dmq_hit_or_miss),
        sim_method("testDMQHitOrEmpty", test_dmq_hit_or_empty),
        sim_method("testSaneNormValues", test_sane_norm_values),
        sim_method("testSameNormsAsBM25", test_classic_same_norms_as_bm25),
    ]);
    sim_class(
        "org.apache.lucene.search.similarities.TestClassicSimilarity",
        classic_similarity,
        methods,
    )
}

/// `TestClassicSimilarity.setUp()` after `super.setUp()`.
fn classic_set_up() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    {
        let index_writer = IndexWriter::new(
            directory.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let result = (|| -> Result<()> {
            let mut document = Document::new();
            document.add(Box::new(StringField::new("test", "hit".into(), Store::NO)?));
            index_writer.add_document(&document)?;
            index_writer.commit()?;
            Ok(())
        })();
        result.and(index_writer.close())?;
    }
    let index_reader = directory_reader::open(directory.as_directory())?;
    let mut index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
    index_searcher.set_similarity(Arc::new(ClassicSimilarity::new()));
    *CLASSIC_STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(SearcherState {
        directory,
        index_reader,
        index_searcher,
    });
    Ok(())
}

/// The assertions every `testXXHit*` method makes on its one hit.
fn assert_one_scored_hit(query: Arc<dyn Query>) -> Result<()> {
    with_state(&CLASSIC_STATE, |s| {
        let top_docs = s.index_searcher.search(query, 1)?;
        assert_equals(&1, &top_docs.total_hits.value());
        assert_equals(&1, &top_docs.score_docs.len());
        Assert::assert_true(top_docs.score_docs[0].score != 0.0);
        Ok(())
    })
}

fn test_hit() -> Result<()> {
    assert_one_scored_hit(tq("test", "hit"))
}

fn test_miss() -> Result<()> {
    with_state(&CLASSIC_STATE, |s| {
        let top_docs = s.index_searcher.search(tq("test", "miss"), 1)?;
        assert_equals(&0, &top_docs.total_hits.value());
        Ok(())
    })
}

fn test_empty() -> Result<()> {
    with_state(&CLASSIC_STATE, |s| {
        let top_docs = s.index_searcher.search(tq("empty", "miss"), 1)?;
        assert_equals(&0, &top_docs.total_hits.value());
        Ok(())
    })
}

/// `new BooleanQuery.Builder().add(c, SHOULD)….build()`.
fn should_bq(clauses: Vec<Arc<dyn Query>>) -> Result<Arc<dyn Query>> {
    let mut builder = BooleanQueryBuilder::new();
    for c in clauses {
        builder.add(c, Occur::SHOULD)?;
    }
    Ok(Arc::new(builder.build()))
}

fn test_bq_hit() -> Result<()> {
    assert_one_scored_hit(should_bq(vec![tq("test", "hit")])?)
}

fn test_bq_hit_or_miss() -> Result<()> {
    assert_one_scored_hit(should_bq(vec![tq("test", "hit"), tq("test", "miss")])?)
}

fn test_bq_hit_or_empty() -> Result<()> {
    assert_one_scored_hit(should_bq(vec![tq("test", "hit"), tq("empty", "miss")])?)
}

fn test_dmq_hit() -> Result<()> {
    assert_one_scored_hit(Arc::new(DisjunctionMaxQuery::new(
        vec![tq("test", "hit")],
        0.0,
    )?))
}

fn test_dmq_hit_or_miss() -> Result<()> {
    assert_one_scored_hit(Arc::new(DisjunctionMaxQuery::new(
        vec![tq("test", "hit"), tq("test", "miss")],
        0.0,
    )?))
}

fn test_dmq_hit_or_empty() -> Result<()> {
    assert_one_scored_hit(Arc::new(DisjunctionMaxQuery::new(
        vec![tq("test", "hit"), tq("empty", "miss")],
        0.0,
    )?))
}

fn test_sane_norm_values() -> Result<()> {
    let sim = ClassicSimilarity::new();
    let collection_stats = with_state(&CLASSIC_STATE, |s| {
        s.index_searcher.collection_statistics("test")
    })?;
    // Java passes `collectionStatistics("test")` as is; the field holds one
    // document, so it is never null here.
    let collection_stats = collection_stats.expect("field test is indexed");
    // `(TFIDFScorer) sim.scorer(1f, collectionStats)`: the concrete scorer,
    // which a non-'static `dyn SimScorer` cannot be cast back to.
    let stats = new_tfidf_scorer(&sim, 1.0, &collection_stats, &[]);
    let norm_table = stats.norm_table();
    for i in 0..256 {
        let boost = norm_table[i];
        Assert::assert_false_msg(
            &format!("negative boost: {}, byte={i}", java_float(boost)),
            boost < 0.0,
        );
        Assert::assert_false_msg(
            &format!("inf bost: {}, byte={i}", java_float(boost)),
            boost.is_infinite(),
        );
        Assert::assert_false_msg(&format!("nan boost for byte={i}"), boost.is_nan());
        if i > 0 {
            Assert::assert_true_msg(
                &format!("boost is not decreasing: {},byte={i}", java_float(boost)),
                boost < norm_table[i - 1],
            );
        }
    }
    Ok(())
}

fn test_classic_same_norms_as_bm25() -> Result<()> {
    let sim1 = ClassicSimilarity::new();
    let sim2 = BM25Similarity::new();
    for _iter in 0..100 {
        let length = TestUtil::next_int(&*random(), 1, 1000);
        let position = random().next_int_bound(length);
        let num_overlaps = random().next_int_bound(length);
        let state = spoof_state(length, position, num_overlaps);
        assert_equals(&sim2.compute_norm(&state)?, &sim1.compute_norm(&state)?);
    }
    Ok(())
}

/// `String.valueOf(float)`.
fn java_float(f: f32) -> String {
    rucene::util::java_fmt::float_to_string(f)
}

// -----------------------------------------------------------------------------
// TestRawTFSimilarity
// -----------------------------------------------------------------------------

static RAW_TF_STATE: Mutex<Option<SearcherState>> = Mutex::new(None);

/// `TestRawTFSimilarity.getSimilarity`.
fn raw_tf(_random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    Ok(Arc::new(RawTFSimilarity::new()))
}

/// `TestRawTFSimilarity`.
pub fn raw_tf_class_def() -> Arc<Class> {
    let mut methods = hooks(raw_tf_set_up, || close_state(&RAW_TF_STATE));
    methods.extend([
        sim_method("testOne", || impl_test("one", 1.0)),
        sim_method("testTwo", || impl_test("two", 2.0)),
        sim_method("testThree", || impl_test("three", 3.0)),
        sim_method("testBoostQuery", test_boost_query),
    ]);
    sim_class(
        "org.apache.lucene.search.similarities.TestRawTFSimilarity",
        raw_tf,
        methods,
    )
}

/// `TestRawTFSimilarity.setUp()` after `super.setUp()`.
fn raw_tf_set_up() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    {
        let index_writer = IndexWriter::new(
            directory.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let result = (|| -> Result<()> {
            let mut document1 = Document::new();
            let mut document2 = Document::new();
            let mut document3 = Document::new();
            document1.add(Box::new(LuceneTestCase::new_text_field(
                "test",
                "one",
                Store::YES,
            )));
            document2.add(Box::new(LuceneTestCase::new_text_field(
                "test",
                "two two",
                Store::YES,
            )));
            document3.add(Box::new(LuceneTestCase::new_text_field(
                "test",
                "three three three",
                Store::YES,
            )));
            index_writer.add_document(&document1)?;
            index_writer.add_document(&document2)?;
            index_writer.add_document(&document3)?;
            index_writer.commit()?;
            Ok(())
        })();
        result.and(index_writer.close())?;
    }
    let index_reader = directory_reader::open(directory.as_directory())?;
    let mut index_searcher = LuceneTestCase::new_searcher(Arc::clone(&index_reader));
    index_searcher.set_similarity(Arc::new(RawTFSimilarity::new()));
    *RAW_TF_STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(SearcherState {
        directory,
        index_reader,
        index_searcher,
    });
    Ok(())
}

/// `implTest(String text, float expectedScore)`.
fn impl_test(text: &str, expected_score: f32) -> Result<()> {
    with_state(&RAW_TF_STATE, |s| {
        let top_docs = s.index_searcher.search(tq("test", text), 1)?;
        assert_equals(&1, &top_docs.total_hits.value());
        assert_equals(&1, &top_docs.score_docs.len());
        Assert::assert_equals_double_msg(
            None,
            f64::from(expected_score),
            f64::from(top_docs.score_docs[0].score),
            0.0,
        );
        Ok(())
    })
}

fn test_boost_query() -> Result<()> {
    with_state(&RAW_TF_STATE, |s| {
        let query = tq("test", "three");
        let boost = 14.0;
        let top_docs = s
            .index_searcher
            .search(Arc::new(BoostQuery::new(query, boost)?), 1)?;
        assert_equals(&1, &top_docs.total_hits.value());
        assert_equals(&1, &top_docs.score_docs.len());
        Assert::assert_equals_double_msg(None, 42.0, f64::from(top_docs.score_docs[0].score), 0.0);
        Ok(())
    })
}

// -----------------------------------------------------------------------------
// TestAxiomaticSimilarity
// -----------------------------------------------------------------------------

/// `TestAxiomaticSimilarity`.
pub fn axiomatic_class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.similarities.TestAxiomaticSimilarity",
        vec![
            test("testIllegalS", test_illegal_s),
            test("testIllegalK", test_illegal_k),
            test("testIllegalQL", test_illegal_ql),
        ],
    )
}

fn test_illegal_s() -> Result<()> {
    let expected = iae_message(|| AxiomaticF2EXP::with_s_and_k(f32::INFINITY, 0.1));
    Assert::assert_true(expected.contains("illegal s value"));
    let expected = iae_message(|| AxiomaticF2EXP::with_s_and_k(-1.0, 0.1));
    Assert::assert_true(expected.contains("illegal s value"));
    let expected = iae_message(|| AxiomaticF2EXP::with_s_and_k(f32::NAN, 0.1));
    Assert::assert_true(expected.contains("illegal s value"));
    Ok(())
}

fn test_illegal_k() -> Result<()> {
    let expected = iae_message(|| AxiomaticF2EXP::with_s_and_k(0.35, 2.0));
    Assert::assert_true(expected.contains("illegal k value"));
    let expected = iae_message(|| AxiomaticF2EXP::with_s_and_k(0.35, -1.0));
    Assert::assert_true(expected.contains("illegal k value"));
    let expected = iae_message(|| AxiomaticF2EXP::with_s_and_k(0.35, f32::INFINITY));
    Assert::assert_true(expected.contains("illegal k value"));
    let expected = iae_message(|| AxiomaticF2EXP::with_s_and_k(0.35, f32::NAN));
    Assert::assert_true(expected.contains("illegal k value"));
    Ok(())
}

fn test_illegal_ql() -> Result<()> {
    let expected = iae_message(|| AxiomaticF3EXP::with_s_and_query_len(0.35, -1));
    Assert::assert_true(expected.contains("illegal query length value"));
    // `Integer.MAX_VALUE + 1` wraps to `Integer.MIN_VALUE`, widened to float.
    let expected =
        iae_message(|| AxiomaticF2EXP::with_s_and_k(0.35, i32::MAX.wrapping_add(1) as f32));
    Assert::assert_true(expected.contains("illegal k value"));
    Ok(())
}

// -----------------------------------------------------------------------------
// TestSimilarity2
// -----------------------------------------------------------------------------

/// The instance of `TestSimilarity2`: `List<Similarity> sims`, each with
/// its `toString()`.
#[derive(Default)]
pub struct Similarity2 {
    sims: Vec<(Arc<dyn Similarity>, String)>,
}

/// Adds a similarity whose class overrides `toString()`.
fn push_sim<S: Similarity + Display + 'static>(
    sims: &mut Vec<(Arc<dyn Similarity>, String)>,
    s: S,
) {
    let name = s.to_string();
    sims.push((Arc::new(s), name));
}

/// `BASIC_MODELS`.
fn basic_models() -> Vec<fn() -> Box<dyn BasicModel>> {
    vec![
        || Box::new(BasicModelG::new()),
        || Box::new(BasicModelIF::new()),
        || Box::new(BasicModelIn::new()),
        || Box::new(BasicModelIne::new()),
    ]
}

/// `AFTER_EFFECTS`.
fn after_effects() -> Vec<fn() -> Box<dyn AfterEffect>> {
    vec![|| Box::new(AfterEffectB::new()), || {
        Box::new(AfterEffectL::new())
    }]
}

/// `NORMALIZATIONS`.
fn normalizations() -> Vec<fn() -> Box<dyn Normalization>> {
    vec![
        || Box::new(NormalizationH1::default()),
        || Box::new(NormalizationH2::default()),
        || Box::new(NormalizationH3::default()),
        || Box::new(NormalizationZ::default()),
        || Box::new(NoNormalization::new()),
    ]
}

/// `DISTRIBUTIONS`.
fn distributions() -> Vec<fn() -> Box<dyn Distribution>> {
    vec![|| Box::new(DistributionLL::new()), || {
        Box::new(DistributionSPL::new())
    }]
}

/// `LAMBDAS`.
fn lambdas() -> Vec<fn() -> Box<dyn Lambda>> {
    vec![|| Box::new(LambdaDF::new()), || Box::new(LambdaTTF::new())]
}

/// `INDEPENDENCE_MEASURES`.
fn independence_measures() -> Vec<fn() -> Box<dyn Independence>> {
    vec![
        || Box::new(IndependenceStandardized::new()),
        || Box::new(IndependenceSaturated::new()),
        || Box::new(IndependenceChiSquared::new()),
    ]
}

/// `TestSimilarity2`.
pub fn similarity2_class_def() -> Arc<Class> {
    fixture_class::<Similarity2>(
        "org.apache.lucene.search.similarities.TestSimilarity2",
        vec![
            set_up(similarity2_set_up),
            ftest("testEmptyIndex", test_empty_index),
            ftest("testEmptyField", test_empty_field),
            ftest("testEmptyTerm", test_empty_term),
            ftest("testNoNorms", test_no_norms),
            ftest("testNoFieldSkew", test_no_field_skew),
            ftest("testOmitTF", test_omit_tf),
            ftest("testOmitTFAndNorms", test_omit_tf_and_norms),
        ],
    )
}

fn similarity2_set_up(f: &mut Similarity2) -> Result<()> {
    let mut sims = Vec::new();
    push_sim(&mut sims, ClassicSimilarity::new());
    push_sim(&mut sims, BM25Similarity::new());
    let boolean = BooleanSimilarity::new();
    let name = object_to_string(
        "org.apache.lucene.search.similarities.BooleanSimilarity",
        &boolean,
    );
    sims.push((Arc::new(boolean), name));
    push_sim(&mut sims, AxiomaticF1EXP::new());
    push_sim(&mut sims, AxiomaticF1LOG::new());
    push_sim(&mut sims, AxiomaticF2EXP::new());
    push_sim(&mut sims, AxiomaticF2LOG::new());
    push_sim(&mut sims, AxiomaticF3EXP::with_s_and_query_len(0.25, 3)?);
    push_sim(&mut sims, AxiomaticF3LOG::with_s_and_query_len(0.25, 3)?);
    for basic_model in basic_models() {
        for after_effect in after_effects() {
            for normalization in normalizations() {
                push_sim(
                    &mut sims,
                    DFRSimilarity::new(basic_model(), after_effect(), normalization()),
                );
            }
        }
    }
    for distribution in distributions() {
        for lambda in lambdas() {
            for normalization in normalizations() {
                push_sim(
                    &mut sims,
                    IBSimilarity::new(distribution(), lambda(), normalization()),
                );
            }
        }
    }
    push_sim(&mut sims, LMDirichletSimilarity::new());
    push_sim(&mut sims, LMJelinekMercerSimilarity::with_lambda(0.1)?);
    push_sim(&mut sims, LMJelinekMercerSimilarity::with_lambda(0.7)?);
    for independence in independence_measures() {
        push_sim(&mut sims, DFISimilarity::new(independence()));
    }
    f.sims = sims;
    Ok(())
}

/// because of stupid things like querynorm, it's possible we computeStats on
/// a field that doesnt exist at all test this against a totally empty index,
/// to make sure sims handle it
fn test_empty_index(f: &mut Similarity2) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let ir = iw.get_reader()?;
    iw.close()?;
    let mut is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    for (sim, _) in &f.sims {
        is.set_similarity(Arc::clone(sim));
        assert_equals(&0, &is.search(tq("foo", "bar"), 10)?.total_hits.value());
    }
    drop(is);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// Indexes one document holding `field`, and searches each similarity with
/// the query `make` builds, expecting one hit.
fn one_doc_one_hit(
    f: &Similarity2,
    field: rucene::document::Field,
    make: impl Fn() -> Result<Arc<dyn Query>>,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(field));
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;
    let mut is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    for (sim, _) in &f.sims {
        is.set_similarity(Arc::clone(sim));
        assert_equals(&1, &is.search(make()?, 10)?.total_hits.value());
    }
    drop(is);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// similar to the above, but ORs the query with a real field
fn test_empty_field(f: &mut Similarity2) -> Result<()> {
    let field = LuceneTestCase::new_text_field("foo", "bar", Store::NO);
    one_doc_one_hit(f, field.into(), || {
        should_bq(vec![tq("foo", "bar"), tq("bar", "baz")])
    })
}

/// similar to the above, however the field exists, but we query with a term
/// that doesnt exist too
fn test_empty_term(f: &mut Similarity2) -> Result<()> {
    let field = LuceneTestCase::new_text_field("foo", "bar", Store::NO);
    one_doc_one_hit(f, field.into(), || {
        should_bq(vec![tq("foo", "bar"), tq("foo", "baz")])
    })
}

/// `new FieldType(TextField.TYPE_NOT_STORED)` with `edit` applied, frozen.
fn text_type(edit: impl FnOnce(&mut FieldType) -> Result<()>) -> Result<FieldType> {
    let mut ft = FieldType::new_from(TextField::type_not_stored());
    edit(&mut ft)?;
    ft.freeze();
    Ok(ft)
}

/// make sure we can retrieve when norms are disabled
fn test_no_norms(f: &mut Similarity2) -> Result<()> {
    let ft = text_type(|ft| ft.set_omit_norms(true))?;
    let field = LuceneTestCase::new_field("foo", "bar", &ft);
    one_doc_one_hit(f, field, || should_bq(vec![tq("foo", "bar")]))
}

/// make sure scores are not skewed by docs not containing the field
fn test_no_field_skew(f: &mut Similarity2) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iw_config = LuceneTestCase::new_index_writer_config();
    iw_config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_config)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "foo",
        "bar baz somethingelse",
        Store::NO,
    )));
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    let mut is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    let query = should_bq(vec![tq("foo", "bar"), tq("foo", "baz")])?;

    let mut scores: Vec<Explanation> = Vec::new();
    for (sim, _) in &f.sims {
        is.set_similarity(Arc::clone(sim));
        scores.push(is.explain(Arc::clone(&query), 0)?);
    }
    drop(is);
    ir.close()?;

    let num_extra_docs = TestUtil::next_int(&*random(), 1, 1000);
    for _ in 0..num_extra_docs {
        iw.add_document(&Document::new())?;
    }

    let ir = iw.get_reader()?;
    let mut is = LuceneTestCase::new_searcher(Arc::clone(&ir));
    for (i, (sim, name)) in f.sims.iter().enumerate() {
        is.set_similarity(Arc::clone(sim));
        let expected = &scores[i];
        let actual = is.explain(Arc::clone(&query), 0)?;
        assert_equals_msg(
            Some(&format!("{name}: actual={actual},expected={expected}")),
            &expected.value(),
            &actual.value(),
        );
    }

    iw.close()?;
    drop(is);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// make sure all sims work if TF is omitted
fn test_omit_tf(f: &mut Similarity2) -> Result<()> {
    let ft = text_type(|ft| ft.set_index_options(IndexOptions::DOCS))?;
    let field = LuceneTestCase::new_field("foo", "bar", &ft);
    one_doc_one_hit(f, field, || should_bq(vec![tq("foo", "bar")]))
}

/// make sure all sims work if TF and norms is omitted
fn test_omit_tf_and_norms(f: &mut Similarity2) -> Result<()> {
    let ft = text_type(|ft| {
        ft.set_index_options(IndexOptions::DOCS)?;
        ft.set_omit_norms(true)
    })?;
    let field = LuceneTestCase::new_field("foo", "bar", &ft);
    one_doc_one_hit(f, field, || should_bq(vec![tq("foo", "bar")]))
}

// -----------------------------------------------------------------------------
// TestSimilarityBase
// -----------------------------------------------------------------------------

const FIELD_BODY: &str = "body";
const FIELD_ID: &str = "id";
/// The tolerance range for float equality.
const FLOAT_EPSILON: f32 = 1e-5;

/// The default number of documents in the unit tests.
const NUMBER_OF_DOCUMENTS: i32 = 100;
/// The default total number of tokens in the field in the unit tests.
const NUMBER_OF_FIELD_TOKENS: i64 = 5000;
/// The default average field length in the unit tests.
const AVG_FIELD_LENGTH: f32 = 50.0;
/// The default document frequency in the unit tests.
const DOC_FREQ: i32 = 10;
/// The default total number of occurrences of this term across all
/// documents in the unit tests.
const TOTAL_TERM_FREQ: i64 = 70;
/// The default tf in the unit tests.
const FREQ: f32 = 7.0;
/// The default document length in the unit tests.
const DOC_LEN: i32 = 40;

/// A `SimilarityBase` of `List<SimilarityBase> sims`.
///
/// **Divergence from Lucene 10.5.0.** Java holds the similarities as
/// `SimilarityBase` references; Rucene's `SimilarityBase` has an associated
/// statistics type and is not object safe, so each entry is this adapter
/// over its concrete type, offering what the test calls on it.
trait SimBase: Send + Sync {
    /// `toString()`.
    fn name(&self) -> String;
    /// The similarity, for `searcher.setSimilarity`.
    fn similarity(&self) -> Arc<dyn Similarity>;
    /// `((BasicSimScorer) sim.scorer(boost, collectionStats, termStats))
    /// .stats`, then `(float) sim.score(realStats, freq, docLen)` and
    /// `sim.explain(realStats, Explanation.match(freq, "freq"), docLen)
    /// .getValue().floatValue()`.
    fn score_and_explain(
        &self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
        freq: f32,
        doc_len: i32,
    ) -> (f32, f32);
}

struct SimBaseOf<S>(Arc<S>);

impl<S: SimilarityBase + Display + Send + Sync + 'static> SimBase for SimBaseOf<S> {
    fn name(&self) -> String {
        self.0.to_string()
    }

    fn similarity(&self) -> Arc<dyn Similarity> {
        Arc::clone(&self.0) as Arc<dyn Similarity>
    }

    fn score_and_explain(
        &self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
        freq: f32,
        doc_len: i32,
    ) -> (f32, f32) {
        let scorer = basic_sim_scorer(&*self.0, boost, collection_stats, term_stats);
        let real_stats = scorer.stats();
        let score = self
            .0
            .score(real_stats, f64::from(freq), f64::from(doc_len)) as f32;
        let expl_score = self
            .0
            .explain(
                real_stats,
                &Explanation::matched(freq, "freq", vec![]),
                f64::from(doc_len),
            )
            .value()
            .float_value();
        (score, expl_score)
    }
}

fn sim_base<S: SimilarityBase + Display + Send + Sync + 'static>(s: S) -> Box<dyn SimBase> {
    Box::new(SimBaseOf(Arc::new(s)))
}

/// The instance of `TestSimilarityBase`.
#[derive(Default)]
pub struct SimilarityBaseTest {
    searcher: Option<IndexSearcher>,
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    /// The list of similarities to test.
    sims: Vec<Box<dyn SimBase>>,
}

/// The "collection" for the integration tests.
const DOCS: [&str; 8] = [
    "Tiger, tiger burning bright   In the forest of the night   What immortal hand or eye   Could frame thy fearful symmetry ?",
    "In what distant depths or skies   Burnt the fire of thine eyes ?   On what wings dare he aspire ?   What the hands the seize the fire ?",
    "And what shoulder and what art   Could twist the sinews of thy heart ?   And when thy heart began to beat What dread hand ? And what dread feet ?",
    "What the hammer? What the chain ?   In what furnace was thy brain ?   What the anvil ? And what dread grasp   Dare its deadly terrors clasp ?",
    "And when the stars threw down their spears   And water'd heaven with their tear   Did he smile his work to see ?   Did he, who made the lamb, made thee ?",
    "Tiger, tiger burning bright   In the forest of the night   What immortal hand or eye   Dare frame thy fearful symmetry ?",
    "Cruelty has a human heart   And jealousy a human face   Terror the human form divine   And Secrecy the human dress .",
    "The human dress is forg'd iron   The human form a fiery forge   The human face a furnace seal'd   The human heart its fiery gorge .",
];

/// `TestSimilarityBase`.
pub fn similarity_base_class_def() -> Arc<Class> {
    fixture_class::<SimilarityBaseTest>(
        "org.apache.lucene.search.similarities.TestSimilarityBase",
        vec![
            set_up(similarity_base_set_up),
            tear_down(similarity_base_tear_down),
            ftest("testDefault", test_default),
            ftest("testSparseDocuments", test_sparse_documents),
            ftest("testVerySparseDocuments", test_very_sparse_documents),
            ftest("testOneDocument", test_one_document),
            ftest("testAllDocumentsRelevant", test_all_documents_relevant),
            ftest("testMostDocumentsRelevant", test_most_documents_relevant),
            ftest(
                "testOnlyOneRelevantDocument",
                test_only_one_relevant_document,
            ),
            ftest("testAllTermsRelevant", test_all_terms_relevant),
            ftest("testMoreTermsThanDocuments", test_more_terms_than_documents),
            ftest(
                "testNumberOfTermsAsDocuments",
                test_number_of_terms_as_documents,
            ),
            ftest("testOneTerm", test_one_term),
            ftest("testOneRelevantDocument", test_one_relevant_document),
            ftest(
                "testAllTermsRelevantOnlyOneDocument",
                test_all_terms_relevant_only_one_document,
            ),
            ftest(
                "testOnlyOneTermOneDocument",
                test_only_one_term_one_document,
            ),
            ftest("testOnlyOneTerm", test_only_one_term),
            ftest("testDocumentLengthAverage", test_document_length_average),
            ftest("testLMDirichlet", test_lm_dirichlet),
            ftest("testLMJelinekMercer", test_lm_jelinek_mercer),
            ftest("testLLForIB", test_ll_for_ib),
            ftest("testSPLForIB", test_spl_for_ib),
            ftest("testIneB2", test_ine_b2),
            ftest("testGL1", test_gl1),
            ftest("testIn2", test_in2),
            ftest("testIFB", test_ifb),
            ftest("testHeartList", test_heart_list),
            ftest("testHeartRanking", test_heart_ranking),
            ftest("testDiscountOverlapsBoost", test_discount_overlaps_boost),
        ],
    )
}

fn similarity_base_set_up(f: &mut SimilarityBaseTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for (i, text) in DOCS.iter().enumerate() {
        let mut d = Document::new();
        let mut ft = FieldType::new_from(TextField::type_stored());
        ft.set_index_options(IndexOptions::NONE)?;
        d.add(Box::new(LuceneTestCase::new_field(
            FIELD_ID,
            &i.to_string(),
            &ft,
        )));
        d.add(Box::new(LuceneTestCase::new_text_field(
            FIELD_BODY,
            text,
            Store::YES,
        )));
        writer.add_document(&d)?;
    }
    let reader = writer.get_reader()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.dir = Some(dir);
    writer.close()?;

    let mut sims: Vec<Box<dyn SimBase>> = Vec::new();
    for basic_model in basic_models() {
        for after_effect in after_effects() {
            for normalization in normalizations() {
                sims.push(sim_base(DFRSimilarity::new(
                    basic_model(),
                    after_effect(),
                    normalization(),
                )));
            }
        }
    }
    for distribution in distributions() {
        for lambda in lambdas() {
            for normalization in normalizations() {
                sims.push(sim_base(IBSimilarity::new(
                    distribution(),
                    lambda(),
                    normalization(),
                )));
            }
        }
    }
    sims.push(sim_base(LMDirichletSimilarity::new()));
    sims.push(sim_base(LMJelinekMercerSimilarity::with_lambda(0.1)?));
    sims.push(sim_base(LMJelinekMercerSimilarity::with_lambda(0.7)?));
    for independence in independence_measures() {
        sims.push(sim_base(DFISimilarity::new(independence())));
    }
    f.sims = sims;
    Ok(())
}

fn similarity_base_tear_down(f: &mut SimilarityBaseTest) -> Result<()> {
    drop(f.searcher.take());
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// Creates the default statistics object that the specific tests modify.
fn create_stats() -> BasicStats {
    let mut stats = BasicStats::new("spoof", 1.0);
    stats.set_number_of_documents(i64::from(NUMBER_OF_DOCUMENTS));
    stats.set_number_of_field_tokens(NUMBER_OF_FIELD_TOKENS);
    stats.set_avg_field_length(f64::from(AVG_FIELD_LENGTH));
    stats.set_doc_freq(i64::from(DOC_FREQ));
    stats.set_total_term_freq(TOTAL_TERM_FREQ);
    stats
}

/// `toCollectionStats(BasicStats)` (`stats.field` is `"spoof"`).
fn to_collection_stats(stats: &BasicStats) -> Result<CollectionStatistics> {
    let sum_ttf = stats.number_of_field_tokens();
    let sum_df = if sum_ttf == -1 {
        TestUtil::next_long(
            &*random(),
            stats.number_of_documents(),
            2 * stats.number_of_documents(),
        )
    } else {
        TestUtil::next_long(
            &*random(),
            stats.number_of_documents().min(sum_ttf),
            sum_ttf,
        )
    };
    let doc_count = math_to_int_exact(sum_df.min(stats.number_of_documents()));
    let max_doc = TestUtil::next_int(&*random(), doc_count, doc_count + 10);
    CollectionStatistics::new(
        "spoof",
        i64::from(max_doc),
        i64::from(doc_count),
        sum_ttf,
        sum_df,
    )
}

/// `Math.toIntExact(long)`.
fn math_to_int_exact(value: i64) -> i32 {
    i32::try_from(value).unwrap_or_else(|_| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ArithmeticException",
            "integer overflow",
        )
        .throw()
    })
}

/// `toTermStats(BasicStats)`.
fn to_term_stats(stats: &BasicStats) -> Result<TermStatistics> {
    TermStatistics::new(
        BytesRef::from_char_sequence("spoofyText"),
        stats.doc_freq(),
        stats.total_term_freq(),
    )
}

/// The generic test core called by all unit test methods. It calls the
/// `SimilarityBase#score(BasicStats, double, double)` method of all
/// Similarities in `sims` and checks if the score is valid; i.e. it is a
/// finite positive real number.
fn unit_test_core(
    f: &SimilarityBaseTest,
    stats: &BasicStats,
    freq: f32,
    doc_len: i32,
) -> Result<()> {
    for sim in &f.sims {
        let collection_stats = to_collection_stats(stats)?;
        let term_stats = to_term_stats(stats)?;
        let (score, expl_score) = sim.score_and_explain(
            stats.boost() as f32,
            &collection_stats,
            &term_stats,
            freq,
            doc_len,
        );
        let name = sim.name();
        Assert::assert_false_msg(&format!("Score infinite: {name}"), score.is_infinite());
        Assert::assert_false_msg(&format!("Score NaN: {name}"), score.is_nan());
        Assert::assert_true_msg(&format!("Score negative: {name}"), score >= 0.0);
        Assert::assert_equals_float_msg(
            Some(&format!(
                "score() and explain() return different values: {name}"
            )),
            score,
            expl_score,
            FLOAT_EPSILON,
        );
    }
    Ok(())
}

/// Runs the unit test with the default statistics.
fn test_default(f: &mut SimilarityBaseTest) -> Result<()> {
    unit_test_core(f, &create_stats(), FREQ, DOC_LEN)
}

/// Tests correct behavior when `numberOfDocuments = numberOfFieldTokens`.
fn test_sparse_documents(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_number_of_field_tokens(stats.number_of_documents());
    stats.set_total_term_freq(stats.doc_freq());
    stats.set_avg_field_length(f64::from(
        stats.number_of_field_tokens() as f32 / stats.number_of_documents() as f32,
    ));
    unit_test_core(f, &stats, FREQ, DOC_LEN)
}

/// Tests correct behavior when `numberOfDocuments > numberOfFieldTokens`.
fn test_very_sparse_documents(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_number_of_field_tokens(stats.number_of_documents() * 2 / 3);
    stats.set_total_term_freq(stats.doc_freq());
    stats.set_avg_field_length(f64::from(
        stats.number_of_field_tokens() as f32 / stats.number_of_documents() as f32,
    ));
    unit_test_core(f, &stats, FREQ, DOC_LEN)
}

/// Tests correct behavior when `NumberOfDocuments = 1`.
fn test_one_document(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_number_of_documents(1);
    stats.set_number_of_field_tokens(i64::from(DOC_LEN));
    stats.set_avg_field_length(f64::from(DOC_LEN as f32));
    stats.set_doc_freq(1);
    stats.set_total_term_freq(FREQ as i64);
    unit_test_core(f, &stats, FREQ, DOC_LEN)
}

/// Tests correct behavior when `docFreq = numberOfDocuments`.
fn test_all_documents_relevant(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    let mult = (0.0f32 + stats.number_of_documents() as f32) / stats.doc_freq() as f32;
    stats.set_total_term_freq((stats.total_term_freq() as f32 * mult) as i32 as i64);
    stats.set_doc_freq(stats.number_of_documents());
    unit_test_core(f, &stats, FREQ, DOC_LEN)
}

/// Tests correct behavior when `docFreq > numberOfDocuments / 2`.
fn test_most_documents_relevant(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    let mult = (0.6f32 * stats.number_of_documents() as f32) / stats.doc_freq() as f32;
    stats.set_total_term_freq((stats.total_term_freq() as f32 * mult) as i32 as i64);
    stats.set_doc_freq((stats.number_of_documents() as f64 * 0.6) as i32 as i64);
    unit_test_core(f, &stats, FREQ, DOC_LEN)
}

/// Tests correct behavior when `docFreq = 1`.
fn test_only_one_relevant_document(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_doc_freq(1);
    stats.set_total_term_freq(FREQ as i64 + 3);
    unit_test_core(f, &stats, FREQ, DOC_LEN)
}

/// Tests correct behavior when `totalTermFreq = numberOfFieldTokens`.
fn test_all_terms_relevant(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_total_term_freq(stats.number_of_field_tokens());
    unit_test_core(f, &stats, DOC_LEN as f32, DOC_LEN)?;
    stats.set_avg_field_length(f64::from((DOC_LEN + 10) as f32));
    unit_test_core(f, &stats, DOC_LEN as f32, DOC_LEN)
}

/// Tests correct behavior when `totalTermFreq > numberOfDocuments`.
fn test_more_terms_than_documents(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_total_term_freq(stats.total_term_freq() + stats.number_of_documents());
    unit_test_core(f, &stats, 2.0 * FREQ, DOC_LEN)
}

/// Tests correct behavior when `totalTermFreq = numberOfDocuments`.
fn test_number_of_terms_as_documents(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_total_term_freq(stats.number_of_documents());
    unit_test_core(f, &stats, FREQ, DOC_LEN)
}

/// Tests correct behavior when `totalTermFreq = 1`.
fn test_one_term(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_doc_freq(1);
    stats.set_total_term_freq(1);
    unit_test_core(f, &stats, 1.0, DOC_LEN)
}

/// Tests correct behavior when `totalTermFreq = freq`.
fn test_one_relevant_document(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_doc_freq(1);
    stats.set_total_term_freq(FREQ as i64);
    unit_test_core(f, &stats, FREQ, DOC_LEN)
}

/// Tests correct behavior when `numberOfFieldTokens = freq`.
fn test_all_terms_relevant_only_one_document(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_number_of_documents(10);
    stats.set_number_of_field_tokens(50);
    stats.set_avg_field_length(5.0);
    stats.set_doc_freq(1);
    stats.set_total_term_freq(50);
    unit_test_core(f, &stats, 50.0, 50)
}

/// Tests correct behavior when there is only one document with a single
/// term in the collection.
fn test_only_one_term_one_document(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_number_of_documents(1);
    stats.set_number_of_field_tokens(1);
    stats.set_avg_field_length(1.0);
    stats.set_doc_freq(1);
    stats.set_total_term_freq(1);
    unit_test_core(f, &stats, 1.0, 1)
}

/// Tests correct behavior when there is only one term in the field, but
/// more than one documents.
fn test_only_one_term(f: &mut SimilarityBaseTest) -> Result<()> {
    let mut stats = create_stats();
    stats.set_number_of_field_tokens(1);
    stats.set_avg_field_length(f64::from(1.0f32 / stats.number_of_documents() as f32));
    stats.set_doc_freq(1);
    stats.set_total_term_freq(1);
    unit_test_core(f, &stats, 1.0, DOC_LEN)
}

/// Tests correct behavior when `avgFieldLength = docLen`.
fn test_document_length_average(f: &mut SimilarityBaseTest) -> Result<()> {
    let stats = create_stats();
    let doc_len = stats.avg_field_length() as f32 as i32;
    unit_test_core(f, &stats, FREQ, doc_len)
}

/// Correctness test for the Dirichlet LM model.
fn test_lm_dirichlet(_f: &mut SimilarityBaseTest) -> Result<()> {
    let p = (FREQ
        + 2000.0f32 * (TOTAL_TERM_FREQ + 1) as f32 / (NUMBER_OF_FIELD_TOKENS as f32 + 1.0f32))
        / (DOC_LEN as f32 + 2000.0f32);
    let a = 2000.0f32 / (DOC_LEN as f32 + 2000.0f32);
    let gold = (f64::from(
        p / (a * (TOTAL_TERM_FREQ + 1) as f32 / (NUMBER_OF_FIELD_TOKENS as f32 + 1.0f32)),
    )
    .ln()
        + f64::from(a).ln()) as f32;
    correctness_test_core(&*sim_base(LMDirichletSimilarity::new()), gold)
}

/// Correctness test for the Jelinek-Mercer LM model.
fn test_lm_jelinek_mercer(_f: &mut SimilarityBaseTest) -> Result<()> {
    let p = (1.0 - 0.1f32) * FREQ / DOC_LEN as f32
        + 0.1f32 * (TOTAL_TERM_FREQ + 1) as f32 / (NUMBER_OF_FIELD_TOKENS as f32 + 1.0f32);
    let gold = f64::from(
        p / (0.1f32 * (TOTAL_TERM_FREQ + 1) as f32 / (NUMBER_OF_FIELD_TOKENS as f32 + 1.0f32)),
    )
    .ln() as f32;
    correctness_test_core(
        &*sim_base(LMJelinekMercerSimilarity::with_lambda(0.1)?),
        gold,
    )
}

/// Correctness test for the LL IB model with DF-based lambda and no
/// normalization.
fn test_ll_for_ib(_f: &mut SimilarityBaseTest) -> Result<()> {
    let sim = sim_base(IBSimilarity::new(
        Box::new(DistributionLL::new()),
        Box::new(LambdaDF::new()),
        Box::new(NoNormalization::new()),
    ));
    correctness_test_core(&*sim, 4.178574562072754f32)
}

/// Correctness test for the SPL IB model with TTF-based lambda and no
/// normalization.
fn test_spl_for_ib(_f: &mut SimilarityBaseTest) -> Result<()> {
    let sim = sim_base(IBSimilarity::new(
        Box::new(DistributionSPL::new()),
        Box::new(LambdaTTF::new()),
        Box::new(NoNormalization::new()),
    ));
    correctness_test_core(&*sim, 2.2387237548828125f32)
}

/// Correctness test for the IneB2 DFR model.
fn test_ine_b2(_f: &mut SimilarityBaseTest) -> Result<()> {
    let sim = sim_base(DFRSimilarity::new(
        Box::new(BasicModelIne::new()),
        Box::new(AfterEffectB::new()),
        Box::new(NormalizationH2::default()),
    ));
    correctness_test_core(&*sim, 5.747603416442871f32)
}

/// Correctness test for the GL1 DFR model.
fn test_gl1(_f: &mut SimilarityBaseTest) -> Result<()> {
    let sim = sim_base(DFRSimilarity::new(
        Box::new(BasicModelG::new()),
        Box::new(AfterEffectL::new()),
        Box::new(NormalizationH1::default()),
    ));
    correctness_test_core(&*sim, 1.6390540599822998f32)
}

/// Correctness test for the In2 DFR model with no aftereffect.
fn test_in2(_f: &mut SimilarityBaseTest) -> Result<()> {
    let sim = sim_base(DFRSimilarity::new(
        Box::new(BasicModelIn::new()),
        Box::new(AfterEffectL::new()),
        Box::new(NormalizationH2::default()),
    ));
    let tfn = (f64::from(FREQ)
        * log2(
            // 8.1894750101
            f64::from(1.0 + AVG_FIELD_LENGTH / DOC_LEN as f32),
        )) as f32;
    let gold = (f64::from(tfn)
        * log2(
            // 26.7459577898
            f64::from(NUMBER_OF_DOCUMENTS + 1) / (f64::from(DOC_FREQ) + 0.5),
        )
        / f64::from(1.0 + tfn)) as f32;
    correctness_test_core(&*sim, gold)
}

/// Correctness test for the IFB DFR model with no normalization.
#[allow(non_snake_case)]
fn test_ifb(_f: &mut SimilarityBaseTest) -> Result<()> {
    let sim = sim_base(DFRSimilarity::new(
        Box::new(BasicModelIF::new()),
        Box::new(AfterEffectB::new()),
        Box::new(NoNormalization::new()),
    ));
    let B = (TOTAL_TERM_FREQ + 1 + 1) as f32 / ((DOC_FREQ + 1) as f32 * (FREQ + 1.0)); // 0.8875
    let IF = (f64::from(FREQ)
        * log2(
            // 8.97759389642
            1.0 + f64::from(NUMBER_OF_DOCUMENTS + 1) / (TOTAL_TERM_FREQ as f64 + 0.5),
        )) as f32;
    let gold = B * IF; // 7.96761458307
    correctness_test_core(&*sim, gold)
}

/// The generic test core called by all correctness test methods. It calls
/// the `SimilarityBase#score(BasicStats, double, double)` method of all
/// Similarities in `sims` and compares the score against the manually
/// computed `gold`.
fn correctness_test_core(sim: &dyn SimBase, gold: f32) -> Result<()> {
    let stats = create_stats();
    let collection_stats = to_collection_stats(&stats)?;
    let term_stats = to_term_stats(&stats)?;
    let (score, _) = sim.score_and_explain(
        stats.boost() as f32,
        &collection_stats,
        &term_stats,
        FREQ,
        DOC_LEN,
    );
    Assert::assert_equals_float_msg(
        Some(&format!("{} score not correct.", sim.name())),
        gold,
        score,
        FLOAT_EPSILON,
    );
    Ok(())
}

/// Tests whether all similarities return three documents for the query
/// word "heart".
fn test_heart_list(f: &mut SimilarityBaseTest) -> Result<()> {
    let q = tq(FIELD_BODY, "heart");
    let searcher = f.searcher.as_mut().expect("setUp ran");
    for sim in &f.sims {
        searcher.set_similarity(sim.similarity());
        let top_docs = searcher.search(Arc::clone(&q), 1000)?;
        assert_equals_msg(
            Some(&format!("Failed: {}", sim.name())),
            &3,
            &top_docs.total_hits.value(),
        );
    }
    Ok(())
}

/// Test whether all similarities return document 3 before documents 7 and
/// 8.
fn test_heart_ranking(f: &mut SimilarityBaseTest) -> Result<()> {
    let q = tq(FIELD_BODY, "heart");
    let searcher = f.searcher.as_mut().expect("setUp ran");
    let reader = f.reader.as_ref().expect("setUp ran");
    for sim in &f.sims {
        searcher.set_similarity(sim.similarity());
        let top_docs = searcher.search(Arc::clone(&q), 1000)?;
        assert_equals_msg(
            Some(&format!("Failed: {}", sim.name())),
            &Some("2".to_string()),
            &reader
                .stored_fields()?
                .document(top_docs.score_docs[0].doc)?
                .get(FIELD_ID),
        );
    }
    Ok(())
}

fn test_discount_overlaps_boost(_f: &mut SimilarityBaseTest) -> Result<()> {
    let expected0 = BM25Similarity::with_discount_overlaps(false);
    let actual0 = DFRSimilarity::with_discount_overlaps(
        Box::new(BasicModelIne::new()),
        Box::new(AfterEffectB::new()),
        Box::new(NormalizationH2::default()),
        false,
    );
    let mut state = FieldInvertState::new(
        Version::LATEST.major as i32,
        "foo".to_string(),
        IndexOptions::DOCS_AND_FREQS,
    );
    state.set_length(5);
    state.set_num_overlap(2);
    assert_equals(
        &expected0.compute_norm(&state)?,
        &actual0.compute_norm(&state)?,
    );
    let expected1 = BM25Similarity::with_discount_overlaps(true);
    let actual1 = DFRSimilarity::with_discount_overlaps(
        Box::new(BasicModelIne::new()),
        Box::new(AfterEffectB::new()),
        Box::new(NormalizationH2::default()),
        true,
    );
    assert_equals(
        &expected1.compute_norm(&state)?,
        &actual1.compute_norm(&state)?,
    );
    Ok(())
}
