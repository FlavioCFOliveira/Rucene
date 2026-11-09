//! The own test methods of
//! `lucene/core/src/test/org/apache/lucene/util/TestFixedBitSet.java`
//! (sprint 9, task #402), declared on the `TestFixedBitSet` binding of
//! `base_bit_set_suites.rs` beside the inherited `BaseBitSetTestCase` cases.
//! `testPrevSetBit` (an override) stays in the binding itself.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::bit_set::{BitSet, BitSetIterator, DocBaseBitSetIterator};
use rucene::util::doc_id_set::IntArrayDocIdSetIterator;
use rucene::util::{Bits, FixedBitSet};
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, TEST_NIGHTLY};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use super::TestFixedBitSet;

/// A public `test*` method of `TestFixedBitSet`.
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

/// The own methods of `TestFixedBitSet`, in declaration order.
pub fn methods() -> Vec<Method> {
    vec![
        test("testApproximateCardinality", test_approximate_cardinality),
        test("testSmall", test_small),
        test("testEquals", test_equals),
        test("testHashCodeEquals", test_hash_code_equals),
        test("testSmallBitSets", test_small_bit_sets),
        test("testNextBitSet", test_next_bit_set),
        test("testEnsureCapacity", test_ensure_capacity),
        test("testBits2Words", test_bits2_words),
        test("testIntersectionCount", test_intersection_count),
        test("testAndNot", test_and_not),
        test("testUnionCount", test_union_count),
        test("testAndNotCount", test_and_not_count),
        test("testCopyOf", test_copy_of),
        test("testAsBits", test_as_bits),
        test("testScanIsEmpty", test_scan_is_empty),
        test("testOrRange", test_or_range),
        test("testAndRange", test_and_range),
        test("testRangeCardinality", test_range_cardinality),
        test("testForEach", test_for_each),
        test("testIntoArray", test_into_array),
        test("testOrMask", test_or_mask),
        test("testOrMaskStraddling", test_or_mask_straddling),
    ]
}

fn test_approximate_cardinality() -> Result<()> {
    // The approximate cardinality works in such a way that it should be pretty accurate on a bitset
    // whose bits are uniformly distributed.
    let mut set = FixedBitSet::new(TestUtil::next_int(&*random(), 100_000, 200_000) as usize);
    let first = random().next_int_bound(10) as usize;
    let interval = TestUtil::next_int(&*random(), 10, 20) as usize;
    let mut i = first;
    while i < set.length() {
        set.set(i);
        i += interval;
    }
    let cardinality = set.cardinality();
    // `assertEquals(long, long, long)`: the delta overload over `double`s.
    Assert::assert_equals_double_msg(
        None,
        cardinality as f64,
        set.approximate_cardinality() as f64,
        (cardinality / 20) as f64,
    ); // 5% error at most
    Ok(())
}

/// `doGet(a, b)`.
fn do_get(a: &JavaBitSet, b: &FixedBitSet) {
    Assert::assert_equals(a.cardinality(), b.cardinality());
    let max = b.length();
    for i in 0..max {
        if a.get(i) != b.get(i) {
            Assert::fail(&format!("mismatch: BitSet=[{i}]={}", a.get(i)));
        }
    }
}

/// `doNextSetBit(a, b)`.
fn do_next_set_bit(a: &JavaBitSet, b: &FixedBitSet) {
    Assert::assert_equals(a.cardinality(), b.cardinality());
    let mut aa: i32 = -1;
    let mut bb: i32 = -1;
    loop {
        aa = a.next_set_bit((aa + 1) as usize);
        if aa == -1 {
            aa = NO_MORE_DOCS;
        }
        bb = if bb < b.length() as i32 - 1 {
            BitSet::next_set_bit(b, bb + 1)
        } else {
            NO_MORE_DOCS
        };
        Assert::assert_equals(aa, bb);
        if aa == NO_MORE_DOCS {
            break;
        }
    }
}

/// `doNextClearBit(a, b)`.
fn do_next_clear_bit(a: &JavaBitSet, b: &FixedBitSet) {
    Assert::assert_equals(a.cardinality(), b.cardinality());
    let len = b.length() as i32;
    let mut aa: i32 = -1;
    let mut bb: i32 = -1;
    loop {
        let from = aa + 1;
        if from >= len {
            aa = NO_MORE_DOCS;
        } else {
            let nc = a.next_clear_bit(from as usize);
            aa = if nc < len { nc } else { NO_MORE_DOCS };
        }
        bb = if bb < len - 1 {
            BitSet::next_clear_bit(b, bb + 1)
        } else {
            NO_MORE_DOCS
        };
        Assert::assert_equals(aa, bb);
        if aa == NO_MORE_DOCS {
            break;
        }
    }
}

// test interleaving different FixedBitSetIterator.next()/skipTo()
/// `doIterate(a, b)`.
fn do_iterate(a: &JavaBitSet, b: &FixedBitSet) -> Result<()> {
    Assert::assert_equals(a.cardinality(), b.cardinality());
    let mut aa: i32 = -1;
    let mut bb: i32 = -1;
    let mut iterator = BitSetIterator::new(Arc::new(b.clone()), 0)?;
    loop {
        aa = a.next_set_bit((aa + 1) as usize);
        bb = if random().next_boolean() {
            iterator.next_doc()?
        } else {
            iterator.advance(bb + 1)?
        };
        Assert::assert_equals(if aa == -1 { NO_MORE_DOCS } else { aa }, bb);
        if aa < 0 {
            break;
        }
    }
    Ok(())
}

/// `doRandomSets(maxSize, iter)`.
fn do_random_sets(max_size: i32, iter: i32) -> Result<()> {
    let mut a0: Option<JavaBitSet> = None;
    let mut b0: Option<FixedBitSet> = None;

    for _ in 0..iter {
        let sz = TestUtil::next_int(&*random(), 2, max_size);
        let mut a = JavaBitSet::with_capacity(sz as usize);
        let mut b = FixedBitSet::new(sz as usize);

        // test the various ways of setting bits
        if sz > 0 {
            let n_oper = random().next_int_bound(sz);
            for _ in 0..n_oper {
                let mut idx;

                idx = random().next_int_bound(sz) as usize;
                a.set(idx);
                b.set(idx);

                idx = random().next_int_bound(sz) as usize;
                a.clear(idx);
                b.clear(idx);

                idx = random().next_int_bound(sz) as usize;
                a.flip_range(idx, idx + 1);
                b.flip_range(idx, idx + 1);

                idx = random().next_int_bound(sz) as usize;
                a.flip(idx);
                b.flip(idx);

                let val2 = b.get(idx);
                let val = b.get_and_set(idx);
                Assert::assert_true(val2 == val);
                Assert::assert_true(b.get(idx));

                if !val {
                    b.clear(idx);
                }
                Assert::assert_true(b.get(idx) == val);
            }
        }

        // test that the various ways of accessing the bits are equivalent
        do_get(&a, &b);

        // test ranges, including possible extension
        let mut from_index = random().next_int_bound(sz / 2) as usize;
        let mut to_index = from_index + random().next_int_bound(sz - from_index as i32) as usize;
        let mut aa = a.clone();
        aa.flip_range(from_index, to_index);
        let mut bb = b.clone();
        bb.flip_range(from_index, to_index);

        do_iterate(&aa, &bb)?; // a problem here is from flip or doIterate

        from_index = random().next_int_bound(sz / 2) as usize;
        to_index = from_index + random().next_int_bound(sz - from_index as i32) as usize;
        aa = a.clone();
        aa.clear_range(from_index, to_index);
        bb = b.clone();
        bb.clear_range(from_index, to_index);

        do_next_set_bit(&aa, &bb); // a problem here is from clear() or nextSetBit

        do_next_clear_bit(&aa, &bb);

        TestFixedBitSet::do_prev_set_bit(&aa, &bb);

        from_index = random().next_int_bound(sz / 2) as usize;
        to_index = from_index + random().next_int_bound(sz - from_index as i32) as usize;
        aa = a.clone();
        aa.set_range(from_index, to_index);
        bb = b.clone();
        bb.set_range(from_index, to_index);

        do_next_set_bit(&aa, &bb); // a problem here is from set() or nextSetBit

        do_next_clear_bit(&aa, &bb);

        TestFixedBitSet::do_prev_set_bit(&aa, &bb);

        if let (Some(a0), Some(b0)) = (&a0, &b0) {
            if b0.length() <= b.length() {
                Assert::assert_equals(a.cardinality(), b.cardinality());

                let mut a_and = a.clone();
                a_and.and(a0);
                let mut a_or = a.clone();
                a_or.or(a0);
                let mut a_xor = a.clone();
                a_xor.xor(a0);
                let mut a_andn = a.clone();
                a_andn.and_not(a0);

                let mut b_and = b.clone();
                Assert::assert_true(b == b_and);
                b_and.and(b0);
                let mut b_or = b.clone();
                b_or.or_set(b0);
                let mut b_xor = b.clone();
                b_xor.xor(b0);
                let mut b_andn = b.clone();
                b_andn.and_not(b0);

                Assert::assert_equals(a0.cardinality(), b0.cardinality());
                Assert::assert_equals(a_or.cardinality(), b_or.cardinality());

                do_iterate(&a_and, &b_and)?;
                do_iterate(&a_or, &b_or)?;
                do_iterate(&a_andn, &b_andn)?;
                do_iterate(&a_xor, &b_xor)?;

                Assert::assert_equals(a_and.cardinality(), b_and.cardinality());
                Assert::assert_equals(a_or.cardinality(), b_or.cardinality());
                Assert::assert_equals(a_xor.cardinality(), b_xor.cardinality());
                Assert::assert_equals(a_andn.cardinality(), b_andn.cardinality());
            }
        }

        a0 = Some(a);
        b0 = Some(b);
    }
    Ok(())
}

// large enough to flush obvious bugs, small enough to run in <.5 sec as part of a
// larger testsuite.
fn test_small() -> Result<()> {
    let iters = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(1000)
    } else {
        100
    };
    do_random_sets(LuceneTestCase::at_least(1200), iters)
}

// uncomment to run a bigger test (~2 minutes).
/*
public void testBig() {
  doRandomSets(2000,200000, 1);
  doRandomSets(2000,200000, 2);
}
*/

fn test_equals() -> Result<()> {
    // This test can't handle numBits==0:
    let num_bits = random().next_int_bound(2000) + 1;
    let mut b1 = FixedBitSet::new(num_bits as usize);
    let mut b2 = FixedBitSet::new(num_bits as usize);
    Assert::assert_true(b1 == b2);
    Assert::assert_true(b2 == b1);
    for _ in 0..10 * *RANDOM_MULTIPLIER {
        let idx = random().next_int_bound(num_bits) as usize;
        if !b1.get(idx) {
            b1.set(idx);
            Assert::assert_false(b1 == b2);
            Assert::assert_false(b2 == b1);
            b2.set(idx);
            Assert::assert_true(b1 == b2);
            Assert::assert_true(b2 == b1);
        }
    }

    // try different type of object
    // `b1.equals(new Object())`: `FixedBitSet.equals` first checks the class.
    let object: Box<dyn Any> = Box::new(());
    Assert::assert_false(
        object
            .downcast_ref::<FixedBitSet>()
            .is_some_and(|other| b1 == *other),
    );
    Ok(())
}

fn test_hash_code_equals() -> Result<()> {
    // This test can't handle numBits==0:
    let num_bits = random().next_int_bound(2000) + 1;
    let mut b1 = FixedBitSet::new(num_bits as usize);
    let mut b2 = FixedBitSet::new(num_bits as usize);
    Assert::assert_true(b1 == b2);
    Assert::assert_true(b2 == b1);
    for _ in 0..10 * *RANDOM_MULTIPLIER {
        let idx = random().next_int_bound(num_bits) as usize;
        if !b1.get(idx) {
            b1.set(idx);
            Assert::assert_false(b1 == b2);
            Assert::assert_false(b1.hash_code() == b2.hash_code());
            b2.set(idx);
            Assert::assert_true(b1 == b2);
            Assert::assert_equals(b1.hash_code(), b2.hash_code());
        }
    }
    Ok(())
}

fn test_small_bit_sets() -> Result<()> {
    // Make sure size 0-10 bit sets are OK:
    for num_bits in 0..10usize {
        let mut b1 = FixedBitSet::new(num_bits);
        let b2 = FixedBitSet::new(num_bits);
        Assert::assert_true(b1 == b2);
        Assert::assert_equals(b1.hash_code(), b2.hash_code());
        Assert::assert_equals(0usize, b1.cardinality());
        if num_bits > 0 {
            b1.set_range(0, num_bits);
            Assert::assert_equals(num_bits, b1.cardinality());
            b1.flip_range(0, num_bits);
            Assert::assert_equals(0usize, b1.cardinality());
        }
    }
    Ok(())
}

/// `makeFixedBitSet(a, numBits)`.
fn make_fixed_bit_set(a: &[i32], num_bits: i32) -> Result<FixedBitSet> {
    let a: Vec<usize> = a.iter().map(|&e| e as usize).collect();
    TestFixedBitSet::make_fixed_bit_set(&a, num_bits as usize)
}

/// `makeBitSet(a)`.
fn make_bit_set(a: &[i32]) -> JavaBitSet {
    let a: Vec<usize> = a.iter().map(|&e| e as usize).collect();
    TestFixedBitSet::make_bit_set(&a)
}

/// `checkNextSetBitArray(a, numBits)`.
fn check_next_set_bit_array(a: &[i32], num_bits: i32) -> Result<()> {
    let obs = make_fixed_bit_set(a, num_bits)?;
    let bs = make_bit_set(a);
    do_next_set_bit(&bs, &obs);
    Ok(())
}

fn test_next_bit_set() -> Result<()> {
    let mut set_bits: Vec<i32> = vec![0; random().next_int_bound(1000) as usize];
    let len = set_bits.len() as i32;
    for i in 0..set_bits.len() {
        set_bits[i] = random().next_int_bound(len);
    }
    check_next_set_bit_array(&set_bits, len + random().next_int_bound(10))?;

    check_next_set_bit_array(&[], len + random().next_int_bound(10))
}

/// `FixedBitSet.ensureCapacity` takes its set by value in Rucene; where Java
/// keeps using the set it passed, the port passes a clone.
fn test_ensure_capacity() -> Result<()> {
    let mut bits = FixedBitSet::new(5);
    bits.set(1);
    bits.set(4);

    let mut new_bits = FixedBitSet::ensure_capacity(bits.clone(), 8); // grow within the word
    Assert::assert_true(new_bits.get(1));
    Assert::assert_true(new_bits.get(4));
    new_bits.clear(1);
    // we align to 64-bits, so even though it shouldn't have, it re-allocated a long[1]
    Assert::assert_true(bits.get(1));
    Assert::assert_false(new_bits.get(1));

    new_bits.set(1);
    let len = new_bits.length();
    new_bits = FixedBitSet::ensure_capacity(new_bits, len - 2); // reuse
    Assert::assert_true(new_bits.get(1));

    bits.set(1);
    new_bits = FixedBitSet::ensure_capacity(bits.clone(), 72); // grow beyond one word
    Assert::assert_true(new_bits.get(1));
    Assert::assert_true(new_bits.get(4));
    new_bits.clear(1);
    // we grew the long[], so it's not shared
    Assert::assert_true(bits.get(1));
    Assert::assert_false(new_bits.get(1));
    Ok(())
}

fn test_bits2_words() -> Result<()> {
    Assert::assert_equals(0usize, FixedBitSet::bits2words(0));
    Assert::assert_equals(1usize, FixedBitSet::bits2words(1));
    // ...
    Assert::assert_equals(1usize, FixedBitSet::bits2words(64));
    Assert::assert_equals(2usize, FixedBitSet::bits2words(65));
    // ...
    Assert::assert_equals(2usize, FixedBitSet::bits2words(128));
    Assert::assert_equals(3usize, FixedBitSet::bits2words(129));
    // ...
    Assert::assert_equals(1024usize, FixedBitSet::bits2words(65536));
    Assert::assert_equals(1025usize, FixedBitSet::bits2words(65537));
    // ...
    Assert::assert_equals(
        1usize << (31 - 6),
        FixedBitSet::bits2words(i32::MAX as usize),
    );
    Ok(())
}

/// `makeIntArray(random, count, min, max)`.
fn make_int_array(random: &dyn Random, count: i32, min: i32, max: i32) -> Vec<i32> {
    let mut rv = vec![0; count as usize];

    for i in 0..count as usize {
        rv[i] = TestUtil::next_int(random, min, max);
    }

    rv
}

// Demonstrates that the presence of ghost bits in the last used word can cause spurious failures
fn test_intersection_count() -> Result<()> {
    let random = random();

    let num_bits1 = TestUtil::next_int(&*random, 1000, 2000);
    let num_bits2 = TestUtil::next_int(&*random, 1000, 2000);

    let count1 = TestUtil::next_int(&*random, 0, num_bits1 - 1);
    let count2 = TestUtil::next_int(&*random, 0, num_bits2 - 1);

    let bits1 = make_int_array(&*random, count1, 0, num_bits1 - 1);
    let bits2 = make_int_array(&*random, count2, 0, num_bits2 - 1);

    let fixed_bit_set1 = make_fixed_bit_set(&bits1, num_bits1)?;
    let fixed_bit_set2 = make_fixed_bit_set(&bits2, num_bits2)?;

    // If ghost bits are present, these may fail too, but that's not what we want to demonstrate
    // here
    // assertTrue(fixedBitSet1.cardinality() <= bits1.length);
    // assertTrue(fixedBitSet2.cardinality() <= bits2.length);

    let intersection_count = FixedBitSet::intersection_count(&fixed_bit_set1, &fixed_bit_set2);

    let mut bit_set1 = make_bit_set(&bits1);
    let bit_set2 = make_bit_set(&bits2);

    // If ghost bits are present, these may fail too, but that's not what we want to demonstrate
    // here
    // assertEquals(bitSet1.cardinality(), fixedBitSet1.cardinality());
    // assertEquals(bitSet2.cardinality(), fixedBitSet2.cardinality());

    bit_set1.and(&bit_set2);

    Assert::assert_equals(bit_set1.cardinality() as u64, intersection_count);
    Ok(())
}

fn test_and_not() -> Result<()> {
    let random = random();

    let num_bits2 = TestUtil::next_int(&*random, 1000, 2000);
    let num_bits1 = TestUtil::next_int(&*random, 1000, num_bits2);

    let count1 = TestUtil::next_int(&*random, 0, num_bits1 - 1);
    let count2 = TestUtil::next_int(&*random, 0, num_bits2 - 1);

    let min = TestUtil::next_int(&*random, 0, num_bits1 - 1);
    let off_set_word1 = min >> 6;
    let offset1 = off_set_word1 << 6;
    let bits1 = make_int_array(&*random, count1, min, num_bits1 - 1);
    let bits2 = make_int_array(&*random, count2, 0, num_bits2 - 1);

    let bit_set1 = make_bit_set(&bits1);
    let mut bit_set2 = make_bit_set(&bits2);
    bit_set2.and_not(&bit_set1);

    {
        // test BitSetIterator
        let mut fixed_bit_set2 = make_fixed_bit_set(&bits2, num_bits2)?;
        let mut disi = BitSetIterator::new(
            Arc::new(make_fixed_bit_set(&bits1, num_bits1)?),
            count1 as i64,
        )?;
        fixed_bit_set2.and_not_iterator(&mut disi)?;
        do_get(&bit_set2, &fixed_bit_set2);
    }

    {
        // test DocBaseBitSetIterator
        let mut fixed_bit_set2 = make_fixed_bit_set(&bits2, num_bits2)?;
        let offset_bits: Vec<i32> = bits1.iter().map(|i| i - offset1).collect();
        let mut disi = DocBaseBitSetIterator::new(
            Arc::new(make_fixed_bit_set(&offset_bits, num_bits1 - offset1)?),
            count1 as i64,
            offset1,
        )?;
        fixed_bit_set2.and_not_iterator(&mut disi)?;
        do_get(&bit_set2, &fixed_bit_set2);
    }

    {
        // test other
        let mut fixed_bit_set2 = make_fixed_bit_set(&bits2, num_bits2)?;
        let mut sorted = vec![0; bits1.len() + 1];
        sorted[..bits1.len()].copy_from_slice(&bits1);
        sorted[bits1.len()] = NO_MORE_DOCS;
        let mut disi = IntArrayDocIdSetIterator::new(Arc::new(sorted), count1 as usize);
        fixed_bit_set2.and_not_iterator(&mut disi)?;
        do_get(&bit_set2, &fixed_bit_set2);
    }
    Ok(())
}

// Demonstrates that the presence of ghost bits in the last used word can cause spurious failures
fn test_union_count() -> Result<()> {
    let random = random();

    let num_bits1 = TestUtil::next_int(&*random, 1000, 2000);
    let num_bits2 = TestUtil::next_int(&*random, 1000, 2000);

    let count1 = TestUtil::next_int(&*random, 0, num_bits1 - 1);
    let count2 = TestUtil::next_int(&*random, 0, num_bits2 - 1);

    let bits1 = make_int_array(&*random, count1, 0, num_bits1 - 1);
    let bits2 = make_int_array(&*random, count2, 0, num_bits2 - 1);

    let fixed_bit_set1 = make_fixed_bit_set(&bits1, num_bits1)?;
    let fixed_bit_set2 = make_fixed_bit_set(&bits2, num_bits2)?;

    // If ghost bits are present, these may fail too, but that's not what we want to demonstrate
    // here
    // assertTrue(fixedBitSet1.cardinality() <= bits1.length);
    // assertTrue(fixedBitSet2.cardinality() <= bits2.length);

    let union_count = FixedBitSet::union_count(&fixed_bit_set1, &fixed_bit_set2);

    let mut bit_set1 = make_bit_set(&bits1);
    let bit_set2 = make_bit_set(&bits2);

    // If ghost bits are present, these may fail too, but that's not what we want to demonstrate
    // here
    // assertEquals(bitSet1.cardinality(), fixedBitSet1.cardinality());
    // assertEquals(bitSet2.cardinality(), fixedBitSet2.cardinality());

    bit_set1.or(&bit_set2);

    Assert::assert_equals(bit_set1.cardinality() as u64, union_count);
    Ok(())
}

// Demonstrates that the presence of ghost bits in the last used word can cause spurious failures
fn test_and_not_count() -> Result<()> {
    let random = random();

    let num_bits1 = TestUtil::next_int(&*random, 1000, 2000);
    let num_bits2 = TestUtil::next_int(&*random, 1000, 2000);

    let count1 = TestUtil::next_int(&*random, 0, num_bits1 - 1);
    let count2 = TestUtil::next_int(&*random, 0, num_bits2 - 1);

    let bits1 = make_int_array(&*random, count1, 0, num_bits1 - 1);
    let bits2 = make_int_array(&*random, count2, 0, num_bits2 - 1);

    let fixed_bit_set1 = make_fixed_bit_set(&bits1, num_bits1)?;
    let fixed_bit_set2 = make_fixed_bit_set(&bits2, num_bits2)?;

    // If ghost bits are present, these may fail too, but that's not what we want to demonstrate
    // here
    // assertTrue(fixedBitSet1.cardinality() <= bits1.length);
    // assertTrue(fixedBitSet2.cardinality() <= bits2.length);

    let and_not_count = FixedBitSet::and_not_count(&fixed_bit_set1, &fixed_bit_set2);

    let mut bit_set1 = make_bit_set(&bits1);
    let bit_set2 = make_bit_set(&bits2);

    // If ghost bits are present, these may fail too, but that's not what we want to demonstrate
    // here
    // assertEquals(bitSet1.cardinality(), fixedBitSet1.cardinality());
    // assertEquals(bitSet2.cardinality(), fixedBitSet2.cardinality());

    bit_set1.and_not(&bit_set2);

    Assert::assert_equals(bit_set1.cardinality() as u64, and_not_count);
    Ok(())
}

/// `assertNotSame(a, b)`: identity of the two objects.
fn assert_not_same(a: *const u8, b: *const u8) {
    if std::ptr::eq(a, b) {
        Assert::fail("expected not same")
    }
}

/// The anonymous `Bits` of `testCopyOf`, over `fixedBitSet`.
#[derive(Debug)]
struct DelegatingBits<'a>(&'a FixedBitSet);

impl Bits for DelegatingBits<'_> {
    fn get(&self, index: usize) -> bool {
        self.0.get(index)
    }

    fn length(&self) -> usize {
        self.0.length()
    }
}

fn test_copy_of() -> Result<()> {
    let random = random();
    let num_bits = TestUtil::next_int(&*random, 1000, 2000);
    let count = TestUtil::next_int(&*random, 0, num_bits - 1);
    let bits = make_int_array(&*random, count, 0, num_bits - 1);
    let mut fixed_bit_set = FixedBitSet::new(num_bits as usize);
    for &e in &bits {
        fixed_bit_set.set(e as usize);
    }
    for read_only in [false, true] {
        let read_only_bits;
        let bits_to_copy: &dyn Bits = if read_only {
            read_only_bits = fixed_bit_set.as_read_only_bits();
            &read_only_bits
        } else {
            &fixed_bit_set
        };
        let mutable_copy = FixedBitSet::copy_of(bits_to_copy);
        assert_not_same(
            &mutable_copy as *const FixedBitSet as *const u8,
            bits_to_copy as *const dyn Bits as *const u8,
        );
        Assert::assert_true(mutable_copy == fixed_bit_set);
    }

    let bits_to_copy = DelegatingBits(&fixed_bit_set);
    let mutable_copy = FixedBitSet::copy_of(&bits_to_copy);

    assert_not_same(
        &bits_to_copy as *const DelegatingBits as *const u8,
        &mutable_copy as *const FixedBitSet as *const u8,
    );
    assert_not_same(
        &fixed_bit_set as *const FixedBitSet as *const u8,
        &mutable_copy as *const FixedBitSet as *const u8,
    );
    Assert::assert_true(mutable_copy == fixed_bit_set);
    Ok(())
}

fn test_as_bits() -> Result<()> {
    let mut set = FixedBitSet::new(10);
    set.set(3);
    set.set(4);
    set.set(9);
    let bits = set.as_read_only_bits();
    Assert::assert_false(bits.bits_as_any().is_some_and(|a| a.is::<FixedBitSet>()));
    Assert::assert_equals(set.length(), bits.length());
    for i in 0..set.length() {
        Assert::assert_equals(set.get(i), bits.get(i));
    }
    // Further changes are reflected
    set.set(5);
    Assert::assert_true(bits.get(5));
    Ok(())
}

fn test_scan_is_empty() -> Result<()> {
    let mut set = FixedBitSet::new(0);
    Assert::assert_true(set.scan_is_empty());

    set = FixedBitSet::new(13);
    Assert::assert_true(set.scan_is_empty());
    set.set(10);
    Assert::assert_false(set.scan_is_empty());

    set = FixedBitSet::new(1024);
    Assert::assert_true(set.scan_is_empty());
    set.set(3);
    Assert::assert_false(set.scan_is_empty());
    set.clear(3);
    set.set(1020);
    Assert::assert_false(set.scan_is_empty());

    set = FixedBitSet::new(1030);
    Assert::assert_true(set.scan_is_empty());
    set.set(3);
    Assert::assert_false(set.scan_is_empty());
    set.clear(3);
    set.set(1028);
    Assert::assert_false(set.scan_is_empty());
    Ok(())
}

fn test_or_range() -> Result<()> {
    let mut dest = FixedBitSet::new(1_000);
    let mut source = FixedBitSet::new(10_000);
    let mut i = 0;
    while i < source.length() {
        source.set(i);
        i += 3;
    }

    // Test all possible alignments, and both a "short" (less than 64) and a long length.
    for source_from in 64..128usize {
        for dest_from in 256..320usize {
            for length in [
                0,
                TestUtil::next_int(&*random(), 1, 64 - 1) as usize,
                TestUtil::next_int(&*random(), 64, 512) as usize,
            ] {
                dest.clear_all();
                let mut i = 0;
                while i < dest.length() {
                    dest.set(i);
                    i += 10;
                }
                FixedBitSet::or_range(&source, source_from, &mut dest, dest_from, length);
                for i in 0..dest.length() {
                    let dest_set = i % 10 == 0;
                    if i < dest_from || i >= dest_from + length {
                        // Outside of the range, unmodified
                        Assert::assert_equals_msg(Some(&format!("{i}")), dest_set, dest.get(i));
                    } else {
                        let source_set = source.get(source_from + (i - dest_from));
                        Assert::assert_equals(source_set || dest_set, dest.get(i));
                    }
                }
            }
        }
    }
    Ok(())
}

fn test_and_range() -> Result<()> {
    let mut dest = FixedBitSet::new(1_000);
    let mut source = FixedBitSet::new(10_000);
    let mut i = 0;
    while i < source.length() {
        source.set(i);
        i += 3;
    }

    // Test all possible alignments, and both a "short" (less than 64) and a long length.
    for source_from in 64..128usize {
        for dest_from in 256..320usize {
            for length in [
                0,
                TestUtil::next_int(&*random(), 1, 64 - 1) as usize,
                TestUtil::next_int(&*random(), 64, 512) as usize,
            ] {
                dest.clear_all();
                let mut i = 0;
                while i < dest.length() {
                    dest.set(i);
                    i += 2;
                }
                FixedBitSet::and_range(&source, source_from, &mut dest, dest_from, length);
                for i in 0..dest.length() {
                    let dest_set = i % 2 == 0;
                    if i < dest_from || i >= dest_from + length {
                        // Outside of the range, unmodified
                        Assert::assert_equals_msg(Some(&format!("{i}")), dest_set, dest.get(i));
                    } else {
                        let source_set = source.get(source_from + (i - dest_from));
                        Assert::assert_equals_msg(
                            Some(&format!("{i}")),
                            source_set && dest_set,
                            dest.get(i),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

/// `slowCardinality(bitSet, from, to)`.
fn slow_cardinality(bit_set: &FixedBitSet, from: i32, to: i32) -> Result<i32> {
    let mut iterator =
        BitSetIterator::new(Arc::new(bit_set.clone()), bit_set.cardinality() as i64)?;
    let mut cardinality = 0;
    let mut i = iterator.advance(from)?;
    while i < to {
        cardinality += 1;
        i = iterator.next_doc()?;
    }
    Ok(cardinality)
}

fn test_range_cardinality() -> Result<()> {
    let mut bit_set = FixedBitSet::new(0);
    Assert::assert_equals(0usize, bit_set.cardinality_range(0, 0));

    bit_set = FixedBitSet::new(TestUtil::next_int(&*random(), 64, 4096) as usize);
    let mut i = 0;
    while i < bit_set.length() {
        bit_set.set(i);
        i += TestUtil::next_int(&*random(), 1, 16) as usize;
    }
    let len = bit_set.length();
    Assert::assert_equals(0usize, bit_set.cardinality_range(0, 0));
    Assert::assert_equals(0usize, bit_set.cardinality_range(len, len));
    Assert::assert_equals(
        slow_cardinality(&bit_set, 0, 1)? as usize,
        bit_set.cardinality_range(0, 1),
    );
    Assert::assert_equals(
        slow_cardinality(&bit_set, len as i32 - 1, len as i32)? as usize,
        bit_set.cardinality_range(len - 1, len),
    );
    Assert::assert_equals(
        slow_cardinality(&bit_set, 0, 63)? as usize,
        bit_set.cardinality_range(0, 63),
    );
    Assert::assert_equals(
        slow_cardinality(&bit_set, len as i32 - 63, len as i32)? as usize,
        bit_set.cardinality_range(len - 63, len),
    );
    Assert::assert_equals(
        slow_cardinality(&bit_set, 1, len as i32 - 1)? as usize,
        bit_set.cardinality_range(1, len - 1),
    );
    Ok(())
}

fn test_for_each() -> Result<()> {
    let mut bit_set = FixedBitSet::new(TestUtil::next_int(&*random(), 64, 4096) as usize);
    let mut i = 0;
    while i < bit_set.length() {
        bit_set.set(i);
        i += TestUtil::next_int(&*random(), 1, 16) as usize;
    }
    for _ in 0..100 {
        let from = TestUtil::next_int(&*random(), 0, bit_set.length() as i32);
        let to = TestUtil::next_int(&*random(), from, bit_set.length() as i32);
        do_test_for_each(&bit_set, from, to)?;
    }
    Ok(())
}

/// `doTestForEach(bitSet, from, to)`.
fn do_test_for_each(bit_set: &FixedBitSet, from: i32, to: i32) -> Result<()> {
    let base = TestUtil::next_int(&*random(), 0, 100);

    let mut expected: Vec<i32> = Vec::new();
    let mut iterator = BitSetIterator::new(
        Arc::new(bit_set.clone()),
        bit_set.approximate_cardinality() as i64,
    )?;
    let mut doc = iterator.advance(from)?;
    while doc < to {
        expected.push(base + doc);
        doc = iterator.next_doc()?;
    }

    let mut actual: Vec<i32> = Vec::new();
    bit_set
        .for_each::<std::convert::Infallible, _>(from as usize, to as usize, base, &mut |d| {
            actual.push(d);
            Ok(())
        })
        .unwrap_or_else(|e| match e {});

    Assert::assert_true(expected == actual);
    Ok(())
}

fn test_into_array() -> Result<()> {
    for _ in 0..100 {
        let num_bits = TestUtil::next_int(&*random(), 10, 1_000);
        let num_set_bits = TestUtil::next_int(&*random(), 0, num_bits);

        let mut bit_set = FixedBitSet::new(num_bits as usize);
        while (bit_set.cardinality() as i32) < num_set_bits {
            bit_set.set(random().next_int_bound(bit_set.length() as i32) as usize);
        }

        for _ in 0..100 {}

        let mut array: Vec<i32> =
            vec![0; TestUtil::next_int(&*random(), 0, num_set_bits + 10) as usize];
        let (from, to);
        if random().next_boolean() {
            from = 0;
            to = bit_set.length();
        } else {
            from = random().next_int_bound(bit_set.length() as i32) as usize;
            to = from + random().next_int_bound((bit_set.length() - from) as i32) as usize;
        }

        let base = random().next_int_bound(1000);
        let size = bit_set.into_array(from, to, base, &mut array);

        if size < array.len() {
            // All set bits should have been copied
            Assert::assert_equals(bit_set.cardinality_range(from, to), size);
        }

        let mut index = 0usize;
        bit_set
            .for_each::<std::convert::Infallible, _>(from, to, base, &mut |bit| {
                if index < array.len() {
                    Assert::assert_equals(bit, array[index]);
                    index += 1;
                }
                Ok(())
            })
            .unwrap_or_else(|e| match e {});
        Assert::assert_equals(size, index);
    }
    Ok(())
}

/// Tests that orMask produces the same result as setting bits individually.
fn test_or_mask() -> Result<()> {
    let rng = random();
    // Test with various mask lengths (simulating 2/4/8 SIMD lanes) and all alignments
    for mask_len in [2usize, 4, 8] {
        // Test all possible bit offsets within a word (0-63) to cover alignment cases
        for start_bit in 0..128usize {
            let mask = (rng.next_long() as u64) & ((1u64 << mask_len) - 1);

            let mut fast = FixedBitSet::new(256);
            let mut slow = FixedBitSet::new(256);
            // This is done to ensure some bits are already set and get don't wiped off.
            let mut i = 0;
            while i < 256 {
                fast.set(i);
                slow.set(i);
                i += 7;
            }

            // Fast path: orMask
            fast.or_mask(start_bit, mask, mask_len);

            // Slow path: per-bit set
            let mut m = mask;
            while m != 0 {
                let bit = m.trailing_zeros() as usize;
                slow.set(start_bit + bit);
                m &= m - 1;
            }

            // Verify fast and slow path bit matches
            if slow != fast {
                Assert::fail(&format!(
                    "orMask mismatch: startBit={start_bit}, mask=0b{mask:b}, maskLen={mask_len} expected:<{slow}> but was:<{fast}>"
                ));
            }
        }
    }
    Ok(())
}

/// Tests orMask at word boundaries (straddling two longs).
fn test_or_mask_straddling() -> Result<()> {
    // Force straddling: startBit near end of a 64-bit word
    for mask_len in [4usize, 8] {
        for bit_offset in 64 - mask_len + 1..64 {
            // positions 61-63 for maskLen=4, 57-63 for maskLen=8
            let mask = (1u64 << mask_len) - 1; // all bits set

            let mut fast = FixedBitSet::new(128);
            fast.or_mask(bit_offset, mask, mask_len);

            let mut slow = FixedBitSet::new(128);
            for i in 0..mask_len {
                slow.set(bit_offset + i);
            }

            if slow != fast {
                Assert::fail(&format!(
                    "Straddling mismatch: startBit={bit_offset}, maskLen={mask_len} expected:<{slow}> but was:<{fast}>"
                ));
            }
        }
    }
    Ok(())
}
