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

//! Port of `DynamicRangeUtil.java`, of the Java package
//! `org.apache.lucene.facet.range`.

use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::{LuceneError, Result};
use rucene::search::long_values_source::LongValuesSource;
use rucene::search::task_executor::{Executor, TaskValue};
use rucene::search::NO_MORE_DOCS;

use crate::facets_collector::{FacetsCollector, MatchingDocs};

/// Methods to create dynamic ranges for numeric fields.
///
/// Equivalent to `org.apache.lucene.facet.range.DynamicRangeUtil`.
pub struct DynamicRangeUtil;

impl DynamicRangeUtil {
    /// Construct dynamic ranges using the specified weight field to generate
    /// equi-weight range for the specified numeric bin field.
    ///
    /// Equivalent to `computeDynamicRanges(String, LongValuesSource,
    /// LongValuesSource, FacetsCollector, int, ExecutorService)`.
    pub fn compute_dynamic_ranges(
        weight_field_name: &str,
        weight_value_source: Arc<dyn LongValuesSource>,
        field_value_source: Arc<dyn LongValuesSource>,
        facets_collector: &FacetsCollector,
        top_n: i32,
        exec: &Arc<dyn Executor>,
    ) -> Result<Option<Vec<DynamicRangeInfo>>> {
        let matching_docs_list = facets_collector.get_matching_docs()?;
        let total_doc: i32 = matching_docs_list.iter().map(|m| m.total_hits).sum();
        let mut values = vec![0i64; total_doc.max(0) as usize];
        let mut weights = vec![0i64; total_doc.max(0) as usize];
        let mut total_weight = 0i64;
        let mut overall_length = 0usize;

        let mut futures = Vec::new();
        let mut tasks: Vec<Arc<Mutex<SegmentOutput>>> = Vec::new();
        for matching_docs in matching_docs_list {
            if matching_docs.total_hits > 0 {
                let segment_output = Arc::new(Mutex::new(SegmentOutput::new(matching_docs.total_hits)));
                let task = SegmentTask::new(
                    matching_docs.clone(),
                    Arc::clone(&field_value_source),
                    Arc::clone(&weight_value_source),
                    Arc::clone(&segment_output),
                );
                tasks.push(segment_output);
                futures.push(
                    exec.submit(Box::new(move || {
                        task.call()?;
                        Ok(Box::new(()) as TaskValue)
                    }))
                    .map_err(|e| LuceneError::unchecked("java.util.concurrent.RejectedExecutionException", e.message))?,
                );
            }
        }

        for future in futures {
            // `IOUtils.rethrowAlways(ee.getCause())`.
            future.get()?;
        }

        for task in &tasks {
            let cur_segment_output = task.lock().unwrap_or_else(PoisonError::into_inner);
            debug_assert!(cur_segment_output.values.len() == cur_segment_output.weights.len());
            total_weight = cur_segment_output
                .segment_total_weight
                .checked_add(total_weight)
                .ok_or_else(|| {
                    LuceneError::illegal_argument_with_cause(
                        format!("weight field \"{weight_field_name}\": long totalWeight value out of bounds"),
                        Arc::new(LuceneError::integer_overflow()),
                    )
                })?;
            let curr_segment_len = cur_segment_output.segment_idx;
            values[overall_length..overall_length + curr_segment_len]
                .copy_from_slice(&cur_segment_output.values[..curr_segment_len]);
            weights[overall_length..overall_length + curr_segment_len]
                .copy_from_slice(&cur_segment_output.weights[..curr_segment_len]);
            overall_length += curr_segment_len;
        }
        Ok(Some(Self::compute_dynamic_numeric_ranges(
            &mut values,
            &mut weights,
            overall_length,
            total_weight,
            top_n,
        )))
    }

    /// Compute dynamic numeric ranges using weights.
    ///
    /// Equivalent to `computeDynamicNumericRanges(long[], long[], int, long,
    /// int)`; as in Java, `values` and `weights` are sorted in place.
    pub fn compute_dynamic_numeric_ranges(
        values: &mut [i64],
        weights: &mut [i64],
        len: usize,
        total_weight: i64,
        top_n: i32,
    ) -> Vec<DynamicRangeInfo> {
        debug_assert!(values.len() == weights.len() && len <= values.len());
        debug_assert!(top_n >= 0);
        let mut dynamic_range_result = Vec::new();
        if len == 0 || top_n == 0 {
            return dynamic_range_result;
        }

        // `InPlaceMergeSorter`: a stable sort by value then weight.
        let mut pairs: Vec<(i64, i64)> = (0..len).map(|i| (values[i], weights[i])).collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (i, (v, w)) in pairs.into_iter().enumerate() {
            values[i] = v;
            weights[i] = w;
        }

        let mut accu_weight = 0i64;
        let mut value_sum = 0i64;
        let mut count = 0i32;
        let mut min_idx = 0usize;
        let range_weight_target = total_weight as f64 / (top_n as usize).min(len) as f64;

        for i in 0..len {
            accu_weight = accu_weight.wrapping_add(weights[i]);
            value_sum = value_sum.wrapping_add(values[i]);
            count += 1;

            if accu_weight as f64 >= range_weight_target {
                dynamic_range_result.push(DynamicRangeInfo::new(
                    count,
                    accu_weight,
                    values[min_idx],
                    values[i],
                    value_sum as f64 / count as f64,
                ));
                count = 0;
                accu_weight = 0;
                value_sum = 0;
                min_idx = i + 1;
            }
        }

        // capture the remaining values to create the last range
        if min_idx < len {
            dynamic_range_result.push(DynamicRangeInfo::new(
                count,
                accu_weight,
                values[min_idx],
                values[len - 1],
                value_sum as f64 / count as f64,
            ));
        }
        dynamic_range_result
    }
}

/// Runnable task that computes the counts for a single segment.
///
/// Equivalent to `org.apache.lucene.facet.range.DynamicRangeUtil.SegmentTask`.
/// Java takes the iterator over the matching docs in the constructor; it
/// is taken when the task runs here, as a doc iterator is not `Send`.
pub(crate) struct SegmentTask {
    matching_docs: MatchingDocs,
    field_value_source: Arc<dyn LongValuesSource>,
    weight_value_source: Arc<dyn LongValuesSource>,
    segment_output: Arc<Mutex<SegmentOutput>>,
}

impl SegmentTask {
    /// Equivalent to `SegmentTask(MatchingDocs, LongValuesSource,
    /// LongValuesSource, SegmentOutput)`.
    fn new(
        matching_docs: MatchingDocs,
        field_value_source: Arc<dyn LongValuesSource>,
        weight_value_source: Arc<dyn LongValuesSource>,
        segment_output: Arc<Mutex<SegmentOutput>>,
    ) -> Self {
        Self {
            matching_docs,
            field_value_source,
            weight_value_source,
            segment_output,
        }
    }

    /// Equivalent to `call()`.
    fn call(&self) -> Result<()> {
        let mut matching_parent_docs_itr = self.matching_docs.bits.iterator()?;
        let mut field_value = Arc::clone(&self.field_value_source).get_values(&self.matching_docs.context, None)?;
        let mut weight_value = Arc::clone(&self.weight_value_source).get_values(&self.matching_docs.context, None)?;
        let mut segment_output = self.segment_output.lock().unwrap_or_else(PoisonError::into_inner);
        let mut doc = matching_parent_docs_itr.next_doc()?;
        while doc != NO_MORE_DOCS {
            // If this doc doesn't have a weight, we skip it.
            if !field_value.advance_exact(doc)? || !weight_value.advance_exact(doc)? {
                doc = matching_parent_docs_itr.next_doc()?;
                continue;
            }

            let cur_value = field_value.long_value()?;

            let cur_weight = weight_value.long_value()?;
            // We skip weights equal to zero, otherwise they can skew the
            // ranges. Imagine all the weights were zero - any ranges would
            // be valid.
            if cur_weight == 0 {
                doc = matching_parent_docs_itr.next_doc()?;
                continue;
            }

            let idx = segment_output.segment_idx;
            segment_output.values[idx] = cur_value;
            segment_output.weights[idx] = cur_weight;
            segment_output.segment_total_weight = segment_output
                .segment_total_weight
                .checked_add(cur_weight)
                .ok_or_else(|| {
                    LuceneError::illegal_argument_with_cause(
                        "segment long totalWeight value out of bounds",
                        Arc::new(LuceneError::integer_overflow()),
                    )
                })?;
            segment_output.segment_idx += 1;
            doc = matching_parent_docs_itr.next_doc()?;
        }
        Ok(())
    }
}

/// Holds field value array, weight array, totalWeight, segmentIdx for each
/// segment.
///
/// Equivalent to `org.apache.lucene.facet.range.DynamicRangeUtil.SegmentOutput`.
pub(crate) struct SegmentOutput {
    values: Vec<i64>,
    weights: Vec<i64>,
    segment_total_weight: i64,
    segment_idx: usize,
}

impl SegmentOutput {
    /// Equivalent to `SegmentOutput(int)`.
    fn new(hits_length: i32) -> Self {
        Self {
            values: vec![0; hits_length.max(0) as usize],
            weights: vec![0; hits_length.max(0) as usize],
            segment_total_weight: 0,
            segment_idx: 0,
        }
    }
}

/// Holds parameters of a dynamic numeric range.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.range.DynamicRangeUtil.DynamicRangeInfo`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicRangeInfo {
    /// The number of items in the range.
    pub count: i32,
    /// The summed weight of the items in the range.
    pub weight: i64,
    /// The lower bound of the range (inclusive).
    pub min: i64,
    /// Upper bound of the range (inclusive).
    pub max: i64,
    /// The average value in the range.
    pub centroid: f64,
}

impl DynamicRangeInfo {
    /// The record's canonical constructor.
    pub fn new(count: i32, weight: i64, min: i64, max: i64, centroid: f64) -> Self {
        Self {
            count,
            weight,
            min,
            max,
            centroid,
        }
    }
}
