//! Port of the tests of `lucene/core/src/test/org/apache/lucene/index/`
//! (sprint 9, tasks #375 to #386). Each class runs as a suite under
//! `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop, clippy::explicit_counter_loop)]

mod core_support;

#[path = "core_document/column/column_batch_test_util.rs"]
mod column_batch_test_util;
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
#[path = "core_index/test_codecs.rs"]
mod test_codecs;
#[path = "core_index/test_column_batch_indexing.rs"]
mod test_column_batch_indexing;
#[path = "core_index/test_concurrent_approximate_priority_queue.rs"]
mod test_concurrent_approximate_priority_queue;
#[path = "core_index/test_concurrent_merge_scheduler.rs"]
mod test_concurrent_merge_scheduler;
#[path = "core_index/test_consistent_field_numbers.rs"]
mod test_consistent_field_numbers;
#[path = "core_index/test_crash.rs"]
mod test_crash;
#[path = "core_index/test_crash_causes_corrupt_index.rs"]
mod test_crash_causes_corrupt_index;
#[path = "core_index/test_custom_norms.rs"]
mod test_custom_norms;
#[path = "core_index/test_custom_term_freq.rs"]
mod test_custom_term_freq;
#[path = "core_index/test_default_codec_parallelizes_io.rs"]
mod test_default_codec_parallelizes_io;
#[path = "core_index/test_deletion_policy.rs"]
mod test_deletion_policy;
#[path = "core_index/test_demo_parallel_leaf_reader.rs"]
mod test_demo_parallel_leaf_reader;
#[path = "core_index/test_doc.rs"]
mod test_doc;
#[path = "core_index/test_doc_count.rs"]
mod test_doc_count;
#[path = "core_index/test_doc_id_merger.rs"]
mod test_doc_id_merger;
#[path = "core_index/test_doc_inverter_per_field_error_info.rs"]
mod test_doc_inverter_per_field_error_info;
#[path = "core_index/test_doc_values_field_updates.rs"]
mod test_doc_values_field_updates;
#[path = "core_index/test_docs_and_positions.rs"]
mod test_docs_and_positions;
#[path = "core_index/test_docs_with_field_set.rs"]
mod test_docs_with_field_set;
#[path = "core_index/test_document_writer.rs"]
mod test_document_writer;
#[path = "core_index/test_documents_writer_delete_queue.rs"]
mod test_documents_writer_delete_queue;
#[path = "core_index/test_documents_writer_per_thread_pool.rs"]
mod test_documents_writer_per_thread_pool;
#[path = "core_index/test_documents_writer_stall_control.rs"]
mod test_documents_writer_stall_control;
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

#[test]
fn test_codecs() {
    run(test_codecs::class_def);
}

#[test]
fn test_column_batch_indexing() {
    run(test_column_batch_indexing::class_def);
}

#[test]
fn test_concurrent_approximate_priority_queue() {
    run(test_concurrent_approximate_priority_queue::class_def);
}

#[test]
fn test_concurrent_merge_scheduler() {
    run(test_concurrent_merge_scheduler::class_def);
}

#[test]
fn test_consistent_field_numbers() {
    run(test_consistent_field_numbers::class_def);
}

#[test]
fn test_crash_causes_corrupt_index() {
    run(test_crash_causes_corrupt_index::class_def);
}

#[test]
fn test_crash() {
    run(test_crash::class_def);
}

#[test]
fn test_custom_norms() {
    run(test_custom_norms::class_def);
}

#[test]
fn test_default_codec_parallelizes_io() {
    run(test_default_codec_parallelizes_io::class_def);
}

#[test]
fn test_custom_term_freq() {
    run(test_custom_term_freq::class_def);
}

#[test]
fn test_deletion_policy() {
    run(test_deletion_policy::class_def);
}

#[test]
fn test_demo_parallel_leaf_reader() {
    run(test_demo_parallel_leaf_reader::class_def);
}

#[test]
fn test_doc_count() {
    run(test_doc_count::class_def);
}

#[test]
fn test_doc_id_merger() {
    run(test_doc_id_merger::class_def);
}

#[test]
fn test_doc_inverter_per_field_error_info() {
    run(test_doc_inverter_per_field_error_info::class_def);
}

#[test]
fn test_doc() {
    run(test_doc::class_def);
}

#[test]
fn test_docs_and_positions() {
    run(test_docs_and_positions::class_def);
}

#[test]
fn test_docs_with_field_set() {
    run(test_docs_with_field_set::class_def);
}

#[test]
fn test_documents_writer_delete_queue() {
    run(test_documents_writer_delete_queue::class_def);
}

#[test]
fn test_documents_writer_per_thread_pool() {
    run(test_documents_writer_per_thread_pool::class_def);
}

#[test]
fn test_documents_writer_stall_control() {
    run(test_documents_writer_stall_control::class_def);
}

#[test]
fn test_document_writer() {
    run(test_document_writer::class_def);
}

#[test]
fn test_doc_values_field_updates() {
    run(test_doc_values_field_updates::class_def);
}
