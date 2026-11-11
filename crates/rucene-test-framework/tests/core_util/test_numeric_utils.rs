//! Port of `lucene/core/src/test/org/apache/lucene/util/TestNumericUtils.java`.
//!
//! Rust has no `java.math.BigInteger`. Rucene's `NumericUtils` takes and
//! returns big integers as two's-complement big-endian byte arrays
//! (`BigInteger.toByteArray()` and `new BigInteger(byte[])`), as does
//! `TestUtil.nextBigInteger`; the small helpers below give the test the few
//! `BigInteger` operations it uses on that form: `valueOf`, `equals`,
//! `compareTo`, `new BigInteger(numBits, rnd)`, `add`, `subtract` and
//! `toString`.

use std::cmp::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::{BytesRef, NumericUtils};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

/// `TestNumericUtils`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestNumericUtils",
        vec![
            test(
                "testLongConversionAndOrdering",
                test_long_conversion_and_ordering,
            ),
            test(
                "testIntConversionAndOrdering",
                test_int_conversion_and_ordering,
            ),
            test(
                "testBigIntConversionAndOrdering",
                test_big_int_conversion_and_ordering,
            ),
            test("testLongSpecialValues", test_long_special_values),
            test("testIntSpecialValues", test_int_special_values),
            test("testBigIntSpecialValues", test_big_int_special_values),
            test("testDoubles", test_doubles),
            test("testSortableDoubleNaN", test_sortable_double_nan),
            test("testFloats", test_floats),
            test("testSortableFloatNaN", test_sortable_float_nan),
            test("testAdd", test_add),
            test("testIllegalAdd", test_illegal_add),
            test("testSubtract", test_subtract),
            test("testIllegalSubtract", test_illegal_subtract),
            test("testIntsRoundTrip", test_ints_round_trip),
            test("testLongsRoundTrip", test_longs_round_trip),
            test("testFloatsRoundTrip", test_floats_round_trip),
            test("testDoublesRoundTrip", test_doubles_round_trip),
            test("testBigIntsRoundTrip", test_big_ints_round_trip),
            test("testIntsCompare", test_ints_compare),
            test("testLongsCompare", test_longs_compare),
            test("testFloatsCompare", test_floats_compare),
            test("testDoublesCompare", test_doubles_compare),
            test("testBigIntsCompare", test_big_ints_compare),
        ],
    )
}

// ---------------------------------------------------------------------------
// BigInteger over two's-complement big-endian bytes
// ---------------------------------------------------------------------------

/// `new BigInteger(byte[])` normalised back to `toByteArray()`: the redundant
/// sign bytes dropped.
fn normalize(bytes: &[u8]) -> Vec<u8> {
    if bytes.is_empty() {
        return vec![0];
    }
    let mut i = 0;
    while i + 1 < bytes.len()
        && ((bytes[i] == 0 && bytes[i + 1] & 0x80 == 0)
            || (bytes[i] == 0xff && bytes[i + 1] & 0x80 != 0))
    {
        i += 1;
    }
    bytes[i..].to_vec()
}

/// `BigInteger.valueOf(value).toByteArray()`.
fn big_value_of(value: i64) -> Vec<u8> {
    normalize(&value.to_be_bytes())
}

/// `BigInteger.compareTo` over two's-complement bytes.
fn big_compare(a: &[u8], b: &[u8]) -> Ordering {
    let width = a.len().max(b.len());
    let extend = |x: &[u8]| -> Vec<u8> {
        let fill = if x[0] & 0x80 != 0 { 0xff } else { 0 };
        let mut v = vec![fill; width - x.len()];
        v.extend_from_slice(x);
        v[0] ^= 0x80; // signed order as unsigned order
        v
    };
    extend(a).cmp(&extend(b))
}

/// `Integer.signum(x)` of an [`Ordering`].
fn signum(o: Ordering) -> i32 {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// `new BigInteger(numBits, rnd)`: a uniformly random non-negative integer
/// below `2^numBits`, drawn as `randomBits` does (`nextBytes` over
/// `(numBits + 7) / 8` bytes, the excess high bits cleared), as unsigned
/// big-endian magnitude bytes.
fn big_random(num_bits: i32, rnd: &dyn Random) -> Vec<u8> {
    let num_bytes = ((num_bits as i64 + 7) / 8) as usize;
    let mut random_bits = vec![0u8; num_bytes];
    if num_bytes > 0 {
        rnd.next_bytes(&mut random_bits);
        let excess_bits = 8 * num_bytes as i32 - num_bits;
        random_bits[0] &= ((1u32 << (8 - excess_bits)) - 1) as u8;
    }
    random_bits
}

/// The magnitude `mag` (unsigned big-endian) as `toByteArray()`.
fn mag_to_byte_array(mag: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(mag);
    normalize(&v)
}

/// Unsigned big-endian `a + b`.
fn mag_add(a: &[u8], b: &[u8]) -> Vec<u8> {
    let width = a.len().max(b.len()) + 1;
    let mut out = vec![0u8; width];
    let mut carry = 0u32;
    for k in 0..width {
        let da = if k < a.len() {
            a[a.len() - 1 - k] as u32
        } else {
            0
        };
        let db = if k < b.len() {
            b[b.len() - 1 - k] as u32
        } else {
            0
        };
        let s = da + db + carry;
        out[width - 1 - k] = s as u8;
        carry = s >> 8;
    }
    out
}

/// Unsigned big-endian `a - b`, `a >= b`.
fn mag_sub(a: &[u8], b: &[u8]) -> Vec<u8> {
    let width = a.len().max(b.len());
    let mut out = vec![0u8; width];
    let mut borrow = 0i32;
    for k in 0..width {
        let da = if k < a.len() {
            a[a.len() - 1 - k] as i32
        } else {
            0
        };
        let db = if k < b.len() {
            b[b.len() - 1 - k] as i32
        } else {
            0
        };
        let mut d = da - db - borrow;
        borrow = 0;
        if d < 0 {
            d += 256;
            borrow = 1;
        }
        out[width - 1 - k] = d as u8;
    }
    out
}

/// `BigInteger.toString()` of a non-negative magnitude.
fn mag_to_string(mag: &[u8]) -> String {
    let mut digits: Vec<u8> = Vec::new();
    let mut n: Vec<u8> = mag.to_vec();
    while n.iter().any(|&b| b != 0) {
        let mut rem = 0u32;
        for b in n.iter_mut() {
            let cur = (rem << 8) | *b as u32;
            *b = (cur / 10) as u8;
            rem = cur % 10;
        }
        digits.push(b'0' + rem as u8);
    }
    if digits.is_empty() {
        return "0".to_string();
    }
    digits.reverse();
    String::from_utf8(digits).expect("ASCII digits")
}

/// `previous.compareTo(current)` over `BytesRef`s.
fn bytes_compare(a: &BytesRef, b: &BytesRef) -> Ordering {
    a.cmp(b)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// generate a series of encoded longs, each numerical one bigger than the one
/// before. check for correct ordering of the encoded bytes and that values
/// round-trip.
fn test_long_conversion_and_ordering() -> Result<()> {
    let mut previous: Option<BytesRef> = None;
    let mut current = BytesRef::new(vec![0u8; 8]);
    for value in -100000i64..100000 {
        let off = current.offset;
        NumericUtils::long_to_sortable_bytes(value, &mut current.bytes, off);
        match &previous {
            None => previous = Some(BytesRef::new(vec![0u8; 8])),
            Some(p) => {
                // test if smaller
                Assert::assert_true_msg(
                    "current bigger than previous: ",
                    bytes_compare(p, &current) == Ordering::Less,
                );
            }
        }
        // test is back and forward conversion works
        assert_equals_msg(
            Some("forward and back conversion should generate same long"),
            &value,
            &NumericUtils::sortable_bytes_to_long(&current.bytes, current.offset),
        );
        // next step
        let p = previous.as_mut().expect("previous set");
        let (po, co, cl) = (p.offset, current.offset, current.length);
        p.bytes[po..po + cl].copy_from_slice(&current.bytes[co..co + cl]);
    }
    Ok(())
}

/// generate a series of encoded ints, each numerical one bigger than the one
/// before. check for correct ordering of the encoded bytes and that values
/// round-trip.
fn test_int_conversion_and_ordering() -> Result<()> {
    let mut previous: Option<BytesRef> = None;
    let mut current = BytesRef::new(vec![0u8; 4]);
    for value in -100000i32..100000 {
        let off = current.offset;
        NumericUtils::int_to_sortable_bytes(value, &mut current.bytes, off);
        match &previous {
            None => previous = Some(BytesRef::new(vec![0u8; 4])),
            Some(p) => {
                // test if smaller
                Assert::assert_true_msg(
                    "current bigger than previous: ",
                    bytes_compare(p, &current) == Ordering::Less,
                );
            }
        }
        // test is back and forward conversion works
        assert_equals_msg(
            Some("forward and back conversion should generate same int"),
            &value,
            &NumericUtils::sortable_bytes_to_int(&current.bytes, current.offset),
        );
        // next step
        let p = previous.as_mut().expect("previous set");
        let (po, co, cl) = (p.offset, current.offset, current.length);
        p.bytes[po..po + cl].copy_from_slice(&current.bytes[co..co + cl]);
    }
    Ok(())
}

/// generate a series of encoded BigIntegers, each numerical one bigger than
/// the one before. check for correct ordering of the encoded bytes and that
/// values round-trip.
fn test_big_int_conversion_and_ordering() -> Result<()> {
    // we need at least 3 bytes of storage.
    let size = TestUtil::next_int(&*random(), 3, 16) as usize;
    let mut previous: Option<BytesRef> = None;
    let mut current = BytesRef::new(vec![0u8; size]);
    for value in -100000i64..100000 {
        let off = current.offset;
        NumericUtils::big_int_to_sortable_bytes(
            &big_value_of(value),
            size,
            &mut current.bytes,
            off,
        );
        match &previous {
            None => previous = Some(BytesRef::new(vec![0u8; size])),
            Some(p) => {
                // test if smaller
                Assert::assert_true_msg(
                    "current bigger than previous: ",
                    bytes_compare(p, &current) == Ordering::Less,
                );
            }
        }
        // test is back and forward conversion works
        assert_equals_msg(
            Some("forward and back conversion should generate same BigInteger"),
            &big_value_of(value),
            &normalize(&NumericUtils::sortable_bytes_to_big_int(
                &current.bytes,
                current.offset,
                current.length,
            )),
        );
        // next step
        let p = previous.as_mut().expect("previous set");
        let (po, co, cl) = (p.offset, current.offset, current.length);
        p.bytes[po..po + cl].copy_from_slice(&current.bytes[co..co + cl]);
    }
    Ok(())
}

/// check extreme values of longs check for correct ordering of the encoded
/// bytes and that values round-trip.
fn test_long_special_values() -> Result<()> {
    let values: [i64; 17] = [
        i64::MIN,
        i64::MIN + 1,
        i64::MIN + 2,
        -5003400000000,
        -4000,
        -3000,
        -2000,
        -1000,
        -1,
        0,
        1,
        10,
        300,
        50006789999999999,
        i64::MAX - 2,
        i64::MAX - 1,
        i64::MAX,
    ];
    let mut encoded: Vec<BytesRef> = Vec::with_capacity(values.len());

    for i in 0..values.len() {
        let mut e = BytesRef::new(vec![0u8; 8]);
        let off = e.offset;
        NumericUtils::long_to_sortable_bytes(values[i], &mut e.bytes, off);
        assert_equals_msg(
            Some("forward and back conversion should generate same long"),
            &values[i],
            &NumericUtils::sortable_bytes_to_long(&e.bytes, e.offset),
        );
        encoded.push(e);
    }

    // check sort order (encoded values should be ascending)
    for i in 1..encoded.len() {
        Assert::assert_true_msg(
            "check sort order",
            bytes_compare(&encoded[i - 1], &encoded[i]) == Ordering::Less,
        );
    }
    Ok(())
}

/// check extreme values of ints check for correct ordering of the encoded
/// bytes and that values round-trip.
fn test_int_special_values() -> Result<()> {
    let values: [i32; 17] = [
        i32::MIN,
        i32::MIN + 1,
        i32::MIN + 2,
        -64765767,
        -4000,
        -3000,
        -2000,
        -1000,
        -1,
        0,
        1,
        10,
        300,
        765878989,
        i32::MAX - 2,
        i32::MAX - 1,
        i32::MAX,
    ];
    let mut encoded: Vec<BytesRef> = Vec::with_capacity(values.len());

    for i in 0..values.len() {
        let mut e = BytesRef::new(vec![0u8; 4]);
        let off = e.offset;
        NumericUtils::int_to_sortable_bytes(values[i], &mut e.bytes, off);
        assert_equals_msg(
            Some("forward and back conversion should generate same int"),
            &values[i],
            &NumericUtils::sortable_bytes_to_int(&e.bytes, e.offset),
        );
        encoded.push(e);
    }

    // check sort order (encoded values should be ascending)
    for i in 1..encoded.len() {
        Assert::assert_true_msg(
            "check sort order",
            bytes_compare(&encoded[i - 1], &encoded[i]) == Ordering::Less,
        );
    }
    Ok(())
}

/// check extreme values of big integers (4 bytes) check for correct ordering
/// of the encoded bytes and that values round-trip.
fn test_big_int_special_values() -> Result<()> {
    let values: Vec<Vec<u8>> = [
        i32::MIN as i64,
        i32::MIN as i64 + 1,
        i32::MIN as i64 + 2,
        -64765767,
        -4000,
        -3000,
        -2000,
        -1000,
        -1,
        0,
        1,
        10,
        300,
        765878989,
        i32::MAX as i64 - 2,
        i32::MAX as i64 - 1,
        i32::MAX as i64,
    ]
    .iter()
    .map(|&v| big_value_of(v))
    .collect();
    let mut encoded: Vec<BytesRef> = Vec::with_capacity(values.len());

    for i in 0..values.len() {
        let mut e = BytesRef::new(vec![0u8; 4]);
        let off = e.offset;
        NumericUtils::big_int_to_sortable_bytes(&values[i], 4, &mut e.bytes, off);
        assert_equals_msg(
            Some("forward and back conversion should generate same big integer"),
            &values[i],
            &normalize(&NumericUtils::sortable_bytes_to_big_int(
                &e.bytes, e.offset, 4,
            )),
        );
        encoded.push(e);
    }

    // check sort order (encoded values should be ascending)
    for i in 1..encoded.len() {
        Assert::assert_true_msg(
            "check sort order",
            bytes_compare(&encoded[i - 1], &encoded[i]) == Ordering::Less,
        );
    }
    Ok(())
}

/// `Double.compare(a, b)`.
fn double_compare(a: f64, b: f64) -> i32 {
    let bits = |d: f64| -> i64 {
        if d.is_nan() {
            0x7ff8000000000000
        } else {
            d.to_bits() as i64
        }
    };
    if a < b {
        -1
    } else if a > b {
        1
    } else {
        signum(bits(a).cmp(&bits(b)))
    }
}

/// `Float.compare(a, b)`.
fn float_compare(a: f32, b: f32) -> i32 {
    let bits = |f: f32| -> i32 {
        if f.is_nan() {
            0x7fc00000
        } else {
            f.to_bits() as i32
        }
    };
    if a < b {
        -1
    } else if a > b {
        1
    } else {
        signum(bits(a).cmp(&bits(b)))
    }
}

/// check various sorted values of doubles (including extreme values) check for
/// correct ordering of the encoded bytes and that values round-trip.
fn test_doubles() -> Result<()> {
    let values: [f64; 15] = [
        f64::NEG_INFINITY,
        -2.3E25,
        -1.0E15,
        -1.0,
        -1.0E-1,
        -1.0E-2,
        -0.0,
        0.0,
        1.0E-2,
        1.0E-1,
        1.0,
        1.0E15,
        2.3E25,
        f64::INFINITY,
        f64::NAN,
    ];
    let mut encoded = [0i64; 15];

    // check forward and back conversion
    for i in 0..values.len() {
        encoded[i] = NumericUtils::double_to_sortable_long(values[i]);
        Assert::assert_true_msg(
            "forward and back conversion should generate same double",
            double_compare(values[i], NumericUtils::sortable_long_to_double(encoded[i])) == 0,
        );
    }

    // check sort order (encoded values should be ascending)
    for i in 1..encoded.len() {
        Assert::assert_true_msg("check sort order", encoded[i - 1] < encoded[i]);
    }
    Ok(())
}

/// `DOUBLE_NANs`.
pub fn double_nans() -> [f64; 5] {
    [
        f64::NAN,
        f64::from_bits(0x7ff0000000000001),
        f64::from_bits(0x7fffffffffffffff),
        f64::from_bits(0xfff0000000000001),
        f64::from_bits(0xffffffffffffffff),
    ]
}

fn test_sortable_double_nan() -> Result<()> {
    let plus_inf = NumericUtils::double_to_sortable_long(f64::INFINITY);
    for nan in double_nans() {
        Assert::assert_true(nan.is_nan());
        let sortable = NumericUtils::double_to_sortable_long(nan);
        Assert::assert_true_msg(
            &format!(
                "Double not sorted correctly: {nan}, long repr: {sortable}, positive inf.: {plus_inf}"
            ),
            sortable > plus_inf,
        );
    }
    Ok(())
}

/// check various sorted values of floats (including extreme values) check for
/// correct ordering of the encoded bytes and that values round-trip.
fn test_floats() -> Result<()> {
    let values: [f32; 15] = [
        f32::NEG_INFINITY,
        -2.3E25,
        -1.0E15,
        -1.0,
        -1.0E-1,
        -1.0E-2,
        -0.0,
        0.0,
        1.0E-2,
        1.0E-1,
        1.0,
        1.0E15,
        2.3E25,
        f32::INFINITY,
        f32::NAN,
    ];
    let mut encoded = [0i32; 15];

    // check forward and back conversion
    for i in 0..values.len() {
        encoded[i] = NumericUtils::float_to_sortable_int(values[i]);
        Assert::assert_true_msg(
            "forward and back conversion should generate same float",
            float_compare(values[i], NumericUtils::sortable_int_to_float(encoded[i])) == 0,
        );
    }

    // check sort order (encoded values should be ascending)
    for i in 1..encoded.len() {
        Assert::assert_true_msg("check sort order", encoded[i - 1] < encoded[i]);
    }
    Ok(())
}

/// `FLOAT_NANs`.
pub fn float_nans() -> [f32; 5] {
    [
        f32::NAN,
        f32::from_bits(0x7f800001),
        f32::from_bits(0x7fffffff),
        f32::from_bits(0xff800001),
        f32::from_bits(0xffffffff),
    ]
}

fn test_sortable_float_nan() -> Result<()> {
    let plus_inf = NumericUtils::float_to_sortable_int(f32::INFINITY);
    for nan in float_nans() {
        Assert::assert_true(nan.is_nan());
        let sortable = NumericUtils::float_to_sortable_int(nan);
        Assert::assert_true_msg(
            &format!(
                "Float not sorted correctly: {nan}, int repr: {sortable}, positive inf.: {plus_inf}"
            ),
            sortable > plus_inf,
        );
    }
    Ok(())
}

/// `byte[] vBytes`: `v.toByteArray()` right-aligned in `numBytes` bytes.
fn right_aligned(raw: &[u8], num_bytes: usize) -> Vec<u8> {
    Assert::assert_true(raw.len() <= num_bytes);
    let mut v = vec![0u8; num_bytes];
    v[num_bytes - raw.len()..].copy_from_slice(raw);
    v
}

fn test_add() -> Result<()> {
    let iters = LuceneTestCase::at_least(1000);
    let num_bytes = TestUtil::next_int(&*random(), 1, 100) as usize;
    for _ in 0..iters {
        let v1 = big_random(8 * num_bytes as i32 - 1, &*random());
        let v2 = big_random(8 * num_bytes as i32 - 1, &*random());

        let v1_bytes = right_aligned(&mag_to_byte_array(&v1), num_bytes);
        let v2_bytes = right_aligned(&mag_to_byte_array(&v2), num_bytes);

        let mut result = vec![0u8; num_bytes];
        NumericUtils::add(num_bytes, 0, &v1_bytes, &v2_bytes, &mut result)?;

        let sum = mag_add(&v1, &v2);
        Assert::assert_true_msg(
            &format!(
                "sum={} v1={} v2={} but result={}",
                mag_to_string(&sum),
                mag_to_string(&v1),
                mag_to_string(&v2),
                mag_to_string(&result)
            ),
            mag_to_byte_array(&sum) == mag_to_byte_array(&result),
        );
    }
    Ok(())
}

fn test_illegal_add() -> Result<()> {
    let bytes = [0xffu8; 4];
    let mut one = [0u8; 4];
    one[3] = 1;
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        NumericUtils::add(4, 0, &bytes, &one, &mut [0u8; 4])
    });
    assert_equals(&Some("a + b overflows bytesPerDim=4"), &expected.message());
    Ok(())
}

fn test_subtract() -> Result<()> {
    let iters = LuceneTestCase::at_least(1000);
    let num_bytes = TestUtil::next_int(&*random(), 1, 100) as usize;
    for _ in 0..iters {
        let mut v1 = big_random(8 * num_bytes as i32 - 1, &*random());
        let mut v2 = big_random(8 * num_bytes as i32 - 1, &*random());
        if big_compare(&mag_to_byte_array(&v1), &mag_to_byte_array(&v2)) == Ordering::Less {
            std::mem::swap(&mut v1, &mut v2);
        }

        let v1_bytes = right_aligned(&mag_to_byte_array(&v1), num_bytes);
        let v2_bytes = right_aligned(&mag_to_byte_array(&v2), num_bytes);

        let mut result = vec![0u8; num_bytes];
        NumericUtils::subtract(num_bytes, 0, &v1_bytes, &v2_bytes, &mut result)?;

        let diff = mag_sub(&v1, &v2);
        // `new BigInteger(result)`: signed; the difference is below
        // `2^(8 * numBytes - 1)`, so its top bit is clear.
        Assert::assert_true_msg(
            &format!(
                "diff={} vs result={} v1={} v2={}",
                mag_to_string(&diff),
                mag_to_string(&result),
                mag_to_string(&v1),
                mag_to_string(&v2)
            ),
            mag_to_byte_array(&diff) == normalize(&result),
        );
    }
    Ok(())
}

fn test_illegal_subtract() -> Result<()> {
    let mut v1 = [0u8; 4];
    v1[3] = 0xf0;
    let mut v2 = [0u8; 4];
    v2[3] = 0xf1;
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        NumericUtils::subtract(4, 0, &v1, &v2, &mut [0u8; 4])
    });
    assert_equals(&Some("a < b"), &expected.message());
    Ok(())
}

/// test round-trip encoding of random integers
fn test_ints_round_trip() -> Result<()> {
    let mut encoded = [0u8; 4];
    for _ in 0..10000 {
        let value = random().next_int();
        NumericUtils::int_to_sortable_bytes(value, &mut encoded, 0);
        assert_equals(&value, &NumericUtils::sortable_bytes_to_int(&encoded, 0));
    }
    Ok(())
}

/// test round-trip encoding of random longs
fn test_longs_round_trip() -> Result<()> {
    let mut encoded = [0u8; 8];
    for _ in 0..10000 {
        let value = TestUtil::next_long(&*random(), i64::MIN, i64::MAX);
        NumericUtils::long_to_sortable_bytes(value, &mut encoded, 0);
        assert_equals(&value, &NumericUtils::sortable_bytes_to_long(&encoded, 0));
    }
    Ok(())
}

/// `Float.floatToIntBits(f)`.
fn float_to_int_bits(f: f32) -> i32 {
    if f.is_nan() {
        0x7fc00000
    } else {
        f.to_bits() as i32
    }
}

/// `Double.doubleToLongBits(d)`.
fn double_to_long_bits(d: f64) -> i64 {
    if d.is_nan() {
        0x7ff8000000000000
    } else {
        d.to_bits() as i64
    }
}

/// test round-trip encoding of random floats
fn test_floats_round_trip() -> Result<()> {
    let mut encoded = [0u8; 4];
    for _ in 0..10000 {
        let value = f32::from_bits(random().next_int() as u32);
        NumericUtils::int_to_sortable_bytes(
            NumericUtils::float_to_sortable_int(value),
            &mut encoded,
            0,
        );
        let actual =
            NumericUtils::sortable_int_to_float(NumericUtils::sortable_bytes_to_int(&encoded, 0));
        assert_equals(&float_to_int_bits(value), &float_to_int_bits(actual));
    }
    Ok(())
}

/// test round-trip encoding of random doubles
fn test_doubles_round_trip() -> Result<()> {
    let mut encoded = [0u8; 8];
    for _ in 0..10000 {
        let value = f64::from_bits(TestUtil::next_long(&*random(), i64::MIN, i64::MAX) as u64);
        NumericUtils::long_to_sortable_bytes(
            NumericUtils::double_to_sortable_long(value),
            &mut encoded,
            0,
        );
        let actual = NumericUtils::sortable_long_to_double(NumericUtils::sortable_bytes_to_long(
            &encoded, 0,
        ));
        assert_equals(&double_to_long_bits(value), &double_to_long_bits(actual));
    }
    Ok(())
}

/// test round-trip encoding of random big integers
fn test_big_ints_round_trip() -> Result<()> {
    for _ in 0..10000 {
        let value = TestUtil::next_big_integer(&*random(), 16);
        let length = value.len() as i32;

        // make sure sign extension is tested: sometimes pad to more bytes when
        // encoding:
        let max_length = TestUtil::next_int(&*random(), length, length + 3) as usize;
        let mut encoded = vec![0u8; max_length];
        NumericUtils::big_int_to_sortable_bytes(&value, max_length, &mut encoded, 0);
        assert_equals(
            &value,
            &normalize(&NumericUtils::sortable_bytes_to_big_int(
                &encoded, 0, max_length,
            )),
        );
    }
    Ok(())
}

/// check sort order of random integers consistent with Integer.compare
fn test_ints_compare() -> Result<()> {
    let mut left = BytesRef::new(vec![0u8; 4]);
    let mut right = BytesRef::new(vec![0u8; 4]);

    for _ in 0..10000 {
        let left_value = random().next_int();
        let lo = left.offset;
        NumericUtils::int_to_sortable_bytes(left_value, &mut left.bytes, lo);

        let right_value = random().next_int();
        let ro = right.offset;
        NumericUtils::int_to_sortable_bytes(right_value, &mut right.bytes, ro);

        assert_equals(
            &signum(left_value.cmp(&right_value)),
            &signum(bytes_compare(&left, &right)),
        );
    }
    Ok(())
}

/// check sort order of random longs consistent with Long.compare
fn test_longs_compare() -> Result<()> {
    let mut left = BytesRef::new(vec![0u8; 8]);
    let mut right = BytesRef::new(vec![0u8; 8]);

    for _ in 0..10000 {
        let left_value = TestUtil::next_long(&*random(), i64::MIN, i64::MAX);
        let lo = left.offset;
        NumericUtils::long_to_sortable_bytes(left_value, &mut left.bytes, lo);

        let right_value = TestUtil::next_long(&*random(), i64::MIN, i64::MAX);
        let ro = right.offset;
        NumericUtils::long_to_sortable_bytes(right_value, &mut right.bytes, ro);

        assert_equals(
            &signum(left_value.cmp(&right_value)),
            &signum(bytes_compare(&left, &right)),
        );
    }
    Ok(())
}

/// check sort order of random floats consistent with Float.compare
fn test_floats_compare() -> Result<()> {
    let mut left = BytesRef::new(vec![0u8; 4]);
    let mut right = BytesRef::new(vec![0u8; 4]);

    for _ in 0..10000 {
        let left_value = f32::from_bits(random().next_int() as u32);
        let lo = left.offset;
        NumericUtils::int_to_sortable_bytes(
            NumericUtils::float_to_sortable_int(left_value),
            &mut left.bytes,
            lo,
        );

        let right_value = f32::from_bits(random().next_int() as u32);
        let ro = right.offset;
        NumericUtils::int_to_sortable_bytes(
            NumericUtils::float_to_sortable_int(right_value),
            &mut right.bytes,
            ro,
        );

        assert_equals(
            &float_compare(left_value, right_value).signum(),
            &signum(bytes_compare(&left, &right)),
        );
    }
    Ok(())
}

/// check sort order of random doubles consistent with Double.compare
fn test_doubles_compare() -> Result<()> {
    let mut left = BytesRef::new(vec![0u8; 8]);
    let mut right = BytesRef::new(vec![0u8; 8]);

    for _ in 0..10000 {
        let left_value = f64::from_bits(TestUtil::next_long(&*random(), i64::MIN, i64::MAX) as u64);
        let lo = left.offset;
        NumericUtils::long_to_sortable_bytes(
            NumericUtils::double_to_sortable_long(left_value),
            &mut left.bytes,
            lo,
        );

        let right_value =
            f64::from_bits(TestUtil::next_long(&*random(), i64::MIN, i64::MAX) as u64);
        let ro = right.offset;
        NumericUtils::long_to_sortable_bytes(
            NumericUtils::double_to_sortable_long(right_value),
            &mut right.bytes,
            ro,
        );

        assert_equals(
            &double_compare(left_value, right_value).signum(),
            &signum(bytes_compare(&left, &right)),
        );
    }
    Ok(())
}

/// check sort order of random bigintegers consistent with
/// BigInteger.compareTo
fn test_big_ints_compare() -> Result<()> {
    for _ in 0..10000 {
        let max_length = TestUtil::next_int(&*random(), 1, 16) as usize;

        let left_value = TestUtil::next_big_integer(&*random(), max_length as i32);
        let mut left = BytesRef::new(vec![0u8; max_length]);
        let lo = left.offset;
        NumericUtils::big_int_to_sortable_bytes(&left_value, max_length, &mut left.bytes, lo);

        let right_value = TestUtil::next_big_integer(&*random(), max_length as i32);
        let mut right = BytesRef::new(vec![0u8; max_length]);
        let ro = right.offset;
        NumericUtils::big_int_to_sortable_bytes(&right_value, max_length, &mut right.bytes, ro);

        assert_equals(
            &signum(big_compare(&left_value, &right_value)),
            &signum(bytes_compare(&left, &right)),
        );
    }
    Ok(())
}
