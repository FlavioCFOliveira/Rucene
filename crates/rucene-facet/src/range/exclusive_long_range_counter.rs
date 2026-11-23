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

//! Port of `ExclusiveLongRangeCounter.java`, of the Java package
//! `org.apache.lucene.facet.range`.

use crate::range::long_range::LongRange;
use crate::range::long_range_counter::{
    default_add_single_valued, InclusiveRange, LongRangeCounter, LongRangeCounterBase,
};

/// This implementation assumes the requested ranges _do not_ overlap.
///
/// Equivalent to `org.apache.lucene.facet.range.ExclusiveLongRangeCounter`.
pub(crate) struct ExclusiveLongRangeCounter {
    base: LongRangeCounterBase,
    /// elementary interval boundaries used for efficient counting (bsearch
    /// to find interval)
    boundaries: Vec<i64>,
    /// original range number each elementary interval corresponds to (index
    /// into countBuffer)
    range_nums: Vec<i32>,
    /// number of counted documents that haven't matched any requested ranges
    missing_count: i32,
    /// whether-or-not the multi-valued doc currently being counted has
    /// matched any ranges
    multi_valued_doc_matched_range: bool,
}

/// Simple container for a requested range and its original position.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.range.ExclusiveLongRangeCounter.LongRangeAndPos`.
#[derive(Debug, Clone)]
pub(crate) struct LongRangeAndPos {
    range: LongRange,
    pos: i32,
}

impl ExclusiveLongRangeCounter {
    /// Equivalent to `ExclusiveLongRangeCounter(LongRange[], int[])`.
    pub(crate) fn new(ranges: &[LongRange], count_buffer: Vec<i32>) -> Self {
        // Create a copy of the requested ranges, sorted by min, and keeping
        // track of the original position (stable sort):
        let mut sorted_ranges: Vec<LongRangeAndPos> = ranges
            .iter()
            .enumerate()
            .map(|(i, r)| LongRangeAndPos {
                range: r.clone(),
                pos: i as i32,
            })
            .collect();
        sorted_ranges.sort_by_key(|r| r.range.min);

        // Create elementary intervals, which include requested ranges and
        // "gaps" in-between:
        let elementary_intervals = Self::build_elementary_intervals(&sorted_ranges);

        // Keep track of elementary interval boundary ends (for bsearching)
        // along with the requested range they map back to (and -1 when they
        // map to a "gap" range):
        let mut boundaries = vec![0i64; elementary_intervals.len()];
        let mut range_nums = vec![-1i32; elementary_intervals.len()];
        let mut curr_range = 0usize;
        for i in 0..boundaries.len() {
            boundaries[i] = elementary_intervals[i].end;
            if curr_range < sorted_ranges.len() {
                let curr = &sorted_ranges[curr_range];
                if boundaries[i] == curr.range.max {
                    range_nums[i] = curr.pos;
                    curr_range += 1;
                }
            }
        }

        Self {
            base: LongRangeCounterBase::new(count_buffer),
            boundaries,
            range_nums,
            missing_count: 0,
            multi_valued_doc_matched_range: false,
        }
    }

    /// Equivalent to `buildElementaryIntervals(LongRangeAndPos[])`.
    fn build_elementary_intervals(sorted_ranges: &[LongRangeAndPos]) -> Vec<InclusiveRange> {
        let mut elementary_intervals = Vec::new();
        let mut prev = i64::MIN;
        for range in sorted_ranges {
            if range.range.min > prev {
                // add a "gap" range preceding requested range if necessary:
                elementary_intervals.push(InclusiveRange::new(prev, range.range.min - 1));
            }
            // add the requested range:
            elementary_intervals.push(InclusiveRange::new(range.range.min, range.range.max));
            prev = range.range.max.wrapping_add(1);
        }
        if let Some(last) = elementary_intervals.last() {
            let last_end = last.end;
            if last_end < i64::MAX {
                elementary_intervals.push(InclusiveRange::new(last_end + 1, i64::MAX));
            }
        } else {
            // If no ranges were requested, create a single entry from MIN_VALUE
            // to MAX_VALUE:
            elementary_intervals.push(InclusiveRange::new(i64::MIN, i64::MAX));
        }

        elementary_intervals
    }
}

impl LongRangeCounter for ExclusiveLongRangeCounter {
    fn base(&self) -> &LongRangeCounterBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut LongRangeCounterBase {
        &mut self.base
    }

    fn into_counts(self: Box<Self>) -> Vec<i32> {
        self.base.count_buffer_into()
    }

    fn start_multi_valued_doc(&mut self) {
        self.base.multi_valued_doc_last_seen_elementary_interval = -1;
        self.multi_valued_doc_matched_range = false;
    }

    fn end_multi_valued_doc(&mut self) -> bool {
        self.multi_valued_doc_matched_range
    }

    fn add_single_valued(&mut self, v: i64) {
        if self.base.range_count() == 0 {
            self.missing_count += 1;
            return;
        }

        default_add_single_valued(self, v);
    }

    fn finish(&mut self) -> i32 {
        // Nothing much to do in this case since we're able to count directly
        // into the requested ranges as we go in this implementation. Just
        // report any missing count:
        self.missing_count
    }

    fn boundaries(&self) -> &[i64] {
        &self.boundaries
    }

    fn process_single_valued_hit(&mut self, elementary_interval_num: i32) {
        let range_num = self.range_nums[elementary_interval_num as usize];
        if range_num != -1 {
            // The elementary interval we matched against corresponds to a
            // requested range, so increment it:
            self.base.increment(range_num);
        } else {
            // The matched elementary interval is a "gap" range, so the doc
            // isn't present in any requested ranges:
            self.missing_count += 1;
        }
    }

    fn process_multi_valued_hit(&mut self, elementary_interval_num: i32) {
        let range_num = self.range_nums[elementary_interval_num as usize];
        if range_num != -1 {
            // The elementary interval we matched against corresponds to a
            // requested range, so increment it. We can do this without
            // fear of double-counting since we know the requested ranges
            // don't overlap:
            self.base.increment(range_num);
            self.multi_valued_doc_matched_range = true;
        }
    }
}
