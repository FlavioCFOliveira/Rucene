//! Port of `lucene/core/src/test/org/apache/lucene/util/TestVirtualMethod.java`.
//!
//! Rust has no reflection: Rucene's [`VirtualMethod`] works on a
//! [`MethodClass`] model of the class hierarchy, so the nested classes are
//! declared as `MethodClass`es carrying the methods each declares. A method
//! is named without its parameter types (`publicTest(String)` is
//! `publicTest`), as Rucene's model keys methods by name.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::{MethodClass, VirtualMethod};
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, test};

/// `TestVirtualMethod`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestVirtualMethod",
        vec![
            test("testGeneral", test_general),
            test("testExceptions", test_exceptions),
        ],
    )
}

/// `java.lang.Object`.
static OBJECT: LazyLock<Arc<MethodClass>> =
    LazyLock::new(|| MethodClass::new("java.lang.Object", Vec::<&str>::new()));

/// `static class Base`.
static BASE: LazyLock<Arc<MethodClass>> = LazyLock::new(|| {
    MethodClass::extending(
        "org.apache.lucene.util.TestVirtualMethod$Base",
        OBJECT.clone(),
        ["publicTest", "protectedTest"],
    )
});

/// `static class Nested1 extends Base`.
static NESTED1: LazyLock<Arc<MethodClass>> = LazyLock::new(|| {
    MethodClass::extending(
        "org.apache.lucene.util.TestVirtualMethod$Nested1",
        BASE.clone(),
        ["publicTest", "protectedTest"],
    )
});

/// `static class Nested2 extends Nested1`.
static NESTED2: LazyLock<Arc<MethodClass>> = LazyLock::new(|| {
    MethodClass::extending(
        "org.apache.lucene.util.TestVirtualMethod$Nested2",
        NESTED1.clone(),
        ["protectedTest"],
    )
});

/// `static class Nested3 extends Nested2`.
static NESTED3: LazyLock<Arc<MethodClass>> = LazyLock::new(|| {
    MethodClass::extending(
        "org.apache.lucene.util.TestVirtualMethod$Nested3",
        NESTED2.clone(),
        ["publicTest"],
    )
});

/// `static class Nested4 extends Base`.
static NESTED4: LazyLock<Arc<MethodClass>> = LazyLock::new(|| {
    MethodClass::extending(
        "org.apache.lucene.util.TestVirtualMethod$Nested4",
        BASE.clone(),
        Vec::<&str>::new(),
    )
});

/// `static class Nested5 extends Nested4`.
static NESTED5: LazyLock<Arc<MethodClass>> = LazyLock::new(|| {
    MethodClass::extending(
        "org.apache.lucene.util.TestVirtualMethod$Nested5",
        NESTED4.clone(),
        Vec::<&str>::new(),
    )
});

/// `private static final VirtualMethod<Base> publicTestMethod`.
static PUBLIC_TEST_METHOD: LazyLock<VirtualMethod> = LazyLock::new(|| {
    VirtualMethod::new(BASE.clone(), "publicTest")
        .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e))
});

/// `private static final VirtualMethod<Base> protectedTestMethod`.
static PROTECTED_TEST_METHOD: LazyLock<VirtualMethod> = LazyLock::new(|| {
    VirtualMethod::new(BASE.clone(), "protectedTest")
        .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e))
});

fn test_general() -> Result<()> {
    let public_test_method = &*PUBLIC_TEST_METHOD;
    let protected_test_method = &*PROTECTED_TEST_METHOD;
    assert_equals(&0, &public_test_method.get_implementation_distance(&BASE)?);
    assert_equals(
        &1,
        &public_test_method.get_implementation_distance(&NESTED1)?,
    );
    assert_equals(
        &1,
        &public_test_method.get_implementation_distance(&NESTED2)?,
    );
    assert_equals(
        &3,
        &public_test_method.get_implementation_distance(&NESTED3)?,
    );
    Assert::assert_false(public_test_method.is_overridden_as_of(&NESTED4)?);
    Assert::assert_false(public_test_method.is_overridden_as_of(&NESTED5)?);

    assert_equals(
        &0,
        &protected_test_method.get_implementation_distance(&BASE)?,
    );
    assert_equals(
        &1,
        &protected_test_method.get_implementation_distance(&NESTED1)?,
    );
    assert_equals(
        &2,
        &protected_test_method.get_implementation_distance(&NESTED2)?,
    );
    assert_equals(
        &2,
        &protected_test_method.get_implementation_distance(&NESTED3)?,
    );
    Assert::assert_false(protected_test_method.is_overridden_as_of(&NESTED4)?);
    Assert::assert_false(protected_test_method.is_overridden_as_of(&NESTED5)?);

    Assert::assert_true(
        VirtualMethod::compare_implementation_distance(
            &NESTED3,
            public_test_method,
            protected_test_method,
        )? == std::cmp::Ordering::Greater,
    );
    // assertEquals(0, compareImplementationDistance(...)): `Integer.compare`.
    assert_equals(
        &std::cmp::Ordering::Equal,
        &VirtualMethod::compare_implementation_distance(
            &NESTED5,
            public_test_method,
            protected_test_method,
        )?,
    );
    Ok(())
}

fn test_exceptions() -> Result<()> {
    // LuceneTestCase is not a subclass and can never override publicTest(String)
    expect_throws("java.lang.IllegalArgumentException", || {
        PUBLIC_TEST_METHOD.get_implementation_distance(&OBJECT)
    });

    // Method bogus() does not exist, so IAE should be thrown
    expect_throws("java.lang.IllegalArgumentException", || {
        VirtualMethod::new(BASE.clone(), "bogus")
    });

    // Method publicTest(String) is not declared in TestClass2, so IAE should be
    // thrown
    expect_throws("java.lang.IllegalArgumentException", || {
        VirtualMethod::new(NESTED2.clone(), "publicTest")
    });

    // try to create a second instance of the same baseClass / method combination
    LazyLock::force(&PUBLIC_TEST_METHOD);
    expect_throws("java.lang.UnsupportedOperationException", || {
        VirtualMethod::new(BASE.clone(), "publicTest")
    });
    Ok(())
}
