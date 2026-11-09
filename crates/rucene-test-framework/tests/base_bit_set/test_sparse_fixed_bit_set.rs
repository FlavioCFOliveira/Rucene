//! The own test methods of
//! `lucene/core/src/test/org/apache/lucene/util/TestSparseFixedBitSet.java`
//! (sprint 9, task #403), declared on the `TestSparseFixedBitSet` binding of
//! `base_bit_set_suites.rs` beside the inherited `BaseBitSetTestCase` cases.
//! `assertEquals(BitSet, SparseFixedBitSet, int)` (an override) stays in the
//! binding itself.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_id_set_iterator;
use rucene::search::NO_MORE_DOCS;
use rucene::util::bit_set::{BitSet, BitSetIterator};
use rucene::util::bit_sets::SparseFixedBitSet;
use rucene::util::{Accountable, FixedBitSet};
use rucene_test_framework::util::base_bit_set_test_case::BaseBitSetTestCase;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use super::TestSparseFixedBitSet;

/// A public `test*` method of `TestSparseFixedBitSet`.
fn test(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body_any(move || {
            if let Err(e) = body() {
                rethrow_java(e)
            }
        })
}

fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// The own methods of `TestSparseFixedBitSet`, in declaration order.
pub fn methods() -> Vec<Method> {
    vec![
        test("testApproximateCardinality", test_approximate_cardinality),
        test(
            "testApproximateCardinalityOnDenseSet",
            test_approximate_cardinality_on_dense_set,
        ),
        test("testRamBytesUsed", test_ram_bytes_used),
        test("testSetRange", test_set_range),
        test("testNextClearBit", test_next_clear_bit),
    ]
}

fn test_approximate_cardinality() -> Result<()> {
    let mut set = SparseFixedBitSet::new(10000)?;
    let first = random().next_int_bound(1000) as usize;
    let interval = 200 + random().next_int_bound(1000) as usize;
    let mut i = first;
    while i < set.length() {
        set.set(i);
        i += interval;
    }
    Assert::assert_equals_double_msg(
        None,
        set.cardinality() as f64,
        set.approximate_cardinality() as f64,
        20.0,
    );
    Ok(())
}

fn test_approximate_cardinality_on_dense_set() -> Result<()> {
    // this tests that things work as expected in approximateCardinality when
    // all longs are different than 0, in which case we divide by zero
    let num_docs = TestUtil::next_int(&*random(), 1, 10000) as usize;
    let mut set = SparseFixedBitSet::new(num_docs)?;
    for i in 0..set.length() {
        set.set(i);
    }
    Assert::assert_equals(num_docs, set.approximate_cardinality());
    Ok(())
}

fn test_ram_bytes_used() -> Result<()> {
    let size = 1000 + random().next_int_bound(10000) as usize;
    let mut original = SparseFixedBitSet::new(size)?;
    for _ in 0..3 {
        original.set(random().next_int_bound(size as i32) as usize);
    }
    Assert::assert_true(original.ram_bytes_used() > 0);
    let mut copy = TestSparseFixedBitSet.copy_of(&original, size)?;
    let mut other_bit_set = SparseFixedBitSet::new(size)?;
    let interval = 10 + random().next_int_bound(100) as usize;
    let mut i = 0;
    while i < size {
        other_bit_set.set(i);
        i += interval;
    }
    // `new BitSetIterator(otherBitSet, size)`: the iterator shares the set.
    copy.or(&mut BitSetIterator::new(
        Arc::new(other_bit_set),
        size as i64,
    )?)?;
    Assert::assert_true(copy.ram_bytes_used() > original.ram_bytes_used());

    copy = TestSparseFixedBitSet.copy_of(&original, size)?;
    copy.or(&mut doc_id_set_iterator::all(size as i32)?)?;
    Assert::assert_true(copy.ram_bytes_used() > original.ram_bytes_used());
    Assert::assert_true(copy.ram_bytes_used() > (size / 8) as i64);

    // `copyOf` and `or` must preserve RAM accounting; regression for issue #15934.
    let set_copy = TestSparseFixedBitSet.copy_of(&original, size)?;
    Assert::assert_equals(set_copy.ram_bytes_used(), original.ram_bytes_used());

    let mut or_copy = SparseFixedBitSet::new(size)?;
    or_copy.or(&mut BitSetIterator::new(
        Arc::new(original.clone()),
        size as i64,
    )?)?;
    Assert::assert_true((original.ram_bytes_used() - or_copy.ram_bytes_used()).abs() <= 64);
    Ok(())
}

/// `bruteNextUnset(SparseFixedBitSet set, int from, int upperBound)`.
fn brute_next_unset(set: &SparseFixedBitSet, from: i32, upper_bound: i32) -> i32 {
    for i in from..upper_bound {
        if !set.get(i as usize) {
            return i;
        }
    }
    NO_MORE_DOCS
}

fn test_set_range() -> Result<()> {
    let random = random();
    let num_bits = 1 + random.next_int_bound(100000);
    for percent_set in [0f32, 0.01, 0.1, 0.5, 0.9, 0.99, 1.0] {
        let mut expected = FixedBitSet::new(num_bits as usize);
        let num_initial = (percent_set * num_bits as f32) as i32;
        for _ in 0..num_initial {
            expected.set(random.next_int_bound(num_bits) as usize);
        }
        let mut set = TestSparseFixedBitSet.copy_of(&expected, num_bits as usize)?;
        let iters = LuceneTestCase::at_least_with(&*random, 10);
        for _ in 0..iters {
            let from = random.next_int_bound(num_bits) as usize;
            let to = random.next_int_bound(num_bits + 1) as usize;
            expected.set_range(from, to);
            set.set_range(from, to);
            TestSparseFixedBitSet.assert_equals_sets(&expected, &set, num_bits as usize);
        }
    }
    Ok(())
}

fn test_next_clear_bit() -> Result<()> {
    let rand = random();
    let outer = if *TEST_NIGHTLY { 300 } else { 60 };
    for _ in 0..outer {
        let n = TestUtil::next_int(&*rand, 1, 40_000);
        let mut set = SparseFixedBitSet::new(n as usize)?;
        let num_sets = TestUtil::next_int(&*rand, 0, n.min(8_000));
        for _ in 0..num_sets {
            set.set(rand.next_int_bound(n) as usize);
        }
        for _ in 0..300 {
            let from = rand.next_int_bound(n);
            Assert::assert_equals(
                brute_next_unset(&set, from, n),
                BitSet::next_clear_bit(&set, from),
            );
            let ub = from + 1 + rand.next_int_bound(n - from);
            Assert::assert_equals(
                brute_next_unset(&set, from, ub),
                BitSet::next_clear_bit_in_range(&set, from, ub),
            );
        }
    }
    Ok(())
}
