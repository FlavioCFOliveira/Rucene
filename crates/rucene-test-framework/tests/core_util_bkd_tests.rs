//! Port of the tests of `lucene/core/src/test/org/apache/lucene/util/bkd/`
//! (sprint 9, tasks #400 and #401): `Test4BBKDPoints`, `TestBKDConfig`,
//! `TestBKD`, `TestBKDRadixSelector`, `TestBKDRadixSort`, `TestBKDUtil`,
//! `TestDocIdsWriter` and `TestMutablePointTreeReaderUtils`. Each class runs
//! as a suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit `NO_OUTPUT`
// bindings line by line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

#[path = "core_util_bkd/big_integer.rs"]
mod big_integer;
#[path = "core_util_bkd/shared_io.rs"]
mod shared_io;
#[path = "core_util_bkd/test_4b_bkd_points.rs"]
mod test_4b_bkd_points;
#[path = "core_util_bkd/test_bkd.rs"]
mod test_bkd;
#[path = "core_util_bkd/test_bkd_config.rs"]
mod test_bkd_config;
#[path = "core_util_bkd/test_bkd_radix_selector.rs"]
mod test_bkd_radix_selector;
#[path = "core_util_bkd/test_bkd_radix_sort.rs"]
mod test_bkd_radix_sort;
#[path = "core_util_bkd/test_bkd_util.rs"]
mod test_bkd_util;
#[path = "core_util_bkd/test_doc_ids_writer.rs"]
mod test_doc_ids_writer;
#[path = "core_util_bkd/test_mutable_point_tree_reader_utils.rs"]
mod test_mutable_point_tree_reader_utils;

use core_support::run;

#[test]
fn test_4b_bkd_points() {
    run(test_4b_bkd_points::class_def);
}

#[test]
fn test_bkd() {
    run(test_bkd::class_def);
}

#[test]
fn test_bkd_config() {
    run(test_bkd_config::class_def);
}

#[test]
fn test_bkd_radix_selector() {
    run(test_bkd_radix_selector::class_def);
}

#[test]
fn test_bkd_radix_sort() {
    run(test_bkd_radix_sort::class_def);
}

#[test]
fn test_bkd_util() {
    run(test_bkd_util::class_def);
}

#[test]
fn test_doc_ids_writer() {
    run(test_doc_ids_writer::class_def);
}

#[test]
fn test_mutable_point_tree_reader_utils() {
    run(test_mutable_point_tree_reader_utils::class_def);
}
