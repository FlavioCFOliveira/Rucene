//! Port of tests of `lucene/core/src/test/org/apache/lucene/util/hnsw/`
//! (sprint 9, task #401): `TestBlockingFloatHeap`, `TestFloatHeap`,
//! `TestHnswUtil`, `TestNeighborArray`, `TestNeighborQueue`,
//! `TestOnHeapHnswGraph` (task #401), and `TestHnswByteVectorGraph` and
//! `TestHnswFloatVectorGraph` over `HnswGraphTestCase` (task #400). Each class runs as a suite under
//! `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit `NO_OUTPUT`
// bindings line by line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

#[path = "core_util_hnsw/hnsw_graph_test_case.rs"]
mod hnsw_graph_test_case;
#[path = "core_util_hnsw/mock_vector_values.rs"]
mod mock_vector_values;
#[path = "core_util_hnsw/test_heaps.rs"]
mod test_heaps;
#[path = "core_util_hnsw/test_hnsw_byte_vector_graph.rs"]
mod test_hnsw_byte_vector_graph;
#[path = "core_util_hnsw/test_hnsw_float_vector_graph.rs"]
mod test_hnsw_float_vector_graph;
#[path = "core_util_hnsw/test_hnsw_util.rs"]
mod test_hnsw_util;
#[path = "core_util_hnsw/test_neighbor_array.rs"]
mod test_neighbor_array;
#[path = "core_util_hnsw/test_neighbor_queue.rs"]
mod test_neighbor_queue;
#[path = "core_util_hnsw/test_on_heap_hnsw_graph.rs"]
mod test_on_heap_hnsw_graph;

use core_support::run;

#[test]
fn test_blocking_float_heap() {
    run(test_heaps::blocking_float_heap_class_def);
}

#[test]
fn test_float_heap() {
    run(test_heaps::float_heap_class_def);
}

#[test]
fn test_hnsw_byte_vector_graph() {
    run(test_hnsw_byte_vector_graph::class_def);
}

#[test]
fn test_hnsw_float_vector_graph() {
    run(test_hnsw_float_vector_graph::class_def);
}

#[test]
fn test_hnsw_util() {
    run(test_hnsw_util::class_def);
}

#[test]
fn test_neighbor_array() {
    run(test_neighbor_array::class_def);
}

#[test]
fn test_neighbor_queue() {
    run(test_neighbor_queue::class_def);
}

#[test]
fn test_on_heap_hnsw_graph() {
    run(test_on_heap_hnsw_graph::class_def);
}
