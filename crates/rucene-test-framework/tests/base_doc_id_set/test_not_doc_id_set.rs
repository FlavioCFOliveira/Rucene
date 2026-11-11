//! The own test methods of
//! `lucene/core/src/test/org/apache/lucene/util/TestNotDocIdSet.java`
//! (sprint 9, task #403), declared on the `TestNotDocIdSet` binding of
//! `base_doc_id_set_suites.rs` beside the inherited `BaseDocIdSetTestCase`
//! cases.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set::{self, DocIdSet};
use rucene::util::doc_id_set::{BitDocIdSet, NotDocIdSet};
use rucene::util::FixedBitSet;
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::util::base_doc_id_set_test_case::BaseDocIdSetTestCase;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use super::doc_id_run_end::{assert_doc_id_run_end_matches, random_bit_set};
use super::{own_test, TestNotDocIdSet};

/// The own methods of `TestNotDocIdSet`, in declaration order.
pub fn methods() -> Vec<Method> {
    vec![
        own_test("testBits", test_bits),
        own_test(
            "testDocIDRunEndContiguousRun",
            test_doc_id_run_end_contiguous_run,
        ),
        own_test("testRandomDocIDRunEnd", test_random_doc_id_run_end),
    ]
}

fn test_bits() -> Result<()> {
    Assert::assert_true(
        NotDocIdSet::new(3, Arc::new(doc_id_set::empty()))
            .bits()?
            .is_none(),
    );
    Assert::assert_true(
        NotDocIdSet::new(
            3,
            Arc::new(BitDocIdSet::from_set(Arc::new(FixedBitSet::new(3)))?),
        )
        .bits()?
        .is_some(),
    );
    Ok(())
}

fn test_doc_id_run_end_contiguous_run() -> Result<()> {
    let random = LuceneTestCase::random();
    let max_doc = random.next_int_range(2, 1_000);
    let mut bs = JavaBitSet::new();
    let start = random.next_int_range(0, max_doc - 1);
    let end = random.next_int_range(start + 1, max_doc);
    for d in start..end {
        bs.set(d as usize);
    }
    let set = TestNotDocIdSet.copy_of(&bs, max_doc)?;
    assert_doc_id_run_end_matches(&bs, max_doc, &set.set)
}

fn test_random_doc_id_run_end() -> Result<()> {
    let iters = if *TEST_NIGHTLY { 50 } else { 5 };
    for _ in 0..iters {
        let random = LuceneTestCase::random();
        let max_doc = random.next_int_range(50, 1 << 18);
        let bs = random_bit_set(max_doc, random.next_float());
        // bs.isEmpty()
        if bs.cardinality() == 0 {
            continue;
        }
        let set = TestNotDocIdSet.copy_of(&bs, max_doc)?;
        assert_doc_id_run_end_matches(&bs, max_doc, &set.set)?;
    }
    Ok(())
}
