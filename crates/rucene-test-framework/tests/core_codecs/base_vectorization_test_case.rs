//! Port of `lucene/core/src/test/org/apache/lucene/internal/vectorization/BaseVectorizationTestCase.java`,
//! for the `core_codecs` test classes that
//! extend it (`TestFlatVectorScorer`). The class is the same as the one
//! `core_internal_vectorization_tests.rs` declares for its own suites; each
//! test binary declares its own copy.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::BeforeClass;
use rucene::internal::vectorization::vectorization_provider::{self, VectorizationProvider};
use rucene::internal::vectorization::DefaultVectorizationProvider;
use rucene_test_framework::util::LuceneTestCase;

/// `LUCENE_PROVIDER` (`protected static final`): `defaultProvider()`.
pub fn lucene_provider() -> Arc<dyn VectorizationProvider> {
    static P: LazyLock<Arc<dyn VectorizationProvider>> = LazyLock::new(default_provider);
    Arc::clone(&P)
}

/// `PANAMA_PROVIDER` (`protected static final`): `maybePanamaProvider()`.
pub fn panama_provider() -> Arc<dyn VectorizationProvider> {
    static P: LazyLock<Arc<dyn VectorizationProvider>> = LazyLock::new(maybe_panama_provider);
    Arc::clone(&P)
}

/// `defaultProvider()` (`public static`).
pub fn default_provider() -> Arc<dyn VectorizationProvider> {
    Arc::new(DefaultVectorizationProvider::new())
}

/// `maybePanamaProvider()` (`public static`).
pub fn maybe_panama_provider() -> Arc<dyn VectorizationProvider> {
    vectorization_provider::lookup(true)
}

/// `BaseVectorizationTestCase.beforeClass()` (`@BeforeClass`).
///
/// Java compares the two providers' runtime classes
/// (`PANAMA_PROVIDER.getClass() != LUCENE_PROVIDER.getClass()`); the
/// providers' `Debug` names stand for the classes.
fn before_class() {
    let panama = format!("{:?}", panama_provider());
    let lucene = format!("{:?}", lucene_provider());
    let class_of = |s: &str| s.split([' ', '{', '(']).next().unwrap_or("").to_string();
    LuceneTestCase::assume_true(
        "Test only works when JDK's vector incubator module is enabled.",
        class_of(&panama) != class_of(&lucene),
    );
}

/// The `BaseVectorizationTestCase` class, with its `@BeforeClass`.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.internal.vectorization.BaseVectorizationTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(
                Method::new("beforeClass")
                    .annotate(BeforeClass)
                    .throws(&["java.lang.Exception"])
                    .static_body(before_class),
            )
            .build()
    });
    Arc::clone(&CLASS)
}
