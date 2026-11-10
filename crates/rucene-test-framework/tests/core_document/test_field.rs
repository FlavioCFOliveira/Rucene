//! Port of `lucene/core/src/test/org/apache/lucene/document/TestField.java`:
//! sanity checks of some basics of fields.
//!
//! The `trySet*` helpers call the setters every Java `Field` subclass
//! inherits, which the port declares as
//! [`FieldValueSetters`](rucene::document::FieldValueSetters).

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::StringCharReader;
use rucene::document::{
    BinaryDocValuesField, Document, DoubleDocValuesField, DoubleField, DoublePoint, Field,
    FieldType, FieldValueSetters, FloatDocValuesField, FloatField, FloatPoint, IntField, IntPoint,
    KnnByteVectorField, KnnFloatVectorField, LongField, LongPoint, NumericDocValuesField,
    NumericValue, SortedDocValuesField, Store, StoredField, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexOptions, IndexReader, IndexableField, Term, VectorSimilarityFunction};
use rucene::search::{TermQuery, NO_MORE_DOCS};
use rucene::util::{BytesRef, NumericUtils};
use rucene_test_framework::analysis::{CannedTokenStream, Token};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::java_number::{double_value, float_value, int_value, long_value};
use crate::core_support::{
    assert_array_equals, assert_equals, assert_equals_delta, assert_not_equals, assert_not_null,
    assert_null, class, expect_throws_panic, random, test, IAE,
};

/// `TestField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestField",
        vec![
            test("testDoublePoint", test_double_point),
            test("testDoublePoint2D", test_double_point_2d),
            test("testDoubleDocValuesField", test_double_doc_values_field),
            test("testFloatDocValuesField", test_float_doc_values_field),
            test("testFloatPoint", test_float_point),
            test("testFloatPoint2D", test_float_point_2d),
            test("testIntPoint", test_int_point),
            test("testIntPoint2D", test_int_point_2d),
            test("testIntField", test_int_field),
            test("testLongField", test_long_field),
            test("testFloatField", test_float_field),
            test("testDoubleField", test_double_field),
            test("testNumericDocValuesField", test_numeric_doc_values_field),
            test("testLongPoint", test_long_point),
            test("testLongPoint2D", test_long_point_2d),
            test(
                "testSortedBytesDocValuesField",
                test_sorted_bytes_doc_values_field,
            ),
            test("testBinaryDocValuesField", test_binary_doc_values_field),
            test("testStringField", test_string_field),
            test("testBinaryStringField", test_binary_string_field),
            test("testTextFieldString", test_text_field_string),
            test("testTextFieldReader", test_text_field_reader),
            test("testStoredFieldBytes", test_stored_field_bytes),
            test("testStoredFieldString", test_stored_field_string),
            test("testStoredFieldInt", test_stored_field_int),
            test("testStoredFieldDouble", test_stored_field_double),
            test("testStoredFieldFloat", test_stored_field_float),
            test("testStoredFieldLong", test_stored_field_long),
            test("testIndexedBinaryField", test_indexed_binary_field),
            test("testKnnVectorField", test_knn_vector_field),
            test("testKnnFieldZeroVectors", test_knn_field_zero_vectors),
            test(
                "testKnnFieldSetVectorValueZeroVectors",
                test_knn_field_set_vector_value_zero_vectors,
            ),
            test("testDisabledField", test_disabled_field),
            test("testTokenizedBinaryField", test_tokenized_binary_field),
            test("testOffsetsBinaryField", test_offsets_binary_field),
            test(
                "testTermVectorsOffsetsBinaryField",
                test_term_vectors_offsets_binary_field,
            ),
        ],
    )
}

const ISE: &str = "java.lang.IllegalStateException";

/// `newBytesRef(s)`.
fn nbr(s: &str) -> BytesRef {
    LuceneTestCase::new_bytes_ref_str(s)
}

/// `"s".getBytes(StandardCharsets.UTF_8)` as the `BytesRef` that
/// `setBytesValue(byte[])` wraps it in.
fn utf8(s: &str) -> BytesRef {
    BytesRef::new(s.as_bytes().to_vec())
}

/// `field.toString()`.
fn to_string(f: &dyn std::fmt::Display) -> String {
    format!("{f}")
}

/// `expectThrows(IllegalStateException.class, () -> field.numericValue())`,
/// then the message check of the 2D point tests.
///
/// Java's points throw from `numericValue()` for more than one dimension;
/// the port's `numeric_value` returns an `Option` and must panic with the
/// Java exception to satisfy this check.
fn expect_numeric_value_ise(field: &dyn IndexableField) {
    let expected = expect_throws_panic(ISE, || {
        field.numeric_value();
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or_default()
            .contains("cannot convert to a single numeric value"),
    );
}

/// Port of `testDoublePoint`.
fn test_double_point() -> Result<()> {
    let mut field = DoublePoint::new("foo", &[5.0])?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    FieldValueSetters::set_double_value(&mut field, 6.0)?; // ok
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals_delta(6.0, double_value(field.numeric_value()), 0.0);
    assert_equals("DoublePoint <foo:6.0>", to_string(&field).as_str());
    Ok(())
}

/// Port of `testDoublePoint2D`.
fn test_double_point_2d() -> Result<()> {
    let mut field = DoublePoint::new("foo", &[5.0, 4.0])?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    field.set_double_values(&[6.0, 7.0])?; // ok
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    expect_numeric_value_ise(&field);
    assert_equals("DoublePoint <foo:6.0,7.0>", to_string(&field).as_str());
    Ok(())
}

/// Port of `testDoubleDocValuesField`.
fn test_double_doc_values_field() -> Result<()> {
    let mut field = DoubleDocValuesField::new("foo", 5.0);
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    FieldValueSetters::set_double_value(&mut field, 6.0)?; // ok
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals_delta(
        6.0,
        f64::from_bits(long_value(field.numeric_value()) as u64),
        0.0,
    );
    Ok(())
}

/// Port of `testFloatDocValuesField`.
fn test_float_doc_values_field() -> Result<()> {
    let mut field = FloatDocValuesField::new("foo", 5.0);
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    FieldValueSetters::set_float_value(&mut field, 6.0)?; // ok
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals_delta(
        6.0,
        f32::from_bits(int_value(field.numeric_value()) as u32) as f64,
        0.0,
    );
    Ok(())
}

/// Port of `testFloatPoint`.
fn test_float_point() -> Result<()> {
    let mut field = FloatPoint::new("foo", &[5.0])?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    FieldValueSetters::set_float_value(&mut field, 6.0)?; // ok
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals_delta(6.0, float_value(field.numeric_value()) as f64, 0.0);
    assert_equals("FloatPoint <foo:6.0>", to_string(&field).as_str());
    Ok(())
}

/// Port of `testFloatPoint2D`.
fn test_float_point_2d() -> Result<()> {
    let mut field = FloatPoint::new("foo", &[5.0, 4.0])?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    field.set_float_values(&[6.0, 7.0])?; // ok
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    expect_numeric_value_ise(&field);
    assert_equals("FloatPoint <foo:6.0,7.0>", to_string(&field).as_str());
    Ok(())
}

/// Port of `testIntPoint`.
fn test_int_point() -> Result<()> {
    let mut field = IntPoint::new("foo", &[5])?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    FieldValueSetters::set_int_value(&mut field, 6)?; // ok
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals(&6, &int_value(field.numeric_value()));
    assert_equals("IntPoint <foo:6>", to_string(&field).as_str());
    Ok(())
}

/// Port of `testIntPoint2D`.
fn test_int_point_2d() -> Result<()> {
    let mut field = IntPoint::new("foo", &[5, 4])?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    field.set_int_values(&[6, 7])?; // ok
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    expect_numeric_value_ise(&field);
    assert_equals("IntPoint <foo:6,7>", to_string(&field).as_str());
    Ok(())
}

/// Port of `testIntField`.
fn test_int_field() -> Result<()> {
    let fields = vec![
        IntField::new("foo", 12, Store::NO),
        IntField::new("foo", 12, Store::YES),
    ];
    for mut field in fields {
        try_set_byte_value(&mut field);
        try_set_bytes_value(&mut field);
        try_set_bytes_ref_value(&mut field);
        try_set_double_value(&mut field);
        FieldValueSetters::set_int_value(&mut field, 6)?;
        try_set_long_value(&mut field);
        try_set_float_value(&mut field);
        try_set_long_value(&mut field);
        try_set_reader_value(&mut field);
        try_set_short_value(&mut field);
        try_set_string_value(&mut field);
        try_set_token_stream_value(&mut field);
        assert_equals(&6, &int_value(field.numeric_value()));
        assert_equals(
            &6,
            &NumericUtils::sortable_bytes_to_int(&field.binary_value().expect("binary").bytes, 0),
        );
        assert_equals("IntField <foo:6>", to_string(&field).as_str());
        if field.field_type().stored() {
            assert_equals(&6, &field.stored_value()?.expect("stored").int_value()?);
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// Port of `testLongField`.
fn test_long_field() -> Result<()> {
    let fields = vec![
        LongField::new("foo", 12, Store::NO),
        LongField::new("foo", 12, Store::YES),
    ];
    for mut field in fields {
        try_set_byte_value(&mut field);
        try_set_bytes_value(&mut field);
        try_set_bytes_ref_value(&mut field);
        try_set_double_value(&mut field);
        try_set_int_value(&mut field);
        FieldValueSetters::set_long_value(&mut field, 6)?;
        try_set_float_value(&mut field);
        try_set_reader_value(&mut field);
        try_set_short_value(&mut field);
        try_set_string_value(&mut field);
        try_set_token_stream_value(&mut field);
        assert_equals(&6i64, &long_value(field.numeric_value()));
        assert_equals(
            &6i64,
            &NumericUtils::sortable_bytes_to_long(&field.binary_value().expect("binary").bytes, 0),
        );
        assert_equals("LongField <foo:6>", to_string(&field).as_str());
        if field.field_type().stored() {
            assert_equals(&6i64, &field.stored_value()?.expect("stored").long_value()?);
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// Port of `testFloatField`.
fn test_float_field() -> Result<()> {
    let fields = vec![
        FloatField::new("foo", 12.6, Store::NO),
        FloatField::new("foo", 12.6, Store::YES),
    ];
    for mut field in fields {
        assert_equals_delta(
            12.6f32 as f64,
            NumericUtils::sortable_int_to_float(int_value(field.numeric_value())) as f64,
            0.0,
        );
        assert_equals_delta(
            12.6f32 as f64,
            FloatPoint::decode_dimension(&field.binary_value().expect("binary").bytes, 0) as f64,
            0.0,
        );
        assert_equals("FloatField <foo:12.6>", to_string(&field).as_str());

        try_set_byte_value(&mut field);
        try_set_bytes_value(&mut field);
        try_set_bytes_ref_value(&mut field);
        try_set_double_value(&mut field);
        try_set_int_value(&mut field);
        try_set_long_value(&mut field);
        FieldValueSetters::set_float_value(&mut field, -28.8)?;
        try_set_reader_value(&mut field);
        try_set_short_value(&mut field);
        try_set_string_value(&mut field);
        try_set_token_stream_value(&mut field);

        assert_equals_delta(
            -28.8f32 as f64,
            NumericUtils::sortable_int_to_float(int_value(field.numeric_value())) as f64,
            0.0,
        );
        assert_equals_delta(
            -28.8f32 as f64,
            FloatPoint::decode_dimension(&field.binary_value().expect("binary").bytes, 0) as f64,
            0.0,
        );
        assert_equals("FloatField <foo:-28.8>", to_string(&field).as_str());
        if field.field_type().stored() {
            assert_equals_delta(
                -28.8f32 as f64,
                field.stored_value()?.expect("stored").float_value()? as f64,
                0.0,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// Port of `testDoubleField`.
fn test_double_field() -> Result<()> {
    let fields = vec![
        DoubleField::new("foo", 12.7, Store::NO),
        DoubleField::new("foo", 12.7, Store::YES),
    ];
    for mut field in fields {
        assert_equals_delta(
            12.7,
            NumericUtils::sortable_long_to_double(long_value(field.numeric_value())),
            0.0,
        );
        assert_equals_delta(
            12.7,
            DoublePoint::decode_dimension(&field.binary_value().expect("binary").bytes, 0),
            0.0,
        );
        assert_equals("DoubleField <foo:12.7>", to_string(&field).as_str());

        try_set_byte_value(&mut field);
        try_set_bytes_value(&mut field);
        try_set_bytes_ref_value(&mut field);
        try_set_int_value(&mut field);
        try_set_long_value(&mut field);
        try_set_float_value(&mut field);
        FieldValueSetters::set_double_value(&mut field, -28.8)?;
        try_set_reader_value(&mut field);
        try_set_short_value(&mut field);
        try_set_string_value(&mut field);
        try_set_token_stream_value(&mut field);

        assert_equals_delta(
            -28.8,
            NumericUtils::sortable_long_to_double(long_value(field.numeric_value())),
            0.0,
        );
        assert_equals_delta(
            -28.8,
            DoublePoint::decode_dimension(&field.binary_value().expect("binary").bytes, 0),
            0.0,
        );
        assert_equals("DoubleField <foo:-28.8>", to_string(&field).as_str());
        if field.field_type().stored() {
            assert_equals_delta(
                -28.8,
                field.stored_value()?.expect("stored").double_value()?,
                0.0,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// Port of `testNumericDocValuesField`.
fn test_numeric_doc_values_field() -> Result<()> {
    let mut field = NumericDocValuesField::new("foo", 5);
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    FieldValueSetters::set_long_value(&mut field, 6)?; // ok
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals(&6i64, &long_value(field.numeric_value()));
    Ok(())
}

/// Port of `testLongPoint`.
fn test_long_point() -> Result<()> {
    let mut field = LongPoint::new("foo", &[5])?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    FieldValueSetters::set_long_value(&mut field, 6)?; // ok
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals(&6, &int_value(field.numeric_value()));
    assert_equals("LongPoint <foo:6>", to_string(&field).as_str());
    Ok(())
}

/// Port of `testLongPoint2D`.
fn test_long_point_2d() -> Result<()> {
    let mut field = LongPoint::new("foo", &[5, 4])?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    field.set_long_values(&[6, 7])?; // ok
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    expect_numeric_value_ise(&field);
    assert_equals("LongPoint <foo:6,7>", to_string(&field).as_str());
    Ok(())
}

/// Port of `testSortedBytesDocValuesField`.
fn test_sorted_bytes_doc_values_field() -> Result<()> {
    let mut field = SortedDocValuesField::new("foo", nbr("bar"));
    try_set_byte_value(&mut field);
    FieldValueSetters::set_bytes_value(&mut field, utf8("fubar"))?;
    FieldValueSetters::set_bytes_value(&mut field, nbr("baz"))?;
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals(&Some(nbr("baz")), &field.binary_value());
    Ok(())
}

/// Port of `testBinaryDocValuesField`.
fn test_binary_doc_values_field() -> Result<()> {
    let mut field = BinaryDocValuesField::new("foo", nbr("bar"));
    try_set_byte_value(&mut field);
    FieldValueSetters::set_bytes_value(&mut field, utf8("fubar"))?;
    FieldValueSetters::set_bytes_value(&mut field, nbr("baz"))?;
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals(&Some(nbr("baz")), &field.binary_value());
    Ok(())
}

/// Port of `testStringField`.
fn test_string_field() -> Result<()> {
    let fields = vec![
        StringField::new("foo", "bar".to_string(), Store::NO)?,
        StringField::new("foo", "bar".to_string(), Store::YES)?,
    ];
    for mut field in fields {
        try_set_byte_value(&mut field);
        try_set_bytes_value(&mut field);
        try_set_bytes_ref_value(&mut field);
        try_set_double_value(&mut field);
        try_set_int_value(&mut field);
        try_set_float_value(&mut field);
        try_set_long_value(&mut field);
        try_set_reader_value(&mut field);
        try_set_short_value(&mut field);
        FieldValueSetters::set_string_value(&mut field, "baz".to_string())?;
        try_set_token_stream_value(&mut field);

        assert_equals(&Some("baz".to_string()), &field.string_value());
        if field.field_type().stored() {
            assert_equals(
                "baz",
                field.stored_value()?.expect("stored").string_value()?,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// Port of `testBinaryStringField`.
fn test_binary_string_field() -> Result<()> {
    let fields = vec![
        StringField::new_with_bytes("foo", BytesRef::from_char_sequence("bar"), Store::NO)?,
        StringField::new_with_bytes("foo", BytesRef::from_char_sequence("bar"), Store::YES)?,
    ];
    for mut field in fields {
        try_set_byte_value(&mut field);
        FieldValueSetters::set_bytes_value(&mut field, utf8("baz"))?;
        assert_equals(
            &Some(BytesRef::from_char_sequence("baz")),
            &field.binary_value(),
        );
        FieldValueSetters::set_bytes_value(&mut field, BytesRef::from_char_sequence("baz"))?;
        try_set_double_value(&mut field);
        try_set_int_value(&mut field);
        try_set_float_value(&mut field);
        try_set_long_value(&mut field);
        try_set_reader_value(&mut field);
        try_set_short_value(&mut field);
        try_set_string_value(&mut field);
        try_set_token_stream_value(&mut field);

        assert_equals(
            &Some(BytesRef::from_char_sequence("baz")),
            &field.binary_value(),
        );
        if field.field_type().stored() {
            assert_equals(
                &BytesRef::from_char_sequence("baz"),
                field.stored_value()?.expect("stored").binary_value()?,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// Port of `testTextFieldString`.
fn test_text_field_string() -> Result<()> {
    let fields = vec![
        TextField::new("foo", "bar".to_string(), Store::NO)?,
        TextField::new("foo", "bar".to_string(), Store::YES)?,
    ];
    for mut field in fields {
        try_set_byte_value(&mut field);
        try_set_bytes_value(&mut field);
        try_set_bytes_ref_value(&mut field);
        try_set_double_value(&mut field);
        try_set_int_value(&mut field);
        try_set_float_value(&mut field);
        try_set_long_value(&mut field);
        try_set_reader_value(&mut field);
        try_set_short_value(&mut field);
        FieldValueSetters::set_string_value(&mut field, "baz".to_string())?;
        try_set_token_stream_value(&mut field);

        assert_equals(&Some("baz".to_string()), &field.string_value());
        if field.field_type().stored() {
            assert_equals(
                "baz",
                field.stored_value()?.expect("stored").string_value()?,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// Port of `testTextFieldReader`.
fn test_text_field_reader() -> Result<()> {
    let mut field = TextField::new_with_reader("foo", Box::new(StringCharReader::new("bar")))?;

    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    FieldValueSetters::set_reader_value(&mut field, Box::new(StringCharReader::new("foobar")))?;
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);

    assert_not_null(&field.reader_value());
    assert_null(&field.stored_value()?);
    Ok(())
}

// TODO (Lucene): testTextFieldTokenStream — "this is pretty expert and crazy,
// see if we can fix it up later"; commented out in Lucene 10.5.0.

/// Port of `testStoredFieldBytes`.
fn test_stored_field_bytes() -> Result<()> {
    let fields = vec![
        StoredField::new_bytes("foo", utf8("bar"))?,
        // `new StoredField("foo", bytes, 0, 3)`: `new BytesRef(bytes, 0, 3)`.
        StoredField::new_bytes(
            "foo",
            BytesRef {
                bytes: "bar".as_bytes().to_vec(),
                offset: 0,
                length: 3,
            },
        )?,
        StoredField::new_bytes("foo", nbr("bar"))?,
    ];
    for mut field in fields {
        try_set_byte_value(&mut field);
        FieldValueSetters::set_bytes_value(&mut field, utf8("baz"))?;
        FieldValueSetters::set_bytes_value(&mut field, nbr("baz"))?;
        try_set_double_value(&mut field);
        try_set_int_value(&mut field);
        try_set_float_value(&mut field);
        try_set_long_value(&mut field);
        try_set_reader_value(&mut field);
        try_set_short_value(&mut field);
        try_set_string_value(&mut field);
        try_set_token_stream_value(&mut field);

        assert_equals(&Some(nbr("baz")), &field.binary_value());
    }
    Ok(())
}

/// Port of `testStoredFieldString`.
fn test_stored_field_string() -> Result<()> {
    let mut field = StoredField::new_string("foo", "bar".to_string())?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    FieldValueSetters::set_string_value(&mut field, "baz".to_string())?;
    try_set_token_stream_value(&mut field);

    assert_equals(&Some("baz".to_string()), &field.string_value());
    Ok(())
}

/// Port of `testStoredFieldInt`.
fn test_stored_field_int() -> Result<()> {
    let mut field = StoredField::new_number("foo", NumericValue::Int(1))?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    FieldValueSetters::set_int_value(&mut field, 5)?;
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals(&5, &int_value(field.numeric_value()));
    Ok(())
}

/// Port of `testStoredFieldDouble`.
fn test_stored_field_double() -> Result<()> {
    let mut field = StoredField::new_number("foo", NumericValue::Double(1.0))?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    FieldValueSetters::set_double_value(&mut field, 5.0)?;
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals_delta(5.0, double_value(field.numeric_value()), 0.0);
    Ok(())
}

/// Port of `testStoredFieldFloat`.
fn test_stored_field_float() -> Result<()> {
    let mut field = StoredField::new_number("foo", NumericValue::Float(1.0))?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    FieldValueSetters::set_float_value(&mut field, 5.0)?;
    try_set_long_value(&mut field);
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals_delta(5.0, float_value(field.numeric_value()) as f64, 0.0);
    Ok(())
}

/// Port of `testStoredFieldLong`.
fn test_stored_field_long() -> Result<()> {
    let mut field = StoredField::new_number("foo", NumericValue::Long(1))?;
    try_set_byte_value(&mut field);
    try_set_bytes_value(&mut field);
    try_set_bytes_ref_value(&mut field);
    try_set_double_value(&mut field);
    try_set_int_value(&mut field);
    try_set_float_value(&mut field);
    FieldValueSetters::set_long_value(&mut field, 5)?;
    try_set_reader_value(&mut field);
    try_set_short_value(&mut field);
    try_set_string_value(&mut field);
    try_set_token_stream_value(&mut field);
    assert_equals(&5i64, &long_value(field.numeric_value()));
    Ok(())
}

/// Port of `testIndexedBinaryField`.
fn test_indexed_binary_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let br = LuceneTestCase::new_bytes_ref_bytes(&[0u8; 5]);
    let field = StringField::new_with_bytes("binary", br.clone(), Store::YES)?;
    assert_equals(&Some(br.clone()), &field.binary_value());
    doc.add(Box::new(field));
    w.add_document(&doc)?;
    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let hits = s.search(Arc::new(TermQuery::new(Term::new("binary", br.clone()))), 1)?;
    assert_equals(&1i64, &hits.total_hits.value());
    let stored_doc = s.stored_fields()?.document(hits.score_docs[0].doc)?;
    assert_equals(
        &Some(br),
        &stored_doc
            .get_field("binary")
            .expect("binary")
            .binary_value(),
    );

    (r as Arc<dyn IndexReader>).close()?;
    w.close()?;
    dir.close()
}

/// Port of `testKnnVectorField`.
fn test_knn_vector_field() -> Result<()> {
    if rucene::codecs::get_default().is_some_and(|c| c.name() == "SimpleText") {
        return Ok(());
    }
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let outcome = (|| -> Result<()> {
        let mut doc = Document::new();
        let b = vec![0u8; 5];
        let field = KnnByteVectorField::new("binary", &b, VectorSimilarityFunction::EUCLIDEAN)?;
        assert_null(&field.binary_value());
        assert_array_equals(&b, &field.vector_value());
        // `(FieldType) field.fieldType()`: the field's own (frozen) type.
        let byte_type = FieldType::new_from(IndexableField::field_type(&field));
        expect_throws(IAE, || {
            KnnFloatVectorField::with_field_type("bogus", &[1.0], byte_type.clone())
        });
        let vector = vec![1f32, 2.0];
        let field2 = KnnFloatVectorField::with_euclidean("float", &vector)?;
        assert_null(&field2.binary_value());
        doc.add(Box::new(field));
        doc.add(Box::new(field2));
        w.add_document(&doc)?;
        let r: Arc<dyn IndexReader> = rucene::index::directory_reader::open_with_writer(&*w)?;
        let inner = (|| -> Result<()> {
            let leaves = Arc::clone(&r).leaves()?;
            let binary = leaves[0]
                .leaf_reader()
                .get_byte_vector_values("binary")?
                .expect("binary");
            assert_equals(&1, &binary.size());
            let mut iterator = binary.iterator()?;
            assert_not_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
            assert_not_null(&Some(binary.vector_value(0)?));
            assert_array_equals(&b, &binary.vector_value(0)?);
            assert_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
            expect_throws("java.io.IOException", || binary.vector_value(1));

            let float_values = leaves[0]
                .leaf_reader()
                .get_float_vector_values("float")?
                .expect("float");
            assert_equals(&1, &float_values.size());
            let mut iterator1 = float_values.iterator()?;
            assert_not_equals(&NO_MORE_DOCS, &iterator1.next_doc()?);
            assert_equals(&vector.len(), &float_values.vector_value(0)?.len());
            assert_equals_delta(
                vector[0] as f64,
                float_values.vector_value(0)?[0] as f64,
                0.0,
            );
            assert_equals(&NO_MORE_DOCS, &iterator1.next_doc()?);
            expect_throws("java.io.IOException", || float_values.vector_value(1));
            Ok(())
        })();
        let closed = r.close();
        inner?;
        closed
    })();
    let closed = w.close();
    let dir_closed = dir.close();
    outcome?;
    closed?;
    dir_closed
}

/// `assertTrue(e.getMessage().contains("zero vector not allowed"))`.
fn assert_zero_error(e: randomizedtesting::jdk::exception::Throwable) {
    Assert::assert_true(
        e.message()
            .unwrap_or_default()
            .contains("zero vector not allowed"),
    );
}

/// Port of `testKnnFieldZeroVectors`.
fn test_knn_field_zero_vectors() -> Result<()> {
    let v = vec![0f32; 5];
    assert_zero_error(expect_throws(IAE, || {
        KnnFloatVectorField::new("knnFloats", &v, VectorSimilarityFunction::COSINE)
    }));
    let field_type = KnnFloatVectorField::create_field_type(5, VectorSimilarityFunction::COSINE)?;
    assert_zero_error(expect_throws(IAE, || {
        KnnFloatVectorField::with_field_type("knnFloats", &v, field_type.clone())
    }));
    let bv = vec![0u8; 5];
    assert_zero_error(expect_throws(IAE, || {
        KnnByteVectorField::new("knnBytes", &bv, VectorSimilarityFunction::COSINE)
    }));
    let byte_field_type =
        KnnByteVectorField::create_field_type(5, VectorSimilarityFunction::COSINE)?;
    assert_zero_error(expect_throws(IAE, || {
        KnnByteVectorField::with_field_type("knnBytes", &bv, byte_field_type.clone())
    }));
    Ok(())
}

/// Port of `testKnnFieldSetVectorValueZeroVectors`.
fn test_knn_field_set_vector_value_zero_vectors() -> Result<()> {
    // Float vector field: setVectorValue with zero vector on COSINE field should fail
    let float_field = KnnFloatVectorField::new(
        "knnFloats",
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        VectorSimilarityFunction::COSINE,
    )?;
    assert_zero_error(expect_throws(IAE, || {
        float_field.set_vector_value(&[0.0; 5])
    }));
    // Non-zero setVectorValue should succeed
    float_field.set_vector_value(&[5.0, 4.0, 3.0, 2.0, 1.0])?;

    // Byte vector field: setVectorValue with zero vector on COSINE field should fail
    let byte_field = KnnByteVectorField::new(
        "knnBytes",
        &[1, 2, 3, 4, 5],
        VectorSimilarityFunction::COSINE,
    )?;
    assert_zero_error(expect_throws(IAE, || byte_field.set_vector_value(&[0; 5])));
    // Non-zero setVectorValue should succeed
    byte_field.set_vector_value(&[5, 4, 3, 2, 1])?;

    // EUCLIDEAN fields should allow zero vectors via setVectorValue
    let euclidean_float_field = KnnFloatVectorField::new(
        "knnFloatsEuc",
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        VectorSimilarityFunction::EUCLIDEAN,
    )?;
    euclidean_float_field.set_vector_value(&[0.0; 5])?;
    let euclidean_byte_field = KnnByteVectorField::new(
        "knnBytesEuc",
        &[1, 2, 3, 4, 5],
        VectorSimilarityFunction::EUCLIDEAN,
    )?;
    euclidean_byte_field.set_vector_value(&[0; 5])?;

    // Float vector field: setVectorValue with NaN should fail
    let nan_field = KnnFloatVectorField::new(
        "knnFloatsNaN",
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        VectorSimilarityFunction::EUCLIDEAN,
    )?;
    let nan_error = expect_throws(IAE, || {
        nan_field.set_vector_value(&[1.0, 2.0, f32::NAN, 4.0, 5.0])
    });
    Assert::assert_true(
        nan_error
            .message()
            .unwrap_or_default()
            .contains("non-finite"),
    );
    Ok(())
}

fn try_set_byte_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || f.set_byte_value(10));
}

fn try_set_bytes_value<F: FieldValueSetters>(f: &mut F) {
    // `f.setBytesValue(new byte[] {5, 5})`: `setBytesValue(new BytesRef(value))`.
    expect_throws(IAE, || f.set_bytes_value(BytesRef::new(vec![5, 5])));
}

fn try_set_bytes_ref_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || f.set_bytes_value(nbr("bogus")));
}

fn try_set_double_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || f.set_double_value(f64::MAX));
}

fn try_set_int_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || f.set_int_value(i32::MAX));
}

fn try_set_long_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || f.set_long_value(i64::MAX));
}

fn try_set_float_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || f.set_float_value(f32::MAX));
}

fn try_set_reader_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || {
        f.set_reader_value(Box::new(StringCharReader::new("BOO!")))
    });
}

fn try_set_short_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || f.set_short_value(i16::MAX));
}

fn try_set_string_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || f.set_string_value("BOO!".to_string()));
}

fn try_set_token_stream_value<F: FieldValueSetters>(f: &mut F) {
    expect_throws(IAE, || {
        f.set_token_stream(Arc::new(AtomicRefCell::new(CannedTokenStream::new(vec![
            Token::with_offsets("foo", 0, 3),
        ]))))
    });
}

/// Port of `testDisabledField`.
fn test_disabled_field() -> Result<()> {
    // neither indexed nor stored
    let ft = FieldType::new();
    expect_throws(IAE, || Field::new("name", String::new(), ft.clone()));
    Ok(())
}

/// Port of `testTokenizedBinaryField`.
fn test_tokenized_binary_field() -> Result<()> {
    let ft = FieldType::new();
    ft.set_tokenized(true)?;
    ft.set_index_options(IndexOptions::DOCS)?;
    expect_throws(IAE, || {
        Field::new_with_bytes("name", BytesRef::default(), ft.clone())
    });
    Ok(())
}

/// Port of `testOffsetsBinaryField`.
fn test_offsets_binary_field() -> Result<()> {
    let ft = FieldType::new();
    ft.set_tokenized(false)?;
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
    expect_throws(IAE, || {
        Field::new_with_bytes("name", BytesRef::default(), ft.clone())
    });
    Ok(())
}

/// Port of `testTermVectorsOffsetsBinaryField`.
fn test_term_vectors_offsets_binary_field() -> Result<()> {
    let ft = FieldType::new();
    ft.set_tokenized(false)?;
    ft.set_store_term_vectors(true)?;
    ft.set_store_term_vector_offsets(true)?;
    expect_throws(IAE, || {
        Field::new_with_bytes("name", BytesRef::default(), ft.clone())
    });
    Ok(())
}
