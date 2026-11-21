//! Port of the test classes of `lucene/core/src/test/org/apache/lucene/codecs/`
//! and its sub-packages (sprint 9, task #371) that have no base-suite
//! binding. Each class runs as a suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit bindings line by
// line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

mod core_codecs {
    pub mod support;

    pub mod abstract_test_compression_mode;
    pub mod base_vectorization_test_case;
    pub mod test_codec_util;
    pub mod test_competitive_freq_norm_accumulator;
    pub mod test_flat_vector_scorer;
    pub mod test_for_util;
    pub mod test_indexed_disi;
    pub mod test_merged_vector_values;
    pub mod test_minimal_codec;
    pub mod test_off_heap_vector_values;
    pub mod test_p_for_util;
    pub mod test_per_field_postings_format2;
    pub mod test_postings_util;
    pub mod test_prefetchable_flat_vector_scorer;
    pub mod test_stored_fields_int;
    pub mod test_trie;
}

use core_codecs::*;
use core_support::run;

#[test]
fn test_deflate_with_preset_dict_compression_mode() {
    run(abstract_test_compression_mode::deflate_with_preset_dict_class_def);
}

#[test]
fn test_fast_compression_mode() {
    run(abstract_test_compression_mode::fast_class_def);
}

#[test]
fn test_fast_decompression_mode() {
    run(abstract_test_compression_mode::fast_decompression_class_def);
}

#[test]
fn test_high_compression_mode() {
    run(abstract_test_compression_mode::high_compression_class_def);
}

#[test]
fn test_lz4_with_preset_dict_compression_mode() {
    run(abstract_test_compression_mode::lz4_with_preset_dict_class_def);
}

#[test]
fn test_for_util() {
    run(test_for_util::class_def);
}

#[test]
fn test_p_for_util() {
    run(test_p_for_util::class_def);
}

#[test]
fn test_postings_util() {
    run(test_postings_util::class_def);
}

#[test]
fn test_stored_fields_int() {
    run(test_stored_fields_int::class_def);
}

#[test]
fn test_merged_vector_values() {
    run(test_merged_vector_values::class_def);
}

#[test]
fn test_flat_vector_scorer() {
    run(test_flat_vector_scorer::class_def);
}

#[test]
fn test_prefetchable_flat_vector_scorer() {
    run(test_prefetchable_flat_vector_scorer::class_def);
}

#[test]
fn test_trie() {
    run(test_trie::class_def);
}

#[test]
fn test_indexed_disi() {
    run(test_indexed_disi::class_def);
}

#[test]
fn test_off_heap_vector_values() {
    run(test_off_heap_vector_values::class_def);
}

#[test]
fn test_codec_util() {
    run(test_codec_util::class_def);
}

#[test]
fn test_competitive_freq_norm_accumulator() {
    run(test_competitive_freq_norm_accumulator::class_def);
}

#[test]
fn test_minimal_codec() {
    run(test_minimal_codec::class_def);
}

#[test]
fn test_per_field_postings_format2() {
    run(test_per_field_postings_format2::class_def);
}
