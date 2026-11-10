//! Port of `lucene/core/src/test/org/apache/lucene/document/TestFeatureField.java`.
//!
//! **Divergence from Lucene 10.5.0 (several methods).** Java adds a
//! `FeatureField` (or `TextField`) to a `Document` once and calls
//! `setFeatureValue`/`setStringValue` on it between `addDocument` calls,
//! mutating the instance the document holds. The port's `Document` owns its
//! fields and exposes no mutable access, so each `addDocument` indexes the
//! document rebuilt with the fields' current values (same fields, same
//! order, same types): what reaches the writer is identical.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::feature_field::{
    decode_feature_value, FeatureField, LinearFunction, LogFunction, SaturationFunction,
    SigmoidFunction,
};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::postings_enum::POSTINGS_ENUM_FREQS;
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::similarities::{BM25Similarity, BulkSimScorer, SimScorer};
use rucene::search::{
    BooleanQueryBuilder, IndexSearcher, Query, ScoreMode, TermQuery, NO_MORE_DOCS,
};
use rucene::util::{ArrayUtil, BytesRef};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::search::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::hamcrest::{assert_contains_string, assert_equal_to};
use crate::core_support::{
    assert_array_equals_f32, assert_equals, assert_equals_delta, assert_equals_msg, assert_null,
    class, random, test,
};

/// `TestFeatureField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestFeatureField",
        vec![
            test("testBasics", test_basics),
            test("testExplanations", test_explanations),
            test("testLogSimScorer", test_log_sim_scorer),
            test("testLinearSimScorer", test_linear_sim_scorer),
            test("testSatuSimScorer", test_satu_sim_scorer),
            test("testSigmSimScorer", test_sigm_sim_scorer),
            test(
                "testComputePivotFeatureValue",
                test_compute_pivot_feature_value,
            ),
            test("testDemo", test_demo),
            test("testBasicsNonScoringCase", test_basics_non_scoring_case),
            test("testStoreTermVectors", test_store_term_vectors),
            test("testLinearBulkScorer", test_linear_bulk_scorer),
            test("testLogBulkScorer", test_log_bulk_scorer),
            test("testSaturationBulkScorer", test_saturation_bulk_scorer),
            test("testSigmoidBulkScorer", test_sigmoid_bulk_scorer),
        ],
    )
}

/// `round(f)`: rounds a float the same way that `FeatureField` rounds
/// feature values.
fn round(f: f32) -> f32 {
    let mut bits = f.to_bits() as i32;
    bits &= !0 << 15; // clear last 15 bits
    f32::from_bits(bits as u32)
}

/// `new RandomIndexWriter(random(), dir,
/// newIndexWriterConfig().setMergePolicy(newLogMergePolicy(random().nextBoolean())))`.
fn log_mp_writer(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    let mut config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    ))?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)
}

/// A document of `FeatureField("features", name, value)`s, in order.
fn features(fs: &[(&str, f32)]) -> Result<Document> {
    let mut doc = Document::new();
    for (name, value) in fs {
        doc.add(Box::new(FeatureField::new("features", *name, *value)?));
    }
    Ok(doc)
}

/// A document of `FeatureField("features", name, value, true)`s, in order.
fn features_tv(fs: &[(&str, f32)]) -> Result<Document> {
    let mut doc = Document::new();
    for (name, value) in fs {
        doc.add(Box::new(FeatureField::with_term_vectors(
            "features", *name, *value, true,
        )?));
    }
    Ok(doc)
}

/// `q.createWeight(searcher, mode, boost).scorer(context)`.
fn scorer_of(
    q: &Arc<dyn Query>,
    searcher: &IndexSearcher,
    mode: ScoreMode,
    boost: f32,
    context: &rucene::index::LeafReaderContext,
) -> Result<Box<dyn rucene::search::Scorer>> {
    let w = q.create_weight(searcher, mode, boost)?;
    Ok(w.scorer(context)?.expect("scorer"))
}

/// Port of `testBasics`.
fn test_basics() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = log_mp_writer(&dir)?;
    writer.add_document(&features(&[("pagerank", 10.0), ("urlLen", 1.0 / 24.0)])?)?;
    writer.add_document(&features(&[("pagerank", 100.0), ("urlLen", 1.0 / 20.0)])?)?;
    writer.add_document(&Document::new())?; // gap
    writer.add_document(&features(&[("pagerank", 1.0), ("urlLen", 1.0 / 100.0)])?)?;
    writer.add_document(&features(&[("pagerank", 42.0), ("urlLen", 1.0 / 23.0)])?)?;
    writer.force_merge(1)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let context = Arc::clone(searcher.get_index_reader()).leaves()?[0].clone();
    let field_info_has_tv = context
        .leaf_reader()
        .get_field_infos()
        .field_info("features")
        .expect("features")
        .has_term_vectors();
    Assert::assert_false(field_info_has_tv);

    let check = |q: Arc<dyn Query>, expected: [(i32, f32); 4]| -> Result<()> {
        let mut s = scorer_of(&q, &searcher, ScoreMode::TOP_SCORES, 2.0, &context)?;
        for (doc, score) in expected {
            assert_equals(&doc, &s.iterator().next_doc()?);
            assert_equals_delta(score as f64, s.score()? as f64, 0.0);
        }
        assert_equals(&NO_MORE_DOCS, &s.iterator().next_doc()?);
        Ok(())
    };

    let log = |v: f32| (6.0 * ((4.5f32 + v) as f64).ln()) as f32;
    check(
        FeatureField::new_log_query("features", "pagerank", 3.0, 4.5)?,
        [
            (0, log(10.0)),
            (1, log(100.0)),
            (3, log(1.0)),
            (4, log(42.0)),
        ],
    )?;

    check(
        FeatureField::new_linear_query("features", "pagerank", 3.0)?,
        [
            (0, (6.0f64 * 10.0) as f32),
            (1, (6.0f64 * 100.0) as f32),
            (3, 6.0),
            (4, (6.0f64 * 42.0) as f32),
        ],
    )?;

    let sat = |v: f32| 6f32 * (1.0 - 4.5f32 / (4.5f32 + v));
    check(
        FeatureField::new_saturation_query("features", "pagerank", 3.0, 4.5)?,
        [
            (0, sat(10.0)),
            (1, sat(100.0)),
            (3, sat(1.0)),
            (4, sat(42.0)),
        ],
    )?;

    let k_pa = (4.5f32 as f64).powf(0.6f32 as f64);
    let sig = |v: f64| (6.0 * (1.0 - k_pa / (k_pa + v.powf(0.6f32 as f64)))) as f32;
    check(
        FeatureField::new_sigmoid_query("features", "pagerank", 3.0, 4.5, 0.6)?,
        [
            (0, sig(10.0)),
            (1, sig(100.0)),
            (3, sig(1.0)),
            (4, sig(42.0)),
        ],
    )?;

    let p = 1f32 / 24.0;
    let url = |v: f32| 6f32 * (1.0 - p / (p + round(v)));
    check(
        FeatureField::new_saturation_query("features", "urlLen", 3.0, 1.0 / 24.0)?,
        [
            (0, url(1.0 / 24.0)),
            (1, url(1.0 / 20.0)),
            (3, url(1.0 / 100.0)),
            (4, url(1.0 / 23.0)),
        ],
    )?;

    // IOUtils.close(reader, dir)
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testExplanations`.
fn test_explanations() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = log_mp_writer(&dir)?;
    writer.add_document(&features(&[("pagerank", 10.0)])?)?;
    writer.add_document(&features(&[("pagerank", 100.0)])?)?;
    writer.add_document(&Document::new())?; // gap
    writer.add_document(&features(&[("pagerank", 1.0)])?)?;
    writer.add_document(&features(&[("pagerank", 42.0)])?)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let check = |q: Arc<dyn Query>| QueryUtils::check_searcher(&*random(), &q, Some(&searcher));

    check(FeatureField::new_log_query(
        "features", "pagerank", 1.0, 4.5,
    )?);
    check(FeatureField::new_linear_query("features", "pagerank", 1.0)?);
    check(FeatureField::new_saturation_query(
        "features", "pagerank", 1.0, 12.0,
    )?);
    check(FeatureField::new_sigmoid_query(
        "features", "pagerank", 1.0, 12.0, 0.6,
    )?);

    // Test boosts that are > 1
    check(FeatureField::new_log_query(
        "features", "pagerank", 3.0, 4.5,
    )?);
    check(FeatureField::new_linear_query("features", "pagerank", 3.0)?);
    check(FeatureField::new_saturation_query(
        "features", "pagerank", 3.0, 12.0,
    )?);
    check(FeatureField::new_sigmoid_query(
        "features", "pagerank", 3.0, 12.0, 0.6,
    )?);

    // Test boosts that are < 1
    check(FeatureField::new_log_query(
        "features", "pagerank", 0.2, 4.5,
    )?);
    check(FeatureField::new_linear_query("features", "pagerank", 0.2)?);
    check(FeatureField::new_saturation_query(
        "features", "pagerank", 0.2, 12.0,
    )?);
    check(FeatureField::new_sigmoid_query(
        "features", "pagerank", 0.2, 12.0, 0.6,
    )?);

    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testLogSimScorer`.
fn test_log_sim_scorer() -> Result<()> {
    do_test_sim_scorer(&LogFunction::new(4.5).scorer(3.0));
    Ok(())
}

/// Port of `testLinearSimScorer`.
fn test_linear_sim_scorer() -> Result<()> {
    do_test_sim_scorer(&LinearFunction::new().scorer(1.0));
    Ok(())
}

/// Port of `testSatuSimScorer`.
fn test_satu_sim_scorer() -> Result<()> {
    do_test_sim_scorer(&SaturationFunction::new("foo", "bar", Some(20.0)).scorer(3.0)?);
    Ok(())
}

/// Port of `testSigmSimScorer`.
fn test_sigm_sim_scorer() -> Result<()> {
    do_test_sim_scorer(&SigmoidFunction::new(20.0, 0.6).scorer(3.0));
    Ok(())
}

/// `doTestSimScorer(s)`.
fn do_test_sim_scorer(s: &dyn SimScorer) {
    let max_score = s.score(f32::MAX, 1);
    Assert::assert_true(max_score.is_finite()); // used to compute max scores
                                                // Test that the score doesn't decrease with freq
    for freq in 2..65536 {
        Assert::assert_true(s.score((freq - 1) as f32, 1) <= s.score(freq as f32, 1));
    }
    Assert::assert_true(s.score(65535.0, 1) <= max_score);
}

/// Port of `testComputePivotFeatureValue`.
fn test_compute_pivot_feature_value() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;

    // Make sure that we create a legal pivot on missing features
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let pivot = FeatureField::compute_pivot_feature_value(&searcher, "features", "pagerank")?;
    Assert::assert_true(pivot.is_finite());
    Assert::assert_true(pivot > 0.0);
    (reader as Arc<dyn IndexReader>).close()?;

    writer.add_document(&features(&[("pagerank", 10.0)])?)?;
    writer.add_document(&features(&[("pagerank", 100.0)])?)?;
    writer.add_document(&Document::new())?; // gap
    writer.add_document(&features(&[("pagerank", 1.0)])?)?;
    writer.add_document(&features(&[("pagerank", 42.0)])?)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let pivot = FeatureField::compute_pivot_feature_value(&searcher, "features", "pagerank")?;
    let expected = ((10 * 100 * 42) as f64).powf(1.0 / 4.0); // geometric mean
    assert_equals_delta(expected, pivot as f64, 0.1);

    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testDemo`.
fn test_demo() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = log_mp_writer(&dir)?;
    let demo_doc = |pagerank: f32, body: &str| -> Result<Document> {
        let mut doc = Document::new();
        doc.add(Box::new(FeatureField::new(
            "features", "pagerank", pagerank,
        )?));
        doc.add(Box::new(TextField::new(
            "body",
            body.to_string(),
            Store::NO,
        )?));
        Ok(doc)
    };
    writer.add_document(&demo_doc(10.0, "Apache Lucene")?)?;
    writer.add_document(&demo_doc(1000.0, "Apache Web HTTP server")?)?;
    writer.add_document(&demo_doc(1.0, "Lucene is a search engine")?)?;
    writer.add_document(&demo_doc(42.0, "Lucene in the sky with diamonds")?)?;

    let reader = writer.get_reader()?;
    writer.close()?;

    // NOTE: If you need to make changes below, then you likely also need to
    // update javadocs of FeatureField.

    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(BM25Similarity::new()));
    let mut b = BooleanQueryBuilder::new();
    b.add(
        Arc::new(TermQuery::new(Term::from_text("body", "apache"))),
        Occur::SHOULD,
    )?;
    b.add(
        Arc::new(TermQuery::new(Term::from_text("body", "lucene"))),
        Occur::SHOULD,
    )?;
    let query: Arc<dyn Query> = Arc::new(b.build());
    let boost = FeatureField::new_saturation_query_auto_pivot("features", "pagerank")?;
    let mut b = BooleanQueryBuilder::new();
    b.add(query, Occur::MUST)?;
    b.add(boost, Occur::SHOULD)?;
    let boosted_query: Arc<dyn Query> = Arc::new(b.build());
    let top_docs = searcher.search(boosted_query, 10)?;
    assert_equals(&4, &top_docs.score_docs.len());
    assert_equals(&1, &top_docs.score_docs[0].doc);
    assert_equals(&0, &top_docs.score_docs[1].doc);
    assert_equals(&3, &top_docs.score_docs[2].doc);
    assert_equals(&2, &top_docs.score_docs[3].doc);

    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testBasicsNonScoringCase`.
fn test_basics_non_scoring_case() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let reader;
    {
        let writer = log_mp_writer(&dir)?;
        writer.add_document(&features(&[("pagerank", 10.0), ("urlLen", 1.0 / 24.0)])?)?;
        writer.add_document(&features(&[("pagerank", 100.0), ("urlLen", 1.0 / 20.0)])?)?;
        writer.add_document(&Document::new())?; // gap
        writer.add_document(&features(&[("pagerank", 1.0), ("urlLen", 1.0 / 100.0)])?)?;
        writer.add_document(&features(&[("pagerank", 42.0), ("urlLen", 1.0 / 23.0)])?)?;
        // `urlLenDoc.add(urlLength)`: the same field, at its current value.
        writer.add_document(&features(&[("urlLen", 1.0 / 23.0)])?)?;
        // `pageRankDoc.add(pagerank)`.
        writer.add_document(&features(&[("pagerank", 42.0)])?)?;
        writer.force_merge(1)?;
        reader = writer.get_reader()?;
        writer.close()?;
    }
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let context = Arc::clone(searcher.get_index_reader()).leaves()?[0].clone();

    let run = |qs: Vec<Arc<dyn Query>>, last: i32| -> Result<()> {
        for q in qs {
            let mut s = scorer_of(&q, &searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0, &context)?;
            let label = q.to_query_string("");
            for doc in [0, 1, 3, 4, last, NO_MORE_DOCS] {
                assert_equals_msg(Some(&label), &doc, &s.iterator().next_doc()?);
            }
        }
        Ok(())
    };
    run(
        vec![
            FeatureField::new_log_query("features", "pagerank", 3.0, 4.5)?,
            FeatureField::new_linear_query("features", "pagerank", 2.0)?,
            FeatureField::new_saturation_query("features", "pagerank", 3.0, 4.5)?,
            FeatureField::new_sigmoid_query("features", "pagerank", 3.0, 4.5, 0.6)?,
        ],
        6,
    )?;
    run(
        vec![
            FeatureField::new_log_query("features", "urlLen", 3.0, 4.5)?,
            FeatureField::new_linear_query("features", "urlLen", 2.0)?,
            FeatureField::new_saturation_query("features", "urlLen", 3.0, 4.5)?,
            FeatureField::new_sigmoid_query("features", "urlLen", 3.0, 4.5, 0.6)?,
        ],
        5,
    )?;
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testStoreTermVectors`.
fn test_store_term_vectors() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = log_mp_writer(&dir)?;
    writer.add_document(&features_tv(&[("pagerank", 10.0), ("urlLen", 0.5)])?)?;
    writer.add_document(&Document::new())?; // gap
    writer.add_document(&features_tv(&[("pagerank", 42.0), ("urlLen", 1.5)])?)?;
    let mut doc = features_tv(&[("pagerank", 42.0), ("urlLen", 1.5)])?;
    doc.clear();
    doc.add(Box::new(FeatureField::with_term_vectors(
        "features", "pagerank", 1.0, false,
    )?));
    let exc =
        rucene_test_framework::util::java_exceptions::expect_throws("java.lang.Exception", || {
            writer.add_document(&doc)
        });
    let message = exc.message().unwrap_or_default().to_string();
    // `anyOf(containsString("store term vector"), containsString("storeTermVector"))`.
    if !message.contains("store term vector") {
        assert_contains_string(Some(&message), "storeTermVector");
    }
    writer.force_merge(1)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let context = Arc::clone(searcher.get_index_reader()).leaves()?[0].clone();
    let leaf = context.leaf_reader();
    let has_tv = leaf
        .get_field_infos()
        .field_info("features")
        .expect("features")
        .has_term_vectors();
    Assert::assert_true(has_tv);

    let tv = leaf.term_vectors()?;
    let terms = tv.get_field(0, "features")?.expect("terms");
    let mut terms_enum = terms.iterator()?;
    assert_equal_to_bytes(terms_enum.next()?, "pagerank");
    let mut postings = terms_enum.postings(None, POSTINGS_ENUM_FREQS)?;
    assert_equal_to(postings.next_doc()?, 0);
    assert_equal_to(decode_feature_value(postings.freq()? as f32), 10.0);
    assert_equal_to(postings.next_doc()?, NO_MORE_DOCS);
    assert_equal_to_bytes(terms_enum.next()?, "urlLen");
    postings = terms_enum.postings(Some(postings), POSTINGS_ENUM_FREQS)?;
    assert_equal_to(postings.next_doc()?, 0);
    assert_equal_to(decode_feature_value(postings.freq()? as f32), 0.5);
    assert_equal_to(postings.next_doc()?, NO_MORE_DOCS);

    let terms = tv.get_field(1, "features")?;
    assert_null(&terms.map(|_| "terms"));

    let terms = tv.get_field(2, "features")?.expect("terms");
    let mut terms_enum = terms.iterator()?;
    assert_equal_to_bytes(terms_enum.next()?, "pagerank");
    postings = terms_enum.postings(Some(postings), POSTINGS_ENUM_FREQS)?;
    assert_equal_to(postings.next_doc()?, 0);
    assert_equal_to(decode_feature_value(postings.freq()? as f32), 42.0);
    assert_equal_to(postings.next_doc()?, NO_MORE_DOCS);
    assert_equal_to_bytes(terms_enum.next()?, "urlLen");
    let mut postings = terms_enum.postings(None, POSTINGS_ENUM_FREQS)?;
    assert_equal_to(postings.next_doc()?, 0);
    assert_equal_to(decode_feature_value(postings.freq()? as f32), 1.5);
    assert_equal_to(postings.next_doc()?, NO_MORE_DOCS);

    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// `assertThat(termsEnum.next(), equalTo(new BytesRef(text)))`.
fn assert_equal_to_bytes(actual: Option<BytesRef>, text: &str) {
    assert_equals(&Some(BytesRef::from_char_sequence(text)), &actual);
}

/// Port of `testLinearBulkScorer`.
fn test_linear_bulk_scorer() -> Result<()> {
    let func = LinearFunction::new();
    let scorer = func.scorer(2.0); // weight = 2
    do_test_bulk_scorer(&scorer, scorer.as_bulk_sim_scorer());
    Ok(())
}

/// Port of `testLogBulkScorer`.
fn test_log_bulk_scorer() -> Result<()> {
    let func = LogFunction::new(4.5);
    let scorer = func.scorer(3.0); // weight = 3
    do_test_bulk_scorer(&scorer, scorer.as_bulk_sim_scorer());
    Ok(())
}

/// Port of `testSaturationBulkScorer`.
fn test_saturation_bulk_scorer() -> Result<()> {
    let func = SaturationFunction::new("foo", "bar", Some(4.5));
    let scorer = func.scorer(3.0)?;
    do_test_bulk_scorer(&scorer, scorer.as_bulk_sim_scorer());
    Ok(())
}

/// Port of `testSigmoidBulkScorer`.
fn test_sigmoid_bulk_scorer() -> Result<()> {
    let func = SigmoidFunction::new(4.5, 0.6);
    let scorer = func.scorer(3.0);
    do_test_bulk_scorer(&scorer, scorer.as_bulk_sim_scorer());
    Ok(())
}

/// `doTestBulkScorer(scorer, bulkScorer)`.
fn do_test_bulk_scorer(scorer: &dyn SimScorer, mut bulk_scorer: Box<dyn BulkSimScorer + '_>) {
    let random = random();
    let iters = LuceneTestCase::at_least(3);
    let mut freqs: Vec<f32> = Vec::new();
    let mut norms: Vec<i64> = Vec::new();
    let mut scores: Vec<f32> = Vec::new();
    for _iter in 0..iters {
        let size = TestUtil::next_int(&*random, 0, 200) as usize;
        if size > freqs.len() {
            freqs = vec![0.0; ArrayUtil::oversize(size, 4)];
            norms = vec![0; freqs.len()];
            scores = vec![0.0; freqs.len()];
        }
        for i in 0..size {
            freqs[i] = TestUtil::next_int(&*random, 1, 1000) as f32; // freq values
            norms[i] = TestUtil::next_long(&*random, 1, 255); // norms in byte range
        }
        let mut expected = vec![0f32; size];
        for i in 0..size {
            expected[i] = scorer.score(freqs[i], norms[i]);
        }
        bulk_scorer.score(size, &freqs, &norms, &mut scores);
        assert_array_equals_f32(&expected[..size], &scores[..size], 0.0);
    }
}
