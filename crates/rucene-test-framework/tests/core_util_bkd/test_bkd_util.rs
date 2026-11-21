//! Port of `lucene/core/src/test/org/apache/lucene/util/bkd/TestBKDUtil.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::bkd::BKDUtil;
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};

/// `TestBKDUtil`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testEquals4", test_equals4),
        test("testEquals8", test_equals8),
        test("testCommonPrefixLength4", test_common_prefix_length4),
        test("testCommonPrefixLength8", test_common_prefix_length8),
        test("testCommonPrefixLengthN", test_common_prefix_length_n),
    ];
    class("org.apache.lucene.util.bkd.TestBKDUtil", methods)
}

/// `(byte) random().nextInt(1 << 8)`.
fn random_byte() -> u8 {
    random().next_int_bound(1 << 8) as u8
}

fn test_equals4() -> Result<()> {
    let a_offset = TestUtil::next_int(&*random(), 0, 3) as usize;
    let mut a = vec![0u8; 4 + a_offset];
    let b_offset = TestUtil::next_int(&*random(), 0, 3) as usize;
    let mut b = vec![0u8; 4 + b_offset];

    for i in 0..4 {
        a[a_offset + i] = random_byte();
    }
    b[b_offset..b_offset + 4].copy_from_slice(&a[a_offset..a_offset + 4]);

    Assert::assert_true(BKDUtil::equals4(&a, a_offset, &b, b_offset));

    for i in 0..4 {
        loop {
            b[b_offset + i] = random_byte();
            if b[b_offset + i] != a[a_offset + i] {
                break;
            }
        }

        Assert::assert_false(BKDUtil::equals4(&a, a_offset, &b, b_offset));

        b[b_offset + i] = a[a_offset + i];
    }
    Ok(())
}

fn test_equals8() -> Result<()> {
    let a_offset = TestUtil::next_int(&*random(), 0, 7) as usize;
    let mut a = vec![0u8; 8 + a_offset];
    let b_offset = TestUtil::next_int(&*random(), 0, 7) as usize;
    let mut b = vec![0u8; 8 + b_offset];

    for i in 0..8 {
        a[a_offset + i] = random_byte();
    }
    b[b_offset..b_offset + 8].copy_from_slice(&a[a_offset..a_offset + 8]);

    Assert::assert_true(BKDUtil::equals8(&a, a_offset, &b, b_offset));

    for i in 0..8 {
        loop {
            b[b_offset + i] = random_byte();
            if b[b_offset + i] != a[a_offset + i] {
                break;
            }
        }

        Assert::assert_false(BKDUtil::equals8(&a, a_offset, &b, b_offset));

        b[b_offset + i] = a[a_offset + i];
    }
    Ok(())
}

fn test_common_prefix_length4() -> Result<()> {
    let a_offset = TestUtil::next_int(&*random(), 0, 3) as usize;
    let mut a = vec![0u8; 4 + a_offset];
    let b_offset = TestUtil::next_int(&*random(), 0, 3) as usize;
    let mut b = vec![0u8; 4 + b_offset];

    for i in 0..4 {
        a[a_offset + i] = random_byte();
        loop {
            b[b_offset + i] = random_byte();
            if b[b_offset + i] != a[a_offset + i] {
                break;
            }
        }
    }

    for i in 0..4 {
        assert_equals(
            &i,
            &BKDUtil::common_prefix_length4(&a, a_offset, &b, b_offset),
        );
        b[b_offset + i] = a[a_offset + i];
    }

    assert_equals(
        &4,
        &BKDUtil::common_prefix_length4(&a, a_offset, &b, b_offset),
    );
    Ok(())
}

fn test_common_prefix_length8() -> Result<()> {
    let a_offset = TestUtil::next_int(&*random(), 0, 7) as usize;
    let mut a = vec![0u8; 8 + a_offset];
    let b_offset = TestUtil::next_int(&*random(), 0, 7) as usize;
    let mut b = vec![0u8; 8 + b_offset];

    for i in 0..8 {
        a[a_offset + i] = random_byte();
        loop {
            b[b_offset + i] = random_byte();
            if b[b_offset + i] != a[a_offset + i] {
                break;
            }
        }
    }

    for i in 0..8 {
        assert_equals(
            &i,
            &BKDUtil::common_prefix_length8(&a, a_offset, &b, b_offset),
        );
        b[b_offset + i] = a[a_offset + i];
    }

    assert_equals(
        &8,
        &BKDUtil::common_prefix_length8(&a, a_offset, &b, b_offset),
    );
    Ok(())
}

fn test_common_prefix_length_n() -> Result<()> {
    let num_bytes = TestUtil::next_int(&*random(), 2, 16) as usize;
    let a_offset = TestUtil::next_int(&*random(), 0, num_bytes as i32 - 1) as usize;
    let mut a = vec![0u8; num_bytes + a_offset];
    let b_offset = TestUtil::next_int(&*random(), 0, num_bytes as i32 - 1) as usize;
    let mut b = vec![0u8; num_bytes + b_offset];

    for i in 0..num_bytes {
        a[a_offset + i] = random_byte();
        loop {
            b[b_offset + i] = random_byte();
            if b[b_offset + i] != a[a_offset + i] {
                break;
            }
        }
    }

    for i in 0..num_bytes {
        assert_equals(
            &i,
            &BKDUtil::common_prefix_length(&a, a_offset, &b, b_offset, num_bytes),
        );
        b[b_offset + i] = a[a_offset + i];
    }

    assert_equals(
        &num_bytes,
        &BKDUtil::common_prefix_length(&a, a_offset, &b, b_offset, num_bytes),
    );
    Ok(())
}
