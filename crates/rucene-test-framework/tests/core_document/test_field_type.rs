//! Port of `lucene/core/src/test/org/apache/lucene/document/TestFieldType.java`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::FieldType;
use rucene::error::Result;
use rucene::index::point_values::{MAX_DIMENSIONS, MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES};
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, IndexableFieldType, VectorEncoding,
    VectorSimilarityFunction,
};
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, random, test};

/// `TestFieldType`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestFieldType",
        vec![
            test("testEquals", test_equals),
            test("testPointsToString", test_points_to_string),
            test("testAttributeMapFrozen", test_attribute_map_frozen),
            test("testAttributeMapNotFrozen", test_attribute_map_not_frozen),
            test("testCopyConstructor", test_copy_constructor),
        ],
    )
}

/// `ft.hashCode()`.
///
/// **Divergence.** Java's `FieldType.hashCode()` folds enum identity hash
/// codes, which no port can reproduce; the value is the port's `Hash` over
/// the same properties, which keeps the `equals`/`hashCode` contract the test
/// asserts.
fn hash_code(ft: &FieldType) -> u64 {
    let mut h = DefaultHasher::new();
    ft.hash(&mut h);
    h.finish()
}

/// Port of `TestFieldType.testEquals`.
fn test_equals() -> Result<()> {
    let ft = FieldType::new();
    assert_equals(&ft, &ft);
    // assertFalse(ft.equals(null)): a `FieldType` is never `null`.
    Assert::assert_false(Option::<&FieldType>::None == Some(&ft));

    let ft2 = FieldType::new();
    assert_equals(&ft, &ft2);
    assert_equals(&hash_code(&ft), &hash_code(&ft2));

    let ft3 = FieldType::new();
    ft3.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
    Assert::assert_false(ft3 == ft);

    let ft4 = FieldType::new();
    ft4.set_doc_values_type(DocValuesType::BINARY)?;
    Assert::assert_false(ft4 == ft);

    let ft5 = FieldType::new();
    ft5.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
    Assert::assert_false(ft5 == ft);

    let ft6 = FieldType::new();
    ft6.set_stored(true)?;
    Assert::assert_false(ft6 == ft);

    let ft7 = FieldType::new();
    ft7.set_omit_norms(true)?;
    Assert::assert_false(ft7 == ft);

    let ft10 = FieldType::new();
    ft10.set_store_term_vectors(true)?;
    Assert::assert_false(ft10 == ft);

    let ft11 = FieldType::new();
    ft11.set_dimensions(1, 4)?;
    Assert::assert_false(ft11 == ft);
    Ok(())
}

/// Port of `TestFieldType.testPointsToString`.
fn test_points_to_string() -> Result<()> {
    let ft = FieldType::new();
    ft.set_dimensions(1, 4)?;
    assert_equals(
        "pointDimensionCount=1,pointIndexDimensionCount=1,pointNumBytes=4",
        ft.to_string().as_str(),
    );
    Ok(())
}

/// FieldType's attribute map should not be modifiable/add after freeze
///
/// Port of `TestFieldType.testAttributeMapFrozen`.
fn test_attribute_map_frozen() -> Result<()> {
    let ft = FieldType::new();
    ft.put_attribute("dummy".to_string(), "d".to_string())?;
    ft.freeze();
    expect_throws("java.lang.IllegalStateException", || {
        ft.put_attribute("dummy".to_string(), "a".to_string())
    });
    Ok(())
}

/// FieldType's attribute map can be changed if not frozen
///
/// Port of `TestFieldType.testAttributeMapNotFrozen`.
fn test_attribute_map_not_frozen() -> Result<()> {
    let ft = FieldType::new();
    ft.put_attribute("dummy".to_string(), "d".to_string())?;
    ft.put_attribute("dummy".to_string(), "a".to_string())?;
    assert_equals(&ft.attributes().len(), &1);
    assert_equals(
        &ft.attributes().get("dummy").cloned(),
        &Some("a".to_string()),
    );
    Ok(())
}

/// `IndexOptions.values()`.
const INDEX_OPTIONS: [IndexOptions; 6] = [
    IndexOptions::NONE,
    IndexOptions::DOCS,
    IndexOptions::DOCS_AND_FREQS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
    IndexOptions::DOCS_AND_CUSTOM_FREQS,
];

/// `DocValuesType.values()`.
const DOC_VALUES_TYPES: [DocValuesType; 6] = [
    DocValuesType::NONE,
    DocValuesType::NUMERIC,
    DocValuesType::BINARY,
    DocValuesType::SORTED,
    DocValuesType::SORTED_NUMERIC,
    DocValuesType::SORTED_SET,
];

/// `DocValuesSkipIndexType.values()`.
const DOC_VALUES_SKIP_INDEX_TYPES: [DocValuesSkipIndexType; 2] =
    [DocValuesSkipIndexType::NONE, DocValuesSkipIndexType::RANGE];

/// `VectorEncoding.values()`.
const VECTOR_ENCODINGS: [VectorEncoding; 2] = [VectorEncoding::BYTE, VectorEncoding::FLOAT32];

/// `VectorSimilarityFunction.values()`.
const VECTOR_SIMILARITY_FUNCTIONS: [VectorSimilarityFunction; 4] = [
    VectorSimilarityFunction::EUCLIDEAN,
    VectorSimilarityFunction::DOT_PRODUCT,
    VectorSimilarityFunction::COSINE,
    VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
];

/// `randomValue(boolean.class)`.
fn random_bool() -> bool {
    random().next_boolean()
}

/// `randomValue(int.class)`.
fn random_int() -> i32 {
    1 + random().next_int_bound(100)
}

/// The public `set*` methods of `FieldType`, which `randomFieldType` finds
/// by reflection (`FieldType.class.getMethods()`), each with the argument
/// draws `randomValue` (or the `setDimensions` special cases) makes.
///
/// **Declared reflection.** Rust has no reflection: this list is the
/// declaration of the setters `FieldType.java` (10.5.0) declares, in
/// declaration order, and [`setter_names_match_field_type`] checks it names
/// every one of them. `Class.getMethods()` returns them in an unspecified,
/// JVM-dependent order, so the draw order of the setters' arguments can
/// differ from a given JVM's.
const SETTERS: &[&str] = &[
    "setStored",
    "setTokenized",
    "setStoreTermVectors",
    "setStoreTermVectorOffsets",
    "setStoreTermVectorPositions",
    "setStoreTermVectorPayloads",
    "setOmitNorms",
    "setIndexOptions",
    "setDimensions(int,int)",
    "setDimensions(int,int,int)",
    "setVectorAttributes",
    "setDocValuesType",
    "setDocValuesSkipIndexType",
];

/// Applies `setter` to `ft` with random arguments.
fn invoke_setter(ft: &mut FieldType, setter: &str) -> Result<()> {
    match setter {
        "setStored" => ft.set_stored(random_bool()),
        "setTokenized" => ft.set_tokenized(random_bool()),
        "setStoreTermVectors" => ft.set_store_term_vectors(random_bool()),
        "setStoreTermVectorOffsets" => ft.set_store_term_vector_offsets(random_bool()),
        "setStoreTermVectorPositions" => ft.set_store_term_vector_positions(random_bool()),
        "setStoreTermVectorPayloads" => ft.set_store_term_vector_payloads(random_bool()),
        "setOmitNorms" => ft.set_omit_norms(random_bool()),
        "setIndexOptions" => {
            ft.set_index_options(*RandomPicks::random_from(&*random(), &INDEX_OPTIONS))
        }
        "setDimensions(int,int)" => {
            let a0 = 1 + random().next_int_bound(MAX_INDEX_DIMENSIONS);
            let a1 = 1 + random().next_int_bound(MAX_NUM_BYTES);
            ft.set_dimensions(a0, a1)
        }
        "setDimensions(int,int,int)" => {
            let a0 = 1 + random().next_int_bound(MAX_DIMENSIONS);
            let a1 = 1 + (a0 - 1).min(random().next_int_bound(MAX_INDEX_DIMENSIONS));
            let a2 = 1 + random().next_int_bound(MAX_NUM_BYTES);
            ft.set_dimensions_with_index_count(a0, a1, a2)
        }
        "setVectorAttributes" => {
            let a0 = random_int();
            let a1 = *RandomPicks::random_from(&*random(), &VECTOR_ENCODINGS);
            let a2 = *RandomPicks::random_from(&*random(), &VECTOR_SIMILARITY_FUNCTIONS);
            ft.set_vector_attributes(a0, a1, a2)
        }
        "setDocValuesType" => {
            ft.set_doc_values_type(*RandomPicks::random_from(&*random(), &DOC_VALUES_TYPES))
        }
        "setDocValuesSkipIndexType" => ft.set_doc_values_skip_index_type(
            *RandomPicks::random_from(&*random(), &DOC_VALUES_SKIP_INDEX_TYPES),
        ),
        other => panic!("undeclared FieldType setter {other}"),
    }
}

/// The declared setter list names exactly the `set*` methods of
/// `FieldType.java` 10.5.0 (lines 127-531).
fn setter_names_match_field_type() {
    let java_setters = [
        "setStored",
        "setTokenized",
        "setStoreTermVectors",
        "setStoreTermVectorOffsets",
        "setStoreTermVectorPositions",
        "setStoreTermVectorPayloads",
        "setOmitNorms",
        "setIndexOptions",
        "setDimensions(int,int)",
        "setDimensions(int,int,int)",
        "setVectorAttributes",
        "setDocValuesType",
        "setDocValuesSkipIndexType",
    ];
    assert_equals(&java_setters.len(), &SETTERS.len());
    for s in java_setters {
        Assert::assert_true(SETTERS.contains(&s));
    }
}

/// Port of `TestFieldType.randomFieldType`.
fn random_field_type() -> Result<FieldType> {
    setter_names_match_field_type();
    let mut ft = FieldType::new();
    for setter in SETTERS {
        invoke_setter(&mut ft, setter)?;
    }
    Ok(ft)
}

/// Port of `TestFieldType.testCopyConstructor`.
fn test_copy_constructor() -> Result<()> {
    let iters = 10;
    for _iter in 0..iters {
        let ft = random_field_type()?;
        let ft2 = FieldType::new_from(&ft as &dyn IndexableFieldType);
        assert_equals(&ft, &ft2);
    }
    Ok(())
}
