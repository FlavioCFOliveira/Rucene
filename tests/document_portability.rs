//! Portability tests for the document and analysis modules against the
//! reference behavior of Apache Lucene Core 10.5.0.
//!
//! These tests lock in encoding and formatting decisions that must match the
//! Java implementation for index compatibility.

#![deny(unsafe_code)]

use rucene::analysis::{Analyzer, StandardAnalyzer};
use rucene::document::{
    BinaryPoint, DateTools, Document, DoublePoint, FloatPoint, IntPoint, LongField, LongPoint,
    NumericValue, Resolution, Store, StoredField, StringField, TextField,
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
