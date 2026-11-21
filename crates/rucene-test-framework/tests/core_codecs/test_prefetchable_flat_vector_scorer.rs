//! Port of `lucene/core/src/test/org/apache/lucene/codecs/hnsw/TestPrefetchableFlatVectorScorer.java`.
//!
//! Tests that `PrefetchableFlatVectorScorer` overrides all methods from
//! `FlatVectorsScorer`.
//!
//! `implTestDeclaredMethodsOverridden` reflects on `getDeclaredMethods()`;
//! Rucene has no run-time reflection, so the methods are declared lists
//! (name, parameter types, return type, modifiers): the declared methods of
//! the Java interface or superclass (`FlatVectorsScorer.java:42-77`,
//! `RandomVectorScorerSupplier.java:30-36`,
//! `RandomVectorScorer.java:90-113`,
//! `UpdateableRandomVectorScorer.java:49-72`), and the methods the Rust
//! `impl` blocks of the `Prefetchable*` types define
//! (`src/codecs/hnsw/graph_provider.rs`), under their Java names and
//! signatures. The Java `float[]`/`byte[]` overloads of
//! `getRandomVectorScorer` are Rust's `get_random_vector_scorer_float` and
//! `get_random_vector_scorer_byte`; `values()` is Rust's
//! `HasKnnVectorValues::values`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;

use crate::core_support::{assert_equals_msg, class, test};

/// A declared Java method: name, parameter types, return type, and whether
/// it is `final` or `static`.
type DeclaredMethod = (&'static str, &'static [&'static str], &'static str, bool);

/// `FlatVectorsScorer.class.getDeclaredMethods()`.
const FLAT_VECTORS_SCORER: &[DeclaredMethod] = &[
    (
        "getRandomVectorScorerSupplier",
        &["VectorSimilarityFunction", "KnnVectorValues"],
        "RandomVectorScorerSupplier",
        false,
    ),
    (
        "getRandomVectorScorer",
        &["VectorSimilarityFunction", "KnnVectorValues", "float[]"],
        "RandomVectorScorer",
        false,
    ),
    (
        "getRandomVectorScorer",
        &["VectorSimilarityFunction", "KnnVectorValues", "byte[]"],
        "RandomVectorScorer",
        false,
    ),
    ("checkDimensions", &["int", "int"], "void", true),
];

/// The methods of `impl FlatVectorsScorer for PrefetchableFlatVectorScorer`
/// and its `Display` (`toString`).
const PREFETCHABLE_FLAT_VECTOR_SCORER: &[DeclaredMethod] = &[
    (
        "getRandomVectorScorerSupplier",
        &["VectorSimilarityFunction", "KnnVectorValues"],
        "RandomVectorScorerSupplier",
        false,
    ),
    (
        "getRandomVectorScorer",
        &["VectorSimilarityFunction", "KnnVectorValues", "float[]"],
        "RandomVectorScorer",
        false,
    ),
    (
        "getRandomVectorScorer",
        &["VectorSimilarityFunction", "KnnVectorValues", "byte[]"],
        "RandomVectorScorer",
        false,
    ),
    ("toString", &[], "String", false),
];

/// `RandomVectorScorerSupplier.class.getDeclaredMethods()`.
const RANDOM_VECTOR_SCORER_SUPPLIER: &[DeclaredMethod] = &[
    ("scorer", &[], "UpdateableRandomVectorScorer", false),
    ("copy", &[], "RandomVectorScorerSupplier", false),
];

/// The methods of `impl RandomVectorScorerSupplier for
/// PrefetchableRandomVectorScorerSupplier`.
const PREFETCHABLE_RANDOM_VECTOR_SCORER_SUPPLIER: &[DeclaredMethod] = &[
    ("scorer", &[], "UpdateableRandomVectorScorer", false),
    ("copy", &[], "RandomVectorScorerSupplier", false),
];

/// `RandomVectorScorer.AbstractRandomVectorScorer.class.getDeclaredMethods()`
/// (the superclass of `PrefetchableRandomVectorScorer`), and likewise of
/// `UpdateableRandomVectorScorer.AbstractUpdateableRandomVectorScorer`.
const ABSTRACT_RANDOM_VECTOR_SCORER: &[DeclaredMethod] = &[
    ("maxOrd", &[], "int", false),
    ("ordToDoc", &["int"], "int", false),
    ("getAcceptOrds", &["Bits"], "Bits", false),
    ("values", &[], "KnnVectorValues", false),
];

/// The methods of `impl RandomVectorScorer` and `impl HasKnnVectorValues`
/// for `PrefetchableRandomVectorScorer`.
const PREFETCHABLE_RANDOM_VECTOR_SCORER: &[DeclaredMethod] = &[
    ("score", &["int"], "float", false),
    ("bulkScore", &["int[]", "float[]", "int"], "float", false),
    ("maxOrd", &[], "int", false),
    ("ordToDoc", &["int"], "int", false),
    ("getAcceptOrds", &["Bits"], "Bits", false),
    ("values", &[], "KnnVectorValues", false),
];

/// The methods of `impl RandomVectorScorer`, `impl
/// UpdateableRandomVectorScorer` and `impl HasKnnVectorValues` for
/// `PrefetchableUpdatableRandomVectorScorer`.
const PREFETCHABLE_UPDATABLE_RANDOM_VECTOR_SCORER: &[DeclaredMethod] = &[
    ("setScoringOrdinal", &["int"], "void", false),
    ("score", &["int"], "float", false),
    ("bulkScore", &["int[]", "float[]", "int"], "float", false),
    ("maxOrd", &[], "int", false),
    ("ordToDoc", &["int"], "int", false),
    ("getAcceptOrds", &["Bits"], "Bits", false),
    ("values", &[], "KnnVectorValues", false),
];

/// `TestPrefetchableFlatVectorScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.hnsw.TestPrefetchableFlatVectorScorer",
        vec![test(
            "testDeclaredMethodsOverridden",
            test_declared_methods_overridden,
        )
        .annotate(Test::new())],
    )
}

/// Port of `TestPrefetchableFlatVectorScorer.testDeclaredMethodsOverridden`.
fn test_declared_methods_overridden() -> Result<()> {
    impl_test_declared_methods_overridden(
        FLAT_VECTORS_SCORER,
        "PrefetchableFlatVectorScorer",
        PREFETCHABLE_FLAT_VECTOR_SCORER,
    );
    impl_test_declared_methods_overridden(
        RANDOM_VECTOR_SCORER_SUPPLIER,
        "PrefetchableFlatVectorScorer$PrefetchableRandomVectorScorerSupplier",
        PREFETCHABLE_RANDOM_VECTOR_SCORER_SUPPLIER,
    );
    impl_test_declared_methods_overridden(
        ABSTRACT_RANDOM_VECTOR_SCORER,
        "PrefetchableFlatVectorScorer$PrefetchableRandomVectorScorer",
        PREFETCHABLE_RANDOM_VECTOR_SCORER,
    );
    impl_test_declared_methods_overridden(
        ABSTRACT_RANDOM_VECTOR_SCORER,
        "PrefetchableFlatVectorScorer$PrefetchableUpdatableRandomVectorScorer",
        PREFETCHABLE_UPDATABLE_RANDOM_VECTOR_SCORER,
    );
    Ok(())
}

/// Port of `TestPrefetchableFlatVectorScorer.implTestDeclaredMethodsOverridden`.
fn impl_test_declared_methods_overridden(
    interface_methods: &[DeclaredMethod],
    impl_class: &str,
    impl_methods: &[DeclaredMethod],
) {
    for (name, parameter_types, return_type, final_or_static) in interface_methods {
        if *final_or_static {
            continue;
        }
        match impl_methods
            .iter()
            .find(|(n, p, _, _)| n == name && p == parameter_types)
        {
            Some((_, _, sub_return_type, _)) => assert_equals_msg(
                Some("getReturnType() difference"),
                return_type,
                sub_return_type,
            ),
            None => Assert::fail(&format!(
                "class org.apache.lucene.codecs.hnsw.{impl_class} needs to override '{name}({})'",
                parameter_types.join(",")
            )),
        }
    }
}
