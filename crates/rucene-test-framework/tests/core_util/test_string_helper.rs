//! Port of `lucene/core/src/test/org/apache/lucene/util/TestStringHelper.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::StringHelper;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test};

/// `TestStringHelper`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestStringHelper",
        vec![
            test("testBytesDifference", test_bytes_difference),
            test("testStartsWith", test_starts_with),
            test("testEndsWith", test_ends_with),
            test("testStartsWithWhole", test_starts_with_whole),
            test("testEndsWithWhole", test_ends_with_whole),
            test("testMurmurHash3", test_murmur_hash3),
            test("testSortKeyLength", test_sort_key_length),
        ],
    )
}

use LuceneTestCase as Ltc;

fn nb(s: &str) -> rucene::util::BytesRef {
    Ltc::new_bytes_ref_str(s)
}

fn test_bytes_difference() -> Result<()> {
    let left = nb("foobar");
    let right = nb("foozo");
    assert_equals(&3, &StringHelper::bytes_difference(&left, &right)?);
    assert_equals(&2, &StringHelper::bytes_difference(&nb("foo"), &nb("for"))?);
    assert_equals(
        &2,
        &StringHelper::bytes_difference(&nb("foo1234"), &nb("for1234"))?,
    );
    assert_equals(&1, &StringHelper::bytes_difference(&nb("foo"), &nb("fz"))?);
    assert_equals(&0, &StringHelper::bytes_difference(&nb("foo"), &nb("g"))?);
    assert_equals(
        &3,
        &StringHelper::bytes_difference(&nb("foo"), &nb("food"))?,
    );
    // we can detect terms are out of order if we see a duplicate
    expect_throws("java.lang.IllegalArgumentException", || {
        StringHelper::bytes_difference(&nb("ab"), &nb("ab"))
    });
    Ok(())
}

fn test_starts_with() -> Result<()> {
    let r = nb("foobar");
    let slice = nb("foo");
    Assert::assert_true(StringHelper::starts_with(&r, &slice));
    Ok(())
}

fn test_ends_with() -> Result<()> {
    let r = nb("foobar");
    let slice = nb("bar");
    Assert::assert_true(StringHelper::ends_with(&r, &slice));
    Ok(())
}

fn test_starts_with_whole() -> Result<()> {
    let r = nb("foobar");
    let slice = nb("foobar");
    Assert::assert_true(StringHelper::starts_with(&r, &slice));
    Ok(())
}

fn test_ends_with_whole() -> Result<()> {
    let r = nb("foobar");
    let slice = nb("foobar");
    Assert::assert_true(StringHelper::ends_with(&r, &slice));
    Ok(())
}

fn test_murmur_hash3() -> Result<()> {
    // Hashes computed using murmur3_32 from https://code.google.com/p/pyfasthash
    assert_equals(
        &(0xf6a5c420u32 as i32),
        &StringHelper::murmurhash3_x86_32_bytes_ref(&nb("foo"), 0),
    );
    assert_equals(
        &(0xcd018ef6u32 as i32),
        &StringHelper::murmurhash3_x86_32_bytes_ref(&nb("foo"), 16),
    );
    assert_equals(
        &0x111e7435,
        &StringHelper::murmurhash3_x86_32_bytes_ref(
            &nb("You want weapons? We're in a library! Books! The best weapons in the world!"),
            0,
        ),
    );
    assert_equals(
        &0x2c628cd0,
        &StringHelper::murmurhash3_x86_32_bytes_ref(
            &nb("You want weapons? We're in a library! Books! The best weapons in the world!"),
            3476,
        ),
    );
    Ok(())
}

fn test_sort_key_length() -> Result<()> {
    assert_equals(&3, &StringHelper::sort_key_length(&nb("foo"), &nb("for"))?);
    assert_equals(
        &3,
        &StringHelper::sort_key_length(&nb("foo1234"), &nb("for1234"))?,
    );
    assert_equals(&2, &StringHelper::sort_key_length(&nb("foo"), &nb("fz"))?);
    assert_equals(&1, &StringHelper::sort_key_length(&nb("foo"), &nb("g"))?);
    assert_equals(&4, &StringHelper::sort_key_length(&nb("foo"), &nb("food"))?);
    // we can detect terms are out of order if we see a duplicate
    expect_throws("java.lang.IllegalArgumentException", || {
        StringHelper::sort_key_length(&nb("ab"), &nb("ab"))
    });
    Ok(())
}
