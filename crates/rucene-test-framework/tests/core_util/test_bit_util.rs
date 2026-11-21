//! Port of `lucene/core/src/test/org/apache/lucene/util/TestBitUtil.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::BitUtil;

use crate::core_support::{class, test};

/// `TestBitUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestBitUtil",
        vec![test("testIsZeroOrPowerOfTwo", test_is_zero_or_power_of_two)],
    )
}

fn test_is_zero_or_power_of_two() -> Result<()> {
    Assert::assert_true(BitUtil::is_zero_or_power_of_two(0));
    for shift in 0..=31 {
        Assert::assert_true(BitUtil::is_zero_or_power_of_two(1i32.wrapping_shl(shift)));
    }
    Assert::assert_false(BitUtil::is_zero_or_power_of_two(3));
    Assert::assert_false(BitUtil::is_zero_or_power_of_two(5));
    Assert::assert_false(BitUtil::is_zero_or_power_of_two(6));
    Assert::assert_false(BitUtil::is_zero_or_power_of_two(7));
    Assert::assert_false(BitUtil::is_zero_or_power_of_two(9));
    Assert::assert_false(BitUtil::is_zero_or_power_of_two(i32::MAX));
    Assert::assert_false(BitUtil::is_zero_or_power_of_two(i32::MAX.wrapping_add(2)));
    Assert::assert_false(BitUtil::is_zero_or_power_of_two(-1));
    Ok(())
}
