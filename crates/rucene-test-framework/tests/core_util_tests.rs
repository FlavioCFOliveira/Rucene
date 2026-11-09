//! Port of tests of `lucene/core/src/test/org/apache/lucene/util/` (sprint
//! 9, task #402): `Test2BPagedBytes`, `TestArrayUtil`, `TestAttributeSource`, `TestBitSetIterator`, `TestBitUtil`, `TestByteBlockPool`, `TestBytesRef`, `TestBytesRefArray`, `TestBytesRefHash`, `TestCharsRef`, `TestCharsRefBuilder`, `TestClassLoaderUtils`, `TestCloseableThreadLocal`, `TestCollectionUtil`, `TestDocIdSetBuilder`, `TestFilterIterator`, `TestFixedLengthBytesRefArray`, `TestFrequencyTrackingRingBuffer`, `TestIOUtils`, `TestInPlaceMergeSorter`, `TestIntroSelector`, `TestIntroSorter`, `TestIntsRef`, `TestLSBRadixSorter`, `TestLiveDocs`, `TestLongBitSet`, `TestLongHeap`, `TestLongsRef`.
//! Each class runs as a suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit bindings line by
// line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

#[path = "core_util/base_sort_test_case.rs"]
mod base_sort_test_case;
mod core_support;
#[path = "core_util/test_2b_paged_bytes.rs"]
mod test_2b_paged_bytes;
#[path = "core_util/test_array_util.rs"]
mod test_array_util;
#[path = "core_util/test_attribute_source.rs"]
mod test_attribute_source;
#[path = "core_util/test_bit_set_iterator.rs"]
mod test_bit_set_iterator;
#[path = "core_util/test_bit_util.rs"]
mod test_bit_util;
#[path = "core_util/test_byte_block_pool.rs"]
mod test_byte_block_pool;
#[path = "core_util/test_bytes_ref.rs"]
mod test_bytes_ref;
#[path = "core_util/test_bytes_ref_array.rs"]
mod test_bytes_ref_array;
#[path = "core_util/test_bytes_ref_hash.rs"]
mod test_bytes_ref_hash;
#[path = "core_util/test_chars_ref.rs"]
mod test_chars_ref;
#[path = "core_util/test_chars_ref_builder.rs"]
mod test_chars_ref_builder;
#[path = "core_util/test_class_loader_utils.rs"]
mod test_class_loader_utils;
#[path = "core_util/test_closeable_thread_local.rs"]
mod test_closeable_thread_local;
#[path = "core_util/test_collection_util.rs"]
mod test_collection_util;
#[path = "core_util/test_doc_id_set_builder.rs"]
mod test_doc_id_set_builder;
#[path = "core_util/test_filter_iterator.rs"]
mod test_filter_iterator;
#[path = "core_util/test_fixed_length_bytes_ref_array.rs"]
mod test_fixed_length_bytes_ref_array;
#[path = "core_util/test_frequency_tracking_ring_buffer.rs"]
mod test_frequency_tracking_ring_buffer;
#[path = "core_util/test_intro_selector.rs"]
mod test_intro_selector;
#[path = "core_util/test_ints_ref.rs"]
mod test_ints_ref;
#[path = "core_util/test_io_utils.rs"]
mod test_io_utils;
#[path = "core_util/test_live_docs.rs"]
mod test_live_docs;
#[path = "core_util/test_long_bit_set.rs"]
mod test_long_bit_set;
#[path = "core_util/test_long_heap.rs"]
mod test_long_heap;
#[path = "core_util/test_longs_ref.rs"]
mod test_longs_ref;
#[path = "core_util/test_lsb_radix_sorter.rs"]
mod test_lsb_radix_sorter;

#[path = "core_util/test_java_logging_info_stream.rs"]
mod test_java_logging_info_stream;
#[path = "core_util/test_math_util.rs"]
mod test_math_util;
#[path = "core_util/test_merged_iterator.rs"]
mod test_merged_iterator;
#[path = "core_util/test_msb_radix_sorter.rs"]
mod test_msb_radix_sorter;
#[path = "core_util/test_named_spi_loader.rs"]
mod test_named_spi_loader;
#[path = "core_util/test_numeric_utils.rs"]
mod test_numeric_utils;
#[path = "core_util/test_offline_sorter.rs"]
mod test_offline_sorter;
#[path = "core_util/test_paged_bytes.rs"]
mod test_paged_bytes;
#[path = "core_util/test_priority_queue.rs"]
mod test_priority_queue;
#[path = "core_util/test_radix_selector.rs"]
mod test_radix_selector;
#[path = "core_util/test_ram_usage_estimator.rs"]
mod test_ram_usage_estimator;
#[path = "core_util/test_recycling_byte_block_allocator.rs"]
mod test_recycling_byte_block_allocator;
#[path = "core_util/test_recycling_int_block_allocator.rs"]
mod test_recycling_int_block_allocator;
#[path = "core_util/test_rolling_buffer.rs"]
mod test_rolling_buffer;
#[path = "core_util/test_sentinel_int_set.rs"]
mod test_sentinel_int_set;
#[path = "core_util/test_set_once.rs"]
mod test_set_once;
#[path = "core_util/test_sloppy_math.rs"]
mod test_sloppy_math;
#[path = "core_util/test_small_float.rs"]
mod test_small_float;
#[path = "core_util/test_stress_ram_usage_estimator.rs"]
mod test_stress_ram_usage_estimator;
#[path = "core_util/test_string_helper.rs"]
mod test_string_helper;
#[path = "core_util/test_string_sorter.rs"]
mod test_string_sorter;
#[path = "core_util/test_ternary_long_heap.rs"]
mod test_ternary_long_heap;
#[path = "core_util/test_tim_sorter_worst_case.rs"]
mod test_tim_sorter_worst_case;
#[path = "core_util/test_unicode_util.rs"]
mod test_unicode_util;
#[path = "core_util/test_vector_util.rs"]
mod test_vector_util;
#[path = "core_util/test_version.rs"]
mod test_version;
#[path = "core_util/test_virtual_method.rs"]
mod test_virtual_method;
#[path = "core_util/test_weak_identity_map.rs"]
mod test_weak_identity_map;
use randomizedtesting::jdk::reflect::Method;
use rucene::error::Result;
use rucene_test_framework::util::java_exceptions::rethrow_java;

use core_support::run;

/// [`core_support::test`] over a closure: the public `test*` method `name`
/// of a class whose body captures the state of its (abstract-base)
/// subclass.
#[allow(dead_code)]
pub fn test_with(name: &str, body: impl Fn() -> Result<()> + Send + Sync + 'static) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body_any(move || {
            if let Err(e) = body() {
                rethrow_java(e)
            }
        })
}

#[test]
fn test2_b_paged_bytes() {
    run(test_2b_paged_bytes::class_def);
}

#[test]
fn test_array_util() {
    run(test_array_util::class_def);
}

#[test]
fn test_attribute_source() {
    run(test_attribute_source::class_def);
}

#[test]
fn test_bit_set_iterator() {
    run(test_bit_set_iterator::class_def);
}

#[test]
fn test_bit_util() {
    run(test_bit_util::class_def);
}

#[test]
fn test_byte_block_pool() {
    run(test_byte_block_pool::class_def);
}

#[test]
fn test_bytes_ref() {
    run(test_bytes_ref::class_def);
}

#[test]
fn test_bytes_ref_array() {
    run(test_bytes_ref_array::class_def);
}

#[test]
fn test_bytes_ref_hash() {
    run(test_bytes_ref_hash::class_def);
}

#[test]
fn test_chars_ref() {
    run(test_chars_ref::class_def);
}

#[test]
fn test_chars_ref_builder() {
    run(test_chars_ref_builder::class_def);
}

#[test]
fn test_class_loader_utils() {
    run(test_class_loader_utils::class_def);
}

#[test]
fn test_closeable_thread_local() {
    run(test_closeable_thread_local::class_def);
}

#[test]
fn test_collection_util() {
    run(test_collection_util::class_def);
}

#[test]
fn test_doc_id_set_builder() {
    run(test_doc_id_set_builder::class_def);
}

#[test]
fn test_filter_iterator() {
    run(test_filter_iterator::class_def);
}

#[test]
fn test_fixed_length_bytes_ref_array() {
    run(test_fixed_length_bytes_ref_array::class_def);
}

#[test]
fn test_frequency_tracking_ring_buffer() {
    run(test_frequency_tracking_ring_buffer::class_def);
}

#[test]
fn test_i_o_utils() {
    run(test_io_utils::class_def);
}

#[test]
fn test_in_place_merge_sorter() {
    run(base_sort_test_case::in_place_merge_sorter_class_def);
}

#[test]
fn test_intro_selector() {
    run(test_intro_selector::class_def);
}

#[test]
fn test_intro_sorter() {
    run(base_sort_test_case::intro_sorter_class_def);
}

#[test]
fn test_ints_ref() {
    run(test_ints_ref::class_def);
}

#[test]
fn test_l_s_b_radix_sorter() {
    run(test_lsb_radix_sorter::class_def);
}

#[test]
fn test_live_docs() {
    run(test_live_docs::class_def);
}

#[test]
fn test_long_bit_set() {
    run(test_long_bit_set::class_def);
}

#[test]
fn test_long_heap() {
    run(test_long_heap::class_def);
}

#[test]
fn test_longs_ref() {
    run(test_longs_ref::class_def);
}

#[test]
fn test_math_util() {
    run(test_math_util::class_def);
}

#[test]
fn test_merged_iterator() {
    run(test_merged_iterator::class_def);
}

#[test]
fn test_named_s_p_i_loader() {
    run(test_named_spi_loader::class_def);
}

#[test]
fn test_rolling_buffer() {
    run(test_rolling_buffer::class_def);
}

#[test]
fn test_sentinel_int_set() {
    run(test_sentinel_int_set::class_def);
}

#[test]
fn test_set_once() {
    run(test_set_once::class_def);
}

#[test]
fn test_small_float() {
    run(test_small_float::class_def);
}

#[test]
fn test_string_helper() {
    run(test_string_helper::class_def);
}

#[test]
fn test_java_logging_info_stream() {
    run(test_java_logging_info_stream::class_def);
}

#[test]
fn test_recycling_byte_block_allocator() {
    run(test_recycling_byte_block_allocator::class_def);
}

#[test]
fn test_recycling_int_block_allocator() {
    run(test_recycling_int_block_allocator::class_def);
}

#[test]
fn test_sloppy_math() {
    run(test_sloppy_math::class_def);
}

#[test]
fn test_ternary_long_heap() {
    run(test_ternary_long_heap::class_def);
}

#[test]
fn test_tim_sorter() {
    run(base_sort_test_case::tim_sorter_class_def);
}

#[test]
fn test_priority_queue() {
    run(test_priority_queue::class_def);
}

#[test]
fn test_radix_selector() {
    run(test_radix_selector::class_def);
}

#[test]
fn test_m_s_b_radix_sorter() {
    run(test_msb_radix_sorter::class_def);
}

#[test]
fn test_stable_m_s_b_radix_sorter() {
    run(test_msb_radix_sorter::stable_class_def);
}

#[test]
fn test_string_sorter() {
    run(test_string_sorter::class_def);
}

#[test]
fn test_tim_sorter_worst_case() {
    run(test_tim_sorter_worst_case::class_def);
}

#[test]
fn test_unicode_util() {
    run(test_unicode_util::class_def);
}

#[test]
fn test_version() {
    run(test_version::class_def);
}

#[test]
fn test_virtual_method() {
    run(test_virtual_method::class_def);
}

#[test]
fn test_weak_identity_map() {
    run(test_weak_identity_map::class_def);
}

#[test]
fn test_numeric_utils() {
    run(test_numeric_utils::class_def);
}

#[test]
fn test_paged_bytes() {
    run(test_paged_bytes::class_def);
}

#[test]
fn test_offline_sorter() {
    run(test_offline_sorter::class_def);
}

#[test]
fn test_ram_usage_estimator() {
    run(test_ram_usage_estimator::class_def);
}

#[test]
fn test_stress_ram_usage_estimator() {
    run(test_stress_ram_usage_estimator::class_def);
}

#[test]
fn test_vector_util() {
    run(test_vector_util::class_def);
}
