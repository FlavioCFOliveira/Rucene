//! The own test methods of
//! `lucene/core/src/test/org/apache/lucene/util/TestRoaringDocIdSet.java`
//! (sprint 9, task #403), declared on the `TestRoaringDocIdSet` binding of
//! `base_doc_id_set_suites.rs` beside the inherited `BaseDocIdSetTestCase`
//! cases. Java's chained `Builder.add(…)` returns the builder; Rucene's
//! returns `Result<()>`, and `add(min, max)` is `add_range`.

use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::NO_MORE_DOCS;
use rucene::util::bit_sets::RoaringDocIdSet;
use rucene::util::{Accountable, FixedBitSet};
use rucene_test_framework::hamcrest::{greater_than, MatcherAssert};
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::util::base_doc_id_set_test_case::{BaseDocIdSetTestCase, JavaShaped};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use super::doc_id_run_end::{assert_doc_id_run_end_matches, random_bit_set};
use super::{own_test, roaring_shape, TestRoaringDocIdSet};

/// The own methods of `TestRoaringDocIdSet`, in declaration order.
pub fn methods() -> Vec<Method> {
    vec![
        own_test(
            "testDocIDRunEndContiguousWithinBlock",
            test_doc_id_run_end_contiguous_within_block,
        ),
        own_test(
            "testDocIDRunEndFinishInBoundary",
            test_doc_id_run_end_finish_in_boundary,
        ),
        own_test(
            "testDocIDRunEndAcross64KBoundary",
            test_doc_id_run_end_across64_k_boundary,
        ),
        own_test("testRandomDocIDRunEnd", test_random_doc_id_run_end),
        own_test("testAddRangeSmall", test_add_range_small),
        own_test("testAddRangeDenseBig", test_add_range_dense_big),
        own_test(
            "testAddRangeCrossesBlockBoundary",
            test_add_range_crosses_block_boundary,
        ),
        own_test(
            "testAddRangeAfterSingletonAdd",
            test_add_range_after_singleton_add,
        ),
        own_test(
            "testAddRangeBlockCardinalityAtSparseBufferCapacity",
            test_add_range_block_cardinality_at_sparse_buffer_capacity,
        ),
        own_test("testAddRangeEmptyNoOp", test_add_range_empty_no_op),
        own_test("testAddRandomRange", test_add_random_range),
        own_test(
            "testAddRangeOutOfOrderThrows",
            test_add_range_out_of_order_throws,
        ),
        own_test(
            "testAddRangeExceedsMaxDocThrows",
            test_add_range_exceeds_max_doc_throws,
        ),
        own_test(
            "testAddRangeMinGreaterThanMaxThrows",
            test_add_range_min_greater_than_max_throws,
        ),
        own_test(
            "testIntoBitSetWhenExhausted",
            test_into_bit_set_when_exhausted,
        ),
    ]
}

/// `copyOf(bs, length)`.
fn copy_of(bs: &JavaBitSet, length: i32) -> Result<JavaShaped<RoaringDocIdSet>> {
    TestRoaringDocIdSet.copy_of(bs, length)
}

/// `assertEquals(int numBits, BitSet ds1, RoaringDocIdSet ds2)` (the
/// override) over a set this test built.
fn assert_equals_set(num_bits: i32, ds1: &JavaBitSet, set: RoaringDocIdSet) -> Result<()> {
    let shape = roaring_shape(&set);
    TestRoaringDocIdSet.assert_equals_sets(num_bits, ds1, &JavaShaped { set, shape })
}

fn test_doc_id_run_end_contiguous_within_block() -> Result<()> {
    let random = LuceneTestCase::random();
    let max_doc = random.next_int_range(2, 50_000);
    let mut bs = JavaBitSet::new();
    let start = random.next_int_range(0, max_doc - 1);
    let end = random.next_int_range(start + 1, max_doc);
    for d in start..end {
        bs.set(d as usize);
    }
    let set = copy_of(&bs, max_doc)?;
    assert_doc_id_run_end_matches(&bs, max_doc, &set.set)
}

fn test_doc_id_run_end_finish_in_boundary() -> Result<()> {
    let boundary = 1 << 16;
    let max_doc = boundary + 100;
    let mut bs = JavaBitSet::new();
    for d in boundary - 3..boundary {
        bs.set(d as usize);
    }
    let set = copy_of(&bs, max_doc)?;
    assert_doc_id_run_end_matches(&bs, max_doc, &set.set)
}

fn test_doc_id_run_end_across64_k_boundary() -> Result<()> {
    let boundary = 1 << 16;
    let max_doc = boundary + 100;
    let mut bs = JavaBitSet::new();
    for d in boundary - 3..boundary + 7 {
        bs.set(d as usize);
    }
    let set = copy_of(&bs, max_doc)?;
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
            // assertNull(copyOf(bs, maxDoc).iterator()): Java's null iterator
            // of an empty set (see the binding's `iterator_is_null`).
            let set = copy_of(&bs, max_doc)?;
            Assert::assert_true(TestRoaringDocIdSet.iterator_is_null(&set));
            continue;
        }
        let set = copy_of(&bs, max_doc)?;
        assert_doc_id_run_end_matches(&bs, max_doc, &set.set)?;
    }
    Ok(())
}

fn test_add_range_small() -> Result<()> {
    let max_doc = 100;
    let bytes_sparse_values;
    let bytes_dense_range;
    {
        let mut expected = JavaBitSet::with_capacity(max_doc as usize);
        expected.set_range(10, 79);
        expected.set(80);
        let mut b = RoaringDocIdSet::builder(max_doc as usize);
        b.add_range(10, 79)?;
        b.add(80)?;
        let set = b.build();
        bytes_sparse_values = set.ram_bytes_used();
        assert_equals_set(max_doc, &expected, set)?;
    }
    {
        let mut expected = JavaBitSet::with_capacity(max_doc as usize);
        expected.set_range(10, 80);
        let mut b = RoaringDocIdSet::builder(max_doc as usize);
        b.add_range(10, 80)?;
        let set = b.build();
        bytes_dense_range = set.ram_bytes_used();
        assert_equals_set(max_doc, &expected, set)?;
    }
    MatcherAssert::assert_that(&bytes_sparse_values, &greater_than(bytes_dense_range));
    Ok(())
}

fn test_add_range_dense_big() -> Result<()> {
    let max_doc = 50_000;
    let bytes_sparse_values;
    let bytes_dense_range;
    {
        let mut expected = JavaBitSet::with_capacity(max_doc as usize);
        expected.set_range(7, 42_999);
        expected.set(43_000);
        let mut b = RoaringDocIdSet::builder(max_doc as usize);
        b.add_range(7, 42_999)?;
        b.add(43_000)?;
        let set = b.build();
        bytes_sparse_values = set.ram_bytes_used();
        assert_equals_set(max_doc, &expected, set)?;
    }
    {
        let mut expected = JavaBitSet::with_capacity(max_doc as usize);
        expected.set_range(7, 43_000);
        let mut b = RoaringDocIdSet::builder(max_doc as usize);
        b.add_range(7, 43_000)?;
        let set = b.build();
        bytes_dense_range = set.ram_bytes_used();
        assert_equals_set(max_doc, &expected, set)?;
    }
    MatcherAssert::assert_that(&bytes_sparse_values, &greater_than(bytes_dense_range));
    Ok(())
}

fn test_add_range_crosses_block_boundary() -> Result<()> {
    let max_doc = 70_000;
    let mut expected = JavaBitSet::with_capacity(max_doc as usize);
    expected.set_range(65_530, 65_545);
    let mut b = RoaringDocIdSet::builder(max_doc as usize);
    b.add_range(65_530, 65_545)?;
    assert_equals_set(max_doc, &expected, b.build())
}

fn test_add_range_after_singleton_add() -> Result<()> {
    let max_doc = 1_000;
    let mut expected = JavaBitSet::with_capacity(max_doc as usize);
    expected.set(5);
    expected.set_range(100, 200);
    let mut b = RoaringDocIdSet::builder(max_doc as usize);
    b.add(5)?;
    b.add_range(100, 200)?;
    assert_equals_set(max_doc, &expected, b.build())
}

fn test_add_range_block_cardinality_at_sparse_buffer_capacity() -> Result<()> {
    let max_doc = 5_000;
    for block_cardinality in [4095usize, 4096, 4097] {
        let mut expected = JavaBitSet::with_capacity(max_doc as usize);
        expected.set(0);
        let docs_in_range = block_cardinality - 1; // contiguous-range after the singleton at doc 0
        let range_end = 2 + docs_in_range; // added via the contiguous-range add(min, max)
        expected.set_range(2, range_end);
        let mut b = RoaringDocIdSet::builder(max_doc as usize);
        b.add(0)?;
        b.add_range(2, range_end)?;
        assert_equals_set(max_doc, &expected, b.build())?;
    }
    Ok(())
}

fn test_add_range_empty_no_op() -> Result<()> {
    let max_doc = 100;
    let mut b = RoaringDocIdSet::builder(max_doc as usize);
    b.add_range(10, 10)?;
    b.add_range(0, 100)?;
    let mut expected = JavaBitSet::with_capacity(max_doc as usize);
    expected.set_range(0, 100);
    assert_equals_set(max_doc, &expected, b.build())
}

fn test_add_random_range() -> Result<()> {
    let max_doc = LuceneTestCase::random().next_int_range(10, 250_000);
    let mut expected = JavaBitSet::with_capacity(max_doc as usize);
    let mut b = RoaringDocIdSet::builder(max_doc as usize);
    let mut index = 0;
    let mut iters = 0;
    while index + 1 < max_doc && {
        let go = iters < 10;
        iters += 1;
        go
    } {
        let end = LuceneTestCase::random().next_int_range(index + 1, max_doc);
        expected.set_range(index as usize, end as usize);
        b.add_range(index as usize, end as usize)?;
        index = end;
    }
    assert_equals_set(max_doc, &expected, b.build())
}

fn test_add_range_out_of_order_throws() -> Result<()> {
    let mut b = RoaringDocIdSet::builder(100);
    b.add(5)?;
    expect_throws("java.lang.IllegalArgumentException", || b.add_range(3, 10));
    Ok(())
}

fn test_add_range_exceeds_max_doc_throws() -> Result<()> {
    let mut b = RoaringDocIdSet::builder(100);
    expect_throws("java.lang.IllegalArgumentException", || b.add_range(0, 101));
    Ok(())
}

fn test_add_range_min_greater_than_max_throws() -> Result<()> {
    let mut b = RoaringDocIdSet::builder(100);
    expect_throws("java.lang.IllegalArgumentException", || b.add_range(10, 5));
    Ok(())
}

fn test_into_bit_set_when_exhausted() -> Result<()> {
    let max_doc = 3 << 16; // 3 blocks; docs only in block 0
    let mut builder = RoaringDocIdSet::builder(max_doc as usize);
    builder.add(5)?;
    builder.add(10)?;
    builder.add(20)?;
    let set = builder.build();

    // Exhaust via nextDoc, then intoBitSet must be a no-op.
    let mut it = set.iterator()?;
    while it.next_doc()? != NO_MORE_DOCS {}
    let mut bit_set = FixedBitSet::new(max_doc as usize);
    it.into_bit_set(max_doc, &mut bit_set, 0)?;
    Assert::assert_equals(NO_MORE_DOCS, it.doc_id());
    Assert::assert_true(bit_set.scan_is_empty());

    // Exhaust via advance past the last block.
    let mut it = set.iterator()?;
    Assert::assert_equals(NO_MORE_DOCS, it.advance(max_doc)?);
    let mut bit_set = FixedBitSet::new(max_doc as usize);
    it.into_bit_set(max_doc, &mut bit_set, 0)?;
    Assert::assert_equals(NO_MORE_DOCS, it.doc_id());
    Assert::assert_true(bit_set.scan_is_empty());
    Ok(())
}
