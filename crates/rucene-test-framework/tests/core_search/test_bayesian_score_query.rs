//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBayesianScoreQuery.java`.
//!
//! Tests for `BayesianScoreQuery`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::reader_context::LeafReaderContext;
use rucene::index::IndexReader;
use rucene::search::bayesian_score_query::sigmoid;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::BM25Similarity;
use rucene::search::{
    sample_vocabulary_terms, BayesianScoreEstimator, BayesianScoreQuery, BooleanQueryBuilder,
    JavaRandom, Occur, Query, ScoreDoc, ScoreMode, Weight,
};
use rucene::util::BytesRef;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{
    close_dir, close_reader, fixture_class, ftest, set_up, tear_down, tq,
};
use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_equals, iae_message, random,
};

/// The instance of `TestBayesianScoreQuery`.
#[derive(Default)]
pub struct Bayesian {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
}

impl Bayesian {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }
}

/// `TestBayesianScoreQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<Bayesian>(
        "org.apache.lucene.search.TestBayesianScoreQuery",
        vec![
            set_up(bayesian_set_up),
            tear_down(bayesian_tear_down),
            ftest(
                "testScoresInProbabilityRange",
                test_scores_in_probability_range,
            ),
            ftest("testRankingPreserved", test_ranking_preserved),
            ftest(
                "testMultiTermQueryRankingPreserved",
                test_multi_term_query_ranking_preserved,
            ),
            ftest("testSigmoidProperties", test_sigmoid_properties),
            ftest("testIllegalParameters", test_illegal_parameters),
            ftest("testRewrite", test_rewrite),
            ftest("testEqualsAndHashCode", test_equals_and_hash_code),
            ftest("testExplanation", test_explanation),
            ftest("testQueryUtils", test_query_utils),
            ftest("testMaxScoreCorrectness", test_max_score_correctness),
            ftest("testToString", test_to_string),
            ftest("testCountDelegatesToInner", test_count_delegates_to_inner),
            ftest("testCountWithNoScoring", test_count_with_no_scoring),
            ftest("testDifferentAlphaBeta", test_different_alpha_beta),
            ftest("testBaseRateLowersScores", test_base_rate_lowers_scores),
            ftest("testBaseRateScoresInRange", test_base_rate_scores_in_range),
            ftest(
                "testBaseRateMaxScoreCorrectness",
                test_base_rate_max_score_correctness,
            ),
            ftest("testBaseRateExplanation", test_base_rate_explanation),
            ftest("testBaseRateQueryUtils", test_base_rate_query_utils),
            ftest(
                "testBaseRateEqualsAndHashCode",
                test_base_rate_equals_and_hash_code,
            ),
            ftest("testIllegalBaseRate", test_illegal_base_rate),
            ftest(
                "testEstimatorReturnsFiniteValues",
                test_estimator_returns_finite_values,
            ),
            ftest(
                "testEstimatorSamplesIndexedVocabularyFromUnstoredField",
                test_estimator_samples_indexed_vocabulary_from_unstored_field,
            ),
            ftest(
                "testEstimatedParametersProduceValidScores",
                test_estimated_parameters_produce_valid_scores,
            ),
            ftest(
                "testEstimatedMaxScoreCorrectness",
                test_estimated_max_score_correctness,
            ),
            ftest(
                "testEstimatorReproducibleWithSeed",
                test_estimator_reproducible_with_seed,
            ),
        ],
    )
}

fn text_doc(text: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "body",
        text.to_string(),
        Store::NO,
    )?));
    Ok(doc)
}

fn bayesian_set_up(f: &mut Bayesian) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;

    writer.add_document(&text_doc("alpha alpha alpha alpha alpha beta")?)?;
    writer.add_document(&text_doc("alpha gamma delta epsilon")?)?;
    writer.add_document(&text_doc("beta gamma delta epsilon zeta")?)?;

    for i in 0..50 {
        writer.add_document(&text_doc(&format!("filler content number {i}"))?)?;
    }

    writer.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_similarity(Arc::new(BM25Similarity::new()));
    f.searcher = Some(searcher);
    f.reader = Some(reader);
    f.dir = Some(dir);
    Ok(())
}

fn bayesian_tear_down(f: &mut Bayesian) -> Result<()> {
    drop(f.searcher.take());
    close_reader(f.reader.take())?;
    close_dir(f.dir.take())
}

fn alpha() -> Arc<dyn Query> {
    tq("body", "alpha")
}

fn bsq(inner: Arc<dyn Query>, a: f32, b: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BayesianScoreQuery::with_alpha_beta(inner, a, b)?))
}

fn bsq_rate(inner: Arc<dyn Query>, a: f32, b: f32, rate: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BayesianScoreQuery::new(inner, a, b, rate)?))
}

fn hits(f: &Bayesian, q: Arc<dyn Query>) -> Result<Vec<ScoreDoc>> {
    Ok(f.searcher().search(q, 10)?.score_docs)
}

fn first_leaf(f: &Bayesian) -> Result<Arc<LeafReaderContext>> {
    let leaves = Arc::clone(f.searcher().get_index_reader()).leaves()?;
    Ok(Arc::clone(&leaves[0]))
}

fn test_scores_in_probability_range(f: &mut Bayesian) -> Result<()> {
    let hits = hits(f, bsq(alpha(), 0.5, 5.0)?)?;
    Assert::assert_true_msg("should have hits", !hits.is_empty());

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
    Ok(())
}

fn test_ranking_preserved(f: &mut Bayesian) -> Result<()> {
    let bm25_query = alpha();
    let bm25_hits = hits(f, Arc::clone(&bm25_query))?;
    assert_equals_msg(
        Some("doc0 should rank first in BM25"),
        &0,
        &bm25_hits[0].doc,
    );
    assert_equals_msg(
        Some("doc1 should rank second in BM25"),
        &1,
        &bm25_hits[1].doc,
    );

    let bsq_hits = hits(f, bsq(bm25_query, 0.5, 5.0)?)?;
    assert_equals_msg(Some("doc0 should still rank first"), &0, &bsq_hits[0].doc);
    assert_equals_msg(Some("doc1 should still rank second"), &1, &bsq_hits[1].doc);
    Assert::assert_true_msg(
        &format!(
            "doc0 ({}) > doc1 ({})",
            bsq_hits[0].score, bsq_hits[1].score
        ),
        bsq_hits[0].score > bsq_hits[1].score,
    );
    Ok(())
}

fn test_multi_term_query_ranking_preserved(f: &mut Bayesian) -> Result<()> {
    let mut b = BooleanQueryBuilder::new();
    b.add(tq("body", "alpha"), Occur::SHOULD)?;
    b.add(tq("body", "beta"), Occur::SHOULD)?;
    let bm25_query: Arc<dyn Query> = Arc::new(b.build());

    let bm25_hits = hits(f, Arc::clone(&bm25_query))?;
    let bsq_hits = hits(f, bsq(bm25_query, 0.3, 8.0)?)?;

    assert_equals_msg(
        Some("same number of hits"),
        &bm25_hits.len(),
        &bsq_hits.len(),
    );

    for i in 0..bm25_hits.len() {
        assert_equals_msg(
            Some(&format!("ranking should be preserved at position {i}")),
            &bm25_hits[i].doc,
            &bsq_hits[i].doc,
        );
    }

    for hit in &bsq_hits {
        Assert::assert_true_msg("score should be > 0", hit.score > 0.0);
        Assert::assert_true_msg("score should be < 1", hit.score < 1.0);
    }
    Ok(())
}

fn test_sigmoid_properties(_f: &mut Bayesian) -> Result<()> {
    Assert::assert_equals_float_msg(None, 0.5, sigmoid(0.0), 1e-6);

    let mut prev = sigmoid(-50.0);
    let mut x = -49.0f32;
    while x <= 50.0 {
        let curr = sigmoid(x);
        Assert::assert_true_msg("sigmoid should be monotone", curr >= prev);
        prev = curr;
        x += 1.0;
    }

    let large = sigmoid(100.0);
    Assert::assert_true_msg("sigmoid(100) near 1", large > 0.99 && large <= 1.0);
    let small = sigmoid(-100.0);
    Assert::assert_true_msg("sigmoid(-100) near 0", (0.0..0.01).contains(&small));
    Ok(())
}

fn test_illegal_parameters(_f: &mut Bayesian) -> Result<()> {
    let q = alpha();

    iae_message(|| BayesianScoreQuery::with_alpha_beta(Arc::clone(&q), -1.0, 5.0));
    iae_message(|| BayesianScoreQuery::with_alpha_beta(Arc::clone(&q), 0.0, 5.0));
    iae_message(|| BayesianScoreQuery::with_alpha_beta(Arc::clone(&q), f32::INFINITY, 5.0));
    iae_message(|| BayesianScoreQuery::with_alpha_beta(Arc::clone(&q), 1.0, f32::NAN));
    Ok(())
}

fn test_rewrite(f: &mut Bayesian) -> Result<()> {
    let bsq = bsq(alpha(), 0.5, 5.0)?;

    let rewritten = f.searcher().rewrite(bsq)?;
    Assert::assert_true_msg(
        // `rewritten.getClass().getName()`: the query's string form stands
        // for the class name, which a `dyn Query` does not carry.
        &format!(
            "should rewrite to BayesianScoreQuery, got {}",
            rucene::search::query::query_to_string(&*rewritten)
        ),
        rewritten.as_any().is::<BayesianScoreQuery>(),
    );
    Ok(())
}

/// `assertEquals(a, b)` / `assertNotEquals(a, b)` over `Query.equals`.
fn query_eq(a: &Arc<dyn Query>, b: &Arc<dyn Query>) -> bool {
    a.query_eq(&**b)
}

fn test_equals_and_hash_code(_f: &mut Bayesian) -> Result<()> {
    let q1 = alpha();
    let q2 = tq("body", "beta");

    let a = bsq(Arc::clone(&q1), 0.5, 5.0)?;
    let b = bsq(Arc::clone(&q1), 0.5, 5.0)?;
    let c = bsq(Arc::clone(&q1), 0.3, 5.0)?;
    let d = bsq(q2, 0.5, 5.0)?;

    assert_equals(&true, &query_eq(&a, &b));
    assert_equals(&a.query_hash(), &b.query_hash());
    assert_not_equals(&true, &query_eq(&a, &c));
    assert_not_equals(&true, &query_eq(&a, &d));
    Ok(())
}

fn create_weight(f: &Bayesian, q: Arc<dyn Query>, mode: ScoreMode) -> Result<Arc<dyn Weight>> {
    let s = f.searcher();
    s.create_weight(s.rewrite(q)?, mode, 1.0)
}

fn test_explanation(f: &mut Bayesian) -> Result<()> {
    let w = create_weight(f, bsq(alpha(), 0.5, 5.0)?, ScoreMode::COMPLETE)?;
    let context = first_leaf(f)?;

    let expl = w.explain(&context, 0)?;
    Assert::assert_true_msg("should match", expl.is_match());
    Assert::assert_true_msg(
        "should mention sigmoid",
        expl.description().contains("sigmoid"),
    );
    let score = expl.value().float_value();
    Assert::assert_true_msg("score in (0,1)", score > 0.0 && score < 1.0);

    let no_match = w.explain(&context, 2)?;
    Assert::assert_false_msg("should not match", no_match.is_match());
    Ok(())
}

fn test_query_utils(f: &mut Bayesian) -> Result<()> {
    let bsq = bsq(alpha(), 0.5, 5.0)?;
    QueryUtils::check_searcher(&*random(), &bsq, Some(f.searcher()));
    Ok(())
}

fn test_max_score_correctness(f: &mut Bayesian) -> Result<()> {
    let bsq = bsq(alpha(), 0.5, 5.0)?;
    CheckHits::check_top_scores(&*random(), &bsq, f.searcher())
}

fn test_to_string(_f: &mut Bayesian) -> Result<()> {
    let bsq = bsq(alpha(), 0.5, 5.0)?;
    let s = bsq.to_query_string("body");
    Assert::assert_true_msg("should contain BayesianScore", s.contains("BayesianScore"));
    Assert::assert_true_msg("should contain alpha", s.contains("alpha"));
    Ok(())
}

fn test_count_delegates_to_inner(f: &mut Bayesian) -> Result<()> {
    let inner = alpha();
    let w = create_weight(f, bsq(Arc::clone(&inner), 0.5, 5.0)?, ScoreMode::COMPLETE)?;
    let context = first_leaf(f)?;

    let count = w.count(&context)?;
    assert_equals(&2, &count);

    let inner_w = f
        .searcher()
        .create_weight(inner, ScoreMode::COMPLETE, 1.0)?;
    assert_equals(&inner_w.count(&context)?, &count);
    Ok(())
}

fn test_count_with_no_scoring(f: &mut Bayesian) -> Result<()> {
    let w = create_weight(f, bsq(alpha(), 0.5, 5.0)?, ScoreMode::COMPLETE_NO_SCORES)?;
    let context = first_leaf(f)?;

    let count = w.count(&context)?;
    assert_equals(&2, &count);
    Ok(())
}

fn test_different_alpha_beta(f: &mut Bayesian) -> Result<()> {
    let inner = alpha();

    let gentle_hits = hits(f, bsq(Arc::clone(&inner), 0.1, 5.0)?)?;
    let steep_hits = hits(f, bsq(inner, 2.0, 5.0)?)?;

    for hit in &gentle_hits {
        Assert::assert_true_msg("gentle score in (0,1)", hit.score > 0.0 && hit.score < 1.0);
    }
    for hit in &steep_hits {
        Assert::assert_true_msg("steep score in (0,1)", hit.score > 0.0 && hit.score < 1.0);
    }

    assert_equals_msg(
        Some("same top doc"),
        &gentle_hits[0].doc,
        &steep_hits[0].doc,
    );
    Ok(())
}

fn test_base_rate_lowers_scores(f: &mut Bayesian) -> Result<()> {
    let inner = alpha();

    let hits_no = hits(f, bsq(Arc::clone(&inner), 0.5, 3.0)?)?;
    let hits_br = hits(f, bsq_rate(inner, 0.5, 3.0, 0.01)?)?;

    Assert::assert_true_msg(
        "both should have hits",
        !hits_no.is_empty() && !hits_br.is_empty(),
    );

    assert_equals_msg(Some("same top doc"), &hits_no[0].doc, &hits_br[0].doc);

    for i in 0..hits_no.len().min(hits_br.len()) {
        if hits_no[i].doc == hits_br[i].doc {
            Assert::assert_true_msg(
                &format!(
                    "base rate 0.01 should lower score: {} < {}",
                    hits_br[i].score, hits_no[i].score
                ),
                hits_br[i].score < hits_no[i].score,
            );
        }
    }
    Ok(())
}

fn test_base_rate_scores_in_range(f: &mut Bayesian) -> Result<()> {
    let hits = hits(f, bsq_rate(alpha(), 0.5, 3.0, 0.001)?)?;
    for hit in &hits {
        Assert::assert_true_msg(
            &format!("score in (0,1): {}", hit.score),
            hit.score > 0.0 && hit.score < 1.0,
        );
    }
    Ok(())
}

fn test_base_rate_max_score_correctness(f: &mut Bayesian) -> Result<()> {
    let bsq = bsq_rate(alpha(), 0.5, 3.0, 0.01)?;
    CheckHits::check_top_scores(&*random(), &bsq, f.searcher())
}

fn test_base_rate_explanation(f: &mut Bayesian) -> Result<()> {
    let w = create_weight(f, bsq_rate(alpha(), 0.5, 3.0, 0.05)?, ScoreMode::COMPLETE)?;
    let ctx = first_leaf(f)?;
    let expl = w.explain(&ctx, 0)?;
    Assert::assert_true_msg("should match", expl.is_match());
    Assert::assert_true_msg(
        "should mention base rate",
        expl.description().contains("base rate"),
    );
    Ok(())
}

fn test_base_rate_query_utils(f: &mut Bayesian) -> Result<()> {
    let bsq = bsq_rate(alpha(), 0.5, 3.0, 0.01)?;
    QueryUtils::check_searcher(&*random(), &bsq, Some(f.searcher()));
    Ok(())
}

fn test_base_rate_equals_and_hash_code(_f: &mut Bayesian) -> Result<()> {
    let inner = alpha();
    let a = bsq_rate(Arc::clone(&inner), 0.5, 3.0, 0.01)?;
    let b = bsq_rate(Arc::clone(&inner), 0.5, 3.0, 0.01)?;
    let c = bsq_rate(Arc::clone(&inner), 0.5, 3.0, 0.05)?;
    let d = bsq(inner, 0.5, 3.0)?;

    assert_equals(&true, &query_eq(&a, &b));
    assert_equals(&a.query_hash(), &b.query_hash());
    assert_not_equals(&true, &query_eq(&a, &c));
    assert_not_equals(&true, &query_eq(&a, &d));
    Ok(())
}

fn test_illegal_base_rate(_f: &mut Bayesian) -> Result<()> {
    let inner = alpha();
    iae_message(|| BayesianScoreQuery::new(Arc::clone(&inner), 0.5, 3.0, -0.1));
    iae_message(|| BayesianScoreQuery::new(Arc::clone(&inner), 0.5, 3.0, 1.0));
    Ok(())
}

fn test_estimator_returns_finite_values(f: &mut Bayesian) -> Result<()> {
    let params = BayesianScoreEstimator::estimate(f.searcher(), "body")?;

    Assert::assert_true_msg(
        "alpha should be positive and finite",
        params.alpha > 0.0 && params.alpha.is_finite(),
    );
    Assert::assert_true_msg("beta should be finite", params.beta.is_finite());
    Assert::assert_true_msg(
        "beta should reflect matching indexed terms",
        params.beta > 0.0,
    );
    Assert::assert_true_msg(
        "baseRate in (0, 0.5]",
        params.base_rate > 0.0 && params.base_rate <= 0.5,
    );
    Ok(())
}

fn test_estimator_samples_indexed_vocabulary_from_unstored_field(_f: &mut Bayesian) -> Result<()> {
    let prefix_dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(prefix_dir.as_directory(), IndexWriterConfig::new())?;
    for i in 0..20 {
        writer.add_document(&text_doc(&format!(
            "license header boilerplate sharedprefix topic{i}"
        ))?)?;
    }
    writer.force_merge(1)?;
    let prefix_reader = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;

    let result = (|| -> Result<()> {
        let reader: Arc<dyn IndexReader> = Arc::clone(&prefix_reader) as Arc<dyn IndexReader>;
        let sampled_terms = sample_vocabulary_terms(&reader, "body", 100, &mut JavaRandom::new(7))?;
        Assert::assert_true_msg(
            "sample should include indexed suffix term topic0",
            contains_term(&sampled_terms, "topic0"),
        );
        Assert::assert_true_msg(
            "sample should include indexed suffix term topic19",
            contains_term(&sampled_terms, "topic19"),
        );

        let mut prefix_searcher = IndexSearcher::new(reader)?;
        prefix_searcher.set_similarity(Arc::new(BM25Similarity::new()));
        let params = BayesianScoreEstimator::estimate_with(&prefix_searcher, "body", 4, 3, 7)?;
        Assert::assert_true_msg(
            "estimation should use indexed terms from non-stored fields",
            params.beta > 0.0,
        );
        Ok(())
    })();
    // finally
    let closed = close_reader(Some(prefix_reader)).and(close_dir(Some(prefix_dir)));
    result.and(closed)
}

fn test_estimated_parameters_produce_valid_scores(f: &mut Bayesian) -> Result<()> {
    let params = BayesianScoreEstimator::estimate(f.searcher(), "body")?;

    let bsq = bsq_rate(alpha(), params.alpha, params.beta, params.base_rate)?;

    let hits = hits(f, bsq)?;
    Assert::assert_true_msg("should have hits", !hits.is_empty());
    for hit in &hits {
        Assert::assert_true_msg(
            &format!("score in (0,1): {}", hit.score),
            hit.score > 0.0 && hit.score < 1.0,
        );
    }
    Ok(())
}

fn test_estimated_max_score_correctness(f: &mut Bayesian) -> Result<()> {
    let params = BayesianScoreEstimator::estimate(f.searcher(), "body")?;

    let bsq = bsq_rate(alpha(), params.alpha, params.beta, params.base_rate)?;
    CheckHits::check_top_scores(&*random(), &bsq, f.searcher())
}

fn test_estimator_reproducible_with_seed(f: &mut Bayesian) -> Result<()> {
    let p1 = BayesianScoreEstimator::estimate_with(f.searcher(), "body", 20, 3, 123)?;
    let p2 = BayesianScoreEstimator::estimate_with(f.searcher(), "body", 20, 3, 123)?;

    Assert::assert_equals_float_msg(
        Some("same seed should produce same alpha"),
        p1.alpha,
        p2.alpha,
        0.0,
    );
    Assert::assert_equals_float_msg(
        Some("same seed should produce same beta"),
        p1.beta,
        p2.beta,
        0.0,
    );
    Assert::assert_equals_float_msg(
        Some("same seed should produce same baseRate"),
        p1.base_rate,
        p2.base_rate,
        0.0,
    );
    Ok(())
}

fn contains_term(terms: &[BytesRef], expected: &str) -> bool {
    for term in terms {
        if term.utf8_to_string().is_ok_and(|t| t == expected) {
            return true;
        }
    }
    false
}
