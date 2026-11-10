//! Port of `lucene/core/src/test/org/apache/lucene/search/TestLogOddsFusionQuery.java`.
//!
//! Tests for {@link LogOddsFusionQuery}. Clauses are wrapped with {@link BayesianScoreQuery} to
//! produce (0,1) probability scores, matching the paper's query-level calibration design.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, KnnFloatVectorField, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, VectorSimilarityFunction};
use rucene::search::bayesian_score_query::BayesianScoreQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::log_odds_fusion_query::LogOddsFusionQuery;
use rucene::search::similarities::BM25Similarity;
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, KnnFloatVectorQuery, MatchNoDocsQuery, Occur, Query, ScoreDoc, ScoreMode,
};
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{
    assert_query_not_equals, fixture_class, ftest, set_up, tear_down, term,
};
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, assert_not_null, random, IAE,
};

const BSQ_ALPHA: f32 = 0.5;
const BSQ_BETA: f32 = 3.0;

/// Wraps a query with BayesianScoreQuery for sigmoid calibration to (0,1).
fn bayesian(q: Arc<dyn Query>) -> Arc<dyn Query> {
    Arc::new(
        BayesianScoreQuery::with_alpha_beta(q, BSQ_ALPHA, BSQ_BETA)
            .expect("valid BayesianScoreQuery parameters"),
    )
}

/// `new TermQuery(new Term(field, text))`.
fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `bayesian(new TermQuery(new Term(field, text)))`.
fn btq(field: &str, text: &str) -> Arc<dyn Query> {
    bayesian(tq(field, text))
}

/// `new LogOddsFusionQuery(clauses, alpha)` as a `Query`.
fn loq(clauses: Vec<Arc<dyn Query>>, alpha: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(LogOddsFusionQuery::with_alpha(clauses, alpha)?))
}

/// `new LogOddsFusionQuery(clauses, alpha, weights)` as a `Query`.
fn wloq(clauses: Vec<Arc<dyn Query>>, alpha: f32, weights: &[f32]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(LogOddsFusionQuery::with_weights(
        clauses,
        alpha,
        Some(weights.to_vec()),
    )?))
}

/// The instance of `TestLogOddsFusionQuery`.
#[derive(Default)]
pub struct LogOddsFusion {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
    hybrid_dir: Option<TestDirectory>,
    hybrid_reader: Option<Arc<dyn DirectoryReader>>,
    hybrid_searcher: Option<IndexSearcher>,
}

impl LogOddsFusion {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }

    fn hybrid_searcher(&self) -> &IndexSearcher {
        self.hybrid_searcher.as_ref().expect("buildHybridIndex ran")
    }
}

/// `TestLogOddsFusionQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<LogOddsFusion>(
        "org.apache.lucene.search.TestLogOddsFusionQuery",
        vec![
            set_up(log_odds_set_up),
            tear_down(log_odds_tear_down),
            ftest("testBasicFusion", test_basic_fusion),
            ftest("testSingleClauseRewrite", test_single_clause_rewrite),
            ftest("testEmptyClauseRewrite", test_empty_clause_rewrite),
            ftest("testRewriteAllMatchNoDocs", test_rewrite_all_match_no_docs),
            ftest("testRewriteSingleSurvivor", test_rewrite_single_survivor),
            ftest(
                "testRewriteFilterMatchNoDocs",
                test_rewrite_filter_match_no_docs,
            ),
            ftest(
                "testRewriteFilterMatchNoDocsWithAlpha",
                test_rewrite_filter_match_no_docs_with_alpha,
            ),
            ftest(
                "testRewriteFilterMatchNoDocsWithWeights",
                test_rewrite_filter_match_no_docs_with_weights,
            ),
            ftest(
                "testRewriteFilterMatchNoDocsWithLogitBounds",
                test_rewrite_filter_match_no_docs_with_logit_bounds,
            ),
            ftest("testAlphaValues", test_alpha_values),
            ftest("testIllegalAlpha", test_illegal_alpha),
            ftest("testNonMatchingSubQueries", test_non_matching_sub_queries),
            ftest("testExplanation", test_explanation),
            ftest("testEqualsAndHashCode", test_equals_and_hash_code),
            ftest("testQueryUtils", test_query_utils),
            ftest("testMaxScoreCorrectness", test_max_score_correctness),
            ftest("testToString", test_to_string),
            ftest("testThreeWayCombination", test_three_way_combination),
            ftest("testMultiFieldHybridSearch", test_multi_field_hybrid_search),
            ftest(
                "testMoreSignalsIncreaseScore",
                test_more_signals_increase_score,
            ),
            ftest(
                "testTextAndVectorHybridSearch",
                test_text_and_vector_hybrid_search,
            ),
            ftest(
                "testVectorAndTextBothContribute",
                test_vector_and_text_both_contribute,
            ),
            ftest("testMultipleVectorFields", test_multiple_vector_fields),
            ftest("testBooleanMustWithHybrid", test_boolean_must_with_hybrid),
            ftest(
                "testBooleanMustNotWithHybrid",
                test_boolean_must_not_with_hybrid,
            ),
            ftest(
                "testBooleanShouldWithHybrid",
                test_boolean_should_with_hybrid,
            ),
            ftest("testVectorAndVector", test_vector_and_vector),
            ftest("testVectorNot", test_vector_not),
            ftest("testWeightedFusion", test_weighted_fusion),
            ftest(
                "testWeightedFusionAffectsRanking",
                test_weighted_fusion_affects_ranking,
            ),
            ftest("testWeightedExplanation", test_weighted_explanation),
            ftest(
                "testWeightedEqualsAndHashCode",
                test_weighted_equals_and_hash_code,
            ),
            ftest(
                "testWeightedMaxScoreCorrectness",
                test_weighted_max_score_correctness,
            ),
            ftest("testWeightedToString", test_weighted_to_string),
            ftest("testWeightedRewrite", test_weighted_rewrite),
            ftest("testIllegalWeights", test_illegal_weights),
            ftest("testWeightedQueryUtils", test_weighted_query_utils),
            ftest(
                "testWeightedThreeWayCombination",
                test_weighted_three_way_combination,
            ),
            ftest(
                "testNormalizedFusionProducesValidScores",
                test_normalized_fusion_produces_valid_scores,
            ),
            ftest(
                "testNormalizedMaxScoreCorrectness",
                test_normalized_max_score_correctness,
            ),
        ],
    )
}

fn text_field(name: &str, value: &str, store: Store) -> Result<Box<TextField>> {
    Ok(Box::new(TextField::new(name, value.to_string(), store)?))
}

fn log_odds_set_up(f: &mut LogOddsFusion) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;

    let mut doc0 = Document::new();
    doc0.add(text_field("body", "alpha beta gamma", Store::YES)?);
    writer.add_document(&doc0)?;

    let mut doc1 = Document::new();
    doc1.add(text_field("body", "alpha gamma delta", Store::YES)?);
    writer.add_document(&doc1)?;

    let mut doc2 = Document::new();
    doc2.add(text_field("body", "beta gamma delta", Store::YES)?);
    writer.add_document(&doc2)?;

    let mut doc3 = Document::new();
    doc3.add(text_field("body", "gamma delta epsilon", Store::YES)?);
    writer.add_document(&doc3)?;

    writer.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_similarity(Arc::new(BM25Similarity::new()));
    f.dir = Some(dir);
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    Ok(())
}

fn log_odds_tear_down(f: &mut LogOddsFusion) -> Result<()> {
    drop(f.searcher.take());
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

fn hits(searcher: &IndexSearcher, q: &Arc<dyn Query>, n: i32) -> Result<Vec<ScoreDoc>> {
    Ok(searcher.search(Arc::clone(q), n)?.score_docs)
}

// ---- Core query behavior ----

fn test_basic_fusion(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");

    let loq = loq(vec![q1, q2], 0.5)?;
    QueryUtils::check_searcher(&*random(), &loq, Some(f.searcher()));

    let hits = hits(f.searcher(), &loq, 10)?;
    Assert::assert_true_msg("should have at least 1 hit", !hits.is_empty());

    for hit in &hits {
        Assert::assert_true_msg(
            &format!("score should be > 0, got {}", hit.score),
            hit.score > 0.0,
        );
        Assert::assert_true_msg(
            &format!("score should be < 1, got {}", hit.score),
            hit.score < 1.0,
        );
    }

    if hits.len() >= 2 {
        Assert::assert_true_msg(
            "doc matching both terms should score higher than doc matching one",
            hits[0].score >= hits[1].score,
        );
    }
    Ok(())
}

fn test_single_clause_rewrite(f: &mut LogOddsFusion) -> Result<()> {
    let sub = btq("body", "alpha");
    let loq: Arc<dyn Query> = Arc::new(LogOddsFusionQuery::with_clauses(vec![sub])?);

    let rewritten = f.searcher().rewrite(loq)?;
    Assert::assert_true_msg(
        &format!("single clause should rewrite to BayesianScoreQuery, got {rewritten:?}"),
        rewritten.as_any().is::<BayesianScoreQuery>(),
    );
    Ok(())
}

fn match_no_docs() -> Arc<dyn Query> {
    Arc::new(MatchNoDocsQuery::instance())
}

fn test_empty_clause_rewrite(f: &mut LogOddsFusion) -> Result<()> {
    let loq = loq(Vec::new(), 0.5)?;
    let rewritten = f.searcher().rewrite(loq)?;
    assert_query_equals(&match_no_docs(), &rewritten);
    Ok(())
}

fn test_rewrite_all_match_no_docs(f: &mut LogOddsFusion) -> Result<()> {
    let q1: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("q1"));
    let q2: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("q2"));
    let loq = loq(vec![q1, q2], 0.5)?;
    let rewritten = f.searcher().rewrite(loq)?;
    assert_query_equals(&match_no_docs(), &rewritten);
    Ok(())
}

fn test_rewrite_single_survivor(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("q2"));
    let loq = loq(vec![Arc::clone(&q1), q2], 0.5)?;
    let rewritten = f.searcher().rewrite(loq)?;
    assert_query_equals(&q1, &rewritten);
    Ok(())
}

fn test_rewrite_filter_match_no_docs(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let q3: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("q3"));
    let loq = loq(vec![Arc::clone(&q1), Arc::clone(&q2), q3], 0.5)?;
    let rewritten = f.searcher().rewrite(loq)?;
    let expected = self::loq(vec![q1, q2], 0.5)?;
    assert_query_equals(&expected, &rewritten);
    Ok(())
}

fn test_rewrite_filter_match_no_docs_with_alpha(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let q3: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("q3"));
    let loq = loq(vec![Arc::clone(&q1), Arc::clone(&q2), q3], 0.3)?;
    let rewritten = f.searcher().rewrite(loq)?;
    let expected = self::loq(vec![q1, q2], 0.3)?;
    assert_query_equals(&expected, &rewritten);
    Ok(())
}

/// `rewritten instanceof LogOddsFusionQuery`, with the cast.
fn as_log_odds<'a>(rewritten: &'a Arc<dyn Query>) -> &'a LogOddsFusionQuery {
    let result = rewritten.as_any().downcast_ref::<LogOddsFusionQuery>();
    Assert::assert_true_msg("should rewrite to LogOddsFusionQuery", result.is_some());
    result.expect("instanceof checked")
}

fn test_rewrite_filter_match_no_docs_with_weights(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let q3: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("q3"));
    let weights = [0.4f32, 0.4, 0.2];
    let loq = wloq(vec![q1, q2, q3], 0.5, &weights)?;
    let rewritten = f.searcher().rewrite(loq)?;
    let result = as_log_odds(&rewritten);
    assert_equals(&2, &result.get_clauses().len());
    let result_weights = result.get_weights();
    assert_not_null(&result_weights);
    let result_weights = result_weights.expect("checked");
    assert_equals(&2, &result_weights.len());
    assert_equals_delta(0.5, f64::from(result_weights[0]), f64::from(1e-6f32));
    assert_equals_delta(0.5, f64::from(result_weights[1]), f64::from(1e-6f32));
    Ok(())
}

fn test_rewrite_filter_match_no_docs_with_logit_bounds(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let q3: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("q3"));
    let weights = vec![0.4f32, 0.4, 0.2];
    let logit_min = vec![-5f32, -4.0, -3.0];
    let logit_max = vec![5f32, 4.0, 3.0];
    let loq: Arc<dyn Query> = Arc::new(LogOddsFusionQuery::new(
        vec![q1, q2, q3],
        0.5,
        Some(weights),
        Some(logit_min),
        Some(logit_max),
    )?);
    let rewritten = f.searcher().rewrite(loq)?;
    let result = as_log_odds(&rewritten);
    assert_equals(&2, &result.get_clauses().len());
    Ok(())
}

fn test_alpha_values(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");

    for alpha in [0.0f32, 0.5, 1.0] {
        let loq = loq(vec![Arc::clone(&q1), Arc::clone(&q2)], alpha)?;
        let hits = hits(f.searcher(), &loq, 10)?;
        Assert::assert_true_msg(
            &format!("alpha={alpha:?} should have hits"),
            !hits.is_empty(),
        );
        for hit in &hits {
            Assert::assert_true_msg(
                "score should be finite and > 0",
                hit.score.is_finite() && hit.score > 0.0,
            );
            Assert::assert_true_msg("score should be < 1", hit.score < 1.0);
        }
    }
    Ok(())
}

fn test_illegal_alpha(_f: &mut LogOddsFusion) -> Result<()> {
    expect_throws(IAE, || LogOddsFusionQuery::with_alpha(Vec::new(), -0.1));
    expect_throws(IAE, || LogOddsFusionQuery::with_alpha(Vec::new(), 1.1));
    expect_throws(IAE, || LogOddsFusionQuery::with_alpha(Vec::new(), f32::NAN));
    Ok(())
}

fn test_non_matching_sub_queries(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "nonexistent1");
    let q2 = btq("body", "nonexistent2");
    let loq = loq(vec![q1, q2], 0.5)?;
    assert_equals_msg(
        Some("no docs should match"),
        &0,
        &hits(f.searcher(), &loq, 10)?.len(),
    );
    Ok(())
}

/// `searcher.createWeight(searcher.rewrite(loq), COMPLETE, 1)` and the first leaf.
fn explain_on_first_leaf(
    searcher: &IndexSearcher,
    loq: Arc<dyn Query>,
    docs: &[i32],
) -> Result<Vec<rucene::search::similarities::Explanation>> {
    let w = searcher.create_weight(searcher.rewrite(loq)?, ScoreMode::COMPLETE, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let context = &leaves[0];
    docs.iter().map(|&doc| w.explain(context, doc)).collect()
}

fn test_explanation(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let loq = loq(vec![q1, q2], 0.5)?;

    let expls = explain_on_first_leaf(f.searcher(), loq, &[0, 3])?;

    let expl = &expls[0];
    Assert::assert_true_msg("doc0 should match", expl.is_match());
    Assert::assert_true_msg(
        "should mention log-odds",
        expl.description().contains("log-odds"),
    );

    let no_match = &expls[1];
    Assert::assert_false_msg("doc3 should not match", no_match.is_match());
    Ok(())
}

fn test_equals_and_hash_code(_f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");

    let a = loq(vec![Arc::clone(&q1), Arc::clone(&q2)], 0.5)?;
    let b = loq(vec![Arc::clone(&q1), Arc::clone(&q2)], 0.5)?;
    let c = loq(vec![Arc::clone(&q1), Arc::clone(&q2)], 0.3)?;
    let d = loq(vec![q2, q1], 0.5)?;

    assert_query_equals(&a, &b);
    assert_equals(&a.query_hash(), &b.query_hash());
    assert_query_not_equals(&a, &c);
    assert_query_equals(&a, &d);
    Ok(())
}

fn test_query_utils(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let loq = loq(vec![q1, q2], 0.5)?;
    QueryUtils::check_searcher(&*random(), &loq, Some(f.searcher()));
    Ok(())
}

fn test_max_score_correctness(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let loq = loq(vec![q1, q2], 0.5)?;
    CheckHits::check_top_scores(&*random(), &loq, f.searcher())
}

fn test_to_string(_f: &mut LogOddsFusion) -> Result<()> {
    let q1 = tq("body", "alpha");
    let q2 = tq("body", "beta");
    let loq = loq(vec![q1, q2], 0.5)?;
    let str = loq.to_query_string("body");
    Assert::assert_true_msg("should contain LogOdds", str.contains("LogOdds"));
    Assert::assert_true_msg("should contain alpha", str.contains("0.5"));
    Ok(())
}

fn test_three_way_combination(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let q3 = btq("body", "gamma");
    let loq = loq(vec![q1, q2, q3], 0.5)?;
    QueryUtils::check_searcher(&*random(), &loq, Some(f.searcher()));

    let hits = hits(f.searcher(), &loq, 10)?;
    Assert::assert_true_msg("should have hits", !hits.is_empty());
    for hit in &hits {
        Assert::assert_true_msg("score in (0,1)", hit.score > 0.0 && hit.score < 1.0);
    }
    Ok(())
}

// ---- Multi-field hybrid text tests ----

fn test_multi_field_hybrid_search(_f: &mut LogOddsFusion) -> Result<()> {
    let mut hybrid_dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(hybrid_dir.as_directory(), IndexWriterConfig::new())?;

    let mut doc0 = Document::new();
    doc0.add(text_field("title", "lucene search engine", Store::YES)?);
    doc0.add(text_field(
        "body",
        "lucene is a full-text search library",
        Store::YES,
    )?);
    writer.add_document(&doc0)?;

    let mut doc1 = Document::new();
    doc1.add(text_field("title", "lucene search engine", Store::YES)?);
    doc1.add(text_field(
        "body",
        "this is about information retrieval",
        Store::YES,
    )?);
    writer.add_document(&doc1)?;

    let mut doc2 = Document::new();
    doc2.add(text_field(
        "title",
        "database systems overview",
        Store::YES,
    )?);
    doc2.add(text_field(
        "body",
        "relational databases use SQL",
        Store::YES,
    )?);
    writer.add_document(&doc2)?;

    writer.force_merge(1)?;
    writer.close()?;
    let hybrid_reader = directory_reader::open(hybrid_dir.as_directory())?;
    let result = (|| -> Result<()> {
        let mut hybrid_searcher =
            IndexSearcher::new(Arc::clone(&hybrid_reader) as Arc<dyn IndexReader>)?;
        hybrid_searcher.set_similarity(Arc::new(BM25Similarity::new()));

        let title_query = btq("title", "lucene");
        let body_query = btq("body", "lucene");
        let hybrid_query = loq(vec![title_query, body_query], 0.5)?;

        let hits = hits(&hybrid_searcher, &hybrid_query, 10)?;
        assert_equals_msg(Some("should have 2 hits"), &2, &hits.len());

        for hit in &hits {
            Assert::assert_true_msg("score in (0,1)", hit.score > 0.0 && hit.score < 1.0);
        }

        // doc0 matches both fields, doc1 matches title only
        assert_equals_msg(Some("doc0 should rank first"), &0, &hits[0].doc);
        Assert::assert_true_msg(
            &format!("doc0 ({}) > doc1 ({})", hits[0].score, hits[1].score),
            hits[0].score > hits[1].score,
        );
        Ok(())
    })();
    hybrid_reader.close()?;
    hybrid_dir.close()?;
    result
}

fn test_more_signals_increase_score(_f: &mut LogOddsFusion) -> Result<()> {
    let mut signal_dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(signal_dir.as_directory(), IndexWriterConfig::new())?;

    let mut doc = Document::new();
    doc.add(text_field("f1", "search engine optimization", Store::NO)?);
    doc.add(text_field(
        "f2",
        "search algorithms and data structures",
        Store::NO,
    )?);
    doc.add(text_field("f3", "search relevance tuning", Store::NO)?);
    writer.add_document(&doc)?;

    writer.force_merge(1)?;
    writer.close()?;
    let signal_reader = directory_reader::open(signal_dir.as_directory())?;
    let result = (|| -> Result<()> {
        let mut signal_searcher =
            IndexSearcher::new(Arc::clone(&signal_reader) as Arc<dyn IndexReader>)?;
        signal_searcher.set_similarity(Arc::new(BM25Similarity::new()));

        let q1 = btq("f1", "search");
        let q2 = btq("f2", "search");
        let q3 = btq("f3", "search");

        let score1 = hits(&signal_searcher, &q1, 1)?[0].score;

        let two_signals = loq(vec![Arc::clone(&q1), Arc::clone(&q2)], 0.5)?;
        let score2 = hits(&signal_searcher, &two_signals, 1)?[0].score;

        let three_signals = loq(vec![q1, q2, q3], 0.5)?;
        let score3 = hits(&signal_searcher, &three_signals, 1)?[0].score;

        // With softplus gating, more matching signals always produce >= score.
        Assert::assert_true_msg("3 >= 2 signals", score3 >= score2);
        Assert::assert_true_msg("2 >= 1 signal", score2 >= score1);
        Assert::assert_true_msg(
            "all in (0,1)",
            score1 > 0.0 && score1 < 1.0 && score2 > 0.0 && score3 < 1.0,
        );
        Ok(())
    })();
    signal_reader.close()?;
    signal_dir.close()?;
    result
}

// ---- Vector + Text hybrid search tests ----

const HYBRID_NUM_FILLER: i32 = 196;
static QUERY_A: LazyLock<Vec<f32>> = LazyLock::new(|| normalize(&[1.0, 0.0, 0.0]));

fn knn_field(name: &str, v: &[f32]) -> Result<Box<KnnFloatVectorField>> {
    Ok(Box::new(KnnFloatVectorField::new(
        name,
        &normalize(v),
        VectorSimilarityFunction::COSINE,
    )?))
}

/// `new IndexWriterConfig()` with `config.setCodec(TestUtil.getDefaultCodec())`.
fn default_codec_config() -> Result<IndexWriterConfig> {
    let mut config = IndexWriterConfig::new();
    config.set_codec(TestUtil::get_default_codec())?;
    Ok(config)
}

fn build_hybrid_index(f: &mut LogOddsFusion) -> Result<()> {
    let hybrid_dir = LuceneTestCase::new_directory();
    let config = default_codec_config()?;
    let writer = IndexWriter::new(hybrid_dir.as_directory(), config)?;

    // doc0: text match + vector near queryA
    let mut doc0 = Document::new();
    doc0.add(text_field(
        "body",
        "lucene search engine optimization library",
        Store::NO,
    )?);
    doc0.add(knn_field("embedding", &[0.95, 0.05, 0.0])?);
    writer.add_document(&doc0)?;

    // doc1: text match + vector far from queryA
    let mut doc1 = Document::new();
    doc1.add(text_field(
        "body",
        "lucene full text search library index",
        Store::NO,
    )?);
    doc1.add(knn_field("embedding", &[0.0, 0.0, 1.0])?);
    writer.add_document(&doc1)?;

    // doc2: no text match + vector near queryA
    let mut doc2 = Document::new();
    doc2.add(text_field(
        "body",
        "unrelated topic about cooking recipes",
        Store::NO,
    )?);
    doc2.add(knn_field("embedding", &[0.90, 0.10, 0.0])?);
    writer.add_document(&doc2)?;

    // doc3: neither
    let mut doc3 = Document::new();
    doc3.add(text_field(
        "body",
        "gardening tips for spring planting",
        Store::NO,
    )?);
    doc3.add(knn_field("embedding", &[0.0, 1.0, 0.0])?);
    writer.add_document(&doc3)?;

    // Filler docs with vectors away from queryA
    for i in 0..HYBRID_NUM_FILLER {
        let mut filler = Document::new();
        filler.add(text_field(
            "body",
            &format!("filler document about topic {i}"),
            Store::NO,
        )?);
        let y = (f64::from(i) * 0.1).sin() as f32;
        let z = (f64::from(i) * 0.1).cos() as f32;
        filler.add(knn_field("embedding", &[0.0, y, z])?);
        writer.add_document(&filler)?;
    }

    writer.force_merge(1)?;
    writer.close()?;
    let hybrid_reader = directory_reader::open(hybrid_dir.as_directory())?;
    let mut hybrid_searcher =
        IndexSearcher::new(Arc::clone(&hybrid_reader) as Arc<dyn IndexReader>)?;
    hybrid_searcher.set_similarity(Arc::new(BM25Similarity::new()));
    f.hybrid_dir = Some(hybrid_dir);
    f.hybrid_reader = Some(hybrid_reader);
    f.hybrid_searcher = Some(hybrid_searcher);
    Ok(())
}

fn close_hybrid_index(f: &mut LogOddsFusion) -> Result<()> {
    drop(f.hybrid_searcher.take());
    if let Some(reader) = f.hybrid_reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = f.hybrid_dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// `buildHybridIndex(); try { body } finally { closeHybridIndex(); }`.
fn with_hybrid_index(
    f: &mut LogOddsFusion,
    body: impl FnOnce(&IndexSearcher) -> Result<()>,
) -> Result<()> {
    build_hybrid_index(f)?;
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(f.hybrid_searcher())));
    let closed = close_hybrid_index(f);
    match result {
        Ok(r) => {
            r?;
            closed
        }
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn knn_query(field: &str, v: &[f32], k: i32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(KnnFloatVectorQuery::new(field, v.to_vec(), k)?))
}

fn test_text_and_vector_hybrid_search(f: &mut LogOddsFusion) -> Result<()> {
    with_hybrid_index(f, |hybrid_searcher| {
        // Text query wrapped with BayesianScoreQuery -> probability in (0,1)
        let text_query = btq("body", "lucene");
        // KNN COSINE already produces (0,1) scores via (1+cos)/2
        let vector_query = knn_query("embedding", &QUERY_A, 10)?;

        let hybrid_query = loq(vec![text_query, vector_query], 0.5)?;

        let hits = hits(hybrid_searcher, &hybrid_query, 10)?;
        Assert::assert_true_msg("should have hits", hits.len() >= 3);

        for hit in &hits {
            Assert::assert_true_msg("score > 0", hit.score > 0.0);
            Assert::assert_true_msg("score < 1", hit.score < 1.0);
        }

        // doc0 matches both text and vector
        assert_equals_msg(
            Some("doc0 (text+vector) should rank first"),
            &0,
            &hits[0].doc,
        );
        Assert::assert_true_msg("doc0 > second", hits[0].score > hits[1].score);
        Ok(())
    })
}

fn test_vector_and_text_both_contribute(f: &mut LogOddsFusion) -> Result<()> {
    with_hybrid_index(f, |hybrid_searcher| {
        let text_query = btq("body", "lucene");
        let vector_query = knn_query("embedding", &QUERY_A, 10)?;

        // Verify KNN returns doc0 and doc2
        let knn_hits = hits(hybrid_searcher, &vector_query, 10)?;
        let mut knn_found_doc0 = false;
        let mut knn_found_doc2 = false;
        for hit in &knn_hits {
            if hit.doc == 0 {
                knn_found_doc0 = true;
            }
            if hit.doc == 2 {
                knn_found_doc2 = true;
            }
        }
        Assert::assert_true_msg("KNN should return doc0", knn_found_doc0);
        Assert::assert_true_msg("KNN should return doc2", knn_found_doc2);

        let hybrid_query = loq(vec![text_query, vector_query], 0.5)?;
        let hits = hits(hybrid_searcher, &hybrid_query, 20)?;

        let mut found_text_only = false;
        let mut found_vector_only = false;
        for hit in &hits {
            if hit.doc == 1 {
                found_text_only = true;
            }
            if hit.doc == 2 {
                found_vector_only = true;
            }
        }
        Assert::assert_true_msg("text-only doc1 should appear", found_text_only);
        Assert::assert_true_msg("vector-only doc2 should appear", found_vector_only);
        Ok(())
    })
}

fn test_multiple_vector_fields(_f: &mut LogOddsFusion) -> Result<()> {
    let query_a = normalize(&[1.0, 0.0]);
    let query_b = normalize(&[0.0, 1.0]);

    let mut test_dir = LuceneTestCase::new_directory();
    let config = default_codec_config()?;
    let writer = IndexWriter::new(test_dir.as_directory(), config)?;

    let mut doc0 = Document::new();
    doc0.add(knn_field("emb_a", &[0.95, 0.05])?);
    doc0.add(knn_field("emb_b", &[0.05, 0.95])?);
    writer.add_document(&doc0)?;

    let mut doc1 = Document::new();
    doc1.add(knn_field("emb_a", &[0.90, 0.10])?);
    doc1.add(knn_field("emb_b", &[1.0, 0.0])?);
    writer.add_document(&doc1)?;

    let mut doc2 = Document::new();
    doc2.add(knn_field("emb_a", &[0.0, 1.0])?);
    doc2.add(knn_field("emb_b", &[0.10, 0.90])?);
    writer.add_document(&doc2)?;

    writer.force_merge(1)?;
    writer.close()?;
    let test_reader = directory_reader::open(test_dir.as_directory())?;
    let result = (|| -> Result<()> {
        let test_searcher = LuceneTestCase::new_searcher(Arc::clone(&test_reader));

        let vec_query_a = knn_query("emb_a", &query_a, 3)?;
        let vec_query_b = knn_query("emb_b", &query_b, 3)?;

        let multi_vec_query = loq(vec![vec_query_a, vec_query_b], 0.5)?;

        let hits = hits(&test_searcher, &multi_vec_query, 3)?;
        assert_equals_msg(Some("all 3 docs should match"), &3, &hits.len());
        assert_equals_msg(
            Some("doc0 (close to both) should rank first"),
            &0,
            &hits[0].doc,
        );
        Ok(())
    })();
    test_reader.close()?;
    test_dir.close()?;
    result
}

// ---- Boolean logic combinations ----

fn test_boolean_must_with_hybrid(f: &mut LogOddsFusion) -> Result<()> {
    with_hybrid_index(f, |hybrid_searcher| {
        let text_query = btq("body", "lucene");
        let vector_query = knn_query("embedding", &QUERY_A, 10)?;

        let hybrid_query = loq(vec![text_query, vector_query], 0.5)?;

        let mut filtered = BooleanQueryBuilder::new();
        filtered.add(tq("body", "lucene"), Occur::FILTER)?;
        filtered.add(hybrid_query, Occur::MUST)?;
        let filtered: Arc<dyn Query> = Arc::new(filtered.build());

        let hits = hits(hybrid_searcher, &filtered, 10)?;
        for hit in &hits {
            Assert::assert_true_msg("only docs 0 and 1 match text", hit.doc == 0 || hit.doc == 1);
        }
        Assert::assert_true_msg("should have at least 1 hit", !hits.is_empty());
        Ok(())
    })
}

fn test_boolean_must_not_with_hybrid(f: &mut LogOddsFusion) -> Result<()> {
    with_hybrid_index(f, |hybrid_searcher| {
        let text_query = btq("body", "lucene");
        let vector_query = knn_query("embedding", &QUERY_A, 10)?;

        let hybrid_query = loq(vec![text_query, vector_query], 0.5)?;

        let exclude_filter = tq("body", "cooking");
        let mut exclude_query = BooleanQueryBuilder::new();
        exclude_query.add(hybrid_query, Occur::MUST)?;
        exclude_query.add(exclude_filter, Occur::MUST_NOT)?;
        let exclude_query: Arc<dyn Query> = Arc::new(exclude_query.build());

        let hits = hits(hybrid_searcher, &exclude_query, 10)?;
        for hit in &hits {
            if hit.doc == 2 {
                randomizedtesting::junit::Assert::fail(
                    "doc2 (cooking) should be excluded. Actual: 2",
                );
            }
        }
        Ok(())
    })
}

fn test_boolean_should_with_hybrid(f: &mut LogOddsFusion) -> Result<()> {
    with_hybrid_index(f, |hybrid_searcher| {
        let text_lucene = btq("body", "lucene");
        let text_cooking = btq("body", "cooking");
        let vec_a = knn_query("embedding", &QUERY_A, 10)?;

        let hybrid1 = loq(vec![text_lucene, Arc::clone(&vec_a)], 0.5)?;
        let hybrid2 = loq(vec![text_cooking, vec_a], 0.5)?;

        let mut or_query = BooleanQueryBuilder::new();
        or_query.add(hybrid1, Occur::SHOULD)?;
        or_query.add(hybrid2, Occur::SHOULD)?;
        let or_query: Arc<dyn Query> = Arc::new(or_query.build());

        let hits = hits(hybrid_searcher, &or_query, 20)?;
        Assert::assert_true_msg("should have hits", !hits.is_empty());

        let mut found_doc0 = false;
        let mut found_doc2 = false;
        for hit in &hits {
            if hit.doc == 0 {
                found_doc0 = true;
            }
            if hit.doc == 2 {
                found_doc2 = true;
            }
        }
        Assert::assert_true_msg("doc0 (lucene + near vector) should appear", found_doc0);
        Assert::assert_true_msg("doc2 (cooking + near vector) should appear", found_doc2);
        Ok(())
    })
}

fn test_vector_and_vector(_f: &mut LogOddsFusion) -> Result<()> {
    let query_x = normalize(&[1.0, 0.0]);
    let query_y = normalize(&[0.0, 1.0]);

    let mut test_dir = LuceneTestCase::new_directory();
    let config = default_codec_config()?;
    let writer = IndexWriter::new(test_dir.as_directory(), config)?;

    let mut doc0 = Document::new();
    doc0.add(knn_field("emb", &[0.7, 0.7])?);
    writer.add_document(&doc0)?;

    let mut doc1 = Document::new();
    doc1.add(knn_field("emb", &[0.95, 0.05])?);
    writer.add_document(&doc1)?;

    let mut doc2 = Document::new();
    doc2.add(knn_field("emb", &[0.05, 0.95])?);
    writer.add_document(&doc2)?;

    writer.force_merge(1)?;
    writer.close()?;
    let test_reader = directory_reader::open(test_dir.as_directory())?;
    let result = (|| -> Result<()> {
        let test_searcher = LuceneTestCase::new_searcher(Arc::clone(&test_reader));

        let vec_x = knn_query("emb", &query_x, 3)?;
        let vec_y = knn_query("emb", &query_y, 3)?;

        let mut vec_and = BooleanQueryBuilder::new();
        vec_and.add(vec_x, Occur::MUST)?;
        vec_and.add(vec_y, Occur::MUST)?;
        let vec_and: Arc<dyn Query> = Arc::new(vec_and.build());

        let hits = hits(&test_searcher, &vec_and, 3)?;
        assert_equals_msg(Some("all 3 docs should match"), &3, &hits.len());
        assert_equals_msg(Some("doc0 (near both) should rank first"), &0, &hits[0].doc);
        Ok(())
    })();
    test_reader.close()?;
    test_dir.close()?;
    result
}

fn test_vector_not(f: &mut LogOddsFusion) -> Result<()> {
    with_hybrid_index(f, |hybrid_searcher| {
        let text_query = tq("body", "lucene");
        let vec_near = knn_query("embedding", &QUERY_A, 10)?;

        let mut text_not_vector = BooleanQueryBuilder::new();
        text_not_vector.add(text_query, Occur::MUST)?;
        text_not_vector.add(vec_near, Occur::MUST_NOT)?;
        let text_not_vector: Arc<dyn Query> = Arc::new(text_not_vector.build());

        let hits = hits(hybrid_searcher, &text_not_vector, 10)?;
        for hit in &hits {
            if hit.doc == 0 {
                randomizedtesting::junit::Assert::fail(
                    "doc0 should be excluded (near QUERY_A). Actual: 0",
                );
            }
        }
        Ok(())
    })
}

// ---- Weighted fusion tests ----

fn test_weighted_fusion(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");

    let loq = wloq(vec![q1, q2], 0.5, &[0.7, 0.3])?;
    QueryUtils::check_searcher(&*random(), &loq, Some(f.searcher()));

    let hits = hits(f.searcher(), &loq, 10)?;
    Assert::assert_true_msg("should have at least 1 hit", !hits.is_empty());
    for hit in &hits {
        Assert::assert_true_msg("score should be > 0", hit.score > 0.0);
        Assert::assert_true_msg("score should be < 1", hit.score < 1.0);
    }
    Ok(())
}

fn test_weighted_fusion_affects_ranking(f: &mut LogOddsFusion) -> Result<()> {
    let q_alpha = btq("body", "alpha");
    let q_beta = btq("body", "beta");

    // doc0: alpha beta gamma -> matches both
    // doc1: alpha gamma delta -> matches alpha only
    // doc2: beta gamma delta -> matches beta only

    // Heavy weight on alpha: doc1 (alpha-only) should rank above doc2 (beta-only)
    let alpha_heavy = wloq(
        vec![Arc::clone(&q_alpha), Arc::clone(&q_beta)],
        0.5,
        &[0.9, 0.1],
    )?;
    let hits_alpha = hits(f.searcher(), &alpha_heavy, 10)?;

    // Heavy weight on beta: doc2 (beta-only) should rank above doc1 (alpha-only)
    let beta_heavy = wloq(vec![q_alpha, q_beta], 0.5, &[0.1, 0.9])?;
    let hits_beta = hits(f.searcher(), &beta_heavy, 10)?;

    // Find scores of doc1 and doc2 in each case
    let (mut doc1_alpha_heavy, mut doc2_alpha_heavy) = (0f32, 0f32);
    let (mut doc1_beta_heavy, mut doc2_beta_heavy) = (0f32, 0f32);
    for hit in &hits_alpha {
        if hit.doc == 1 {
            doc1_alpha_heavy = hit.score;
        }
        if hit.doc == 2 {
            doc2_alpha_heavy = hit.score;
        }
    }
    for hit in &hits_beta {
        if hit.doc == 1 {
            doc1_beta_heavy = hit.score;
        }
        if hit.doc == 2 {
            doc2_beta_heavy = hit.score;
        }
    }

    Assert::assert_true_msg(
        "alpha-heavy: doc1 (alpha) > doc2 (beta)",
        doc1_alpha_heavy > doc2_alpha_heavy,
    );
    Assert::assert_true_msg(
        "beta-heavy: doc2 (beta) > doc1 (alpha)",
        doc2_beta_heavy > doc1_beta_heavy,
    );
    Ok(())
}

fn test_weighted_explanation(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let loq = wloq(vec![q1, q2], 0.5, &[0.6, 0.4])?;

    let expls = explain_on_first_leaf(f.searcher(), loq, &[0])?;
    let expl = &expls[0];
    Assert::assert_true_msg("doc0 should match", expl.is_match());
    Assert::assert_true_msg(
        "should mention weighted",
        expl.description().contains("weighted"),
    );
    Ok(())
}

fn test_weighted_equals_and_hash_code(_f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");

    let a = wloq(vec![Arc::clone(&q1), Arc::clone(&q2)], 0.5, &[0.7, 0.3])?;
    let b = wloq(vec![Arc::clone(&q1), Arc::clone(&q2)], 0.5, &[0.7, 0.3])?;
    let c = wloq(vec![Arc::clone(&q1), Arc::clone(&q2)], 0.5, &[0.3, 0.7])?;
    let d = loq(vec![q1, q2], 0.5)?;

    assert_query_equals(&a, &b);
    assert_equals(&a.query_hash(), &b.query_hash());
    assert_query_not_equals(&a, &c);
    assert_query_not_equals(&a, &d);
    Ok(())
}

fn test_weighted_max_score_correctness(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let loq = wloq(vec![q1, q2], 0.5, &[0.7, 0.3])?;
    CheckHits::check_top_scores(&*random(), &loq, f.searcher())
}

fn test_weighted_to_string(_f: &mut LogOddsFusion) -> Result<()> {
    let q1 = tq("body", "alpha");
    let q2 = tq("body", "beta");
    let loq = wloq(vec![q1, q2], 0.5, &[0.7, 0.3])?;
    let str = loq.to_query_string("body");
    Assert::assert_true_msg("should contain LogOdds", str.contains("LogOdds"));
    Assert::assert_true_msg("should contain w=", str.contains("w="));
    Assert::assert_true_msg("should contain 0.7", str.contains("0.7"));
    Ok(())
}

fn test_weighted_rewrite(f: &mut LogOddsFusion) -> Result<()> {
    let sub1 = btq("body", "alpha");
    let sub2 = btq("body", "beta");
    let loq = wloq(vec![sub1, sub2], 0.5, &[0.6, 0.4])?;

    let rewritten = f.searcher().rewrite(loq)?;
    let rewritten_loq = rewritten.as_any().downcast_ref::<LogOddsFusionQuery>();
    Assert::assert_true_msg(
        "weighted rewrite should produce LogOddsFusionQuery",
        rewritten_loq.is_some(),
    );
    let rewritten_loq = rewritten_loq.expect("instanceof checked");
    Assert::assert_true_msg(
        "weights should be preserved",
        rewritten_loq.get_weights().is_some(),
    );
    let weights = rewritten_loq.get_weights().expect("checked");
    assert_equals_delta(0.6f32.into(), f64::from(weights[0]), f64::from(1e-6f32));
    assert_equals_delta(0.4f32.into(), f64::from(weights[1]), f64::from(1e-6f32));
    Ok(())
}

fn test_illegal_weights(_f: &mut LogOddsFusion) -> Result<()> {
    let q1 = tq("body", "alpha");
    let q2 = tq("body", "beta");
    let clauses = vec![q1, q2];

    // Wrong length
    expect_throws(IAE, || {
        LogOddsFusionQuery::with_weights(clauses.clone(), 0.5, Some(vec![1.0]))
    });

    // Negative weight
    expect_throws(IAE, || {
        LogOddsFusionQuery::with_weights(clauses.clone(), 0.5, Some(vec![-0.1, 1.1]))
    });

    // NaN weight
    expect_throws(IAE, || {
        LogOddsFusionQuery::with_weights(clauses.clone(), 0.5, Some(vec![f32::NAN, 0.5]))
    });

    // Sum != 1.0
    expect_throws(IAE, || {
        LogOddsFusionQuery::with_weights(clauses.clone(), 0.5, Some(vec![0.3, 0.3]))
    });
    Ok(())
}

fn test_weighted_query_utils(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let loq = wloq(vec![q1, q2], 0.5, &[0.7, 0.3])?;
    QueryUtils::check_searcher(&*random(), &loq, Some(f.searcher()));
    Ok(())
}

fn test_weighted_three_way_combination(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");
    let q3 = btq("body", "gamma");
    let loq = wloq(vec![q1, q2, q3], 0.5, &[0.5, 0.3, 0.2])?;
    QueryUtils::check_searcher(&*random(), &loq, Some(f.searcher()));

    let hits = hits(f.searcher(), &loq, 10)?;
    Assert::assert_true_msg("should have hits", !hits.is_empty());
    for hit in &hits {
        Assert::assert_true_msg("score in (0,1)", hit.score > 0.0 && hit.score < 1.0);
    }
    Ok(())
}

// ---- Logit normalization tests ----

fn test_normalized_fusion_produces_valid_scores(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");

    // logit bounds: signal 0 range [-3, 3], signal 1 range [-1, 1]
    let loq: Arc<dyn Query> = Arc::new(LogOddsFusionQuery::new(
        vec![q1, q2],
        0.5,
        Some(vec![0.6, 0.4]),
        Some(vec![-3.0, -1.0]),
        Some(vec![3.0, 1.0]),
    )?);

    let hits = hits(f.searcher(), &loq, 10)?;
    Assert::assert_true_msg("should have hits", !hits.is_empty());
    for hit in &hits {
        Assert::assert_true_msg(
            &format!("score in (0,1): {}", hit.score),
            hit.score > 0.0 && hit.score < 1.0,
        );
    }
    Ok(())
}

fn test_normalized_max_score_correctness(f: &mut LogOddsFusion) -> Result<()> {
    let q1 = btq("body", "alpha");
    let q2 = btq("body", "beta");

    let loq: Arc<dyn Query> = Arc::new(LogOddsFusionQuery::new(
        vec![q1, q2],
        0.5,
        Some(vec![0.7, 0.3]),
        Some(vec![-3.0, -1.0]),
        Some(vec![3.0, 1.0]),
    )?);
    CheckHits::check_top_scores(&*random(), &loq, f.searcher())
}

/// L2-normalize a float vector.
fn normalize(v: &[f32]) -> Vec<f32> {
    let mut norm = 0f64;
    for &x in v {
        norm += f64::from(x * x);
    }
    norm = norm.sqrt();
    let mut result = vec![0f32; v.len()];
    for i in 0..v.len() {
        result[i] = (f64::from(v[i]) / norm) as f32;
    }
    result
}
