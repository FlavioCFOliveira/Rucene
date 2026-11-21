//! Port of `lucene/core/src/test/org/apache/lucene/search/TestFilterWeight.java`.
//!
//! **Divergence from Lucene 10.5.0.** Java reflects on `Weight` and
//! `FilterWeight` (`getDeclaredMethods`, modifiers, return types). Rucene has
//! no run-time reflection, so both method sets are declared lists: the
//! non-final, non-static methods `Weight.java` declares, and the `Weight`
//! methods Rucene's `impl Weight for FilterWeight` (`src/search/weight.rs`)
//! defines. A Rust override has the trait's signature, so the return types
//! are equal by construction.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;

use crate::core_support::{assert_equals_msg, class, test};

/// `TestFilterWeight`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestFilterWeight",
        vec![test(
            "testDeclaredMethodsOverridden",
            test_declared_methods_overridden,
        )
        .annotate(randomizedtesting::junit::Test::new())],
    )
}

/// A declared method: name, parameter types, return type, `final`, `static`.
struct DeclaredMethod {
    name: &'static str,
    parameter_types: &'static [&'static str],
    return_type: &'static str,
    is_final: bool,
    is_static: bool,
}

const fn m(
    name: &'static str,
    parameter_types: &'static [&'static str],
    return_type: &'static str,
    is_final: bool,
) -> DeclaredMethod {
    DeclaredMethod {
        name,
        parameter_types,
        return_type,
        is_final,
        is_static: false,
    }
}

const CTX: &str = "org.apache.lucene.index.LeafReaderContext";

/// `Weight.class.getDeclaredMethods()` (`Weight.java:78-198`).
const WEIGHT_METHODS: &[DeclaredMethod] = &[
    m(
        "matches",
        &[CTX, "int"],
        "org.apache.lucene.search.Matches",
        false,
    ),
    m(
        "explain",
        &[CTX, "int"],
        "org.apache.lucene.search.Explanation",
        false,
    ),
    m("getQuery", &[], "org.apache.lucene.search.Query", true),
    m("scorer", &[CTX], "org.apache.lucene.search.Scorer", true),
    m(
        "scorerSupplier",
        &[CTX],
        "org.apache.lucene.search.ScorerSupplier",
        false,
    ),
    m(
        "bulkScorer",
        &[CTX],
        "org.apache.lucene.search.BulkScorer",
        true,
    ),
    m("count", &[CTX], "int", false),
];

/// `FilterWeight.class.getDeclaredMethod(...)`: the `Weight` (and
/// `SegmentCacheable`) methods `impl Weight for FilterWeight` defines.
const FILTER_WEIGHT_METHODS: &[DeclaredMethod] = &[
    m("isCacheable", &[CTX], "boolean", false),
    m("getQuery", &[], "org.apache.lucene.search.Query", false),
    m(
        "explain",
        &[CTX, "int"],
        "org.apache.lucene.search.Explanation",
        false,
    ),
    m(
        "matches",
        &[CTX, "int"],
        "org.apache.lucene.search.Matches",
        false,
    ),
    m(
        "scorerSupplier",
        &[CTX],
        "org.apache.lucene.search.ScorerSupplier",
        false,
    ),
    m("count", &[CTX], "int", false),
];

fn test_declared_methods_overridden() -> Result<()> {
    // `FilterWeight.class.getSuperclass()` is `Weight`.
    impl_test_declared_methods_overridden(
        ("org.apache.lucene.search.Weight", WEIGHT_METHODS),
        (
            "org.apache.lucene.search.FilterWeight",
            FILTER_WEIGHT_METHODS,
        ),
    );
    Ok(())
}

fn impl_test_declared_methods_overridden(
    super_class: (&str, &[DeclaredMethod]),
    sub_class: (&str, &[DeclaredMethod]),
) {
    for super_class_method in super_class.1 {
        if super_class_method.is_final {
            continue;
        }
        if super_class_method.is_static {
            continue;
        }
        match sub_class.1.iter().find(|s| {
            s.name == super_class_method.name
                && s.parameter_types == super_class_method.parameter_types
        }) {
            Some(sub_class_method) => assert_equals_msg(
                Some("getReturnType() difference"),
                super_class_method.return_type,
                sub_class_method.return_type,
            ),
            None => Assert::fail(&format!(
                "class {} needs to override '{} {}.{}({})'",
                sub_class.0,
                super_class_method.return_type,
                super_class.0,
                super_class_method.name,
                super_class_method.parameter_types.join(",")
            )),
        }
    }
}
