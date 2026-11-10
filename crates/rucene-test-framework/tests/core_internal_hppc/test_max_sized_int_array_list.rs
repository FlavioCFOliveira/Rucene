//! Port of `org.apache.lucene.internal.hppc.TestMaxSizedIntArrayList`: tests
//! for `MaxSizedIntArrayList`. The class extends `TestIntArrayList`, whose
//! test methods it inherits; its own methods use no instance state.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::internal::hppc::{IntArrayList, MaxSizedIntArrayList};
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::support::{assert_array_equals, assert_equals_obj, assert_not_equals};
use super::test_int_array_list::{self, TestIntArrayList};
use crate::lucene_core_shims::t;

const MAX_SIZE: i32 = 5;

/// `IllegalStateException`.
const ISE: &str = "java.lang.IllegalStateException";

fn test_max_size_constructor() -> rucene::error::Result<()> {
    // Test valid maxSize
    let list = MaxSizedIntArrayList::with_expected_elements(MAX_SIZE, 3);
    Assert::assert_equals(3, list.buffer.len());
    Assert::assert_equals(MAX_SIZE, list.max_size());

    // Test invalid maxSize (expectedElements > maxSize)
    expect_throws("java.lang.AssertionError", || {
        MaxSizedIntArrayList::with_expected_elements(2, 3);
        Ok(())
    });
    Ok(())
}

fn test_max_size_limit() -> rucene::error::Result<()> {
    let mut list = MaxSizedIntArrayList::new(MAX_SIZE);

    // Fill up to maxSize
    for i in 0..MAX_SIZE {
        list.add(i);
    }
    Assert::assert_equals(MAX_SIZE, list.size());

    // Try to add beyond maxSize
    expect_throws(ISE, || {
        list.add(MAX_SIZE);
        Ok(())
    });

    // Try to add array beyond maxSize
    expect_throws(ISE, || {
        list.add_array(&[1, 2, 3]);
        Ok(())
    });

    // Try to addAll beyond maxSize
    let mut other_list = IntArrayList::new();
    other_list.add(1);
    expect_throws(ISE, || {
        list.add_all(&other_list);
        Ok(())
    });
    Ok(())
}

fn test_max_size_with_insert() -> rucene::error::Result<()> {
    let mut list = MaxSizedIntArrayList::new(MAX_SIZE);

    // Fill up to maxSize - 1
    for i in 0..MAX_SIZE - 1 {
        list.add(i);
    }
    Assert::assert_equals(MAX_SIZE - 1, list.size());

    // Insert at the end (should succeed)
    list.insert(MAX_SIZE - 1, MAX_SIZE - 1);
    Assert::assert_equals(MAX_SIZE, list.size());

    // Try to insert beyond maxSize
    expect_throws(ISE, || {
        list.insert(0, MAX_SIZE);
        Ok(())
    });
    Ok(())
}

fn test_max_size_with_resize() -> rucene::error::Result<()> {
    let mut list = MaxSizedIntArrayList::new(MAX_SIZE);

    // Try to resize beyond maxSize
    expect_throws(ISE, || {
        list.resize(MAX_SIZE + 1);
        Ok(())
    });

    // Resize to maxSize (should succeed)
    list.resize(MAX_SIZE);
    Assert::assert_equals(MAX_SIZE, list.size());
    Ok(())
}

fn test_max_size_with_ensure_capacity() -> rucene::error::Result<()> {
    let mut list = MaxSizedIntArrayList::new(MAX_SIZE);

    // Try to ensure capacity beyond maxSize
    expect_throws(ISE, || {
        list.ensure_capacity(MAX_SIZE + 1);
        Ok(())
    });

    // Ensure capacity to maxSize (should succeed)
    list.ensure_capacity(MAX_SIZE);
    Assert::assert_equals(MAX_SIZE as usize, list.buffer.len());
    Ok(())
}

fn test_max_size_with_copy_constructor() -> rucene::error::Result<()> {
    let mut original = MaxSizedIntArrayList::new(MAX_SIZE);
    original.add_array(&[1, 2, 3]);

    let copy = MaxSizedIntArrayList::from_list(&original);
    Assert::assert_equals(original.max_size(), copy.max_size());
    Assert::assert_equals(original.size(), copy.size());
    assert_array_equals(&original.to_array(), &copy.to_array());
    Ok(())
}

fn test_hash_code_with_max_size() -> rucene::error::Result<()> {
    let mut list1 = MaxSizedIntArrayList::new(5);
    let mut list2 = MaxSizedIntArrayList::new(10);

    // Same elements but different maxSize should have different hash codes
    list1.add_array(&[1, 2, 3]);
    list2.add_array(&[1, 2, 3]);
    assert_not_equals(&list1.hash_code(), &list2.hash_code());

    // Same maxSize and same elements should have same hash code
    let mut list3 = MaxSizedIntArrayList::new(5);
    list3.add_array(&[1, 2, 3]);
    Assert::assert_equals(list1.hash_code(), list3.hash_code());
    Ok(())
}

fn test_equals_with_max_size() -> rucene::error::Result<()> {
    let mut list1 = MaxSizedIntArrayList::new(5);
    let mut list2 = MaxSizedIntArrayList::new(10);
    let mut list3 = MaxSizedIntArrayList::new(5);

    // Same elements but different maxSize should not be equal
    list1.add_array(&[1, 2, 3]);
    list2.add_array(&[1, 2, 3]);
    assert_not_equals(&list1, &list2);

    // Same maxSize and same elements should be equal
    list3.add_array(&[1, 2, 3]);
    assert_equals_obj(&list1, &list3);

    // Different elements but same maxSize should not be equal
    list3.clear();
    list3.add_array(&[1, 2, 4]);
    assert_not_equals(&list1, &list3);
    Ok(())
}

/// The class `org.apache.lucene.internal.hppc.TestMaxSizedIntArrayList`,
/// which extends `TestIntArrayList`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.internal.hppc.TestMaxSizedIntArrayList")
        .extends(&test_int_array_list::class())
        .constructor(Constructor::no_args(TestIntArrayList::new))
        .method(t("testMaxSizeConstructor", test_max_size_constructor))
        .method(t("testMaxSizeLimit", test_max_size_limit))
        .method(t("testMaxSizeWithInsert", test_max_size_with_insert))
        .method(t("testMaxSizeWithResize", test_max_size_with_resize))
        .method(t(
            "testMaxSizeWithEnsureCapacity",
            test_max_size_with_ensure_capacity,
        ))
        .method(t(
            "testMaxSizeWithCopyConstructor",
            test_max_size_with_copy_constructor,
        ))
        .method(t("testHashCodeWithMaxSize", test_hash_code_with_max_size))
        .method(t("testEqualsWithMaxSize", test_equals_with_max_size))
        .build()
}
