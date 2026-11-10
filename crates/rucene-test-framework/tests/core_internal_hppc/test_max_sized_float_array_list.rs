//! Port of `org.apache.lucene.internal.hppc.TestMaxSizedFloatArrayList`: tests
//! for `MaxSizedFloatArrayList`. The class extends `TestFloatArrayList`, whose
//! test methods it inherits; its own methods use no instance state.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::internal::hppc::{FloatArrayList, MaxSizedFloatArrayList};
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::support::{assert_equals_obj, assert_not_equals};
use super::test_float_array_list::{self, TestFloatArrayList};
use crate::lucene_core_shims::t;

const MAX_SIZE: i32 = 5;

/// `assertArrayEquals(float[] expecteds, float[] actuals, float delta)`.
fn assert_array_equals_delta(expecteds: &[f32], actuals: &[f32], delta: f32) {
    Assert::assert_equals(expecteds.len(), actuals.len());
    for (i, (e, a)) in expecteds.iter().zip(actuals).enumerate() {
        Assert::assert_equals_float_msg(
            Some(&format!("arrays first differed at element [{i}];")),
            *e,
            *a,
            delta,
        );
    }
}

/// `IllegalStateException`.
const ISE: &str = "java.lang.IllegalStateException";

fn test_max_size_constructor() -> rucene::error::Result<()> {
    // Test valid maxSize
    let list = MaxSizedFloatArrayList::with_expected_elements(MAX_SIZE, 3);
    Assert::assert_equals(3, list.buffer.len());
    Assert::assert_equals(MAX_SIZE, list.max_size());

    // Test invalid maxSize (expectedElements > maxSize)
    expect_throws("java.lang.AssertionError", || {
        MaxSizedFloatArrayList::with_expected_elements(2, 3);
        Ok(())
    });
    Ok(())
}

fn test_max_size_limit() -> rucene::error::Result<()> {
    let mut list = MaxSizedFloatArrayList::new(MAX_SIZE);

    // Fill up to maxSize
    for i in 0..MAX_SIZE {
        list.add(i as f32 + 0.5);
    }
    Assert::assert_equals(MAX_SIZE, list.size());

    // Try to add beyond maxSize
    expect_throws(ISE, || {
        list.add(MAX_SIZE as f32 + 0.5);
        Ok(())
    });

    // Try to add array beyond maxSize
    expect_throws(ISE, || {
        list.add_array(&[1.5, 2.5, 3.5]);
        Ok(())
    });

    // Try to addAll beyond maxSize
    let mut other_list = FloatArrayList::new();
    other_list.add(1.5);
    expect_throws(ISE, || {
        list.add_all(&other_list);
        Ok(())
    });
    Ok(())
}

fn test_max_size_with_insert() -> rucene::error::Result<()> {
    let mut list = MaxSizedFloatArrayList::new(MAX_SIZE);

    // Fill up to maxSize - 1
    for i in 0..MAX_SIZE - 1 {
        list.add(i as f32 + 0.5);
    }
    Assert::assert_equals(MAX_SIZE - 1, list.size());

    // Insert at the end (should succeed)
    list.insert(MAX_SIZE - 1, (MAX_SIZE - 1) as f32 + 0.5);
    Assert::assert_equals(MAX_SIZE, list.size());

    // Try to insert beyond maxSize
    expect_throws(ISE, || {
        list.insert(0, MAX_SIZE as f32 + 0.5);
        Ok(())
    });
    Ok(())
}

fn test_max_size_with_resize() -> rucene::error::Result<()> {
    let mut list = MaxSizedFloatArrayList::new(MAX_SIZE);

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
    let mut list = MaxSizedFloatArrayList::new(MAX_SIZE);

    // Try to ensure capacity beyond maxSize
    expect_throws(ISE, || {
        list.ensure_capacity(MAX_SIZE + 1);
        Ok(())
    });

    // Ensure capacity to maxSize (should succeed)
    list.ensure_capacity(MAX_SIZE);
    // Buffer can be larger than maxSize due to growth strategy, but not smaller if capacity is
    // ensured to maxSize
    Assert::assert_true(
        list.buffer.len() >= MAX_SIZE as usize || list.buffer.is_empty() && MAX_SIZE == 0,
    );
    Ok(())
}

fn test_max_size_with_copy_constructor() -> rucene::error::Result<()> {
    let mut original = MaxSizedFloatArrayList::new(MAX_SIZE);
    original.add_array(&[1.5, 2.5, 3.5]);

    let copy = MaxSizedFloatArrayList::from_list(&original);
    Assert::assert_equals(original.max_size(), copy.max_size());
    Assert::assert_equals(original.size(), copy.size());
    assert_array_equals_delta(&original.to_array(), &copy.to_array(), 0.001);
    Ok(())
}

fn test_hash_code_with_max_size() -> rucene::error::Result<()> {
    let mut list1 = MaxSizedFloatArrayList::new(5);
    let mut list2 = MaxSizedFloatArrayList::new(10);

    // Same elements but different maxSize should have different hash codes
    list1.add_array(&[1.5, 2.5, 3.5]);
    list2.add_array(&[1.5, 2.5, 3.5]);
    assert_not_equals(&list1.hash_code(), &list2.hash_code());

    // Same maxSize and same elements should have same hash code
    let mut list3 = MaxSizedFloatArrayList::new(5);
    list3.add_array(&[1.5, 2.5, 3.5]);
    Assert::assert_equals(list1.hash_code(), list3.hash_code());
    Ok(())
}

fn test_equals_with_max_size() -> rucene::error::Result<()> {
    let mut list1 = MaxSizedFloatArrayList::new(5);
    let mut list2 = MaxSizedFloatArrayList::new(10);
    let mut list3 = MaxSizedFloatArrayList::new(5);

    // Same elements but different maxSize should not be equal
    list1.add_array(&[1.5, 2.5, 3.5]);
    list2.add_array(&[1.5, 2.5, 3.5]);
    assert_not_equals(&list1, &list2);

    // Same maxSize and same elements should be equal
    list3.add_array(&[1.5, 2.5, 3.5]);
    assert_equals_obj(&list1, &list3);

    // Different elements but same maxSize should not be equal
    list3.clear();
    list3.add_array(&[1.5, 2.5, 4.5]);
    assert_not_equals(&list1, &list3);
    Ok(())
}

/// The class `org.apache.lucene.internal.hppc.TestMaxSizedFloatArrayList`,
/// which extends `TestFloatArrayList`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.internal.hppc.TestMaxSizedFloatArrayList")
        .extends(&test_float_array_list::class())
        .constructor(Constructor::no_args(TestFloatArrayList::new))
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
