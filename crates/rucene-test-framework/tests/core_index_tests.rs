//! Port of the tests of `lucene/core/src/test/org/apache/lucene/index/`
//! (sprint 9, tasks #375 to #386). Each class runs as a suite under
//! `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop, clippy::explicit_counter_loop)]

mod core_support;

#[path = "core_index/support.rs"]
mod support;
#[path = "core_index/test_2b_binary_doc_values.rs"]
mod test_2b_binary_doc_values;
#[path = "core_index/test_2b_docs.rs"]
mod test_2b_docs;
#[path = "core_index/test_2b_numeric_doc_values.rs"]
mod test_2b_numeric_doc_values;
#[path = "core_index/test_2b_points.rs"]
mod test_2b_points;
#[path = "core_index/test_2b_positions.rs"]
mod test_2b_positions;
#[path = "core_index/test_2b_postings.rs"]
mod test_2b_postings;
#[path = "core_index/test_2b_postings_bytes.rs"]
mod test_2b_postings_bytes;
#[path = "core_index/test_2b_sorted_doc_values_fixed_sorted.rs"]
mod test_2b_sorted_doc_values_fixed_sorted;
#[path = "core_index/test_2b_sorted_doc_values_ords.rs"]
mod test_2b_sorted_doc_values_ords;
#[path = "core_index/test_2b_terms.rs"]
mod test_2b_terms;
#[path = "core_index/test_4gb_stored_fields.rs"]
mod test_4gb_stored_fields;

use core_support::run;

#[test]
fn test_2b_binary_doc_values() {
    run(test_2b_binary_doc_values::class_def);
}

#[test]
fn test_2b_docs() {
    run(test_2b_docs::class_def);
}

#[test]
fn test_2b_numeric_doc_values() {
    run(test_2b_numeric_doc_values::class_def);
}

#[test]
fn test_2b_points() {
    run(test_2b_points::class_def);
}

#[test]
fn test_2b_positions() {
    run(test_2b_positions::class_def);
}

#[test]
fn test_2b_postings() {
    run(test_2b_postings::class_def);
}

#[test]
fn test_2b_postings_bytes() {
    run(test_2b_postings_bytes::class_def);
}

#[test]
fn test_2b_sorted_doc_values_fixed_sorted() {
    run(test_2b_sorted_doc_values_fixed_sorted::class_def);
}

#[test]
fn test_2b_sorted_doc_values_ords() {
    run(test_2b_sorted_doc_values_ords::class_def);
}

#[test]
fn test_2b_terms() {
    run(test_2b_terms::class_def);
}

#[test]
fn test_4gb_stored_fields() {
    run(test_4gb_stored_fields::class_def);
}
