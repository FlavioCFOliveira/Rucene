//! Port of the tests of `lucene/core/src/test/org/apache/lucene/util/compress/`
//! (sprint 9, task #401): `TestDecompressLZ4`, `TestFastLZ4`,
//! `TestHighLZ4` (both extending the package's `LZ4TestCase`) and
//! `TestLowercaseAsciiCompression`. Each class runs as a suite under
//! `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit `NO_OUTPUT`
// bindings line by line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

#[path = "core_util_compress/lz4_test_case.rs"]
mod lz4_test_case;
#[path = "core_util_compress/test_lowercase_ascii_compression.rs"]
mod test_lowercase_ascii_compression;
#[path = "core_util_compress/test_lz4.rs"]
mod test_lz4;

use core_support::run;

#[test]
fn test_decompress_lz4() {
    run(test_lz4::decompress_class_def);
}

#[test]
fn test_fast_lz4() {
    run(test_lz4::fast_class_def);
}

#[test]
fn test_high_lz4() {
    run(test_lz4::high_class_def);
}

#[test]
fn test_lowercase_ascii_compression() {
    run(test_lowercase_ascii_compression::class_def);
}
