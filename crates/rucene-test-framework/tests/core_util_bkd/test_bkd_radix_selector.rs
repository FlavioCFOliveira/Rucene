//! Port of `lucene/core/src/test/org/apache/lucene/util/bkd/TestBKDRadixSelector.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::bkd::{
    BKDConfig, BKDRadixSelector, HeapPointWriter, OfflinePointWriter, PathSlice, PointValue,
    PointWriter,
};
use rucene::util::NumericUtils;
use rucene_test_framework::util::lucene_test_case::{Nightly, TestDirectory};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestBKDRadixSelector`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testBasic", test_basic),
        test("testRandomBinaryTiny", test_random_binary_tiny),
        test("testRandomBinaryMedium", test_random_binary_medium),
        test("testRandomBinaryBig", test_random_binary_big).annotate(Nightly),
        test(
            "testRandomAllDimensionsEquals",
            test_random_all_dimensions_equals,
        ),
        test(
            "testRandomLastByteTwoValues",
            test_random_last_byte_two_values,
        ),
        test("testRandomAllDocsEquals", test_random_all_docs_equals),
        test(
            "testRandomFewDifferentValues",
            test_random_few_different_values,
        ),
        test(
            "testRandomDataDimDiffValues",
            test_random_data_dim_diff_values,
        ),
    ];
    class("org.apache.lucene.util.bkd.TestBKDRadixSelector", methods)
}

/// `points.append(packedValue, docID)`.
fn append(points: &mut dyn PointWriter, value: &[u8], doc_id: i32) -> Result<()> {
    points.append(&PointValue::new(value.to_vec(), doc_id))
}

fn test_basic() -> Result<()> {
    let values = 4;
    let dir = get_directory(values);
    let middle = 2;
    let dimensions = 1;
    let bytes_per_dimensions = 4;
    let config = BKDConfig::new(
        dimensions,
        dimensions,
        bytes_per_dimensions,
        BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
    )?;
    let mut points = get_random_point_writer(&config, &dir, values as i64)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    NumericUtils::int_to_sortable_bytes(1, &mut value, 0);
    append(points.as_mut(), &value, 0)?;
    NumericUtils::int_to_sortable_bytes(2, &mut value, 0);
    append(points.as_mut(), &value, 1)?;
    NumericUtils::int_to_sortable_bytes(3, &mut value, 0);
    append(points.as_mut(), &value, 2)?;
    NumericUtils::int_to_sortable_bytes(4, &mut value, 0);
    append(points.as_mut(), &value, 3)?;
    points.close()?;
    let copy = copy_points(&config, &dir, points.as_mut())?;
    verify(&config, &dir, copy, 0, values as i64, middle, 0)?;
    dir.close()
}

fn test_random_binary_tiny() -> Result<()> {
    do_test_random_binary(10)
}

fn test_random_binary_medium() -> Result<()> {
    do_test_random_binary(25000)
}

fn test_random_binary_big() -> Result<()> {
    do_test_random_binary(500000)
}

/// `doTestRandomBinary(count)`.
fn do_test_random_binary(count: i32) -> Result<()> {
    let config = get_random_config()?;
    let values = TestUtil::next_int(&*random(), count, count * 2);
    let dir = get_directory(values);
    let start;
    let end;
    if random().next_boolean() {
        start = 0;
        end = values;
    } else {
        start = TestUtil::next_int(&*random(), 0, values - 3);
        end = TestUtil::next_int(&*random(), start + 2, values);
    }
    let partition_point = TestUtil::next_int(&*random(), start + 1, end - 1);
    let sorted_on_heap = random().next_int_bound(5000);
    let mut points = get_random_point_writer(&config, &dir, values as i64)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    for i in 0..values {
        random().next_bytes(&mut value);
        append(points.as_mut(), &value, i)?;
    }
    points.close()?;
    verify(
        &config,
        &dir,
        points,
        start as i64,
        end as i64,
        partition_point as i64,
        sorted_on_heap,
    )?;
    dir.close()
}

fn test_random_all_dimensions_equals() -> Result<()> {
    let dimensions = TestUtil::next_int(&*random(), 1, BKDConfig::MAX_INDEX_DIMS);
    let bytes_per_dimensions = TestUtil::next_int(&*random(), 2, 30);
    let config = BKDConfig::new(
        dimensions,
        dimensions,
        bytes_per_dimensions,
        BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
    )?;
    let values = TestUtil::next_int(&*random(), 15000, 20000);
    let dir = get_directory(values);
    let partition_point = random().next_int_bound(values);
    let sorted_on_heap = random().next_int_bound(5000);
    let mut points = get_random_point_writer(&config, &dir, values as i64)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    random().next_bytes(&mut value);
    for i in 0..values {
        if random().next_boolean() {
            append(points.as_mut(), &value, i)?;
        } else {
            append(points.as_mut(), &value, random().next_int_bound(values))?;
        }
    }
    points.close()?;
    verify(
        &config,
        &dir,
        points,
        0,
        values as i64,
        partition_point as i64,
        sorted_on_heap,
    )?;
    dir.close()
}

fn test_random_last_byte_two_values() -> Result<()> {
    let values = random().next_int_bound(15000) + 1;
    let dir = get_directory(values);
    let partition_point = random().next_int_bound(values);
    let sorted_on_heap = random().next_int_bound(5000);
    let config = get_random_config()?;
    let mut points = get_random_point_writer(&config, &dir, values as i64)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    random().next_bytes(&mut value);
    for _ in 0..values {
        if random().next_boolean() {
            append(points.as_mut(), &value, 1)?;
        } else {
            append(points.as_mut(), &value, 2)?;
        }
    }
    points.close()?;
    verify(
        &config,
        &dir,
        points,
        0,
        values as i64,
        partition_point as i64,
        sorted_on_heap,
    )?;
    dir.close()
}

fn test_random_all_docs_equals() -> Result<()> {
    let values = random().next_int_bound(15000) + 1;
    let dir = get_directory(values);
    let partition_point = random().next_int_bound(values);
    let sorted_on_heap = random().next_int_bound(5000);
    let config = get_random_config()?;
    let mut points = get_random_point_writer(&config, &dir, values as i64)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    random().next_bytes(&mut value);
    for _ in 0..values {
        append(points.as_mut(), &value, 0)?;
    }
    points.close()?;
    verify(
        &config,
        &dir,
        points,
        0,
        values as i64,
        partition_point as i64,
        sorted_on_heap,
    )?;
    dir.close()
}

fn test_random_few_different_values() -> Result<()> {
    let config = get_random_config()?;
    let values = LuceneTestCase::at_least(15000);
    let dir = get_directory(values);
    let partition_point = random().next_int_bound(values);
    let sorted_on_heap = random().next_int_bound(5000);
    let mut points = get_random_point_writer(&config, &dir, values as i64)?;
    let number_values = random().next_int_bound(8) + 2;
    let mut different_values =
        vec![vec![0u8; config.packed_bytes_length() as usize]; number_values as usize];
    for v in different_values.iter_mut() {
        random().next_bytes(v);
    }
    for i in 0..values {
        let v = &different_values[random().next_int_bound(number_values) as usize];
        append(points.as_mut(), v, i)?;
    }
    points.close()?;
    verify(
        &config,
        &dir,
        points,
        0,
        values as i64,
        partition_point as i64,
        sorted_on_heap,
    )?;
    dir.close()
}

fn test_random_data_dim_diff_values() -> Result<()> {
    let config = get_random_config()?;
    let values = LuceneTestCase::at_least(15000);
    let dir = get_directory(values);
    let partition_point = random().next_int_bound(values);
    let sorted_on_heap = random().next_int_bound(5000);
    let mut points = get_random_point_writer(&config, &dir, values as i64)?;
    let mut value = vec![0u8; config.packed_bytes_length() as usize];
    let bytes_per_dim = config.bytes_per_dim as usize;
    let data_only_dims = (config.num_dims - config.num_index_dims) as usize;
    let mut data_value = vec![0u8; data_only_dims * bytes_per_dim];
    random().next_bytes(&mut value);
    let data_offset = config.num_index_dims as usize * bytes_per_dim;
    for i in 0..values {
        random().next_bytes(&mut data_value);
        value[data_offset..data_offset + data_only_dims * bytes_per_dim]
            .copy_from_slice(&data_value);
        append(points.as_mut(), &value, i)?;
    }
    points.close()?;
    verify(
        &config,
        &dir,
        points,
        0,
        values as i64,
        partition_point as i64,
        sorted_on_heap,
    )?;
    dir.close()
}

/// `verify(config, dir, points, start, end, middle, sortedOnHeap)`.
fn verify(
    config: &BKDConfig,
    dir: &TestDirectory,
    mut points: Box<dyn PointWriter>,
    start: i64,
    end: i64,
    middle: i64,
    sorted_on_heap: i32,
) -> Result<()> {
    let mut radix_selector = BKDRadixSelector::new(
        config.clone(),
        sorted_on_heap as usize,
        dir.as_directory(),
        "test",
    );
    let bytes_per_dim = config.bytes_per_dim as usize;
    let data_only_dims = (config.num_dims - config.num_index_dims) as usize;
    // we only split by indexed dimension so we check for each only those dimension
    for split_dim in 0..config.num_index_dims as usize {
        // We need to make a copy of the data as it is deleted in the process
        let count = points.count() as i64;
        let input_slice = PathSlice::of(copy_points(config, dir, points.as_mut())?, 0, count);
        let common_prefix_length_input = get_random_common_prefix(config, &input_slice, split_dim)?;
        let (partition_point, slices) = radix_selector.select(
            &input_slice,
            start,
            end,
            middle,
            split_dim,
            common_prefix_length_input,
        )?;
        assert_equals(&(middle - start), &slices[0].count);
        assert_equals(&(end - middle), &slices[1].count);
        // check that left and right slices contain the correct points
        let max = get_max(config, &slices[0], split_dim)?;
        let min = get_min(config, &slices[1], split_dim)?;
        let mut cmp = max[..bytes_per_dim].cmp(&min[..bytes_per_dim]);
        Assert::assert_true(cmp.is_le());
        if cmp.is_eq() {
            let max_data_dim = get_max_data_dimension(config, &slices[0], &max, split_dim)?;
            let min_data_dim = get_min_data_dimension(config, &slices[1], &min, split_dim)?;
            cmp = max_data_dim[..data_only_dims * bytes_per_dim]
                .cmp(&min_data_dim[..data_only_dims * bytes_per_dim]);
            Assert::assert_true(cmp.is_le());
            if cmp.is_eq() {
                let max_doc_id = get_max_doc_id(
                    config,
                    &slices[0],
                    split_dim,
                    &partition_point,
                    &max_data_dim,
                )?;
                let min_doc_id = get_min_doc_id(
                    config,
                    &slices[1],
                    split_dim,
                    &partition_point,
                    &min_data_dim,
                )?;
                Assert::assert_true(min_doc_id >= max_doc_id);
            }
        }
        Assert::assert_true(partition_point == min);
        slices[0].writer.borrow_mut().destroy()?;
        slices[1].writer.borrow_mut().destroy()?;
    }
    points.destroy()
}

/// `copyPoints(config, dir, points)`.
fn copy_points(
    config: &BKDConfig,
    dir: &TestDirectory,
    points: &mut dyn PointWriter,
) -> Result<Box<dyn PointWriter>> {
    let count = points.count();
    let mut copy = get_random_point_writer(config, dir, count as i64)?;
    {
        let mut reader = points.get_reader(0, count)?;
        while reader.next()? {
            copy.append(reader.point_value())?;
        }
        reader.close()?;
    }
    copy.close()?;
    Ok(copy)
}

/// `Arrays.mismatch(a, 0, len, b, 0, len)`: the first differing index, or
/// `-1`.
fn mismatch(a: &[u8], b: &[u8]) -> i32 {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => i as i32,
        None if a.len() == b.len() => -1,
        None => a.len().min(b.len()) as i32,
    }
}

/// `getRandomCommonPrefix(config, inputSlice, splitDim)`: a common prefix
/// length equal or lower than the current one.
fn get_random_common_prefix(
    config: &BKDConfig,
    input_slice: &PathSlice,
    split_dim: usize,
) -> Result<usize> {
    let bytes_per_dim = config.bytes_per_dim as usize;
    let points_max = get_max(config, input_slice, split_dim)?;
    let points_min = get_min(config, input_slice, split_dim)?;
    let mut common_prefix_length =
        mismatch(&points_min[..bytes_per_dim], &points_max[..bytes_per_dim]);
    if common_prefix_length == -1 {
        common_prefix_length = bytes_per_dim as i32;
    }
    Ok(if random().next_boolean() {
        common_prefix_length as usize
    } else if common_prefix_length == 0 {
        0
    } else {
        random().next_int_bound(common_prefix_length) as usize
    })
}

/// `getRandomPointWriter(config, dir, numPoints)`.
fn get_random_point_writer(
    config: &BKDConfig,
    dir: &TestDirectory,
    num_points: i64,
) -> Result<Box<dyn PointWriter>> {
    if num_points < 4096 && random().next_boolean() {
        Ok(Box::new(HeapPointWriter::new(
            config.clone(),
            usize::try_from(num_points).expect("Math.toIntExact"),
        )?))
    } else {
        // `new OfflinePointWriter(config, dir, "test", "data", numPoints)`.
        Ok(Box::new(OfflinePointWriter::new(
            config.clone(),
            dir.as_directory(),
            "test",
            "data",
            num_points as u64,
        )?))
    }
}

/// `getDirectory(numPoints)`.
fn get_directory(num_points: i32) -> TestDirectory {
    if num_points > 100000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
            "TestBKDTRadixSelector",
        ))
    } else {
        LuceneTestCase::new_directory()
    }
}

/// Runs `f` over every point of `p` (`try (PointReader reader =
/// p.writer().getReader(p.start(), p.count())) { while (reader.next()) … }`).
fn for_each_point(p: &PathSlice, mut f: impl FnMut(&PointValue)) -> Result<()> {
    let mut writer = p.writer.borrow_mut();
    let mut reader = writer.get_reader(p.start as usize, p.count as usize)?;
    while reader.next()? {
        f(reader.point_value());
    }
    reader.close()
}

/// `getMin(config, pathSlice, dimension)`.
fn get_min(config: &BKDConfig, path_slice: &PathSlice, dimension: usize) -> Result<Vec<u8>> {
    let bytes_per_dim = config.bytes_per_dim as usize;
    let mut min = vec![0xffu8; bytes_per_dim];
    for_each_point(path_slice, |point_value| {
        let packed_value = point_value.packed_value();
        let value = &packed_value[dimension * bytes_per_dim..(dimension + 1) * bytes_per_dim];
        if min[..].cmp(value).is_gt() {
            min.copy_from_slice(value);
        }
    })?;
    Ok(min)
}

/// `getMinDocId(config, p, dimension, partitionPoint, dataDim)`.
fn get_min_doc_id(
    config: &BKDConfig,
    p: &PathSlice,
    dimension: usize,
    partition_point: &[u8],
    data_dim: &[u8],
) -> Result<i32> {
    let mut doc_id = i32::MAX;
    let bytes_per_dim = config.bytes_per_dim as usize;
    for_each_point(p, |point_value| {
        let packed_value = point_value.packed_value();
        let offset = dimension * bytes_per_dim;
        let data_offset = config.packed_index_bytes_length() as usize;
        let data_length = (config.num_dims - config.num_index_dims) as usize * bytes_per_dim;
        if packed_value[offset..offset + bytes_per_dim] == partition_point[..bytes_per_dim]
            && packed_value[data_offset..data_offset + data_length] == data_dim[..data_length]
        {
            let new_doc_id = point_value.doc_id;
            if new_doc_id < doc_id {
                doc_id = new_doc_id;
            }
        }
    })?;
    Ok(doc_id)
}

/// `getMinDataDimension(config, p, minDim, splitDim)`.
fn get_min_data_dimension(
    config: &BKDConfig,
    p: &PathSlice,
    min_dim: &[u8],
    split_dim: usize,
) -> Result<Vec<u8>> {
    let bytes_per_dim = config.bytes_per_dim as usize;
    let num_data_dims = (config.num_dims - config.num_index_dims) as usize;
    let mut min = vec![0xffu8; num_data_dims * bytes_per_dim];
    let offset = split_dim * bytes_per_dim;
    for_each_point(p, |point_value| {
        let packed_value = point_value.packed_value();
        if mismatch(
            &min_dim[..bytes_per_dim],
            &packed_value[offset..offset + bytes_per_dim],
        ) == -1
        {
            let start = config.num_index_dims as usize * bytes_per_dim;
            let value = &packed_value[start..start + num_data_dims * bytes_per_dim];
            if min[..].cmp(value).is_gt() {
                min.copy_from_slice(value);
            }
        }
    })?;
    Ok(min)
}

/// `getMax(config, p, dimension)`.
fn get_max(config: &BKDConfig, p: &PathSlice, dimension: usize) -> Result<Vec<u8>> {
    let bytes_per_dim = config.bytes_per_dim as usize;
    let mut max = vec![0u8; bytes_per_dim];
    for_each_point(p, |point_value| {
        let packed_value = point_value.packed_value();
        let value = &packed_value[dimension * bytes_per_dim..(dimension + 1) * bytes_per_dim];
        if max[..].cmp(value).is_lt() {
            max.copy_from_slice(value);
        }
    })?;
    Ok(max)
}

/// `getMaxDataDimension(config, p, maxDim, splitDim)`.
fn get_max_data_dimension(
    config: &BKDConfig,
    p: &PathSlice,
    max_dim: &[u8],
    split_dim: usize,
) -> Result<Vec<u8>> {
    let bytes_per_dim = config.bytes_per_dim as usize;
    let num_data_dims = (config.num_dims - config.num_index_dims) as usize;
    let mut max = vec![0u8; num_data_dims * bytes_per_dim];
    let offset = split_dim * bytes_per_dim;
    for_each_point(p, |point_value| {
        let packed_value = point_value.packed_value();
        if mismatch(
            &max_dim[..bytes_per_dim],
            &packed_value[offset..offset + bytes_per_dim],
        ) == -1
        {
            let start = config.packed_index_bytes_length() as usize;
            let value = &packed_value[start..start + num_data_dims * bytes_per_dim];
            if max[..].cmp(value).is_lt() {
                max.copy_from_slice(value);
            }
        }
    })?;
    Ok(max)
}

/// `getMaxDocId(config, p, dimension, partitionPoint, dataDim)`.
fn get_max_doc_id(
    config: &BKDConfig,
    p: &PathSlice,
    dimension: usize,
    partition_point: &[u8],
    data_dim: &[u8],
) -> Result<i32> {
    let mut doc_id = i32::MIN;
    let bytes_per_dim = config.bytes_per_dim as usize;
    for_each_point(p, |point_value| {
        let packed_value = point_value.packed_value();
        let offset = dimension * bytes_per_dim;
        let data_offset = config.packed_index_bytes_length() as usize;
        let data_length = (config.num_dims - config.num_index_dims) as usize * bytes_per_dim;
        if packed_value[offset..offset + bytes_per_dim] == partition_point[..bytes_per_dim]
            && packed_value[data_offset..data_offset + data_length] == data_dim[..data_length]
        {
            let new_doc_id = point_value.doc_id;
            if new_doc_id > doc_id {
                doc_id = new_doc_id;
            }
        }
    })?;
    Ok(doc_id)
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
