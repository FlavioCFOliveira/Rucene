//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTopDocsRRF.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::total_hits::{TotalHits, TotalHitsRelation};
use rucene::search::{ScoreDoc, TopDocs};
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, test, IAE};

/// `TestTopDocsRRF`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTopDocsRRF",
        vec![
            test("testBasics", test_basics),
            test("testShardIndex", test_shard_index),
            test("testInconsistentShardIndex", test_inconsistent_shard_index),
            test("testInvalidTopN", test_invalid_top_n),
            test("testInvalidK", test_invalid_k),
        ],
    )
}

/// `new TotalHits(value, TotalHits.Relation.GREATER_THAN_OR_EQUAL_TO)`.
fn gte(value: i64) -> Result<TotalHits> {
    TotalHits::new(value, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)
}

/// `assertEquals(expected, actual, 0f)`.
fn assert_score(expected: f32, actual: f32) {
    Assert::assert_equals_float_msg(None, expected, actual, 0.0);
}

/// Port of `TestTopDocsRRF.testBasics`.
fn test_basics() -> Result<()> {
    let td1 = TopDocs::new(
        gte(100)?,
        vec![
            ScoreDoc::new(42, 10.0),
            ScoreDoc::new(10, 5.0),
            ScoreDoc::new(20, 3.0),
        ],
    );
    let td2 = TopDocs::new(
        gte(80)?,
        vec![ScoreDoc::new(10, 10.0), ScoreDoc::new(20, 5.0)],
    );

    let rrf_td = TopDocs::rrf(3, 20, &[td1, td2])?;
    assert_equals(&gte(100)?, &rrf_td.total_hits);

    let rrf_score_docs = &rrf_td.score_docs;
    assert_equals(&3, &rrf_score_docs.len());

    assert_equals(&10, &rrf_score_docs[0].doc);
    assert_equals(&-1, &rrf_score_docs[0].shard_index);
    assert_score(
        (1.0 / f64::from(20 + 2) + 1.0 / f64::from(20 + 1)) as f32,
        rrf_score_docs[0].score,
    );

    assert_equals(&20, &rrf_score_docs[1].doc);
    assert_equals(&-1, &rrf_score_docs[1].shard_index);
    assert_score(
        (1.0 / f64::from(20 + 3) + 1.0 / f64::from(20 + 2)) as f32,
        rrf_score_docs[1].score,
    );

    assert_equals(&42, &rrf_score_docs[2].doc);
    assert_equals(&-1, &rrf_score_docs[2].shard_index);
    assert_score((1.0 / f64::from(20 + 1)) as f32, rrf_score_docs[2].score);
    Ok(())
}

/// Port of `TestTopDocsRRF.testShardIndex`.
fn test_shard_index() -> Result<()> {
    let td1 = TopDocs::new(
        gte(100)?,
        vec![
            ScoreDoc::with_shard_index(42, 10.0, 0),
            ScoreDoc::with_shard_index(10, 5.0, 1),
            ScoreDoc::with_shard_index(20, 3.0, 0),
        ],
    );
    let td2 = TopDocs::new(
        gte(80)?,
        vec![
            ScoreDoc::with_shard_index(10, 10.0, 1),
            ScoreDoc::with_shard_index(20, 5.0, 1),
        ],
    );

    let rrf_td = TopDocs::rrf(3, 20, &[td1, td2])?;
    assert_equals(&gte(100)?, &rrf_td.total_hits);

    let rrf_score_docs = &rrf_td.score_docs;
    assert_equals(&3, &rrf_score_docs.len());

    assert_equals(&10, &rrf_score_docs[0].doc);
    assert_equals(&1, &rrf_score_docs[0].shard_index);
    assert_score(
        (1.0 / f64::from(20 + 2) + 1.0 / f64::from(20 + 1)) as f32,
        rrf_score_docs[0].score,
    );

    assert_equals(&42, &rrf_score_docs[1].doc);
    assert_equals(&0, &rrf_score_docs[1].shard_index);
    assert_score((1.0 / f64::from(20 + 1)) as f32, rrf_score_docs[1].score);

    assert_equals(&20, &rrf_score_docs[2].doc);
    assert_equals(&1, &rrf_score_docs[2].shard_index);
    assert_score((1.0 / f64::from(20 + 2)) as f32, rrf_score_docs[2].score);
    Ok(())
}

/// Port of `TestTopDocsRRF.testInconsistentShardIndex`.
fn test_inconsistent_shard_index() -> Result<()> {
    let td1 = TopDocs::new(
        gte(100)?,
        vec![
            ScoreDoc::with_shard_index(42, 10.0, 0),
            ScoreDoc::with_shard_index(10, 5.0, 1),
            ScoreDoc::with_shard_index(20, 3.0, 0),
        ],
    );
    let td2 = TopDocs::new(
        gte(80)?,
        vec![
            ScoreDoc::with_shard_index(10, 10.0, -1),
            ScoreDoc::with_shard_index(20, 5.0, -1),
        ],
    );

    let e = expect_throws(IAE, || TopDocs::rrf(3, 20, &[td1, td2]));
    Assert::assert_true(e.message().unwrap_or_default().contains("shardIndex"));
    Ok(())
}

/// Port of `TestTopDocsRRF.testInvalidTopN`.
fn test_invalid_top_n() -> Result<()> {
    let td1 = TopDocs::new(gte(100)?, Vec::new());
    let td2 = TopDocs::new(gte(80)?, Vec::new());

    let e = expect_throws(IAE, || TopDocs::rrf(0, 20, &[td1, td2]));
    Assert::assert_true(e.message().unwrap_or_default().contains("topN"));
    Ok(())
}

/// Port of `TestTopDocsRRF.testInvalidK`.
fn test_invalid_k() -> Result<()> {
    let td1 = TopDocs::new(gte(100)?, Vec::new());
    let td2 = TopDocs::new(gte(80)?, Vec::new());

    let e = expect_throws(IAE, || TopDocs::rrf(3, 0, &[td1, td2]));
    Assert::assert_true(e.message().unwrap_or_default().contains("k"));
    Ok(())
}
