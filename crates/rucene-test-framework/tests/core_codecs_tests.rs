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
    pub mod test_for_util;
    pub mod test_p_for_util;
    pub mod test_postings_util;
    pub mod test_stored_fields_int;
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
