//! Port of `lucene/core/src/test/org/apache/lucene/util/bkd/TestBKDRadixSort.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::bkd::{BKDConfig, BKDRadixSelector, HeapPointWriter, PointValue, PointWriter};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, random, test};

/// `TestBKDRadixSort`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testRandom", test_random),
        test("testRandomAllEquals", test_random_all_equals),
        test(
            "testRandomLastByteTwoValues",
            test_random_last_byte_two_values,
        ),
        test(
            "testRandomFewDifferentValues",
            test_random_few_different_values,
        ),
        test("testRandomDataDimDifferent", test_random_data_dim_different),
    ];
    class("org.apache.lucene.util.bkd.TestBKDRadixSort", methods)
}

/// `points.append(packedValue, docID)`.
fn append(points: &mut HeapPointWriter, value: &[u8], doc_id: i32) -> Result<()> {
    points.append(&PointValue::new(value.to_vec(), doc_id))
}

fn test_random() -> Result<()> {
    let config = get_random_config()?;
    let num_points = TestUtil::next_int(&*random(), 1, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE);
    let mut points = HeapPointWriter::new(config.clone(), num_points as usize)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    for i in 0..num_points {
        random().next_bytes(&mut value);
        append(&mut points, &value, i)?;
    }
    verify_sort(&config, &mut points, 0, num_points as usize)
}

fn test_random_all_equals() -> Result<()> {
    let config = get_random_config()?;
    let num_points = TestUtil::next_int(&*random(), 1, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE);
    let mut points = HeapPointWriter::new(config.clone(), num_points as usize)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    random().next_bytes(&mut value);
    for _ in 0..num_points {
        append(&mut points, &value, random().next_int_bound(num_points))?;
    }
    verify_sort(&config, &mut points, 0, num_points as usize)
}

fn test_random_last_byte_two_values() -> Result<()> {
    let config = get_random_config()?;
    let num_points = TestUtil::next_int(&*random(), 1, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE);
    let mut points = HeapPointWriter::new(config.clone(), num_points as usize)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    random().next_bytes(&mut value);
    for _ in 0..num_points {
        if random().next_boolean() {
            append(&mut points, &value, 1)?;
        } else {
            append(&mut points, &value, 2)?;
        }
    }
    verify_sort(&config, &mut points, 0, num_points as usize)
}

fn test_random_few_different_values() -> Result<()> {
    let config = get_random_config()?;
    let num_points = TestUtil::next_int(&*random(), 1, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE);
    let mut points = HeapPointWriter::new(config.clone(), num_points as usize)?;
    let number_values = random().next_int_bound(8) + 2;
    let mut different_values =
        vec![vec![0u8; config.packed_bytes_length() as usize]; number_values as usize];
    for v in different_values.iter_mut() {
        random().next_bytes(v);
    }
    for i in 0..num_points {
        let v = &different_values[random().next_int_bound(number_values) as usize];
        append(&mut points, v, i)?;
    }
    verify_sort(&config, &mut points, 0, num_points as usize)
}

fn test_random_data_dim_different() -> Result<()> {
    let config = get_random_config()?;
    let num_points = TestUtil::next_int(&*random(), 1, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE);
    let mut points = HeapPointWriter::new(config.clone(), num_points as usize)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    let total_data_dimension = (config.num_dims - config.num_index_dims) as usize;
    let bytes_per_dim = config.bytes_per_dim as usize;
    let mut data_dimension_values = vec![0u8; total_data_dimension * bytes_per_dim];
    random().next_bytes(&mut value);
    let index_len = config.packed_index_bytes_length() as usize;
    for _ in 0..num_points {
        random().next_bytes(&mut data_dimension_values);
        value[index_len..index_len + total_data_dimension * bytes_per_dim]
            .copy_from_slice(&data_dimension_values);
        append(&mut points, &value, random().next_int_bound(num_points))?;
    }
    verify_sort(&config, &mut points, 0, num_points as usize)
}

/// `verifySort(config, points, start, end)`.
fn verify_sort(
    config: &BKDConfig,
    points: &mut HeapPointWriter,
    start: usize,
    end: usize,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut radix_selector =
        BKDRadixSelector::new(config.clone(), 1000, dir.as_directory(), "test");
    let bytes_per_dim = config.bytes_per_dim as usize;
    let packed_len = config.packed_bytes_length() as usize;
    // we check for each dimension
    for split_dim in 0..config.num_dims as usize {
        let common_prefix = get_random_common_prefix(config, points, start, end, split_dim);
        radix_selector.heap_radix_sort(points, start, end, split_dim, common_prefix);
        let mut previous = vec![0u8; packed_len];
        let mut previous_doc_id = -1;
        let dim_offset = split_dim * bytes_per_dim;
        for j in start..end {
            let point_value = points.get_packed_value_slice(j);
            let value = point_value.packed_value();
            let mut cmp = value[dim_offset..dim_offset + bytes_per_dim]
                .cmp(&previous[dim_offset..dim_offset + bytes_per_dim]);
            Assert::assert_true(cmp.is_ge());
            if cmp.is_eq() {
                let data_offset = config.num_index_dims as usize * bytes_per_dim;
                cmp = value[data_offset..packed_len].cmp(&previous[data_offset..packed_len]);
                Assert::assert_true(cmp.is_ge());
            }
            if cmp.is_eq() {
                Assert::assert_true(point_value.doc_id >= previous_doc_id);
            }
            previous[..packed_len].copy_from_slice(&value[..packed_len]);
            previous_doc_id = point_value.doc_id;
        }
    }
    dir.close()
}

/// `getRandomCommonPrefix(config, points, start, end, sortDim)`: a common
/// prefix length equal or lower than the current one.
fn get_random_common_prefix(
    config: &BKDConfig,
    points: &HeapPointWriter,
    start: usize,
    end: usize,
    sort_dim: usize,
) -> usize {
    let bytes_per_dim = config.bytes_per_dim as usize;
    let mut common_prefix_length = bytes_per_dim;
    let value = points.get_packed_value_slice(start);
    let offset = sort_dim * bytes_per_dim;
    let first_value = value.packed_value()[offset..offset + bytes_per_dim].to_vec();
    for i in start + 1..end {
        let value = points.get_packed_value_slice(i);
        let bytes = &value.packed_value()[offset..offset + bytes_per_dim];
        let diff = bytes.iter().zip(&first_value).position(|(a, b)| a != b);
        if let Some(diff) = diff {
            if common_prefix_length > diff {
                if diff == 0 {
                    return diff;
                }
                common_prefix_length = diff;
            }
        }
    }
    if random().next_boolean() {
        common_prefix_length
    } else {
        random().next_int_bound(common_prefix_length as i32) as usize
    }
}

/// `getRandomConfig()`.
fn get_random_config() -> Result<BKDConfig> {
    let num_index_dims = TestUtil::next_int(&*random(), 1, BKDConfig::MAX_INDEX_DIMS);
    let num_dims = TestUtil::next_int(&*random(), num_index_dims, BKDConfig::MAX_DIMS);
    let bytes_per_dim = TestUtil::next_int(&*random(), 2, 30);
    let max_points_in_leaf_node = TestUtil::next_int(&*random(), 50, 2000);
    BKDConfig::new(
        num_dims,
        num_index_dims,
        bytes_per_dim,
        max_points_in_leaf_node,
    )
}
