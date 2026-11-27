/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.facet.range.TestDynamicRangeUtil`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_facet::range::dynamic_range_util::{DynamicRangeInfo, DynamicRangeUtil};
use rucene_test_framework::util::LuceneTestCase;

use crate::facet_support::core_support::{assert_equals, class, test};

/// `TestDynamicRangeUtil` (`extends LuceneTestCase`).
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.facet.range.TestDynamicRangeUtil",
        vec![
            test("testComputeDynamicNumericRangesBasic", basic),
            test(
                "testComputeDynamicNumericRangesWithSameValues",
                with_same_values,
            ),
            test(
                "testComputeDynamicNumericRangesWithSameWeights",
                with_same_weights,
            ),
            test(
                "testComputeDynamicNumericRangesWithOneValue",
                with_one_value,
            ),
            test(
                "testComputeDynamicNumericRangesWithOneLargeWeight",
                with_one_large_weight,
            ),
            test(
                "testComputeDynamicNumericRangesWithLargeTopN",
                with_large_top_n,
            ),
            test(
                "testComputeDynamicNumericRangesWithZeroTopN",
                with_zero_top_n,
            ),
            test(
                "testComputeDynamicNumericRangesWithSingleTopN",
                with_single_top_n,
            ),
            test("testComputeDynamicNumericRangesWithTwoTopN", with_two_top_n),
            test(
                "testComputeDynamicNumericRangesWithSameWeightsShuffled",
                with_same_weights_shuffled,
            ),
            test(
                "testComputeDynamicNumericRangesWithSameValuesShuffled",
                with_same_values_shuffled,
            ),
            test(
                "testComputeDynamicNumericRangesWithMisplacedValue",
                with_misplaced_value,
            ),
            test(
                "testComputeDynamicNumericRangesWithSameWeightsOutOfOrder",
                with_same_weights_out_of_order,
            ),
            test(
                "testComputeDynamicNumericRangesWithRandomValues",
                with_random_values,
            ),
        ],
    )
}

fn info(count: i32, weight: i64, min: i64, max: i64, centroid: f64) -> DynamicRangeInfo {
    DynamicRangeInfo::new(count, weight, min, max, centroid)
}

fn basic() -> Result<()> {
    let mut values = vec![0i64; 1000];
    let mut weights = vec![0i64; 1000];

    let mut total_weight = 0i64;
    for i in 0..1000 {
        values[i] = i as i64 + 1;
        weights[i] = i as i64;
        total_weight += i as i64;
    }

    let expected = vec![
        info(501, 125250, 1, 501, 251.0),
        info(207, 125028, 502, 708, 605.0),
        info(159, 125133, 709, 867, 788.0),
        info(133, 124089, 868, 1000, 934.0),
    ];
    assert_dynamic_numeric_range_results(&mut values, &mut weights, 4, total_weight, &expected);
    Ok(())
}

fn same_values_input() -> (Vec<i64>, Vec<i64>, i64) {
    let mut total_weight = 0i64;
    let mut values = vec![0i64; 100];
    let mut weights = vec![0i64; 100];
    for i in 0..100 {
        values[i] = 50;
        weights[i] = i as i64;
        total_weight += i as i64;
    }
    (values, weights, total_weight)
}

fn same_values_expected() -> Vec<DynamicRangeInfo> {
    vec![
        info(51, 1275, 50, 50, 50.0),
        info(21, 1281, 50, 50, 50.0),
        info(16, 1272, 50, 50, 50.0),
        info(12, 1122, 50, 50, 50.0),
    ]
}

fn with_same_values() -> Result<()> {
    let (mut values, mut weights, total_weight) = same_values_input();
    assert_dynamic_numeric_range_results(
        &mut values,
        &mut weights,
        4,
        total_weight,
        &same_values_expected(),
    );
    Ok(())
}

fn same_weights_input() -> (Vec<i64>, Vec<i64>) {
    let mut values = vec![0i64; 100];
    let mut weights = vec![0i64; 100];
    for i in 0..100 {
        values[i] = i as i64;
        weights[i] = 50;
    }
    (values, weights)
}

fn same_weights_expected() -> Vec<DynamicRangeInfo> {
    vec![
        info(25, 1250, 0, 24, 12.0),
        info(25, 1250, 25, 49, 37.0),
        info(25, 1250, 50, 74, 62.0),
        info(25, 1250, 75, 99, 87.0),
    ]
}

fn with_same_weights() -> Result<()> {
    let (mut values, mut weights) = same_weights_input();

    // Supplying only equal weights should return ranges with equal counts - excluding the last
    // range
    assert_dynamic_numeric_range_results(
        &mut values,
        &mut weights,
        4,
        5000,
        &same_weights_expected(),
    );
    Ok(())
}

fn with_one_value() -> Result<()> {
    let mut values = vec![50i64];
    let mut weights = vec![1i64];
    let expected = vec![info(1, 1, 50, 50, 50.0)];
    assert_dynamic_numeric_range_results(&mut values, &mut weights, 4, 1, &expected);
    Ok(())
}

fn with_one_large_weight() -> Result<()> {
    let mut values = vec![45i64, 32, 52, 14, 455, 342, 53];
    let mut weights = vec![143i64, 23, 1, 52343, 53, 12, 2534];

    // value 14 has its own bin since the weight is large, and the rest of the values fall in the
    // other bin
    let expected = vec![
        info(1, 52343, 14, 14, 14.0),
        info(6, 2766, 32, 455, 163.16666666666666),
    ];
    assert_dynamic_numeric_range_results(&mut values, &mut weights, 4, 55109, &expected);
    Ok(())
}

fn four_values() -> (Vec<i64>, Vec<i64>) {
    (vec![487, 439, 794, 277], vec![59, 508, 736, 560])
}

fn with_large_top_n() -> Result<()> {
    let (mut values, mut weights) = four_values();

    // More requested ranges (TopN) than values should return ranges with weights larger than the
    // average weight - excluding the last range
    let expected = vec![
        info(1, 560, 277, 277, 277.0),
        info(1, 508, 439, 439, 439.0),
        info(2, 795, 487, 794, 640.5),
    ];
    assert_dynamic_numeric_range_results(&mut values, &mut weights, 42, 1863, &expected);
    Ok(())
}

fn with_zero_top_n() -> Result<()> {
    let (mut values, mut weights) = four_values();

    // Zero requested ranges (TopN) should return a empty list of ranges regardless of inputs
    assert_dynamic_numeric_range_results(&mut values, &mut weights, 0, 1863, &[]);
    Ok(())
}

fn with_single_top_n() -> Result<()> {
    let (mut values, mut weights) = four_values();

    // A single requested range (TopN) should return a single range regardless of the weights
    // provided
    let expected = vec![info(4, 1863, 277, 794, 499.25)];
    assert_dynamic_numeric_range_results(&mut values, &mut weights, 1, 1863, &expected);
    Ok(())
}

fn with_two_top_n() -> Result<()> {
    let (mut values, mut weights) = four_values();

    // Two requested ranges (TopN) should return two ranges where the first range's weight is equal
    // or larger than half of the total weight
    let expected = vec![
        info(2, 1068, 277, 439, 358.0),
        info(2, 795, 487, 794, 640.5),
    ];
    assert_dynamic_numeric_range_results(&mut values, &mut weights, 2, 1863, &expected);
    Ok(())
}

fn with_same_weights_shuffled() -> Result<()> {
    let (mut values, mut weights) = same_weights_input();

    // Shuffling the values and weights should not change the answer between runs
    // We expect that returned ranges should come in a strict, deterministic order
    // with the same values and weights
    shuffle_values_weights(&mut values, &mut weights);
    assert_dynamic_numeric_range_results(
        &mut values,
        &mut weights,
        4,
        5000,
        &same_weights_expected(),
    );
    Ok(())
}

fn with_same_values_shuffled() -> Result<()> {
    let (mut values, mut weights, total_weight) = same_values_input();

    // Shuffling the values and weights should not change the answer between runs
    // We expect that returned ranges should come in a strict, deterministic order
    // with the same values and weights
    shuffle_values_weights(&mut values, &mut weights);
    assert_dynamic_numeric_range_results(
        &mut values,
        &mut weights,
        4,
        total_weight,
        &same_values_expected(),
    );
    Ok(())
}

fn with_misplaced_value() -> Result<()> {
    let mut values = vec![
        1i64, 2, 11, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 12, 111, 112, 113, 114, 115,
    ];
    let mut weights = vec![
        2i64, 3, 12, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 13, 112, 113, 114, 115, 116,
    ];

    let expected = vec![
        info(8, 444, 1, 104, 54.5),
        info(4, 430, 105, 108, 106.5),
        info(4, 446, 109, 112, 110.5),
        info(3, 345, 113, 115, 114.0),
    ];
    assert_dynamic_numeric_range_results(&mut values, &mut weights, 4, 1665, &expected);
    Ok(())
}

fn with_same_weights_out_of_order() -> Result<()> {
    let mut values = vec![
        20i64, 15, 59, 49, 13, 93, 72, 21, 36, 81, 57, 1, 90, 79, 16, 51, 7, 17, 25, 63, 12, 5, 83,
        66, 48, 43, 55, 78, 64, 77, 65, 73, 80, 37, 54, 50, 95, 31, 97, 3, 82, 29, 70, 26, 4, 46,
        34, 67, 87, 0, 30, 19, 41, 85, 84, 89, 8, 10, 22, 28, 6, 23, 88, 40, 33, 44, 18, 27, 69,
        38, 91, 98, 62, 14, 35, 2, 92, 47, 94, 75, 32, 99, 86, 71, 74, 24, 52, 96, 9, 58, 39, 76,
        56, 11, 53, 61, 42, 68, 60, 45,
    ];
    let mut weights = vec![50i64; 100];

    // This is testComputeDynamicNumericRangesWithSameWeightsShuffled with seed
    // 9AE79D72C8DD56D8 that failed a previous test
    assert_dynamic_numeric_range_results(
        &mut values,
        &mut weights,
        4,
        5000,
        &same_weights_expected(),
    );
    Ok(())
}

fn with_random_values() -> Result<()> {
    let random = LuceneTestCase::random();
    let array_size = random.next_int_bound(100) as usize;
    let mut values = vec![0i64; array_size];
    let mut weights = vec![0i64; array_size];

    for i in 0..array_size {
        values[i] = random.next_long_bound(1000);
        weights[i] = random.next_long_bound(1000);
    }

    let top_n = random.next_int_bound(100);

    let mut total_weight = 0i64;
    for weight in &weights {
        total_weight += weight;
    }

    assert_dynamic_numeric_range_valid_properties(&mut values, &mut weights, top_n, total_weight);
    Ok(())
}

/// Implementation of Durstenfeld's algorithm for shuffling values and weights.
fn shuffle_values_weights(values: &mut [i64], weights: &mut [i64]) {
    let random = LuceneTestCase::random();
    let mut i = values.len() as i32 - 1;
    while i > 0 {
        let rdm_idx = random.next_int_bound(i + 1) as usize;
        values.swap(i as usize, rdm_idx);
        weights.swap(i as usize, rdm_idx);
        i -= 1;
    }
}

/// `record WeightedPair(long value, long weight)`.
#[derive(Clone, Copy)]
struct WeightedPair {
    value: i64,
    weight: i64,
}

fn assert_dynamic_numeric_range_valid_properties(
    values: &mut [i64],
    weights: &mut [i64],
    top_n: i32,
    total_weight: i64,
) {
    let mut sorted_pairs: Vec<WeightedPair> = values
        .iter()
        .zip(weights.iter())
        .map(|(&value, &weight)| WeightedPair { value, weight })
        .collect();

    sorted_pairs.sort_by(|a, b| a.value.cmp(&b.value).then(a.weight.cmp(&b.weight)));

    let len = values.len();

    let range_weight_target = total_weight as f64 / (top_n.min(len as i32)) as f64;

    let mock_dynamic_range_result =
        DynamicRangeUtil::compute_dynamic_numeric_ranges(values, weights, len, total_weight, top_n);

    // Zero requested ranges (TopN) should return a empty list of ranges regardless of inputs
    if top_n == 0 {
        Assert::assert_true(mock_dynamic_range_result.is_empty());
        return; // Early return; do not check anything else
    }

    // Adjacent ranges do not overlap - only adjacent max-min can overlap
    for pair in mock_dynamic_range_result.windows(2) {
        Assert::assert_true(pair[0].max <= pair[1].min);
    }

    // The count of every range sums to the number of values
    let accu_count: i32 = mock_dynamic_range_result.iter().map(|r| r.count).sum();
    Assert::assert_true(accu_count as usize == len);

    // The sum of every range weight equals the total weight
    let accu_weight: i64 = mock_dynamic_range_result.iter().map(|r| r.weight).sum();
    Assert::assert_true(accu_weight == total_weight);

    // All values appear in atleast one range
    let mut pair_offset = 0usize;
    for range_info in &mock_dynamic_range_result {
        let count = range_info.count as usize;
        for pair in &sorted_pairs[pair_offset..pair_offset + count] {
            let value = pair.value;
            Assert::assert_true(range_info.min <= value && value <= range_info.max);
        }
        pair_offset += count;
    }

    // The minimum/maximum of each range is actually the smallest/largest value
    let mut pair_offset = 0usize;
    for range_info in &mock_dynamic_range_result {
        let count = range_info.count as usize;
        let min = sorted_pairs[pair_offset].value;
        let max = sorted_pairs[pair_offset + count - 1].value;
        Assert::assert_true(range_info.min == min);
        Assert::assert_true(range_info.max == max);
        pair_offset += count;
    }

    let all_but_last =
        &mock_dynamic_range_result[..mock_dynamic_range_result.len().saturating_sub(1)];

    // Weights of each range is over the rangeWeightTarget - exclude last range
    for range_info in all_but_last {
        Assert::assert_true(range_info.weight as f64 >= range_weight_target);
    }

    // Removing the last weight from a range brings it under the rangeWeightTarget - exclude last
    // range
    let mut pair_offset = 0usize;
    for range_info in all_but_last {
        let count = range_info.count as usize;
        let last_weight = sorted_pairs[pair_offset + count - 1].weight;
        pair_offset += count;
        Assert::assert_true(((range_info.weight - last_weight) as f64) < range_weight_target);
    }

    // Centroids for each range are correct
    let mut pair_offset = 0usize;
    for range_info in &mock_dynamic_range_result {
        let count = range_info.count as usize;
        let accu_value: i64 = sorted_pairs[pair_offset..pair_offset + count]
            .iter()
            .map(|p| p.value)
            .sum();
        pair_offset += count;
        Assert::assert_true(range_info.centroid == accu_value as f64 / count as f64);
    }
}

fn assert_dynamic_numeric_range_results(
    values: &mut [i64],
    weights: &mut [i64],
    top_n: i32,
    total_weight: i64,
    expected_dynamic_range_result: &[DynamicRangeInfo],
) {
    let len = values.len();
    let mock_dynamic_range_result =
        DynamicRangeUtil::compute_dynamic_numeric_ranges(values, weights, len, total_weight, top_n);
    assert_equals(
        expected_dynamic_range_result,
        &mock_dynamic_range_result[..],
    );
}
