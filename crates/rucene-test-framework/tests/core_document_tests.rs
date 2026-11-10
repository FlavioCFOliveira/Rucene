//! Port of the tests of `lucene/core/src/test/org/apache/lucene/document/`
//! (sprint 9, tasks #372 and #373). Each class runs as a suite under
//! `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop, clippy::explicit_counter_loop)]

mod core_support;
#[path = "core_document/test_binary_document.rs"]
mod test_binary_document;
#[path = "core_document/test_date_tools.rs"]
mod test_date_tools;
#[path = "core_document/test_doc_values_long_hash_set.rs"]
mod test_doc_values_long_hash_set;
#[path = "core_document/test_double_range.rs"]
mod test_double_range;
#[path = "core_document/test_field_type.rs"]
mod test_field_type;
#[path = "core_document/test_float_range.rs"]
mod test_float_range;
#[path = "core_document/test_int_range.rs"]
mod test_int_range;
#[path = "core_document/test_keyword_field.rs"]
mod test_keyword_field;
#[path = "core_document/test_lat_lon_doc_values_field.rs"]
mod test_lat_lon_doc_values_field;
#[path = "core_document/test_lat_lon_point.rs"]
mod test_lat_lon_point;
#[path = "core_document/test_long_range.rs"]
mod test_long_range;

use core_support::run;

#[test]
fn test_binary_document() {
    run(test_binary_document::class_def);
}

#[test]
fn test_date_tools() {
    run(test_date_tools::class_def);
}

#[test]
fn test_doc_values_long_hash_set() {
    run(test_doc_values_long_hash_set::class_def);
}

#[test]
fn test_double_range() {
    run(test_double_range::class_def);
}

#[test]
fn test_field_type() {
    run(test_field_type::class_def);
}

#[test]
fn test_float_range() {
    run(test_float_range::class_def);
}

#[test]
fn test_int_range() {
    run(test_int_range::class_def);
}

#[test]
fn test_keyword_field() {
    run(test_keyword_field::class_def);
}

#[test]
fn test_lat_lon_doc_values_field() {
    run(test_lat_lon_doc_values_field::class_def);
}

#[test]
fn test_lat_lon_point() {
    run(test_lat_lon_point::class_def);
}

#[test]
fn test_long_range() {
    run(test_long_range::class_def);
}
