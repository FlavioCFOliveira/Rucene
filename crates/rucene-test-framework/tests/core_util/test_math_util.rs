//! Port of `lucene/core/src/test/org/apache/lucene/util/TestMathUtil.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::MathUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestMathUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestMathUtil",
        vec![
            test("testGCD", test_gcd),
            test("testGCD2", test_gcd2),
            test("testAcoshMethod", test_acosh_method),
            test("testAsinhMethod", test_asinh_method),
            test("testAtanhMethod", test_atanh_method),
            test("testUnsignedMin", test_unsigned_min),
        ],
    )
}

const PRIMES: [i64; 10] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29];

/// `randomLong()`.
pub fn random_long() -> i64 {
    if random().next_boolean() {
        let mut l: i64 = 1;
        if random().next_boolean() {
            l = l.wrapping_mul(-1);
        }
        for i in PRIMES {
            let m = random().next_int_bound(3);
            for _ in 0..m {
                l = l.wrapping_mul(i);
            }
        }
        l
    } else if random().next_boolean() {
        random().next_long()
    } else {
        *RandomPicks::random_from(&*random(), &[i64::MIN, i64::MAX, 0, -1, 1])
    }
}

/// `gcd(long l1, long l2)`: the slow `BigInteger` version used for testing,
/// over the magnitudes as `u128` (`BigInteger.gcd` is never negative).
fn slow_gcd(l1: i64, l2: i64) -> i64 {
    let mut a = (l1 as i128).unsigned_abs();
    let mut b = (l2 as i128).unsigned_abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    // assert gcd.bitCount() <= 64;
    Assert::assert_true(a.count_ones() <= 64);
    // `BigInteger.longValue()`: the low 64 bits.
    a as u64 as i64
}

fn test_gcd() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let l1 = random_long();
        let l2 = random_long();
        let gcd = MathUtil::gcd(l1, l2);
        let actual_gcd = slow_gcd(l1, l2);
        assert_equals(&actual_gcd, &gcd);
        if gcd != 0 {
            assert_equals(&l1, &l1.wrapping_div(gcd).wrapping_mul(gcd));
            assert_equals(&l2, &l2.wrapping_div(gcd).wrapping_mul(gcd));
        }
    }
    Ok(())
}

// ported test from commons-math
fn test_gcd2() -> Result<()> {
    let a: i64 = 30;
    let b: i64 = 50;
    let c: i64 = 77;

    assert_equals(&0, &MathUtil::gcd(0, 0));

    assert_equals(&b, &MathUtil::gcd(0, b));
    assert_equals(&a, &MathUtil::gcd(a, 0));
    assert_equals(&b, &MathUtil::gcd(0, -b));
    assert_equals(&a, &MathUtil::gcd(-a, 0));

    assert_equals(&10, &MathUtil::gcd(a, b));
    assert_equals(&10, &MathUtil::gcd(-a, b));
    assert_equals(&10, &MathUtil::gcd(a, -b));
    assert_equals(&10, &MathUtil::gcd(-a, -b));

    assert_equals(&1, &MathUtil::gcd(a, c));
    assert_equals(&1, &MathUtil::gcd(-a, c));
    assert_equals(&1, &MathUtil::gcd(a, -c));
    assert_equals(&1, &MathUtil::gcd(-a, -c));

    assert_equals(
        &(3i64 * (1i64 << 45)),
        &MathUtil::gcd(3i64 * (1i64 << 50), 9i64 * (1i64 << 45)),
    );

    assert_equals(&(1i64 << 45), &MathUtil::gcd(1i64 << 45, i64::MIN));

    assert_equals(&i64::MAX, &MathUtil::gcd(i64::MAX, 0));
    assert_equals(&i64::MAX, &MathUtil::gcd(-i64::MAX, 0));
    assert_equals(&1, &MathUtil::gcd(60247241209, 153092023));

    assert_equals(&i64::MIN, &MathUtil::gcd(i64::MIN, 0));
    assert_equals(&i64::MIN, &MathUtil::gcd(0, i64::MIN));
    assert_equals(&i64::MIN, &MathUtil::gcd(i64::MIN, i64::MIN));
    Ok(())
}

/// `Double.doubleToLongBits(d)`: NaN collapses to the canonical NaN.
fn double_to_long_bits(d: f64) -> i64 {
    if d.is_nan() {
        0x7ff8000000000000
    } else {
        d.to_bits() as i64
    }
}

fn test_acosh_method() -> Result<()> {
    // acosh(NaN) == NaN
    Assert::assert_true(MathUtil::acosh(f64::NAN).is_nan());
    // acosh(1) == +0
    assert_equals(&0, &double_to_long_bits(MathUtil::acosh(1.0)));
    // acosh(POSITIVE_INFINITY) == POSITIVE_INFINITY
    assert_equals(
        &double_to_long_bits(f64::INFINITY),
        &double_to_long_bits(MathUtil::acosh(f64::INFINITY)),
    );
    // acosh(x) : x < 1 == NaN
    Assert::assert_true(MathUtil::acosh(0.9).is_nan()); // x < 1
    Assert::assert_true(MathUtil::acosh(0.0).is_nan()); // x == 0
    Assert::assert_true(MathUtil::acosh(-0.0).is_nan()); // x == -0
    Assert::assert_true(MathUtil::acosh(-0.9).is_nan()); // x < 0
    Assert::assert_true(MathUtil::acosh(-1.0).is_nan()); // x == -1
    Assert::assert_true(MathUtil::acosh(-10.0).is_nan()); // x < -1
    Assert::assert_true(MathUtil::acosh(f64::NEG_INFINITY).is_nan()); // x == -Inf

    let epsilon = 0.000001;
    assert_equals_delta(0.0, MathUtil::acosh(1.0), epsilon);
    assert_equals_delta(1.5667992369724109, MathUtil::acosh(2.5), epsilon);
    assert_equals_delta(14.719378760739708, MathUtil::acosh(1234567.89), epsilon);
    Ok(())
}

fn test_asinh_method() -> Result<()> {
    // asinh(NaN) == NaN
    Assert::assert_true(MathUtil::asinh(f64::NAN).is_nan());
    // asinh(+0) == +0
    assert_equals(&0, &double_to_long_bits(MathUtil::asinh(0.0)));
    // asinh(-0) == -0
    assert_equals(
        &double_to_long_bits(-0.0),
        &double_to_long_bits(MathUtil::asinh(-0.0)),
    );
    // asinh(POSITIVE_INFINITY) == POSITIVE_INFINITY
    assert_equals(
        &double_to_long_bits(f64::INFINITY),
        &double_to_long_bits(MathUtil::asinh(f64::INFINITY)),
    );
    // asinh(NEGATIVE_INFINITY) == NEGATIVE_INFINITY
    assert_equals(
        &double_to_long_bits(f64::NEG_INFINITY),
        &double_to_long_bits(MathUtil::asinh(f64::NEG_INFINITY)),
    );

    let epsilon = 0.000001;
    assert_equals_delta(-14.719378760740035, MathUtil::asinh(-1234567.89), epsilon);
    assert_equals_delta(-1.6472311463710958, MathUtil::asinh(-2.5), epsilon);
    assert_equals_delta(-0.8813735870195429, MathUtil::asinh(-1.0), epsilon);
    assert_equals_delta(0.0, MathUtil::asinh(0.0), 0.0);
    assert_equals_delta(0.8813735870195429, MathUtil::asinh(1.0), epsilon);
    assert_equals_delta(1.6472311463710958, MathUtil::asinh(2.5), epsilon);
    assert_equals_delta(14.719378760740035, MathUtil::asinh(1234567.89), epsilon);
    Ok(())
}

fn test_atanh_method() -> Result<()> {
    // atanh(NaN) == NaN
    Assert::assert_true(MathUtil::atanh(f64::NAN).is_nan());
    // atanh(+0) == +0
    assert_equals(&0, &double_to_long_bits(MathUtil::atanh(0.0)));
    // atanh(-0) == -0
    assert_equals(
        &double_to_long_bits(-0.0),
        &double_to_long_bits(MathUtil::atanh(-0.0)),
    );
    // atanh(1) == POSITIVE_INFINITY
    assert_equals(
        &double_to_long_bits(f64::INFINITY),
        &double_to_long_bits(MathUtil::atanh(1.0)),
    );
    // atanh(-1) == NEGATIVE_INFINITY
    assert_equals(
        &double_to_long_bits(f64::NEG_INFINITY),
        &double_to_long_bits(MathUtil::atanh(-1.0)),
    );
    // atanh(x) : Math.abs(x) > 1 == NaN
    Assert::assert_true(MathUtil::atanh(1.1).is_nan()); // x > 1
    Assert::assert_true(MathUtil::atanh(f64::INFINITY).is_nan()); // x == Inf
    Assert::assert_true(MathUtil::atanh(-1.1).is_nan()); // x < -1
    Assert::assert_true(MathUtil::atanh(f64::NEG_INFINITY).is_nan()); // x == -Inf

    let epsilon = 0.000001;
    assert_equals_delta(f64::NEG_INFINITY, MathUtil::atanh(-1.0), 0.0);
    assert_equals_delta(-0.5493061443340549, MathUtil::atanh(-0.5), epsilon);
    assert_equals_delta(0.0, MathUtil::atanh(0.0), 0.0);
    assert_equals_delta(0.5493061443340549, MathUtil::atanh(0.5), epsilon);
    assert_equals_delta(f64::INFINITY, MathUtil::atanh(1.0), 0.0);
    Ok(())
}

fn test_unsigned_min() -> Result<()> {
    assert_equals(&0, &MathUtil::unsigned_min(0, 0));
    assert_equals(&0, &MathUtil::unsigned_min(0, 3));
    assert_equals(&0, &MathUtil::unsigned_min(3, 0));
    assert_equals(&0, &MathUtil::unsigned_min(0, i32::MAX));
    assert_equals(&0, &MathUtil::unsigned_min(i32::MAX, 0));
    assert_equals(
        &i32::MAX,
        &MathUtil::unsigned_min(i32::MAX, i32::MAX.wrapping_add(1)),
    );
    assert_equals(
        &i32::MAX,
        &MathUtil::unsigned_min(i32::MAX.wrapping_add(1), i32::MAX),
    );
    assert_equals(&i32::MAX, &MathUtil::unsigned_min(i32::MAX, i32::MIN));
    assert_equals(&i32::MAX, &MathUtil::unsigned_min(i32::MIN, i32::MAX));
    assert_equals(&i32::MIN, &MathUtil::unsigned_min(i32::MIN, -1));
    assert_equals(&i32::MIN, &MathUtil::unsigned_min(-1, i32::MIN));
    assert_equals(&i32::MIN, &MathUtil::unsigned_min(i32::MIN, i32::MIN));
    Ok(())
}
