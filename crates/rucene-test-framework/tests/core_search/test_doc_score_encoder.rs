//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDocScoreEncoder.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::DocScoreEncoder;

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestDocScoreEncoder`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDocScoreEncoder",
        vec![
            test("testRandom", test_random),
            test("testSameDoc", test_same_doc),
            test("testSameScore", test_same_score),
        ],
    )
}

fn test_random() -> Result<()> {
    for _i in 0..1000 {
        do_assert(
            f32::from_bits(random().next_int() as u32),
            random().next_int_bound(i32::MAX),
            f32::from_bits(random().next_int() as u32),
            random().next_int_bound(i32::MAX),
        );
    }
    Ok(())
}

fn test_same_doc() -> Result<()> {
    for _i in 0..1000 {
        do_assert(
            f32::from_bits(random().next_int() as u32),
            1,
            f32::from_bits(random().next_int() as u32),
            1,
        );
    }
    Ok(())
}

fn test_same_score() -> Result<()> {
    for _i in 0..1000 {
        do_assert(
            1.0,
            random().next_int_bound(i32::MAX),
            1.0,
            random().next_int_bound(i32::MAX),
        );
    }
    Ok(())
}

fn do_assert(score1: f32, doc1: i32, score2: f32, doc2: i32) {
    if score1.is_nan() || score2.is_nan() {
        return;
    }

    let code1 = DocScoreEncoder::encode(doc1, score1);
    let code2 = DocScoreEncoder::encode(doc2, score2);

    assert_equals(&doc1, &DocScoreEncoder::doc_id(code1));
    assert_equals(&doc2, &DocScoreEncoder::doc_id(code2));
    assert_equals_delta(score1 as f64, DocScoreEncoder::to_score(code1) as f64, 0.0);
    assert_equals_delta(score2 as f64, DocScoreEncoder::to_score(code2) as f64, 0.0);

    if score1 < score2 {
        Assert::assert_true(code1 < code2);
    } else if score1 > score2 {
        Assert::assert_true(code1 > code2);
    } else if doc1 == doc2 {
        assert_equals(&code1, &code2);
    } else {
        assert_equals(&(code1 > code2), &(doc1 < doc2));
    }
}
