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
#[path = "core_index/test_add_indexes.rs"]
mod test_add_indexes;

#[path = "core_index/test_all_files_check_index_header.rs"]
mod test_all_files_check_index_header;
#[path = "core_index/test_all_files_detect_mismatched_checksum.rs"]
mod test_all_files_detect_mismatched_checksum;
#[path = "core_index/test_all_files_detect_truncation.rs"]
mod test_all_files_detect_truncation;
#[path = "core_index/test_all_files_have_checksum_footer.rs"]
mod test_all_files_have_checksum_footer;
#[path = "core_index/test_all_files_have_codec_header.rs"]
mod test_all_files_have_codec_header;
#[path = "core_index/test_approximate_priority_queue.rs"]
mod test_approximate_priority_queue;
#[path = "core_index/test_atomic_update.rs"]
mod test_atomic_update;
#[path = "core_index/test_bag_of_positions.rs"]
mod test_bag_of_positions;
#[path = "core_index/test_bag_of_postings.rs"]
mod test_bag_of_postings;
#[path = "core_index/test_binary_doc_values_updates.rs"]
mod test_binary_doc_values_updates;
#[path = "core_index/test_binary_terms.rs"]
mod test_binary_terms;
#[path = "core_index/test_buffered_updates.rs"]
mod test_buffered_updates;
#[path = "core_index/test_byte_slice_pool.rs"]
mod test_byte_slice_pool;
#[path = "core_index/test_byte_slice_reader.rs"]
mod test_byte_slice_reader;
#[path = "core_index/test_caching_merge_context.rs"]
mod test_caching_merge_context;
#[path = "core_index/test_check_index.rs"]
mod test_check_index;
#[path = "core_index/test_classloading_deadlock.rs"]
mod test_classloading_deadlock;
#[path = "core_index/test_codec_holds_open_files.rs"]
mod test_codec_holds_open_files;
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

#[test]
fn test_add_indexes() {
    run(test_add_indexes::class_def);
}

#[test]
fn test_all_files_check_index_header() {
    run(test_all_files_check_index_header::class_def);
}

#[test]
fn test_all_files_detect_mismatched_checksum() {
    run(test_all_files_detect_mismatched_checksum::class_def);
}

#[test]
fn test_all_files_detect_truncation() {
    run(test_all_files_detect_truncation::class_def);
}

#[test]
fn test_all_files_have_checksum_footer() {
    run(test_all_files_have_checksum_footer::class_def);
}

#[test]
fn test_all_files_have_codec_header() {
    run(test_all_files_have_codec_header::class_def);
}

#[test]
fn test_approximate_priority_queue() {
    run(test_approximate_priority_queue::class_def);
}

#[test]
fn test_atomic_update() {
    run(test_atomic_update::class_def);
}

#[test]
fn test_bag_of_positions() {
    run(test_bag_of_positions::class_def);
}

#[test]
fn test_bag_of_postings() {
    run(test_bag_of_postings::class_def);
}

#[test]
fn test_binary_doc_values_updates() {
    run(test_binary_doc_values_updates::class_def);
}

#[test]
fn test_binary_terms() {
    run(test_binary_terms::class_def);
}

#[test]
fn test_buffered_updates() {
    run(test_buffered_updates::class_def);
}

#[test]
fn test_byte_slice_pool() {
    run(test_byte_slice_pool::class_def);
}

#[test]
fn test_byte_slice_reader() {
    run(test_byte_slice_reader::class_def);
}

#[test]
fn test_caching_merge_context() {
    run(test_caching_merge_context::class_def);
}

#[test]
fn test_codec_holds_open_files() {
    run(test_codec_holds_open_files::class_def);
}

#[test]
fn test_check_index() {
    run(test_check_index::class_def);
}

#[test]
fn test_classloading_deadlock() {
    run(test_classloading_deadlock::class_def);
}

/// The process `TestClassloadingDeadlock.testDeadlock` forks: its `main`,
/// run only when the parent passes the arguments.
#[test]
fn test_classloading_deadlock_main() {
    if let Ok(args) = std::env::var(test_classloading_deadlock::CHILD_ARGS) {
        let args: Vec<String> = args.split('\u{1f}').map(str::to_string).collect();
        test_classloading_deadlock::child_main(&args);
    }
}
