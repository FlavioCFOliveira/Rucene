//! Port of the `lucene/core/src/test/org/apache/lucene/internal/tests/` test
//! class of Lucene 10.5.0, run as a suite by the ported runner:
//! `TestTestSecrets`.
//!
//! `testCannotSet` passes Java `null` to each `TestSecrets.set*Access`
//! setter: the setters take an `Option`, and `null` is `None` (user decision,
//! sprint 9).

mod lucene_core_shims;
mod suite_seeds;

use std::sync::Arc;

use lucene_core_shims::{lucene_class, run, t};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::internal::tests_hooks::test_secrets::TestSecrets;
use rucene_test_framework::util::java_exceptions::expect_throws;

/// `IllegalCallerException`.
const ICE: &str = "java.lang.IllegalCallerException";

/// `TestTestSecrets.testCallerOfGetter()`.
fn test_secrets_test_caller_of_getter() -> Result<()> {
    let expected = expect_throws(ICE, illegal_caller);
    Assert::assert_equals_str(
        None,
        Some("Lucene TestSecrets can only be used by the test-framework."),
        expected.message(),
    );
    Ok(())
}

/// `illegalCaller()` (`private static`).
fn illegal_caller() -> Result<()> {
    TestSecrets::get_index_writer_access().map(drop)
}

/// `TestTestSecrets.testCannotSet()`.
fn test_secrets_test_cannot_set() -> Result<()> {
    expect_throws(ICE, || TestSecrets::set_index_writer_access(None));
    expect_throws(ICE, || {
        TestSecrets::set_concurrent_merge_scheduler_access(None)
    });
    expect_throws(ICE, || TestSecrets::set_index_package_access(None));
    expect_throws(ICE, || TestSecrets::set_segment_reader_access(None));
    Ok(())
}

fn test_secrets_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.internal.tests.TestTestSecrets")
        .method(t("testCallerOfGetter", test_secrets_test_caller_of_getter))
        .method(t("testCannotSet", test_secrets_test_cannot_set))
        .build()
}

#[test]
fn test_test_secrets() {
    run(test_secrets_class);
}
