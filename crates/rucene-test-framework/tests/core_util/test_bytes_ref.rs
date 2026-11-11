//! Port of `lucene/core/src/test/org/apache/lucene/util/TestBytesRef.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::BytesRef;
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, class, expect_throws_panic, random, test, unchecked};

/// `BytesRef.EMPTY_BYTES`.
const EMPTY_BYTES: &[u8] = &[];

/// `TestBytesRef`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestBytesRef",
        vec![
            test("testEmpty", test_empty),
            test("testFromBytes", test_from_bytes),
            test("testFromChars", test_from_chars),
            test("testInvalidDeepCopy", test_invalid_deep_copy),
        ],
    )
}

fn test_empty() -> Result<()> {
    let b = BytesRef::default();
    assert_equals(EMPTY_BYTES, b.bytes.as_slice());
    assert_equals(&0, &b.offset);
    assert_equals(&0, &b.length);
    Ok(())
}

fn test_from_bytes() -> Result<()> {
    let bytes: Vec<u8> = vec![b'a', b'b', b'c', b'd'];
    let b = BytesRef::new(bytes.clone());
    assert_equals(&bytes, &b.bytes);
    assert_equals(&0, &b.offset);
    assert_equals(&4, &b.length);

    let b2 = BytesRef {
        bytes: bytes.clone(),
        offset: 1,
        length: 3,
    };
    assert_equals("bcd", unchecked(b2.utf8_to_string()).as_str());

    Assert::assert_false(b == b2);
    Ok(())
}

fn test_from_chars() -> Result<()> {
    for _ in 0..100 {
        let s = TestUtil::random_unicode_string(&*random());
        let s2 = unchecked(BytesRef::from_char_sequence(&s).utf8_to_string());
        assert_equals(&s, &s2);
    }

    // only for 4.x
    assert_equals(
        "\u{FFFF}",
        unchecked(BytesRef::from_char_sequence("\u{FFFF}").utf8_to_string()).as_str(),
    );
    Ok(())
}

fn test_invalid_deep_copy() -> Result<()> {
    let mut from = BytesRef::new(vec![1, 2]);
    from.offset += 1; // now invalid
    expect_throws_panic("java.lang.IndexOutOfBoundsException", || {
        BytesRef::deep_copy_of(&from);
    });
    Ok(())
}
