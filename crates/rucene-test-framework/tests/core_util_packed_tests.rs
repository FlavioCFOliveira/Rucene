//! Port of `TestDirectMonotonic`, `TestDirectPacked` and `TestPackedInts` of
//! `lucene/core/src/test/org/apache/lucene/util/packed/` (sprint 9, tasks
//! #401 and #402). Each class runs as a suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit `NO_OUTPUT`
// bindings line by line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

#[path = "core_util_packed/test_direct_monotonic.rs"]
mod test_direct_monotonic;
#[path = "core_util_packed/test_direct_packed.rs"]
mod test_direct_packed;
#[path = "core_util_packed/test_packed_ints.rs"]
mod test_packed_ints;

use core_support::run;

#[test]
fn test_direct_monotonic() {
    run(test_direct_monotonic::class_def);
}

#[test]
fn test_direct_packed() {
    run(test_direct_packed::class_def);
}

#[test]
fn test_packed_ints() {
    run(test_packed_ints::class_def);
}
