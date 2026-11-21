//! Port of `lucene/core/src/test/org/apache/lucene/util/TestBitSetIterator.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::bit_set::{BitSet, BitSetIterator};
use rucene::util::{FixedBitSet, SparseFixedBitSet};
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};

/// `TestBitSetIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestBitSetIterator",
        vec![
            test("testDocIDRunEndContiguous", test_doc_id_run_end_contiguous),
            test(
                "testDocIDRunEndFullFixedBitSet",
                test_doc_id_run_end_full_fixed_bit_set,
            ),
            test(
                "testDocIDRunEndFullSparseBitSet",
                test_doc_id_run_end_full_sparse_bit_set,
            ),
            test(
                "testDocIDRunEndAdvanceFixedBitSet",
                test_doc_id_run_end_advance_fixed_bit_set,
            ),
            test(
                "testDocIDRunEndSparseFixedBitSet",
                test_doc_id_run_end_sparse_fixed_bit_set,
            ),
            test(
                "testRandomDocIDRunEndFixedBitSet",
                test_random_doc_id_run_end_fixed_bit_set,
            ),
            test(
                "testRandomDocIDRunEndSparseFixedBitSet",
                test_random_doc_id_run_end_sparse_fixed_bit_set,
            ),
        ],
    )
}

fn test_doc_id_run_end_contiguous() -> Result<()> {
    let total_docs = random().next_int_range(1_000, 10_000);
    let min_doc = random().next_int_bound(total_docs - 2);
    let max_doc = random().next_int_range(min_doc + 1, total_docs);
    let mut fbs: Box<dyn BitSet> = if random().next_boolean() {
        Box::new(FixedBitSet::new(total_docs as usize))
    } else {
        Box::new(SparseFixedBitSet::new(total_docs as usize)?)
    };
    for d in min_doc..max_doc {
        fbs.set(d as usize);
    }
    let cardinality = fbs.cardinality() as i64;
    let mut it = BitSetIterator::new(Arc::from(fbs), cardinality)?;
    assert_equals(&min_doc, &it.advance(0)?);
    assert_equals(&max_doc, &it.doc_id_run_end()?);
    assert_equals(&NO_MORE_DOCS, &it.advance(max_doc)?);
    Ok(())
}

fn test_doc_id_run_end_full_fixed_bit_set() -> Result<()> {
    let n = 500;
    let mut fbs = FixedBitSet::new(n as usize);
    fbs.set_range(0, n as usize);
    let mut it = BitSetIterator::new(Arc::new(fbs), n as i64)?;
    assert_equals(&0, &it.next_doc()?);
    assert_equals(&n, &it.doc_id_run_end()?);
    assert_equals(&42, &it.advance(42)?);
    assert_equals(&n, &it.doc_id_run_end()?);
    Ok(())
}

fn test_doc_id_run_end_full_sparse_bit_set() -> Result<()> {
    let n = 500;
    let mut bs = SparseFixedBitSet::new(n as usize)?;
    for i in 0..n {
        bs.set(i as usize);
    }
    let mut it = BitSetIterator::new(Arc::new(bs), n as i64)?;
    assert_equals(&0, &it.next_doc()?);
    assert_equals(&n, &it.doc_id_run_end()?);
    assert_equals(&42, &it.advance(42)?);
    assert_equals(&n, &it.doc_id_run_end()?);
    Ok(())
}

fn test_doc_id_run_end_advance_fixed_bit_set() -> Result<()> {
    let mut fbs = FixedBitSet::new(2_000);
    for d in 100..800 {
        fbs.set(d);
    }
    let fbs: Arc<dyn BitSet> = Arc::new(fbs);
    let mut it = BitSetIterator::new(Arc::clone(&fbs), fbs.cardinality() as i64)?;
    assert_equals(&100, &it.advance(50)?);
    assert_equals(&expected_doc_id_run_end(&*fbs, 100), &it.doc_id_run_end()?);
    assert_equals(&400, &it.advance(400)?);
    assert_equals(&expected_doc_id_run_end(&*fbs, 400), &it.doc_id_run_end()?);
    assert_equals(&799, &it.advance(799)?);
    assert_equals(&expected_doc_id_run_end(&*fbs, 799), &it.doc_id_run_end()?);
    Ok(())
}

fn test_doc_id_run_end_sparse_fixed_bit_set() -> Result<()> {
    let n = 30_000;
    let mut sbs = SparseFixedBitSet::new(n)?;
    for d in 12_000..12_100 {
        sbs.set(d);
    }
    for d in 20_000..20_003 {
        sbs.set(d);
    }
    let sbs: Arc<dyn BitSet> = Arc::new(sbs);
    let mut it = BitSetIterator::new(Arc::clone(&sbs), sbs.cardinality() as i64)?;
    assert_doc_id_run_end_matches_iterator(&mut it, &*sbs)
}

fn test_random_doc_id_run_end_fixed_bit_set() -> Result<()> {
    let r = random();
    let iters = if *TEST_NIGHTLY { 200 } else { 40 };
    for _ in 0..iters {
        let n = TestUtil::next_int(&*r, 1, 40_000);
        let mut fbs = FixedBitSet::new(n as usize);
        let num_sets = TestUtil::next_int(&*r, 0, n.min(8_000));
        for _ in 0..num_sets {
            fbs.set(r.next_int_bound(n) as usize);
        }
        let fbs: Arc<dyn BitSet> = Arc::new(fbs);
        if fbs.cardinality() == 0 {
            assert_equals(
                &NO_MORE_DOCS,
                &BitSetIterator::new(Arc::clone(&fbs), 0)?.next_doc()?,
            );
            continue;
        }
        let mut it = BitSetIterator::new(Arc::clone(&fbs), fbs.cardinality() as i64)?;
        assert_doc_id_run_end_matches_iterator(&mut it, &*fbs)?;
    }
    Ok(())
}

fn test_random_doc_id_run_end_sparse_fixed_bit_set() -> Result<()> {
    let r = random();
    let iters = if *TEST_NIGHTLY { 200 } else { 40 };
    for _ in 0..iters {
        let n = TestUtil::next_int(&*r, 1, 40_000);
        let mut sbs = SparseFixedBitSet::new(n as usize)?;
        let num_sets = TestUtil::next_int(&*r, 0, n.min(8_000));
        for _ in 0..num_sets {
            sbs.set(r.next_int_bound(n) as usize);
        }
        let sbs: Arc<dyn BitSet> = Arc::new(sbs);
        if sbs.cardinality() == 0 {
            assert_equals(
                &NO_MORE_DOCS,
                &BitSetIterator::new(Arc::clone(&sbs), 0)?.next_doc()?,
            );
            continue;
        }
        let mut it = BitSetIterator::new(Arc::clone(&sbs), sbs.cardinality() as i64)?;
        assert_doc_id_run_end_matches_iterator(&mut it, &*sbs)?;
    }
    Ok(())
}

/// `expectedDocIDRunEnd(bits, doc)`.
fn expected_doc_id_run_end(bits: &dyn BitSet, doc: i32) -> i32 {
    let mut end = doc + 1;
    let len = bits.length() as i32;
    while end < len && bits.get(end as usize) {
        end += 1;
    }
    end
}

/// `assertDocIDRunEndMatchesIterator(it, bits)`.
fn assert_doc_id_run_end_matches_iterator(
    it: &mut BitSetIterator,
    bits: &dyn BitSet,
) -> Result<()> {
    let mut d = it.next_doc()?;
    while d != NO_MORE_DOCS {
        Assert::assert_true(bits.get(d as usize));
        assert_equals(&expected_doc_id_run_end(bits, d), &it.doc_id_run_end()?);
        d = it.next_doc()?;
    }
    Ok(())
}
