//! Port of `org.apache.lucene.analysis.TestAnalysisSPILoader`.
//!
//! A Java `Class` object is identified here by its binary name: the
//! `getClass()` of a factory is its `java_class_name()`, and a class
//! `lookupClass` returns is its `get_name()`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::{
    available_char_filters, available_token_filters, available_tokenizers, char_filter_for_name,
    char_filter_lookup_class, token_filter_for_name, token_filter_lookup_class, tokenizer_for_name,
    tokenizer_lookup_class,
};
use rucene::analysis::standard::StandardTokenizerFactory;
use rucene::error::Result;
use rucene::util::extra::Version;
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::fake_factories::{FakeCharFilterFactory, FakeTokenFilterFactory};
use crate::lucene_core_shims::{lucene_class, t};

/// `IllegalArgumentException`.
const IAE: &str = "java.lang.IllegalArgumentException";

/// `assertSame(Class expected, Class actual)` over two classes' binary
/// names.
fn assert_same_class(expected: &str, actual: &str) {
    if expected != actual {
        Assert::fail(&format!(
            "expected same:<class {expected}> was not:<class {actual}>"
        ));
    }
}

/// `versionArgOnly()` (`private`).
fn version_arg_only() -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert(
        "luceneMatchVersion".to_string(),
        Version::LATEST.to_string(),
    );
    map
}

fn test_lookup_tokenizer() -> Result<()> {
    let standard = StandardTokenizerFactory::class();
    assert_same_class(
        standard.get_name(),
        tokenizer_for_name("Standard", &mut version_arg_only())?.java_class_name(),
    );
    assert_same_class(
        standard.get_name(),
        tokenizer_for_name("STANDARD", &mut version_arg_only())?.java_class_name(),
    );
    assert_same_class(
        standard.get_name(),
        tokenizer_for_name("standard", &mut version_arg_only())?.java_class_name(),
    );
    Ok(())
}

fn test_bogus_lookup_tokenizer() -> Result<()> {
    expect_throws(IAE, || {
        tokenizer_for_name("sdfsdfsdfdsfsdfsdf", &mut HashMap::new())
    });

    expect_throws(IAE, || {
        tokenizer_for_name("!(**#$U*#$*", &mut HashMap::new())
    });
    Ok(())
}

fn test_lookup_tokenizer_class() -> Result<()> {
    let standard = StandardTokenizerFactory::class();
    assert_same_class(
        standard.get_name(),
        tokenizer_lookup_class("Standard")?.get_name(),
    );
    assert_same_class(
        standard.get_name(),
        tokenizer_lookup_class("STANDARD")?.get_name(),
    );
    assert_same_class(
        standard.get_name(),
        tokenizer_lookup_class("standard")?.get_name(),
    );
    Ok(())
}

fn test_bogus_lookup_tokenizer_class() -> Result<()> {
    expect_throws(IAE, || tokenizer_lookup_class("sdfsdfsdfdsfsdfsdf"));

    expect_throws(IAE, || tokenizer_lookup_class("!(**#$U*#$*"));
    Ok(())
}

fn test_available_tokenizers() -> Result<()> {
    Assert::assert_true(available_tokenizers().iter().any(|n| n == "standard"));
    Ok(())
}

fn test_lookup_token_filter() -> Result<()> {
    let fake = FakeTokenFilterFactory::class();
    assert_same_class(
        fake.get_name(),
        token_filter_for_name("Fake", &mut version_arg_only())?.java_class_name(),
    );
    assert_same_class(
        fake.get_name(),
        token_filter_for_name("FAKE", &mut version_arg_only())?.java_class_name(),
    );
    assert_same_class(
        fake.get_name(),
        token_filter_for_name("fake", &mut version_arg_only())?.java_class_name(),
    );
    Ok(())
}

fn test_bogus_lookup_token_filter() -> Result<()> {
    expect_throws(IAE, || {
        token_filter_for_name("sdfsdfsdfdsfsdfsdf", &mut HashMap::new())
    });

    expect_throws(IAE, || {
        token_filter_for_name("!(**#$U*#$*", &mut HashMap::new())
    });
    Ok(())
}

fn test_lookup_token_filter_class() -> Result<()> {
    let fake = FakeTokenFilterFactory::class();
    assert_same_class(
        fake.get_name(),
        token_filter_lookup_class("Fake")?.get_name(),
    );
    assert_same_class(
        fake.get_name(),
        token_filter_lookup_class("FAKE")?.get_name(),
    );
    assert_same_class(
        fake.get_name(),
        token_filter_lookup_class("fake")?.get_name(),
    );
    Ok(())
}

fn test_bogus_lookup_token_filter_class() -> Result<()> {
    expect_throws(IAE, || token_filter_lookup_class("sdfsdfsdfdsfsdfsdf"));

    expect_throws(IAE, || token_filter_lookup_class("!(**#$U*#$*"));
    Ok(())
}

fn test_available_token_filters() -> Result<()> {
    Assert::assert_true(available_token_filters().iter().any(|n| n == "fake"));
    Ok(())
}

fn test_lookup_char_filter() -> Result<()> {
    let fake = FakeCharFilterFactory::class();
    assert_same_class(
        fake.get_name(),
        char_filter_for_name("Fake", &mut version_arg_only())?.java_class_name(),
    );
    assert_same_class(
        fake.get_name(),
        char_filter_for_name("FAKE", &mut version_arg_only())?.java_class_name(),
    );
    assert_same_class(
        fake.get_name(),
        char_filter_for_name("fake", &mut version_arg_only())?.java_class_name(),
    );
    Ok(())
}

fn test_bogus_lookup_char_filter() -> Result<()> {
    expect_throws(IAE, || {
        char_filter_for_name("sdfsdfsdfdsfsdfsdf", &mut HashMap::new())
    });

    expect_throws(IAE, || {
        char_filter_for_name("!(**#$U*#$*", &mut HashMap::new())
    });
    Ok(())
}

fn test_lookup_char_filter_class() -> Result<()> {
    let fake = FakeCharFilterFactory::class();
    assert_same_class(
        fake.get_name(),
        char_filter_lookup_class("Fake")?.get_name(),
    );
    assert_same_class(
        fake.get_name(),
        char_filter_lookup_class("FAKE")?.get_name(),
    );
    assert_same_class(
        fake.get_name(),
        char_filter_lookup_class("fake")?.get_name(),
    );
    Ok(())
}

fn test_bogus_lookup_char_filter_class() -> Result<()> {
    expect_throws(IAE, || char_filter_lookup_class("sdfsdfsdfdsfsdfsdf"));

    expect_throws(IAE, || char_filter_lookup_class("!(**#$U*#$*"));
    Ok(())
}

fn test_available_char_filters() -> Result<()> {
    Assert::assert_true(available_char_filters().iter().any(|n| n == "fake"));
    Ok(())
}

/// The class `org.apache.lucene.analysis.TestAnalysisSPILoader`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestAnalysisSPILoader")
        .method(t("testLookupTokenizer", test_lookup_tokenizer))
        .method(t("testBogusLookupTokenizer", test_bogus_lookup_tokenizer))
        .method(t("testLookupTokenizerClass", test_lookup_tokenizer_class))
        .method(t(
            "testBogusLookupTokenizerClass",
            test_bogus_lookup_tokenizer_class,
        ))
        .method(t("testAvailableTokenizers", test_available_tokenizers))
        .method(t("testLookupTokenFilter", test_lookup_token_filter))
        .method(t(
            "testBogusLookupTokenFilter",
            test_bogus_lookup_token_filter,
        ))
        .method(t(
            "testLookupTokenFilterClass",
            test_lookup_token_filter_class,
        ))
        .method(t(
            "testBogusLookupTokenFilterClass",
            test_bogus_lookup_token_filter_class,
        ))
        .method(t("testAvailableTokenFilters", test_available_token_filters))
        .method(t("testLookupCharFilter", test_lookup_char_filter))
        .method(t(
            "testBogusLookupCharFilter",
            test_bogus_lookup_char_filter,
        ))
        .method(t(
            "testLookupCharFilterClass",
            test_lookup_char_filter_class,
        ))
        .method(t(
            "testBogusLookupCharFilterClass",
            test_bogus_lookup_char_filter_class,
        ))
        .method(t("testAvailableCharFilters", test_available_char_filters))
        .build()
}
