//! Port of `lucene/core/src/test/org/apache/lucene/util/TestLongBitSet.java`.
//!
//! `LongBitSet.ensureCapacity` takes its set by value in Rucene; where Java
//! keeps using the set it passed, the port passes a clone.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::LongBitSet;
use rucene_test_framework::jdk::bit_set::JavaBitSet;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{Monster, RANDOM_MULTIPLIER, TEST_NIGHTLY};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, unchecked};

/// `TestLongBitSet`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestLongBitSet",
        vec![
            test("testSmall", test_small),
            test("testEquals", test_equals),
            test("testHashCodeEquals", test_hash_code_equals),
            test("testTooLarge", test_too_large),
            test("testNegativeNumBits", test_negative_num_bits),
            test("testSmallBitSets", test_small_bit_sets),
            test("testPrevSetBit", test_prev_set_bit),
            test("testNextBitSet", test_next_bit_set),
            test("testEnsureCapacity", test_ensure_capacity),
            test("testHugeCapacity", test_huge_capacity).annotate(Monster {
                value: "needs hundreds of MB of heap".to_string(),
            }),
            test("testBits2Words", test_bits2_words),
        ],
    )
}

/// `doGet(a, b)`.
fn do_get(a: &JavaBitSet, b: &LongBitSet) {
    assert_equals(&(a.cardinality() as i64), &b.cardinality());
    let max = b.length();
    for i in 0..max {
        if a.get(i as usize) != b.get(i) {
            Assert::fail(&format!("mismatch: BitSet=[{i}]={}", a.get(i as usize)));
        }
    }
}

/// `doNextSetBit(a, b)`.
fn do_next_set_bit(a: &JavaBitSet, b: &LongBitSet) {
    assert_equals(&(a.cardinality() as i64), &b.cardinality());
    let mut aa: i32 = -1;
    let mut bb: i64 = -1;
    loop {
        aa = a.next_set_bit((aa + 1) as usize);
        bb = if bb < b.length() - 1 {
            b.next_set_bit(bb + 1)
        } else {
            -1
        };
        assert_equals(&(aa as i64), &bb);
        if aa < 0 {
            break;
        }
    }
}

/// `doPrevSetBit(a, b)`.
fn do_prev_set_bit(a: &JavaBitSet, b: &LongBitSet) {
    assert_equals(&(a.cardinality() as i64), &b.cardinality());
    let mut aa: i32 = a.size() as i32 + random().next_int_bound(100);
    let mut bb: i64 = aa as i64;
    loop {
        // aa = a.prevSetBit(aa-1);
        aa -= 1;
        while aa >= 0 && !a.get(aa as usize) {
            aa -= 1;
        }
        if b.length() == 0 {
            bb = -1;
        } else if bb > b.length() - 1 {
            bb = b.prev_set_bit(b.length() - 1);
        } else if bb < 1 {
            bb = -1;
        } else {
            bb = if bb >= 1 { b.prev_set_bit(bb - 1) } else { -1 };
        }
        assert_equals(&(aa as i64), &bb);
        if aa < 0 {
            break;
        }
    }
}

/// `doRandomSets(maxSize, iter, mode)`.
fn do_random_sets(max_size: i32, iter: i32, _mode: i32) -> Result<()> {
    let mut a0: Option<JavaBitSet> = None;
    let mut b0: Option<LongBitSet> = None;

    for _ in 0..iter {
        let sz = TestUtil::next_int(&*random(), 2, max_size);
        let mut a = JavaBitSet::with_capacity(sz as usize);
        let mut b = LongBitSet::new(sz as i64)?;

        // test the various ways of setting bits
        if sz > 0 {
            let n_oper = random().next_int_bound(sz);
            for _ in 0..n_oper {
                let mut idx;

                idx = random().next_int_bound(sz);
                a.set(idx as usize);
                b.set(idx as i64);

                idx = random().next_int_bound(sz);
                a.clear(idx as usize);
                b.clear(idx as i64);

                idx = random().next_int_bound(sz);
                a.flip_range(idx as usize, idx as usize + 1);
                b.flip_range(idx as i64, idx as i64 + 1);

                idx = random().next_int_bound(sz);
                a.flip(idx as usize);
                b.flip(idx as i64);

                let val2 = b.get(idx as i64);
                let val = b.get_and_set(idx as i64);
                Assert::assert_true(val2 == val);
                Assert::assert_true(b.get(idx as i64));

                if !val {
                    b.clear(idx as i64);
                }
                Assert::assert_true(b.get(idx as i64) == val);
            }
        }

        // test that the various ways of accessing the bits are equivalent
        do_get(&a, &b);

        // test ranges, including possible extension
        let mut from_index = random().next_int_bound(sz / 2);
        let mut to_index = from_index + random().next_int_bound(sz - from_index);
        let mut aa = a.clone();
        aa.flip_range(from_index as usize, to_index as usize);
        let mut bb = b.clone();
        bb.flip_range(from_index as i64, to_index as i64);

        from_index = random().next_int_bound(sz / 2);
        to_index = from_index + random().next_int_bound(sz - from_index);
        aa = a.clone();
        aa.clear_range(from_index as usize, to_index as usize);
        bb = b.clone();
        bb.clear_range(from_index as i64, to_index as i64);

        do_next_set_bit(&aa, &bb); // a problem here is from clear() or nextSetBit

        do_prev_set_bit(&aa, &bb);

        from_index = random().next_int_bound(sz / 2);
        to_index = from_index + random().next_int_bound(sz - from_index);
        aa = a.clone();
        aa.set_range(from_index as usize, to_index as usize);
        bb = b.clone();
        bb.set_range(from_index as i64, to_index as i64);

        do_next_set_bit(&aa, &bb); // a problem here is from set() or nextSetBit

        do_prev_set_bit(&aa, &bb);

        if let (Some(a0), Some(b0)) = (&a0, &b0) {
            if b0.length() <= b.length() {
                assert_equals(&(a.cardinality() as i64), &b.cardinality());

                let mut a_and = a.clone();
                a_and.and(a0);
                let mut a_or = a.clone();
                a_or.or(a0);
                let mut a_xor = a.clone();
                a_xor.xor(a0);
                let mut a_andn = a.clone();
                a_andn.and_not(a0);

                let mut b_and = b.clone();
                assert_equals(&b, &b_and);
                b_and.and(b0);
                let mut b_or = b.clone();
                b_or.or(b0);
                let mut b_xor = b.clone();
                b_xor.xor(b0);
                let mut b_andn = b.clone();
                b_andn.and_not(b0);

                assert_equals(&(a0.cardinality() as i64), &b0.cardinality());
                assert_equals(&(a_or.cardinality() as i64), &b_or.cardinality());

                assert_equals(&(a_and.cardinality() as i64), &b_and.cardinality());
                assert_equals(&(a_or.cardinality() as i64), &b_or.cardinality());
                assert_equals(&(a_xor.cardinality() as i64), &b_xor.cardinality());
                assert_equals(&(a_andn.cardinality() as i64), &b_andn.cardinality());
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
    do_random_sets(LuceneTestCase::at_least(1200), iters, 1)?;
    do_random_sets(LuceneTestCase::at_least(1200), iters, 2)
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
    let mut b1 = LongBitSet::new(num_bits as i64)?;
    let mut b2 = LongBitSet::new(num_bits as i64)?;
    Assert::assert_true(b1 == b2);
    Assert::assert_true(b2 == b1);
    for _ in 0..10 * *RANDOM_MULTIPLIER {
        let idx = random().next_int_bound(num_bits) as i64;
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
    // `b1.equals(new Object())`: `LongBitSet.equals` first checks the class.
    let object: Box<dyn Any> = Box::new(());
    Assert::assert_false(
        object
            .downcast_ref::<LongBitSet>()
            .is_some_and(|other| b1 == *other),
    );
    Ok(())
}

fn test_hash_code_equals() -> Result<()> {
    // This test can't handle numBits==0:
    let num_bits = random().next_int_bound(2000) + 1;
    let mut b1 = LongBitSet::new(num_bits as i64)?;
    let mut b2 = LongBitSet::new(num_bits as i64)?;
    Assert::assert_true(b1 == b2);
    Assert::assert_true(b2 == b1);
    for _ in 0..10 * *RANDOM_MULTIPLIER {
        let idx = random().next_int_bound(num_bits) as i64;
        if !b1.get(idx) {
            b1.set(idx);
            Assert::assert_false(b1 == b2);
            Assert::assert_false(b1.hash_code() == b2.hash_code());
            b2.set(idx);
            assert_equals(&b1, &b2);
            assert_equals(&b1.hash_code(), &b2.hash_code());
        }
    }
    Ok(())
}

fn test_too_large() -> Result<()> {
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        LongBitSet::new(LongBitSet::MAX_NUM_BITS + 1)
    });
    Assert::assert_true(
        e.message()
            .unwrap_or("")
            .starts_with("numBits must be 0 .. "),
    );
    Ok(())
}

fn test_negative_num_bits() -> Result<()> {
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        LongBitSet::new(-17)
    });
    Assert::assert_true(
        e.message()
            .unwrap_or("")
            .starts_with("numBits must be 0 .. "),
    );
    Ok(())
}

fn test_small_bit_sets() -> Result<()> {
    // Make sure size 0-10 bit sets are OK:
    for num_bits in 0..10i64 {
        let mut b1 = LongBitSet::new(num_bits)?;
        let b2 = LongBitSet::new(num_bits)?;
        Assert::assert_true(b1 == b2);
        assert_equals(&b1.hash_code(), &b2.hash_code());
        assert_equals(&0, &b1.cardinality());
        if num_bits > 0 {
            b1.set_range(0, num_bits);
            assert_equals(&num_bits, &b1.cardinality());
            b1.flip_range(0, num_bits);
            assert_equals(&0, &b1.cardinality());
        }
    }
    Ok(())
}

/// `makeLongBitSet(a, numBits)`.
fn make_long_bit_set(a: &[i32], num_bits: i32) -> Result<LongBitSet> {
    let mut bs = if random().next_boolean() {
        let bits2words = LongBitSet::bits2words(num_bits as i64)?;
        let words: Vec<i64> = vec![0; (bits2words + random().next_int_bound(100)) as usize];
        LongBitSet::from_bits(words, num_bits as i64)?
    } else {
        LongBitSet::new(num_bits as i64)?
    };
    for &e in a {
        bs.set(e as i64);
    }
    Ok(bs)
}

/// `makeBitSet(a)`.
fn make_bit_set(a: &[i32]) -> JavaBitSet {
    let mut bs = JavaBitSet::new();
    for &e in a {
        bs.set(e as usize);
    }
    bs
}

/// `checkPrevSetBitArray(a, numBits)`.
fn check_prev_set_bit_array(a: &[i32], num_bits: i32) -> Result<()> {
    let obs = make_long_bit_set(a, num_bits)?;
    let bs = make_bit_set(a);
    do_prev_set_bit(&bs, &obs);
    Ok(())
}

fn test_prev_set_bit() -> Result<()> {
    check_prev_set_bit_array(&[], 0)?;
    check_prev_set_bit_array(&[0], 1)?;
    check_prev_set_bit_array(&[0, 2], 3)
}

/// `checkNextSetBitArray(a, numBits)`.
fn check_next_set_bit_array(a: &[i32], num_bits: i32) -> Result<()> {
    let obs = make_long_bit_set(a, num_bits)?;
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

fn test_ensure_capacity() -> Result<()> {
    let mut bits = LongBitSet::new(5)?;
    bits.set(1);
    bits.set(4);

    let mut new_bits = LongBitSet::ensure_capacity(bits.clone(), 8)?; // grow within the word
    Assert::assert_true(new_bits.get(1));
    Assert::assert_true(new_bits.get(4));
    new_bits.clear(1);
    // we align to 64-bits, so even though it shouldn't have, it re-allocated a long[1]
    Assert::assert_true(bits.get(1));
    Assert::assert_false(new_bits.get(1));

    new_bits.set(1);
    let len = new_bits.length();
    new_bits = LongBitSet::ensure_capacity(new_bits, len - 2)?; // reuse
    Assert::assert_true(new_bits.get(1));

    bits.set(1);
    new_bits = LongBitSet::ensure_capacity(bits.clone(), 72)?; // grow beyond one word
    Assert::assert_true(new_bits.get(1));
    Assert::assert_true(new_bits.get(4));
    new_bits.clear(1);
    // we grew the long[], so it's not shared
    Assert::assert_true(bits.get(1));
    Assert::assert_false(new_bits.get(1));
    Ok(())
}

fn test_huge_capacity() -> Result<()> {
    let more_than_max_int = i32::MAX as i64 + 5;

    let bits = LongBitSet::new(42)?;

    assert_equals(&42, &bits.length());

    let huge_bits = LongBitSet::ensure_capacity(bits, more_than_max_int)?;

    Assert::assert_true(huge_bits.length() >= more_than_max_int);
    Ok(())
}

fn test_bits2_words() -> Result<()> {
    let bits2words = |n: i64| unchecked(LongBitSet::bits2words(n));
    assert_equals(&0, &bits2words(0));
    assert_equals(&1, &bits2words(1));
    // ...
    assert_equals(&1, &bits2words(64));
    assert_equals(&2, &bits2words(65));
    // ...
    assert_equals(&2, &bits2words(128));
    assert_equals(&3, &bits2words(129));
    // ...
    assert_equals(&(1 << (31 - 6)), &bits2words(i32::MAX as i64 + 1)); // == 1L << 31
    assert_equals(&((1 << (31 - 6)) + 1), &bits2words(i32::MAX as i64 + 2)); // == (1L << 31) + 1
                                                                             // ...
    assert_equals(&(1 << (32 - 6)), &bits2words(1i64 << 32));
    assert_equals(&((1 << (32 - 6)) + 1), &(bits2words(1i64 << 32) + 1));

    // ensure the claimed max num_bits doesn't throw exc; we can't enforce exact values here
    // because the value variees with JVM:
    Assert::assert_true(bits2words(LongBitSet::MAX_NUM_BITS) > 0);
    Ok(())
}
