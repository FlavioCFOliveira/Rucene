//! Port of `lucene/core/src/test/org/apache/lucene/util/TestIntsRef.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::IntsRef;

use crate::core_support::{assert_equals, class, expect_throws_panic, test};

/// `IntsRef.EMPTY_INTS`.
const EMPTY_INTS: &[i32] = &[];

/// `TestIntsRef`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestIntsRef",
        vec![
            test("testEmpty", test_empty),
            test("testFromInts", test_from_ints),
            test("testInvalidDeepCopy", test_invalid_deep_copy),
        ],
    )
}

fn test_empty() -> Result<()> {
    let i = IntsRef::default();
    assert_equals(EMPTY_INTS, i.ints.as_slice());
    assert_equals(&0, &i.offset);
    assert_equals(&0, &i.length);
    Ok(())
}

fn test_from_ints() -> Result<()> {
    let ints: Vec<i32> = vec![1, 2, 3, 4];
    let i = IntsRef::with_offset_length(ints.clone(), 0, 4);
    assert_equals(&ints, &i.ints);
    assert_equals(&0, &i.offset);
    assert_equals(&4, &i.length);

    let i2 = IntsRef::with_offset_length(ints.clone(), 1, 3);
    assert_equals(&IntsRef::with_offset_length(vec![2, 3, 4], 0, 3), &i2);

    Assert::assert_false(i == i2);
    Ok(())
}

fn test_invalid_deep_copy() -> Result<()> {
    let mut from = IntsRef::with_offset_length(vec![1, 2], 0, 2);
    from.offset += 1; // now invalid
    expect_throws_panic("java.lang.IndexOutOfBoundsException", || {
        IntsRef::deep_copy_of(&from);
    });
    Ok(())
}
