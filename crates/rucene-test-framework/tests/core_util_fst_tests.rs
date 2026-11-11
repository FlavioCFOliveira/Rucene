//! Port of the tests of `lucene/core/src/test/org/apache/lucene/util/fst/`
//! (sprint 9, task #401): `Test2BFST`, `Test2BFSTOffHeap`,
//! `TestBitTableUtil`, `TestFSTDirectAddressing`, `TestFSTs`,
//! `TestGrowableByteArrayDataOutput`, `TestNodeHash` and `TestUtil`. Each
//! class runs as a suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit `NO_OUTPUT`
// bindings line by line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

#[path = "core_util_fst/test_2b_fst.rs"]
mod test_2b_fst;
#[path = "core_util_fst/test_bit_table_util.rs"]
mod test_bit_table_util;
#[path = "core_util_fst/test_fst_direct_addressing.rs"]
mod test_fst_direct_addressing;
#[path = "core_util_fst/test_fsts.rs"]
mod test_fsts;
#[path = "core_util_fst/test_growable_byte_array_data_output.rs"]
mod test_growable_byte_array_data_output;
#[path = "core_util_fst/test_node_hash.rs"]
mod test_node_hash;
#[path = "core_util_fst/test_util.rs"]
mod test_util;

use core_support::run;

#[test]
fn test_2b_fst() {
    run(test_2b_fst::class_def);
}

#[test]
fn test_2b_fst_off_heap() {
    run(test_2b_fst::off_heap_class_def);
}

#[test]
fn test_bit_table_util() {
    run(test_bit_table_util::class_def);
}

#[test]
fn test_fst_direct_addressing() {
    run(test_fst_direct_addressing::class_def);
}

#[test]
fn test_fsts() {
    run(test_fsts::class_def);
}

#[test]
fn test_growable_byte_array_data_output() {
    run(test_growable_byte_array_data_output::class_def);
}

#[test]
fn test_node_hash() {
    run(test_node_hash::class_def);
}

#[test]
fn test_util() {
    run(test_util::class_def);
}
