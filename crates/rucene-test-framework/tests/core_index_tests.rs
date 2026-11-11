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
#[path = "core_index/test_directory_reader.rs"]
mod test_directory_reader;
#[path = "core_index/test_directory_reader_reopen.rs"]
mod test_directory_reader_reopen;
#[path = "core_index/test_doc.rs"]
mod test_doc;
#[path = "core_index/test_doc_count.rs"]
mod test_doc_count;
#[path = "core_index/test_doc_id_merger.rs"]
mod test_doc_id_merger;
#[path = "core_index/test_doc_inverter_per_field_error_info.rs"]
mod test_doc_inverter_per_field_error_info;
#[path = "core_index/test_doc_values.rs"]
mod test_doc_values;
#[path = "core_index/test_doc_values_field_updates.rs"]
mod test_doc_values_field_updates;
#[path = "core_index/test_doc_values_indexing.rs"]
mod test_doc_values_indexing;
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
#[path = "core_index/test_dueling_codecs.rs"]
mod test_dueling_codecs;
#[path = "core_index/test_dueling_codecs_at_night.rs"]
mod test_dueling_codecs_at_night;
#[path = "core_index/test_exceed_max_term_length.rs"]
mod test_exceed_max_term_length;
#[path = "core_index/test_exitable_directory_reader.rs"]
mod test_exitable_directory_reader;
#[path = "core_index/test_field_infos.rs"]
mod test_field_infos;
#[path = "core_index/test_field_invert_state.rs"]
mod test_field_invert_state;
#[path = "core_index/test_field_updates_buffer.rs"]
mod test_field_updates_buffer;
#[path = "core_index/test_fields_reader.rs"]
mod test_fields_reader;
#[path = "core_index/test_filter_codec_reader.rs"]
mod test_filter_codec_reader;
#[path = "core_index/test_filter_directory_reader.rs"]
mod test_filter_directory_reader;
#[path = "core_index/test_filter_index_input.rs"]
mod test_filter_index_input;
#[path = "core_index/test_filter_leaf_reader.rs"]
mod test_filter_leaf_reader;
#[path = "core_index/test_filter_merge_policy.rs"]
mod test_filter_merge_policy;
#[path = "core_index/test_flex.rs"]
mod test_flex;
#[path = "core_index/test_flush_by_ram_or_counts_policy.rs"]
mod test_flush_by_ram_or_counts_policy;
#[path = "core_index/test_for_too_much_cloning.rs"]
mod test_for_too_much_cloning;
#[path = "core_index/test_force_merge_forever.rs"]
mod test_force_merge_forever;
#[path = "core_index/test_frozen_buffered_updates.rs"]
mod test_frozen_buffered_updates;
#[path = "core_index/test_index_commit.rs"]
mod test_index_commit;
#[path = "core_index/test_index_file_deleter.rs"]
mod test_index_file_deleter;
#[path = "core_index/test_index_input.rs"]
mod test_index_input;
#[path = "core_index/test_index_many_documents.rs"]
mod test_index_many_documents;
#[path = "core_index/test_index_options.rs"]
mod test_index_options;
#[path = "core_index/test_index_reader_close.rs"]
mod test_index_reader_close;
#[path = "core_index/test_index_sorting.rs"]
mod test_index_sorting;
#[path = "core_index/test_index_too_many_docs.rs"]
mod test_index_too_many_docs;
#[path = "core_index/test_index_writer.rs"]
mod test_index_writer;
#[path = "core_index/test_index_writer_commit.rs"]
mod test_index_writer_commit;
#[path = "core_index/test_index_writer_config.rs"]
mod test_index_writer_config;
#[path = "core_index/test_index_writer_delete.rs"]
mod test_index_writer_delete;
#[path = "core_index/test_index_writer_exceptions.rs"]
mod test_index_writer_exceptions;
#[path = "core_index/test_index_writer_exceptions2.rs"]
mod test_index_writer_exceptions2;
#[path = "core_index/test_index_writer_force_merge.rs"]
mod test_index_writer_force_merge;
#[path = "core_index/test_index_writer_from_reader.rs"]
mod test_index_writer_from_reader;
#[path = "core_index/test_index_writer_lock_release.rs"]
mod test_index_writer_lock_release;
#[path = "core_index/test_index_writer_max_docs.rs"]
mod test_index_writer_max_docs;
#[path = "core_index/test_index_writer_merge_policy.rs"]
mod test_index_writer_merge_policy;
#[path = "core_index/test_index_writer_merging.rs"]
mod test_index_writer_merging;
#[path = "core_index/test_index_writer_nrt_is_current.rs"]
mod test_index_writer_nrt_is_current;
#[path = "core_index/test_index_writer_on_disk_full.rs"]
mod test_index_writer_on_disk_full;
#[path = "core_index/test_index_writer_on_error.rs"]
mod test_index_writer_on_error;
#[path = "core_index/test_index_writer_out_of_file_descriptors.rs"]
mod test_index_writer_out_of_file_descriptors;
#[path = "core_index/test_index_writer_reader.rs"]
mod test_index_writer_reader;
#[path = "core_index/test_index_writer_threads_to_segments.rs"]
mod test_index_writer_threads_to_segments;
#[path = "core_index/test_index_writer_unicode.rs"]
mod test_index_writer_unicode;
#[path = "core_index/test_index_writer_with_threads.rs"]
mod test_index_writer_with_threads;
#[path = "core_index/test_indexable_field.rs"]
mod test_indexable_field;
#[path = "core_index/test_indexing_sequence_numbers.rs"]
mod test_indexing_sequence_numbers;
#[path = "core_index/test_info_stream.rs"]
mod test_info_stream;
#[path = "core_index/test_int_block_pool.rs"]
mod test_int_block_pool;
#[path = "core_index/test_is_current.rs"]
mod test_is_current;
#[path = "core_index/test_knn_graph.rs"]
mod test_knn_graph;
#[path = "core_index/test_lockable_concurrent_approximate_priority_queue.rs"]
mod test_lockable_concurrent_approximate_priority_queue;
#[path = "core_index/test_log_merge_policy.rs"]
mod test_log_merge_policy;
#[path = "core_index/test_long_postings.rs"]
mod test_long_postings;
#[path = "core_index/test_many_fields.rs"]
mod test_many_fields;
#[path = "core_index/test_max_position.rs"]
mod test_max_position;
#[path = "core_index/test_max_term_frequency.rs"]
mod test_max_term_frequency;
#[path = "core_index/test_merge_policy.rs"]
mod test_merge_policy;
#[path = "core_index/test_merge_rate_limiter.rs"]
mod test_merge_rate_limiter;
#[path = "core_index/test_mixed_codecs.rs"]
mod test_mixed_codecs;
#[path = "core_index/test_mixed_doc_values_updates.rs"]
mod test_mixed_doc_values_updates;
#[path = "core_index/test_multi_doc_values.rs"]
mod test_multi_doc_values;
#[path = "core_index/test_multi_fields.rs"]
mod test_multi_fields;
#[path = "core_index/test_multi_index_merge_scheduler.rs"]
mod test_multi_index_merge_scheduler;
#[path = "core_index/test_multi_level_skip_list.rs"]
mod test_multi_level_skip_list;
#[path = "core_index/test_multi_terms_enum.rs"]
mod test_multi_terms_enum;
#[path = "core_index/test_never_delete.rs"]
mod test_never_delete;
#[path = "core_index/test_newest_segment.rs"]
mod test_newest_segment;
#[path = "core_index/test_no_deletion_policy.rs"]
mod test_no_deletion_policy;
#[path = "core_index/test_no_merge_policy.rs"]
mod test_no_merge_policy;
#[path = "core_index/test_no_merge_scheduler.rs"]
mod test_no_merge_scheduler;
#[path = "core_index/test_norms.rs"]
mod test_norms;
#[path = "core_index/test_nrt_reader_cleanup.rs"]
mod test_nrt_reader_cleanup;
#[path = "core_index/test_nrt_reader_with_threads.rs"]
mod test_nrt_reader_with_threads;
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
fn test_directory_reader() {
    run(test_directory_reader::class_def);
}

#[test]
fn test_directory_reader_reopen() {
    run(test_directory_reader_reopen::class_def);
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

#[test]
fn test_doc_values() {
    run(test_doc_values::class_def);
}

#[test]
fn test_doc_values_indexing() {
    run(test_doc_values_indexing::class_def);
}

#[test]
fn test_dueling_codecs() {
    run(test_dueling_codecs::class_def);
}

#[test]
fn test_dueling_codecs_at_night() {
    run(test_dueling_codecs_at_night::class_def);
}

#[test]
fn test_exceed_max_term_length() {
    run(test_exceed_max_term_length::class_def);
}

#[test]
fn test_field_invert_state() {
    run(test_field_invert_state::class_def);
}

#[test]
fn test_exitable_directory_reader() {
    run(test_exitable_directory_reader::class_def);
}

#[test]
fn test_field_infos() {
    run(test_field_infos::class_def);
}

#[test]
fn test_fields_reader() {
    run(test_fields_reader::class_def);
}

#[test]
fn test_field_updates_buffer() {
    run(test_field_updates_buffer::class_def);
}

#[test]
fn test_index_input() {
    run(test_index_input::class_def);
}

#[test]
fn test_filter_index_input() {
    run(test_filter_index_input::class_def);
}

#[test]
fn test_filter_merge_policy() {
    run(test_filter_merge_policy::class_def);
}

#[test]
fn test_filter_codec_reader() {
    run(test_filter_codec_reader::class_def);
}

#[test]
fn test_filter_directory_reader() {
    run(test_filter_directory_reader::class_def);
}

#[test]
fn test_filter_leaf_reader() {
    run(test_filter_leaf_reader::class_def);
}

#[test]
fn test_flex() {
    run(test_flex::class_def);
}

#[test]
fn test_force_merge_forever() {
    run(test_force_merge_forever::class_def);
}

#[test]
fn test_index_many_documents() {
    run(test_index_many_documents::class_def);
}

#[test]
fn test_for_too_much_cloning() {
    run(test_for_too_much_cloning::class_def);
}

#[test]
fn test_frozen_buffered_updates() {
    run(test_frozen_buffered_updates::class_def);
}

#[test]
fn test_index_commit() {
    run(test_index_commit::class_def);
}

#[test]
fn test_index_reader_close() {
    run(test_index_reader_close::class_def);
}

#[test]
fn test_index_options() {
    run(test_index_options::class_def);
}

#[test]
fn test_flush_by_ram_or_counts_policy() {
    run(test_flush_by_ram_or_counts_policy::class_def);
}

#[test]
fn test_index_writer_with_threads() {
    run(test_index_writer_with_threads::class_def);
}

#[test]
fn test_indexable_field() {
    run(test_indexable_field::class_def);
}

#[test]
fn test_index_file_deleter() {
    run(test_index_file_deleter::class_def);
}

#[test]
fn test_indexing_sequence_numbers() {
    run(test_indexing_sequence_numbers::class_def);
}

#[test]
fn test_index_too_many_docs() {
    run(test_index_too_many_docs::class_def);
}

#[test]
fn test_index_writer_config() {
    run(test_index_writer_config::class_def);
}

#[test]
fn test_index_writer_delete() {
    run(test_index_writer_delete::class_def);
}

#[test]
fn test_index_writer_exceptions2() {
    run(test_index_writer_exceptions2::class_def);
}

#[test]
fn test_index_sorting() {
    run(test_index_sorting::class_def);
}

#[test]
fn test_index_writer_commit() {
    run(test_index_writer_commit::class_def);
}

#[test]
fn test_index_writer_exceptions() {
    run(test_index_writer_exceptions::class_def);
}

#[test]
fn test_index_writer_force_merge() {
    run(test_index_writer_force_merge::class_def);
}

#[test]
fn test_index_writer() {
    run(test_index_writer::class_def);
}

#[test]
fn test_index_writer_lock_release() {
    run(test_index_writer_lock_release::class_def);
}
#[test]
fn test_index_writer_max_docs() {
    run(test_index_writer_max_docs::class_def);
}

#[test]
fn test_index_writer_merge_policy() {
    run(test_index_writer_merge_policy::class_def);
}

#[test]
fn test_index_writer_merging() {
    run(test_index_writer_merging::class_def);
}

#[test]
fn test_index_writer_nrt_is_current() {
    run(test_index_writer_nrt_is_current::class_def);
}

#[test]
fn test_index_writer_from_reader() {
    run(test_index_writer_from_reader::class_def);
}

#[test]
fn test_index_writer_on_disk_full() {
    run(test_index_writer_on_disk_full::class_def);
}

#[test]
fn test_index_writer_on_error() {
    run(test_index_writer_on_error::class_def);
}

#[test]
fn test_index_writer_out_of_file_descriptors() {
    run(test_index_writer_out_of_file_descriptors::class_def);
}

#[test]
fn test_index_writer_reader() {
    run(test_index_writer_reader::class_def);
}

#[test]
fn test_index_writer_threads_to_segments() {
    run(test_index_writer_threads_to_segments::class_def);
}

#[test]
fn test_index_writer_unicode() {
    run(test_index_writer_unicode::class_def);
}

#[test]
fn test_info_stream() {
    run(test_info_stream::class_def);
}

#[test]
fn test_int_block_pool() {
    run(test_int_block_pool::class_def);
}

#[test]
fn test_is_current() {
    run(test_is_current::class_def);
}

#[test]
fn test_knn_graph() {
    run(test_knn_graph::class_def);
}

#[test]
fn test_lockable_concurrent_approximate_priority_queue() {
    run(test_lockable_concurrent_approximate_priority_queue::class_def);
}

#[test]
fn test_log_merge_policy() {
    run(test_log_merge_policy::class_def);
}

#[test]
fn test_long_postings() {
    run(test_long_postings::class_def);
}

#[test]
fn test_many_fields() {
    run(test_many_fields::class_def);
}

#[test]
fn test_max_position() {
    run(test_max_position::class_def);
}

#[test]
fn test_max_term_frequency() {
    run(test_max_term_frequency::class_def);
}

#[test]
fn test_merge_policy() {
    run(test_merge_policy::class_def);
}

#[test]
fn test_merge_rate_limiter() {
    run(test_merge_rate_limiter::class_def);
}

#[test]
fn test_mixed_codecs() {
    run(test_mixed_codecs::class_def);
}

#[test]
fn test_mixed_doc_values_updates() {
    run(test_mixed_doc_values_updates::class_def);
}

#[test]
fn test_multi_doc_values() {
    run(test_multi_doc_values::class_def);
}

#[test]
fn test_multi_fields() {
    run(test_multi_fields::class_def);
}

#[test]
fn test_multi_index_merge_scheduler() {
    run(test_multi_index_merge_scheduler::class_def);
}

#[test]
fn test_multi_level_skip_list() {
    run(test_multi_level_skip_list::class_def);
}

#[test]
fn test_multi_terms_enum() {
    run(test_multi_terms_enum::class_def);
}

#[test]
fn test_never_delete() {
    run(test_never_delete::class_def);
}

#[test]
fn test_newest_segment() {
    run(test_newest_segment::class_def);
}

#[test]
fn test_no_deletion_policy() {
    run(test_no_deletion_policy::class_def);
}

#[test]
fn test_no_merge_policy() {
    run(test_no_merge_policy::class_def);
}

#[test]
fn test_no_merge_scheduler() {
    run(test_no_merge_scheduler::class_def);
}

#[test]
fn test_norms() {
    run(test_norms::class_def);
}

#[test]
fn test_nrt_reader_cleanup() {
    run(test_nrt_reader_cleanup::class_def);
}

#[test]
fn test_nrt_reader_with_threads() {
    run(test_nrt_reader_with_threads::class_def);
}

// Lane B (tasks #386, #385, #384).
#[path = "core_index/test_transactions.rs"]
mod test_transactions;
#[test]
fn test_transactions() {
    run(test_transactions::class_def);
}
#[path = "core_index/test_try_delete.rs"]
mod test_try_delete;
#[test]
fn test_try_delete() {
    run(test_try_delete::class_def);
}
#[path = "core_index/test_two_phase_commit_tool.rs"]
mod test_two_phase_commit_tool;
#[test]
fn test_two_phase_commit_tool() {
    run(test_two_phase_commit_tool::class_def);
}
#[path = "core_index/test_unique_term_count.rs"]
mod test_unique_term_count;
#[test]
fn test_unique_term_count() {
    run(test_unique_term_count::class_def);
}
#[path = "core_index/test_sorting_stored_fields_consumer.rs"]
mod test_sorting_stored_fields_consumer;
#[test]
fn test_sorting_stored_fields_consumer() {
    run(test_sorting_stored_fields_consumer::class_def);
}
#[path = "core_index/test_stored_fields_consumer.rs"]
mod test_stored_fields_consumer;
#[test]
fn test_stored_fields_consumer() {
    run(test_stored_fields_consumer::class_def);
}
#[path = "core_index/test_stress_advance.rs"]
mod test_stress_advance;
#[test]
fn test_stress_advance() {
    run(test_stress_advance::class_def);
}
#[path = "core_index/test_stress_deletes.rs"]
mod test_stress_deletes;
#[test]
fn test_stress_deletes() {
    run(test_stress_deletes::class_def);
}
#[path = "core_index/test_stress_indexing2.rs"]
mod test_stress_indexing2;
#[test]
fn test_stress_indexing2() {
    run(test_stress_indexing2::class_def);
}
#[path = "core_index/test_stress_indexing.rs"]
mod test_stress_indexing;
#[test]
fn test_stress_indexing() {
    run(test_stress_indexing::class_def);
}
#[path = "core_index/test_stress_nrt.rs"]
mod test_stress_nrt;
#[test]
fn test_stress_nrt() {
    run(test_stress_nrt::class_def);
}
#[path = "core_index/test_sum_doc_freq.rs"]
mod test_sum_doc_freq;
#[test]
fn test_sum_doc_freq() {
    run(test_sum_doc_freq::class_def);
}
#[path = "core_index/test_swapped_index_files.rs"]
mod test_swapped_index_files;
#[test]
fn test_swapped_index_files() {
    run(test_swapped_index_files::class_def);
}
#[path = "core_index/test_temporal_merge_policy_across_buckets.rs"]
mod test_temporal_merge_policy_across_buckets;
#[test]
fn test_temporal_merge_policy_across_buckets() {
    run(test_temporal_merge_policy_across_buckets::class_def);
}
#[path = "core_index/test_temporal_merge_policy.rs"]
mod test_temporal_merge_policy;
#[test]
fn test_temporal_merge_policy() {
    run(test_temporal_merge_policy::class_def);
}
#[path = "core_index/test_termdoc_perf.rs"]
mod test_termdoc_perf;
#[test]
fn test_termdoc_perf() {
    run(test_termdoc_perf::class_def);
}
#[path = "core_index/test_term.rs"]
mod test_term;
#[test]
fn test_term() {
    run(test_term::class_def);
}
#[path = "core_index/test_terms_enum2.rs"]
mod test_terms_enum2;
#[test]
fn test_terms_enum2() {
    run(test_terms_enum2::class_def);
}
#[path = "core_index/test_terms_enum_index.rs"]
mod test_terms_enum_index;
#[test]
fn test_terms_enum_index() {
    run(test_terms_enum_index::class_def);
}
#[path = "core_index/test_terms.rs"]
mod test_terms;
#[test]
fn test_terms() {
    run(test_terms::class_def);
}
#[path = "core_index/test_term_states.rs"]
mod test_term_states;
#[test]
fn test_term_states() {
    run(test_term_states::class_def);
}
#[path = "core_index/test_terms_enum.rs"]
mod test_terms_enum;
#[test]
fn test_terms_enum() {
    run(test_terms_enum::class_def);
}
#[path = "core_index/test_terms_hash_per_field.rs"]
mod test_terms_hash_per_field;
#[test]
fn test_terms_hash_per_field() {
    run(test_terms_hash_per_field::class_def);
}
#[path = "core_index/test_term_vectors.rs"]
mod test_term_vectors;
#[test]
fn test_term_vectors() {
    run(test_term_vectors::class_def);
}
#[path = "core_index/test_term_vectors_reader.rs"]
mod test_term_vectors_reader;
#[test]
fn test_term_vectors_reader() {
    run(test_term_vectors_reader::class_def);
}
#[path = "core_index/test_term_vectors_writer.rs"]
mod test_term_vectors_writer;
#[test]
fn test_term_vectors_writer() {
    run(test_term_vectors_writer::class_def);
}
#[path = "core_index/test_threaded_force_merge.rs"]
mod test_threaded_force_merge;
#[test]
fn test_threaded_force_merge() {
    run(test_threaded_force_merge::class_def);
}
#[path = "core_index/test_tragic_index_writer_deadlock.rs"]
mod test_tragic_index_writer_deadlock;
#[test]
fn test_tragic_index_writer_deadlock() {
    run(test_tragic_index_writer_deadlock::class_def);
}
#[path = "core_index/test_transaction_rollback.rs"]
mod test_transaction_rollback;
#[test]
fn test_transaction_rollback() {
    run(test_transaction_rollback::class_def);
}
#[path = "core_index/test_pending_deletes.rs"]
mod test_pending_deletes;
#[test]
fn test_pending_deletes() {
    run(test_pending_deletes::class_def);
}
#[path = "core_index/test_pending_soft_deletes.rs"]
mod test_pending_soft_deletes;
#[test]
fn test_pending_soft_deletes() {
    run(test_pending_soft_deletes::class_def);
}
#[path = "core_index/test_per_segment_deletes.rs"]
mod test_per_segment_deletes;
#[test]
fn test_per_segment_deletes() {
    run(test_per_segment_deletes::class_def);
}
#[path = "core_index/test_numeric_doc_values_updates.rs"]
mod test_numeric_doc_values_updates;
#[test]
fn test_numeric_doc_values_updates() {
    run(test_numeric_doc_values_updates::class_def);
}
#[path = "core_index/test_omit_norms.rs"]
mod test_omit_norms;
#[test]
fn test_omit_norms() {
    run(test_omit_norms::class_def);
}
#[path = "core_index/test_omit_positions.rs"]
mod test_omit_positions;
#[test]
fn test_omit_positions() {
    run(test_omit_positions::class_def);
}
#[path = "core_index/test_omit_tf.rs"]
mod test_omit_tf;
#[test]
fn test_omit_tf() {
    run(test_omit_tf::class_def);
}
#[path = "core_index/test_one_merge_wrapping_merge_policy.rs"]
mod test_one_merge_wrapping_merge_policy;
#[test]
fn test_one_merge_wrapping_merge_policy() {
    run(test_one_merge_wrapping_merge_policy::class_def);
}
#[path = "core_index/test_ordinal_map.rs"]
mod test_ordinal_map;
#[test]
fn test_ordinal_map() {
    run(test_ordinal_map::class_def);
}
#[path = "core_index/test_parallel_composite_reader.rs"]
mod test_parallel_composite_reader;
#[test]
fn test_parallel_composite_reader() {
    run(test_parallel_composite_reader::class_def);
}
#[path = "core_index/test_parallel_leaf_reader.rs"]
mod test_parallel_leaf_reader;
#[test]
fn test_parallel_leaf_reader() {
    run(test_parallel_leaf_reader::class_def);
}
#[path = "core_index/test_parallel_reader_empty_index.rs"]
mod test_parallel_reader_empty_index;
#[test]
fn test_parallel_reader_empty_index() {
    run(test_parallel_reader_empty_index::class_def);
}
#[path = "core_index/test_parallel_term_enum.rs"]
mod test_parallel_term_enum;
#[test]
fn test_parallel_term_enum() {
    run(test_parallel_term_enum::class_def);
}
#[path = "core_index/test_payloads.rs"]
mod test_payloads;
#[test]
fn test_payloads() {
    run(test_payloads::class_def);
}
#[path = "core_index/test_payloads_on_vectors.rs"]
mod test_payloads_on_vectors;
#[test]
fn test_payloads_on_vectors() {
    run(test_payloads_on_vectors::class_def);
}
#[path = "core_index/test_index_writer_on_jre_crash.rs"]
mod test_index_writer_on_jre_crash;
#[path = "threaded_nrt_threads/mod.rs"]
mod threaded_nrt_threads;
#[test]
fn test_index_writer_on_jre_crash() {
    run(test_index_writer_on_jre_crash::class_def);
}
