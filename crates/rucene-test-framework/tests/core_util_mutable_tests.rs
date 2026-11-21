//! Port of `lucene/core/src/test/org/apache/lucene/util/mutable/TestMutableValues.java`
//! (sprint 9, task #401): simple test of the basic contract of the various
//! `MutableValue` implementations, run as a suite under `LuceneTestCase`'s
//! rule chain.

// The ports keep Java's indexed loops, counters and unit `NO_OUTPUT`
// bindings line by line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::mutable::{
    MutableValue, MutableValueBool, MutableValueDouble, MutableValueFloat, MutableValueInt,
    MutableValueLong, MutableValueStr,
};
use rucene::util::BytesRef;

use core_support::{assert_equals, class, run, test};

/// `TestMutableValues`.
fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testStr", test_str),
        test("testDouble", test_double),
        test("testInt", test_int),
        test("testFloat", test_float),
        test("testLong", test_long),
        test("testBool", test_bool),
    ];
    class("org.apache.lucene.util.mutable.TestMutableValues", methods)
}

#[test]
fn test_mutable_values() {
    run(class_def);
}

const DEFAULTS: &str = "defaults have changed, test utility may not longer be as high";

fn test_str() -> Result<()> {
    let mut xxx = MutableValueStr::new();
    assert!(xxx.value.get() == BytesRef::default(), "{DEFAULTS}");
    assert!(xxx.exists, "{DEFAULTS}");
    assert_sanity(&xxx);
    let mut yyy = MutableValueStr::new();
    assert_sanity(&yyy);

    assert_equality(&xxx, &yyy);

    xxx.exists = false;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.exists = false;
    assert_equality(&xxx, &yyy);

    xxx.value.clear();
    xxx.value.copy_chars("zzz");
    xxx.exists = true;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.value.clear();
    yyy.value.copy_chars("aaa");
    yyy.exists = true;
    assert_sanity(&yyy);

    assert_in_equality(&xxx, &yyy);
    Assert::assert_true(0 < xxx.compare_to(&yyy));
    Assert::assert_true(yyy.compare_to(&xxx) < 0);

    xxx.copy(&yyy);
    assert_sanity(&xxx);
    assert_equality(&xxx, &yyy);

    // special BytesRef considerations...

    xxx.exists = false;
    xxx.value.clear(); // but leave bytes alone
    assert_in_equality(&xxx, &yyy);

    yyy.exists = false;
    yyy.value.clear(); // but leave bytes alone
    assert_equality(&xxx, &yyy);
    Ok(())
}

fn test_double() -> Result<()> {
    let mut xxx = MutableValueDouble::new();
    assert!(xxx.value == 0.0f64, "{DEFAULTS}");
    assert!(xxx.exists, "{DEFAULTS}");
    assert_sanity(&xxx);
    let mut yyy = MutableValueDouble::new();
    assert_sanity(&yyy);

    assert_equality(&xxx, &yyy);

    xxx.exists = false;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.exists = false;
    assert_equality(&xxx, &yyy);

    xxx.value = 42.0f64;
    xxx.exists = true;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.value = -99.0f64;
    yyy.exists = true;
    assert_sanity(&yyy);

    assert_in_equality(&xxx, &yyy);
    Assert::assert_true(0 < xxx.compare_to(&yyy));
    Assert::assert_true(yyy.compare_to(&xxx) < 0);

    xxx.copy(&yyy);
    assert_sanity(&xxx);
    assert_equality(&xxx, &yyy);
    Ok(())
}

fn test_int() -> Result<()> {
    let mut xxx = MutableValueInt::new();
    assert!(xxx.value == 0, "{DEFAULTS}");
    assert!(xxx.exists, "{DEFAULTS}");
    assert_sanity(&xxx);
    let mut yyy = MutableValueInt::new();
    assert_sanity(&yyy);

    assert_equality(&xxx, &yyy);

    xxx.exists = false;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.exists = false;
    assert_equality(&xxx, &yyy);

    xxx.value = 42;
    xxx.exists = true;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.value = -99;
    yyy.exists = true;
    assert_sanity(&yyy);

    assert_in_equality(&xxx, &yyy);
    Assert::assert_true(0 < xxx.compare_to(&yyy));
    Assert::assert_true(yyy.compare_to(&xxx) < 0);

    xxx.copy(&yyy);
    assert_sanity(&xxx);
    assert_equality(&xxx, &yyy);
    Ok(())
}

fn test_float() -> Result<()> {
    let mut xxx = MutableValueFloat::new();
    assert!(xxx.value == 0.0f32, "{DEFAULTS}");
    assert!(xxx.exists, "{DEFAULTS}");
    assert_sanity(&xxx);
    let mut yyy = MutableValueFloat::new();
    assert_sanity(&yyy);

    assert_equality(&xxx, &yyy);

    xxx.exists = false;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.exists = false;
    assert_equality(&xxx, &yyy);

    xxx.value = 42.0f32;
    xxx.exists = true;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.value = -99.0f32;
    yyy.exists = true;
    assert_sanity(&yyy);

    assert_in_equality(&xxx, &yyy);
    Assert::assert_true(0 < xxx.compare_to(&yyy));
    Assert::assert_true(yyy.compare_to(&xxx) < 0);

    xxx.copy(&yyy);
    assert_sanity(&xxx);
    assert_equality(&xxx, &yyy);
    Ok(())
}

fn test_long() -> Result<()> {
    let mut xxx = MutableValueLong::new();
    assert!(xxx.value == 0i64, "{DEFAULTS}");
    assert!(xxx.exists, "{DEFAULTS}");
    assert_sanity(&xxx);
    let mut yyy = MutableValueLong::new();
    assert_sanity(&yyy);

    assert_equality(&xxx, &yyy);

    xxx.exists = false;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.exists = false;
    assert_equality(&xxx, &yyy);

    xxx.value = 42i64;
    xxx.exists = true;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.value = -99i64;
    yyy.exists = true;
    assert_sanity(&yyy);

    assert_in_equality(&xxx, &yyy);
    Assert::assert_true(0 < xxx.compare_to(&yyy));
    Assert::assert_true(yyy.compare_to(&xxx) < 0);

    xxx.copy(&yyy);
    assert_sanity(&xxx);
    assert_equality(&xxx, &yyy);
    Ok(())
}

fn test_bool() -> Result<()> {
    let mut xxx = MutableValueBool::new();
    assert!(!xxx.value, "{DEFAULTS}");
    assert!(xxx.exists, "{DEFAULTS}");
    assert_sanity(&xxx);
    let mut yyy = MutableValueBool::new();
    assert_sanity(&yyy);

    assert_equality(&xxx, &yyy);

    xxx.exists = false;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.exists = false;
    assert_equality(&xxx, &yyy);

    xxx.value = true;
    xxx.exists = true;
    assert_sanity(&xxx);

    assert_in_equality(&xxx, &yyy);

    yyy.value = false;
    yyy.exists = true;
    assert_sanity(&yyy);

    assert_in_equality(&xxx, &yyy);
    Assert::assert_true(0 < xxx.compare_to(&yyy));
    Assert::assert_true(yyy.compare_to(&xxx) < 0);

    xxx.copy(&yyy);
    assert_sanity(&xxx);
    assert_equality(&xxx, &yyy);
    Ok(())
}

/// `assertSanity(x)`.
fn assert_sanity(x: &dyn MutableValue) {
    assert_equality(x, x);
    let y = x.duplicate();
    assert_equality(x, y.as_ref());
}

/// `assertEquality(x, y)`.
fn assert_equality(x: &dyn MutableValue, y: &dyn MutableValue) {
    assert_equals(&x.hash_code(), &y.hash_code());

    // assertEquals(x, y): `x.equals(y)`.
    Assert::assert_true(x.value_equals(y));
    Assert::assert_true(y.value_equals(x));

    Assert::assert_true(x.equals_same_type(y));
    Assert::assert_true(y.equals_same_type(x));

    assert_equals(&0, &x.compare_to(y));
    assert_equals(&0, &y.compare_to(x));

    assert_equals(&0, &x.compare_same_type(y));
    assert_equals(&0, &y.compare_same_type(x));
}

/// `assertInEquality(x, y)`.
fn assert_in_equality(x: &dyn MutableValue, y: &dyn MutableValue) {
    Assert::assert_false(x.value_equals(y));
    Assert::assert_false(y.value_equals(x));

    Assert::assert_false(x.equals_same_type(y));
    Assert::assert_false(y.equals_same_type(x));

    Assert::assert_false(0 == x.compare_to(y));
    Assert::assert_false(0 == y.compare_to(x));
}
