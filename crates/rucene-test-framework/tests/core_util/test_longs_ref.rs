//! Port of `lucene/core/src/test/org/apache/lucene/util/TestLongsRef.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::{LongsRef, EMPTY_LONGS};

use crate::core_support::{assert_equals, class, expect_throws_panic, test};

/// `TestLongsRef`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestLongsRef",
        vec![
            test("testEmpty", test_empty),
            test("testFromLongs", test_from_longs),
            test("testInvalidDeepCopy", test_invalid_deep_copy),
        ],
    )
}

fn test_empty() -> Result<()> {
    let i = LongsRef::new();
    assert_equals(EMPTY_LONGS, i.longs.as_slice());
    assert_equals(&0, &i.offset);
    assert_equals(&0, &i.length);
    Ok(())
}

fn test_from_longs() -> Result<()> {
    let longs: Vec<i64> = vec![1, 2, 3, 4];
    let i = LongsRef::from_longs(longs.clone(), 0, 4);
    assert_equals(&longs, &i.longs);
    assert_equals(&0, &i.offset);
    assert_equals(&4, &i.length);

    let i2 = LongsRef::from_longs(longs.clone(), 1, 3);
    assert_equals(&LongsRef::from_longs(vec![2, 3, 4], 0, 3), &i2);

    Assert::assert_false(i == i2);
    Ok(())
}

fn test_invalid_deep_copy() -> Result<()> {
    let mut from = LongsRef::from_longs(vec![1, 2], 0, 2);
    from.offset += 1; // now invalid
    expect_throws_panic("java.lang.IndexOutOfBoundsException", || {
        LongsRef::deep_copy_of(&from);
    });
    Ok(())
}
