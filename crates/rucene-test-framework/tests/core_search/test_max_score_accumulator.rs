//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMaxScoreAccumulator.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::max_score_accumulator::MaxScoreAccumulator;
use rucene::search::DocScoreEncoder;

use crate::core_support::{assert_equals_delta, class, test};

/// `TestMaxScoreAccumulator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMaxScoreAccumulator",
        vec![test("testSimple", test_simple)],
    )
}

/// `assertEquals(expectedScore, toScore(raw), 0)` and
/// `assertEquals(expectedDoc, docId(raw), 0)` (the `double` overload).
fn check(acc: &MaxScoreAccumulator, score: f32, doc: i32) {
    assert_equals_delta(
        f64::from(score),
        f64::from(DocScoreEncoder::to_score(acc.get_raw())),
        0.0,
    );
    assert_equals_delta(
        f64::from(doc),
        f64::from(DocScoreEncoder::doc_id(acc.get_raw())),
        0.0,
    );
}

fn test_simple() -> Result<()> {
    let acc = MaxScoreAccumulator::new();
    acc.accumulate(DocScoreEncoder::encode(0, 0.0));
    check(&acc, 0.0, 0);
    acc.accumulate(DocScoreEncoder::encode(10, 0.0));
    check(&acc, 0.0, 0);
    acc.accumulate(DocScoreEncoder::encode(100, 1000.0));
    check(&acc, 1000.0, 100);
    acc.accumulate(DocScoreEncoder::encode(1000, 5.0));
    check(&acc, 1000.0, 100);
    acc.accumulate(DocScoreEncoder::encode(99, 1000.0));
    check(&acc, 1000.0, 99);
    acc.accumulate(DocScoreEncoder::encode(1000, 1001.0));
    check(&acc, 1001.0, 1000);
    acc.accumulate(DocScoreEncoder::encode(10, 1001.0));
    check(&acc, 1001.0, 10);
    acc.accumulate(DocScoreEncoder::encode(100, 1001.0));
    check(&acc, 1001.0, 10);
    Ok(())
}
