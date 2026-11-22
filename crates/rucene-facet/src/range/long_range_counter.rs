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

//! Port of `LongRangeCounter.java`, of the Java package
//! `org.apache.lucene.facet.range`.

use std::fmt;

use crate::range::exclusive_long_range_counter::ExclusiveLongRangeCounter;
use crate::range::long_range::LongRange;
use crate::range::overlapping_long_range_counter::OverlappingLongRangeCounter;

/// The state the abstract `LongRangeCounter` keeps for its subclasses.
#[derive(Debug, Clone)]
pub(crate) struct LongRangeCounterBase {
    count_buffer: Vec<i32>,
    /// Track the last counted elementary interval while processing a
    /// multi-valued document.
    pub(crate) multi_valued_doc_last_seen_elementary_interval: i32,
}

impl LongRangeCounterBase {
    /// Equivalent to `LongRangeCounter(int[])`.
    pub(crate) fn new(count_buffer: Vec<i32>) -> Self {
        Self {
            count_buffer,
            multi_valued_doc_last_seen_elementary_interval: 0,
        }
    }

    /// Increment the specified range by one.
    ///
    /// Equivalent to `increment(int)`.
    pub(crate) fn increment(&mut self, range_num: i32) {
        self.count_buffer[range_num as usize] += 1;
    }

    /// Increment the specified range by the specified count.
    ///
    /// Equivalent to `increment(int, int)`.
    pub(crate) fn increment_by(&mut self, range_num: i32, count: i32) {
        self.count_buffer[range_num as usize] += count;
    }

    /// Number of ranges requested by the caller.
    ///
    /// Equivalent to `rangeCount()`.
    pub(crate) fn range_count(&self) -> i32 {
        self.count_buffer.len() as i32
    }
}

/// Counter for numeric ranges.
///
/// Equivalent to `org.apache.lucene.facet.range.LongRangeCounter`. Java's
/// counter increments the caller's `countBuffer` in place; the port's counter
/// owns it and [`into_counts`](Self::into_counts) hands it back.
pub(crate) trait LongRangeCounter {
    /// The shared state.
    fn base(&self) -> &LongRangeCounterBase;

    /// The shared state, mutably.
    fn base_mut(&mut self) -> &mut LongRangeCounterBase;

    /// Takes the counts back.
    fn into_counts(self: Box<Self>) -> Vec<i32>;

    /// Start processing a new doc. It's unnecessary to call this for
    /// single-value cases.
    ///
    /// Equivalent to `startMultiValuedDoc()`.
    fn start_multi_valued_doc(&mut self) {
        self.base_mut().multi_valued_doc_last_seen_elementary_interval = -1;
    }

    /// Finish processing a new doc. Returns whether-or-not the document
    /// contributed a count to at least one range.
    ///
    /// Equivalent to `endMultiValuedDoc()`.
    fn end_multi_valued_doc(&mut self) -> bool;

    /// Count a single valued doc.
    ///
    /// Equivalent to `addSingleValued(long)`.
    fn add_single_valued(&mut self, v: i64) {
        default_add_single_valued(self, v);
    }

    /// Count a multi-valued doc value.
    ///
    /// Equivalent to `addMultiValued(long)`.
    fn add_multi_valued(&mut self, v: i64) {
        if self.base().range_count() == 0 {
            return; // don't bother if there aren't any requested ranges
        }

        let boundaries_len;
        let last_seen = self.base().multi_valued_doc_last_seen_elementary_interval;
        {
            let boundaries = self.boundaries();
            boundaries_len = boundaries.len() as i32;
            // First check if we've already recorded this elementary interval
            // for this doc:
            if last_seen != -1 && v <= boundaries[last_seen as usize] {
                return;
            }
        }

        // Also check if we've already counted the last elementary interval.
        // If so, there's nothing else to count for this doc:
        let next_candidate_elementary_interval = last_seen + 1;
        if next_candidate_elementary_interval == boundaries_len {
            return;
        }

        // Binary search in the range of the next candidate interval up to
        // the last interval:
        let mut lo = next_candidate_elementary_interval;
        let mut hi = boundaries_len - 1;
        loop {
            let mid = ((lo + hi) as u32 >> 1) as i32;
            let (at_mid, after_mid) = {
                let boundaries = self.boundaries();
                (boundaries[mid as usize], boundaries.get(mid as usize + 1).copied())
            };
            if v <= at_mid {
                if mid == next_candidate_elementary_interval {
                    self.process_multi_valued_hit(mid);
                    self.base_mut().multi_valued_doc_last_seen_elementary_interval = mid;
                    return;
                } else {
                    hi = mid - 1;
                }
            } else if v > after_mid.expect("INVARIANT: the last boundary is Long.MAX_VALUE") {
                lo = mid + 1;
            } else {
                let idx = mid + 1;
                self.process_multi_valued_hit(idx);
                self.base_mut().multi_valued_doc_last_seen_elementary_interval = idx;
                return;
            }
        }
    }

    /// Finish processing all documents. This will return the number of docs
    /// that didn't contribute to any ranges (that weren't already reported
    /// when calling `end_multi_valued_doc()`).
    ///
    /// Equivalent to `finish()`.
    fn finish(&mut self) -> i32;

    /// Provide boundary information for elementary intervals (max inclusive
    /// value per interval).
    ///
    /// Equivalent to `boundaries()`.
    fn boundaries(&self) -> &[i64];

    /// Process a single-value "hit" against an elementary interval.
    ///
    /// Equivalent to `processSingleValuedHit(int)`.
    fn process_single_valued_hit(&mut self, elementary_interval_num: i32);

    /// Process a multi-value "hit" against an elementary interval.
    ///
    /// Equivalent to `processMultiValuedHit(int)`.
    fn process_multi_valued_hit(&mut self, elementary_interval_num: i32);
}

/// The base class's `addSingleValued(long)`, which an override calls as
/// `super.addSingleValued(v)`.
pub(crate) fn default_add_single_valued<C: LongRangeCounter + ?Sized>(counter: &mut C, v: i64) {
    // NOTE: this works too, but it's ~6% slower on a simple test with a high
    // freq TermQuery w/ range faceting on wikimediumall:
    // int index = Arrays.binarySearch(boundaries, v);
    // if (index < 0) {
    //   index = -index-1;
    // }
    // increment(index);

    // Binary search to find matched elementary range; we are guaranteed to
    // find a match because the last boundary is Long.MAX_VALUE:
    let len = counter.boundaries().len() as i32;
    let mut lo = 0i32;
    let mut hi = len - 1;
    loop {
        let mid = ((lo + hi) as u32 >> 1) as i32;
        let (at_mid, after_mid) = {
            let boundaries = counter.boundaries();
            (boundaries[mid as usize], boundaries.get(mid as usize + 1).copied())
        };
        if v <= at_mid {
            if mid == 0 {
                counter.process_single_valued_hit(mid);
                return;
            } else {
                hi = mid - 1;
            }
        } else if v > after_mid.expect("INVARIANT: the last boundary is Long.MAX_VALUE") {
            lo = mid + 1;
        } else {
            counter.process_single_valued_hit(mid + 1);
            return;
        }
    }
}

/// Create a `LongRangeCounter` based on the provided ranges.
///
/// Equivalent to `LongRangeCounter.create(LongRange[], int[])`.
pub(crate) fn create(ranges: &[LongRange], count_buffer: Vec<i32>) -> Box<dyn LongRangeCounter> {
    if has_overlapping_ranges(ranges) {
        Box::new(OverlappingLongRangeCounter::new(ranges, count_buffer))
    } else {
        Box::new(ExclusiveLongRangeCounter::new(ranges, count_buffer))
    }
}

/// Equivalent to `hasOverlappingRanges(LongRange[])`.
fn has_overlapping_ranges(ranges: &[LongRange]) -> bool {
    if ranges.is_empty() {
        return false;
    }

    // Copy before sorting so we don't mess with the caller's original
    // ranges (stable, as `Arrays.sort` of objects):
    let mut sorted_ranges: Vec<&LongRange> = ranges.iter().collect();
    sorted_ranges.sort_by_key(|r| r.min);

    let mut previous_max = sorted_ranges[0].max;
    for range in &sorted_ranges[1..] {
        // Ranges overlap if the next min is <= the previous max (note that
        // LongRange models closed ranges, so equal limit points are
        // considered overlapping):
        if range.min <= previous_max {
            return true;
        }
        previous_max = range.max;
    }

    false
}

/// Inclusive range used by the counters.
///
/// Equivalent to `org.apache.lucene.facet.range.LongRangeCounter.InclusiveRange`.
#[derive(Debug, Clone, Copy)]
pub struct InclusiveRange {
    /// Inclusive start.
    pub start: i64,
    /// Inclusive end.
    pub end: i64,
}

impl InclusiveRange {
    /// Equivalent to `InclusiveRange(long, long)`.
    pub fn new(start: i64, end: i64) -> Self {
        debug_assert!(end >= start);
        Self { start, end }
    }
}

impl fmt::Display for InclusiveRange {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} to {}", self.start, self.end)
    }
}
impl LongRangeCounterBase {
    /// The count buffer, handed back to the caller.
    pub(crate) fn count_buffer_into(self) -> Vec<i32> {
        self.count_buffer
    }
}
