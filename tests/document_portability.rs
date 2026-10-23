//! Portability tests for the document and analysis modules against the
//! reference behavior of Apache Lucene Core 10.5.0.
//!
//! These tests lock in encoding and formatting decisions that must match the
//! Java implementation for index compatibility.

#![deny(unsafe_code)]

use rucene::analysis::{Analyzer, StandardAnalyzer};
use rucene::document::{
    BinaryPoint, DateTools, Document, DoubleDocValuesField, DoubleField, DoublePoint, Field,
    FieldType, FloatDocValuesField, FloatField, FloatPoint, IntField, IntPoint, LongField,
    LongPoint, NumericValue, Resolution, Store, StoredField, StoredValue, StringField, TextField,
};
use rucene::index::{DocValuesType, IndexOptions, IndexableField};
use rucene::util::BytesRef;

fn hex(bytes: &BytesRef) -> String {
    bytes.slice().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn standard_analyzer_tokens_match_java() {
    let analyzer = StandardAnalyzer::new();
    let stream = analyzer
        .token_stream_from_str("text", "Hello, World! 123.45")
        .unwrap();
    let mut stream = stream.borrow_mut();
    stream.reset().unwrap();
    let mut tokens = Vec::new();
    while stream.increment_token().unwrap() {
        let term = stream
            .attribute_source()
            .get_attribute::<rucene::analysis::PackedTokenAttributeImpl>()
            .unwrap()
            .term();
        tokens.push(term);
    }
    stream.end().unwrap();
    assert_eq!(tokens, vec!["hello", "world", "123", "45"]);
}

#[test]
fn int_point_encoding_matches_java() {
    let field = IntPoint::new("count", &[42]).unwrap();
    assert_eq!(hex(&field.binary_value().unwrap()), "8000002a");
}

#[test]
fn long_point_encoding_matches_java() {
    let field = LongPoint::new("ts", &[123456789i64]).unwrap();
    assert_eq!(hex(&field.binary_value().unwrap()), "80000000075bcd15");
}

#[test]
fn float_point_encoding_matches_java() {
    let field = FloatPoint::new("temp", &[2.5f32]).unwrap();
    // NumericUtils.intToSortableBytes flips the high bit so signed order matches
    // unsigned byte order; the expected value is c0200000.
    assert_eq!(hex(&field.binary_value().unwrap()), "c0200000");
}

#[test]
fn double_point_encoding_matches_java() {
    let field = DoublePoint::new("temp", &[2.5f64]).unwrap();
    // NumericUtils.longToSortableBytes flips the high bit.
    assert_eq!(hex(&field.binary_value().unwrap()), "c004000000000000");
}

#[test]
fn binary_point_encoding_matches_java() {
    let dims = vec![
        BytesRef::new(vec![0x01, 0x02]),
        BytesRef::new(vec![0x03, 0x04]),
    ];
    let field = BinaryPoint::new("shape", &dims).unwrap();
    assert_eq!(hex(&field.binary_value().unwrap()), "01020304");
}

#[test]
fn date_tools_strings_match_java() {
    // 2024-09-21 13:50:11.123 GMT
    let time = 1726926611123i64;
    assert_eq!(
        DateTools::time_to_string(time, Resolution::MILLISECOND),
        "20240921135011123"
    );
    assert_eq!(
        DateTools::time_to_string(time, Resolution::SECOND),
        "20240921135011"
    );
    assert_eq!(
        DateTools::time_to_string(time, Resolution::MINUTE),
        "202409211350"
    );
    assert_eq!(
        DateTools::time_to_string(time, Resolution::HOUR),
        "2024092113"
    );
    assert_eq!(DateTools::time_to_string(time, Resolution::DAY), "20240921");
    assert_eq!(DateTools::time_to_string(time, Resolution::MONTH), "202409");
    assert_eq!(DateTools::time_to_string(time, Resolution::YEAR), "2024");
}

#[test]
fn document_with_combined_fields_matches_java_types() {
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", "abc".to_string(), Store::YES).unwrap(),
    ));
    doc.add(Box::new(
        TextField::new("body", "Hello World".to_string(), Store::NO).unwrap(),
    ));
    doc.add(Box::new(IntPoint::new("count", &[42]).unwrap()));
    doc.add(Box::new(
        StoredField::new_number("stored_count", NumericValue::Int(42)).unwrap(),
    ));
    doc.add(Box::new(LongField::new("ts", 123456789i64, Store::YES)));

    let id = doc.get_field("id").unwrap();
    assert!(id.field_type().stored());
    assert!(!id.field_type().tokenized());
    assert_eq!(id.field_type().index_options(), IndexOptions::DOCS);

    let body = doc.get_field("body").unwrap();
    assert!(body.field_type().tokenized());
    assert!(!body.field_type().stored());
    assert_eq!(
        body.field_type().index_options(),
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
    );

    let count = doc.get_field("count").unwrap();
    assert_eq!(count.field_type().point_dimension_count(), 1);
    assert_eq!(count.field_type().point_num_bytes(), 4);
    assert_eq!(hex(&count.binary_value().unwrap()), "8000002a");

    let stored_count = doc.get_field("stored_count").unwrap();
    assert!(stored_count.field_type().stored());
    assert_eq!(
        stored_count.field_type().index_options(),
        IndexOptions::NONE
    );
    assert_eq!(stored_count.numeric_value(), Some(NumericValue::Int(42)));

    let ts = doc.get_field("ts").unwrap();
    assert!(ts.field_type().stored());
    assert_eq!(ts.field_type().point_dimension_count(), 1);
    assert_eq!(ts.field_type().point_num_bytes(), 8);
    assert_eq!(
        ts.field_type().doc_values_type(),
        DocValuesType::SORTED_NUMERIC
    );
    assert_eq!(ts.numeric_value(), Some(NumericValue::Long(123456789i64)));
}

// -----------------------------------------------------------------------------
// The static `pack` of each point family
// -----------------------------------------------------------------------------
//
// `IntPoint.pack(int...)` and its three siblings produce exactly the bytes the
// field stores and the BKD tree indexes, so a query that builds its bounds with
// `pack` and an index built through the constructor have to agree byte for
// byte. Every expected value below is the encoding Lucene 10.5.0 documents:
// `NumericUtils.intToSortableBytes` / `longToSortableBytes` flip the sign bit so
// that signed order becomes unsigned byte order, and the float and double
// families go through `floatToSortableInt` / `doubleToSortableLong` first.

#[test]
fn int_point_pack_matches_the_constructor() {
    let packed = IntPoint::pack(&[42]).unwrap();
    assert_eq!(hex(&packed), "8000002a");
    let field = IntPoint::new("count", &[42]).unwrap();
    assert_eq!(hex(&packed), hex(&field.binary_value().unwrap()));
}

#[test]
fn long_point_pack_matches_the_constructor() {
    let packed = LongPoint::pack(&[123456789i64]).unwrap();
    assert_eq!(hex(&packed), "80000000075bcd15");
    let field = LongPoint::new("ts", &[123456789i64]).unwrap();
    assert_eq!(hex(&packed), hex(&field.binary_value().unwrap()));
}

#[test]
fn float_point_pack_matches_the_constructor() {
    let packed = FloatPoint::pack(&[2.5f32]).unwrap();
    assert_eq!(hex(&packed), "c0200000");
    let field = FloatPoint::new("temp", &[2.5f32]).unwrap();
    assert_eq!(hex(&packed), hex(&field.binary_value().unwrap()));
}

#[test]
fn double_point_pack_matches_the_constructor() {
    let packed = DoublePoint::pack(&[2.5f64]).unwrap();
    assert_eq!(hex(&packed), "c004000000000000");
    let field = DoublePoint::new("temp", &[2.5f64]).unwrap();
    assert_eq!(hex(&packed), hex(&field.binary_value().unwrap()));
}

#[test]
fn pack_lays_dimensions_out_end_to_end() {
    // Two dimensions are stored back to back, in order, with no separator: the
    // second dimension starts at `dim * Integer.BYTES`.
    assert_eq!(hex(&IntPoint::pack(&[0, -1]).unwrap()), "800000007fffffff");
    assert_eq!(
        hex(&LongPoint::pack(&[0i64, -1i64]).unwrap()),
        "80000000000000007fffffffffffffff"
    );
}

#[test]
fn pack_rejects_a_point_of_no_dimensions() {
    // Java throws IllegalArgumentException("point must not be 0 dimensions").
    assert!(IntPoint::pack(&[]).is_err());
    assert!(LongPoint::pack(&[]).is_err());
    assert!(FloatPoint::pack(&[]).is_err());
    assert!(DoublePoint::pack(&[]).is_err());
}

// -----------------------------------------------------------------------------
// The single-value setter of each point family
// -----------------------------------------------------------------------------

#[test]
fn point_single_value_setters_rewrite_the_packed_bytes() {
    let mut int_point = IntPoint::new("count", &[7]).unwrap();
    int_point.set_int_value(42).unwrap();
    assert_eq!(hex(&int_point.binary_value().unwrap()), "8000002a");

    let mut long_point = LongPoint::new("ts", &[7i64]).unwrap();
    long_point.set_long_value(123456789i64).unwrap();
    assert_eq!(hex(&long_point.binary_value().unwrap()), "80000000075bcd15");

    let mut float_point = FloatPoint::new("temp", &[7.0f32]).unwrap();
    float_point.set_float_value(2.5f32).unwrap();
    assert_eq!(hex(&float_point.binary_value().unwrap()), "c0200000");

    let mut double_point = DoublePoint::new("temp", &[7.0f64]).unwrap();
    double_point.set_double_value(2.5f64).unwrap();
    assert_eq!(
        hex(&double_point.binary_value().unwrap()),
        "c004000000000000"
    );
}

#[test]
fn point_single_value_setter_refuses_a_multi_dimensional_field() {
    // Java's `setIntValue` delegates to `setIntValues(value)`, whose dimension
    // check throws for a field created with more than one dimension.
    let mut field = IntPoint::new("box", &[1, 2]).unwrap();
    assert!(field.set_int_value(3).is_err());
    assert_eq!(hex(&field.binary_value().unwrap()), "8000000180000002");
}

// -----------------------------------------------------------------------------
// The typed setters of `Field`
// -----------------------------------------------------------------------------
//
// `Field.setIntValue` and its siblings are how a caller reuses one field
// instance across documents, which is the idiom Lucene's own demo uses. They
// decide the value that reaches the indexing chain, and they refuse to change
// the value's *type*: a field built with an `int` stays an `int` field for the
// life of the index, so silently widening it would produce a segment whose
// field infos disagree with the values written into it.

fn numeric_field(value: NumericValue) -> Field {
    Field::new_with_number("n", value, FieldType::new()).unwrap()
}

#[test]
fn field_typed_setters_replace_a_value_of_the_same_type() {
    let mut field = numeric_field(NumericValue::Int(1));
    field.set_int_value(42).unwrap();
    assert_eq!(field.numeric_value(), Some(NumericValue::Int(42)));

    let mut field = numeric_field(NumericValue::Long(1));
    field.set_long_value(123456789).unwrap();
    assert_eq!(field.numeric_value(), Some(NumericValue::Long(123456789)));

    let mut field = numeric_field(NumericValue::Float(1.0));
    field.set_float_value(2.5).unwrap();
    assert_eq!(field.numeric_value(), Some(NumericValue::Float(2.5)));

    let mut field = numeric_field(NumericValue::Double(1.0));
    field.set_double_value(2.5).unwrap();
    assert_eq!(field.numeric_value(), Some(NumericValue::Double(2.5)));
}

#[test]
fn field_typed_setters_refuse_to_change_the_value_type() {
    // Java: "cannot change value type from Integer to Long".
    let mut field = numeric_field(NumericValue::Int(1));
    let error = field.set_long_value(1).unwrap_err().to_string();
    assert!(
        error.contains("cannot change value type from Integer to Long"),
        "unexpected message: {error}"
    );
    // The rejected call left the value alone.
    assert_eq!(field.numeric_value(), Some(NumericValue::Int(1)));

    let mut field = numeric_field(NumericValue::Double(1.0));
    let error = field.set_float_value(1.0).unwrap_err().to_string();
    assert!(
        error.contains("cannot change value type from Double to Float"),
        "unexpected message: {error}"
    );

    let mut field = Field::new("s", "text".to_string(), {
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();
        ft
    })
    .unwrap();
    let error = field.set_int_value(1).unwrap_err().to_string();
    assert!(
        error.contains("cannot change value type from String to Integer"),
        "unexpected message: {error}"
    );
}

#[test]
fn field_byte_and_short_setters_refuse_every_constructible_field() {
    // In Lucene 10.5.0 `Field.setByteValue` and `Field.setShortValue` are the
    // only assignments of a `Byte` or a `Short` to `fieldsData` in the whole of
    // Lucene Core, and each demands one be there already: no constructor and no
    // subclass ever puts one there, so both always throw. They answer the same
    // way here.
    let mut field = numeric_field(NumericValue::Int(1));
    let error = field.set_byte_value(1).unwrap_err().to_string();
    assert!(
        error.contains("cannot change value type from Integer to Byte"),
        "unexpected message: {error}"
    );
    let error = field.set_short_value(1).unwrap_err().to_string();
    assert!(
        error.contains("cannot change value type from Integer to Short"),
        "unexpected message: {error}"
    );
    assert_eq!(field.numeric_value(), Some(NumericValue::Int(1)));
}

// -----------------------------------------------------------------------------
// The numeric doc value each typed field writes
// -----------------------------------------------------------------------------
//
// Every expected number below was measured by running Apache Lucene Core 10.5.0
// itself (lucene-core-10.5.0.jar) and printing `numericValue()` / `storedValue()`
// of the field in question. These are the values that reach the `.dvd`/`.dvm`
// doc-values stream and the stored-fields stream, so a divergence here is a
// divergence in the index on disk, not merely in the API.

#[test]
fn float_doc_values_field_matches_java_raw_bits() {
    // Java stores `Float.floatToRawIntBits(value)`, an `int`, through
    // `Field.setLongValue(long)`. That widening sign-extends (JLS 5.1.2), so
    // every negative float yields a negative long. Reading the bits into an
    // unsigned `u32` and widening that instead would be 2^32 too large.
    for (value, expected) in [
        (-1.0f32, -1_082_130_432i64),
        (-0.5f32, -1_090_519_040i64),
        (-0.0f32, -2_147_483_648i64),
        (2.5f32, 1_075_838_976i64),
    ] {
        let field = FloatDocValuesField::new("score", value);
        assert_eq!(
            field.numeric_value(),
            Some(NumericValue::Long(expected)),
            "FloatDocValuesField({value}) must encode as Lucene does"
        );

        let mut reused = FloatDocValuesField::new("score", 1.0f32);
        reused.set_value(value);
        assert_eq!(
            reused.numeric_value(),
            Some(NumericValue::Long(expected)),
            "the setter must encode exactly as the constructor does"
        );
    }
}

#[test]
fn double_doc_values_field_matches_java_raw_bits() {
    // `Double.doubleToRawLongBits` already yields a `long`, so there is no
    // widening step and the sign is carried by the bit pattern itself.
    for (value, expected) in [
        (-1.0f64, -4_616_189_618_054_758_400i64),
        (2.5f64, 4_612_811_918_334_230_528i64),
    ] {
        let field = DoubleDocValuesField::new("score", value);
        assert_eq!(
            field.numeric_value(),
            Some(NumericValue::Long(expected)),
            "DoubleDocValuesField({value}) must encode as Lucene does"
        );

        let mut reused = DoubleDocValuesField::new("score", 1.0f64);
        reused.set_value(value);
        assert_eq!(
            reused.numeric_value(),
            Some(NumericValue::Long(expected)),
            "the setter must encode exactly as the constructor does"
        );
    }
}

#[test]
fn typed_field_setters_refresh_both_the_indexed_and_the_stored_value() {
    // `IntField.setIntValue`, `FloatField.setFloatValue` and
    // `DoubleField.setDoubleValue` each assign the indexed value and then
    // refresh `storedValue`, so a reused field instance stores what it indexes.
    // The float and double families index the *sortable* encoding while storing
    // the value itself, which is why the two numbers differ.
    let mut int_field = IntField::new("i", 1, Store::YES);
    int_field.set_value(42);
    assert_eq!(int_field.numeric_value(), Some(NumericValue::Int(42)));
    assert_eq!(
        int_field.stored_value().unwrap(),
        Some(StoredValue::Integer(42))
    );

    let mut float_field = FloatField::new("f", 1.0f32, Store::YES);
    float_field.set_value(-2.5f32);
    assert_eq!(
        float_field.numeric_value(),
        Some(NumericValue::Long(-1_075_838_977))
    );
    assert_eq!(
        float_field.stored_value().unwrap(),
        Some(StoredValue::Float(-2.5f32))
    );

    let mut double_field = DoubleField::new("d", 1.0f64, Store::YES);
    double_field.set_value(-2.5f64);
    assert_eq!(
        double_field.numeric_value(),
        Some(NumericValue::Long(-4_612_811_918_334_230_529))
    );
    assert_eq!(
        double_field.stored_value().unwrap(),
        Some(StoredValue::Double(-2.5f64))
    );
}

#[test]
fn range_fields_refuse_more_than_four_dimensions() {
    // Lucene guards this twice: `checkArgs` and the private `getType(int)` that
    // builds the FieldType both reject more than four range dimensions, because
    // a range costs two point dimensions and `FieldType.setDimensions` would
    // otherwise accept up to eight. The message is Lucene's own, measured from
    // `new IntRange("r", new int[5], new int[5])`.
    use rucene::document::range_fields::{DoubleRange, FloatRange, IntRange, LongRange};

    let err = IntRange::new("r", &[0i32; 5], &[1i32; 5]).unwrap_err();
    assert_eq!(
        err.to_string()
            .contains("does not support greater than 4 dimensions"),
        true,
        "unexpected message: {err}"
    );
    assert!(LongRange::new("r", &[0i64; 5], &[1i64; 5]).is_err());
    assert!(FloatRange::new("r", &[0.0f32; 5], &[1.0f32; 5]).is_err());
    assert!(DoubleRange::new("r", &[0.0f64; 5], &[1.0f64; 5]).is_err());

    // Four dimensions is the maximum Lucene accepts, and it yields eight point
    // dimensions.
    assert!(IntRange::new("r", &[0i32; 4], &[1i32; 4]).is_ok());
}
