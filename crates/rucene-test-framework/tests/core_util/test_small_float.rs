//! Port of `lucene/core/src/test/org/apache/lucene/util/TestSmallFloat.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::SmallFloat;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestSmallFloat`. `testAllFloats` is `@Ignore("One-time test.")`: JUnit
/// never runs it, so it is translated below but not declared.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestSmallFloat",
        vec![
            test("testByteToFloat", test_byte_to_float),
            test("testFloatToByte", test_float_to_byte),
            test("testInt4", test_int4),
            test("testByte4", test_byte4),
        ],
    )
}

// original lucene byteToFloat
fn orig_byte_to_float(b: i8) -> f32 {
    if b == 0 {
        // zero is a special case
        return 0.0;
    }
    let mantissa = (b as i32) & 7;
    let exponent = ((b as i32) >> 3) & 31;
    let bits = ((exponent + (63 - 15)) << 24) | (mantissa << 21);
    f32::from_bits(bits as u32)
}

/// `Float.floatToIntBits(f)`.
fn float_to_int_bits(f: f32) -> i32 {
    if f.is_nan() {
        0x7fc00000
    } else {
        f.to_bits() as i32
    }
}

// original lucene floatToByte (since lucene 1.3)
fn orig_float_to_byte_v13(mut f: f32) -> i8 {
    if f < 0.0 {
        // round negatives up to zero
        f = 0.0;
    }
    if f == 0.0 {
        // zero is a special case
        return 0;
    }
    let bits = float_to_int_bits(f); // parse float into parts
    let mut mantissa = (bits & 0xffffff) >> 21;
    let mut exponent = (((bits >> 24) & 0x7f) - 63) + 15;
    if exponent > 31 {
        // overflow: use max value
        exponent = 31;
        mantissa = 7;
    }
    if exponent < 0 {
        // underflow: use min value
        exponent = 0;
        mantissa = 1;
    }
    ((exponent << 3) | mantissa) as i8 // pack into a byte
}

// This is the original lucene floatToBytes (from v1.3)
// except with the underflow detection bug fixed for values like 5.8123817E-10f
fn orig_float_to_byte(mut f: f32) -> i8 {
    if f < 0.0 {
        // round negatives up to zero
        f = 0.0;
    }
    if f == 0.0 {
        // zero is a special case
        return 0;
    }
    let bits = float_to_int_bits(f); // parse float into parts
    let mut mantissa = (bits & 0xffffff) >> 21;
    let mut exponent = (((bits >> 24) & 0x7f) - 63) + 15;
    if exponent > 31 {
        // overflow: use max value
        exponent = 31;
        mantissa = 7;
    }
    if exponent < 0 || exponent == 0 && mantissa == 0 {
        // underflow: use min value
        exponent = 0;
        mantissa = 1;
    }
    ((exponent << 3) | mantissa) as i8 // pack into a byte
}

fn test_byte_to_float() -> Result<()> {
    for i in 0..256 {
        let f1 = orig_byte_to_float(i as u8 as i8);
        let f2 = SmallFloat::byte_to_float(i as u8, 3, 15);
        let f3 = SmallFloat::byte315_to_float(i as u8);
        Assert::assert_equals_float_msg(None, f1, f2, 0.0);
        Assert::assert_equals_float_msg(None, f2, f3, 0.0);
    }
    Ok(())
}

/// Java's signed `byte` of a Rucene `u8`.
fn sb(b: u8) -> i8 {
    b as i8
}

fn test_float_to_byte() -> Result<()> {
    assert_equals(&0, &orig_float_to_byte_v13(5.8123817E-10)); // verify the old bug (see LUCENE-2937)
    assert_equals(&1, &orig_float_to_byte(5.8123817E-10)); // verify it's fixed in this test code
    assert_equals(&1, &sb(SmallFloat::float_to_byte315(5.8123817E-10))); // verify it's fixed

    // test some constants
    assert_equals(&0, &sb(SmallFloat::float_to_byte315(0.0)));
    // underflow rounds up to smallest positive
    assert_equals(&1, &sb(SmallFloat::float_to_byte315(f32::from_bits(1))));
    // overflow rounds down to largest positive
    assert_equals(
        &255,
        &((SmallFloat::float_to_byte315(f32::MAX) as i32) & 0xff),
    );
    assert_equals(
        &255,
        &((SmallFloat::float_to_byte315(f32::INFINITY) as i32) & 0xff),
    );

    // all negatives map to 0
    assert_equals(&0, &sb(SmallFloat::float_to_byte315(-f32::from_bits(1))));
    assert_equals(&0, &sb(SmallFloat::float_to_byte315(-f32::MAX)));
    assert_equals(&0, &sb(SmallFloat::float_to_byte315(f32::NEG_INFINITY)));

    // up iterations for more exhaustive test after changing something
    let num = LuceneTestCase::at_least(100000);
    for _ in 0..num {
        let f = f32::from_bits(random().next_int() as u32);
        if f.is_nan() {
            continue; // skip NaN
        }
        let b1 = orig_float_to_byte(f);
        let b2 = sb(SmallFloat::float_to_byte(f, 3, 15));
        let b3 = sb(SmallFloat::float_to_byte315(f));
        assert_equals(&b1, &b2);
        assert_equals(&b2, &b3);
    }
    Ok(())
}

fn test_int4() -> Result<()> {
    for i in 0..=16i64 {
        // all values in 0-16 are encoded accurately
        assert_equals(&i, &SmallFloat::int4_to_long(SmallFloat::long_to_int4(i)?));
    }
    let max_encoded = SmallFloat::long_to_int4(i64::MAX)?;
    for i in 1..max_encoded {
        Assert::assert_true(SmallFloat::int4_to_long(i) > SmallFloat::int4_to_long(i - 1));
    }
    let iters = LuceneTestCase::at_least(1000);
    for _ in 0..iters {
        let l = TestUtil::next_long(&*random(), 0, 1i64 << TestUtil::next_int(&*random(), 5, 61));
        let num_bits = 64 - l.leading_zeros() as i32;
        let mut expected = l;
        if num_bits > 4 {
            let mask = !0i64 << (num_bits - 4);
            expected &= mask;
        }
        let l2 = SmallFloat::int4_to_long(SmallFloat::long_to_int4(l)?);
        assert_equals(&expected, &l2);
    }
    Ok(())
}

fn test_byte4() -> Result<()> {
    let mut decoded = [0i32; 256];
    for b in 0..256usize {
        decoded[b] = SmallFloat::byte4_to_int(b as u8);
        assert_equals(&(b as u8 as i8), &sb(SmallFloat::int_to_byte4(decoded[b])?));
    }
    for i in 1..256 {
        Assert::assert_true(decoded[i] > decoded[i - 1]);
    }
    assert_equals(&(255u8 as i8), &sb(SmallFloat::int_to_byte4(i32::MAX)?));
    let iters = LuceneTestCase::at_least(1000);
    for _ in 0..iters {
        let i = random().next_int_bound(1 << TestUtil::next_int(&*random(), 5, 30));
        // `Arrays.binarySearch(int[], int)`.
        let mut idx: i32 = match decoded.binary_search(&i) {
            Ok(p) => p as i32,
            Err(p) => -(p as i32) - 1,
        };
        if idx < 0 {
            idx = -2 - idx;
        }
        Assert::assert_true(decoded[idx as usize] <= i);
        assert_equals(&(idx as u8 as i8), &sb(SmallFloat::int_to_byte4(i)?));
    }
    Ok(())
}

// `@Ignore("One-time test.")`.
#[allow(dead_code)]
fn test_all_floats() -> Result<()> {
    let mut i = i32::MIN;
    loop {
        let f = f32::from_bits(i as u32);
        if !f.is_nan() {
            let b1 = orig_float_to_byte(f);
            let b2 = sb(SmallFloat::float_to_byte315(f));
            if b1 != b2 || b2 == 0 && f > 0.0 {
                // `Float.floatToRawIntBits(i)`: `i` widened to a float.
                Assert::fail(&format!(
                    "Failed floatToByte315 for float {f} source bits={:x} float raw bits={:x}",
                    i,
                    (i as f32).to_bits()
                ));
            }
        }
        if i == i32::MAX {
            break;
        }
        i += 1;
    }
    Ok(())
}
