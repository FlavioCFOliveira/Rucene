//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermsEnumIndex.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::terms_enum_index::prefix8_to_comparable_unsigned_long;
use rucene::util::BytesRef;

use crate::core_support::{assert_equals, class, test};

/// `TestTermsEnumIndex`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTermsEnumIndex",
        vec![test(
            "testPrefix8ToComparableUnsignedLong",
            test_prefix8_to_comparable_unsigned_long,
        )],
    )
}

/// `TermsEnumIndex.prefix8ToComparableUnsignedLong(new BytesRef(b, offset, length))`,
/// as the Java `long` it returns.
fn prefix8(b: &[u8], offset: usize, length: usize) -> i64 {
    prefix8_to_comparable_unsigned_long(&BytesRef::new(b[offset..offset + length].to_vec())) as i64
}

/// Port of `TestTermsEnumIndex.testPrefix8ToComparableUnsignedLong`.
fn test_prefix8_to_comparable_unsigned_long() -> Result<()> {
    let b: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    assert_equals(&0i64, &prefix8(&b, 1, 0));
    assert_equals(&(4i64 << 56), &prefix8(&b, 3, 1));
    assert_equals(&((4i64 << 56) | (5i64 << 48)), &prefix8(&b, 3, 2));
    assert_equals(
        &((4i64 << 56) | (5i64 << 48) | (6i64 << 40)),
        &prefix8(&b, 3, 3),
    );
    assert_equals(
        &((4i64 << 56) | (5i64 << 48) | (6i64 << 40) | (7i64 << 32)),
        &prefix8(&b, 3, 4),
    );
    assert_equals(
        &((4i64 << 56) | (5i64 << 48) | (6i64 << 40) | (7i64 << 32) | (8i64 << 24)),
        &prefix8(&b, 3, 5),
    );
    assert_equals(
        &((4i64 << 56) | (5i64 << 48) | (6i64 << 40) | (7i64 << 32) | (8i64 << 24) | (9i64 << 16)),
        &prefix8(&b, 3, 6),
    );
    assert_equals(
        &((4i64 << 56)
            | (5i64 << 48)
            | (6i64 << 40)
            | (7i64 << 32)
            | (8i64 << 24)
            | (9i64 << 16)
            | (10i64 << 8)),
        &prefix8(&b, 3, 7),
    );
    assert_equals(
        &((4i64 << 56)
            | (5i64 << 48)
            | (6i64 << 40)
            | (7i64 << 32)
            | (8i64 << 24)
            | (9i64 << 16)
            | (10i64 << 8)
            | 11i64),
        &prefix8(&b, 3, 8),
    );
    assert_equals(
        &((4i64 << 56)
            | (5i64 << 48)
            | (6i64 << 40)
            | (7i64 << 32)
            | (8i64 << 24)
            | (9i64 << 16)
            | (10i64 << 8)
            | 11i64),
        &prefix8(&b, 3, 9),
    );
    Ok(())
}
