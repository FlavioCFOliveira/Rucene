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

//! Port of `OverlappingLongRangeCounter.java`, of the Java package
//! `org.apache.lucene.facet.range`.

use std::collections::BTreeMap;
use std::fmt;

use rucene::util::{BitSet, FixedBitSet};

use crate::range::long_range::LongRange;
use crate::range::long_range_counter::{InclusiveRange, LongRangeCounter, LongRangeCounterBase};
use crate::taxonomy::facet_label::arrays_to_string;

/// This implementation supports requested ranges that overlap.
///
/// Equivalent to `org.apache.lucene.facet.range.OverlappingLongRangeCounter`.
pub(crate) struct OverlappingLongRangeCounter {
    base: LongRangeCounterBase,
    /// segment tree root node
    root: LongRangeNode,
    /// elementary interval boundaries used for efficient counting (bsearch
    /// to find interval)
    boundaries: Vec<i64>,
    /// whether-or-not there are elementary interval counts that still need
    /// to be rolled up at the end
    has_unflushed_counts: bool,
    // Needed only for counting single-valued docs:
    /// counts seen in each elementary interval
    single_valued_elementary_interval_counts: Option<Vec<i32>>,
    // Needed only for counting multi-valued docs:
    /// whether-or-not an elementary interval has seen at least one match for
    /// a single doc
    multi_valued_doc_elementary_interval_hits: Option<FixedBitSet>,
    /// whether-or-not a requested range has seen at least one match for a
    /// single doc
    multi_valued_doc_range_hits: Option<FixedBitSet>,
    // Used during rollup
    elementary_interval_upto: usize,
    /// number of counted documents that haven't matched any requested ranges
    missing_count: i32,
}

impl OverlappingLongRangeCounter {
    /// Equivalent to `OverlappingLongRangeCounter(LongRange[], int[])`.
    pub(crate) fn new(ranges: &[LongRange], count_buffer: Vec<i32>) -> Self {
        // Build elementary intervals:
        let elementary_intervals = Self::build_elementary_intervals(ranges);

        // Build binary tree on top of intervals:
        let mut root = Self::split(0, elementary_intervals.len(), &elementary_intervals);

        // Set outputs, so we know which range to output for each node in the
        // tree:
        for (i, range) in ranges.iter().enumerate() {
            root.add_outputs(i as i32, range);
        }

        // Keep track of elementary interval max boundaries for bsearch:
        let boundaries = elementary_intervals.iter().map(|r| r.end).collect();

        Self {
            base: LongRangeCounterBase::new(count_buffer),
            root,
            boundaries,
            has_unflushed_counts: false,
            single_valued_elementary_interval_counts: None,
            multi_valued_doc_elementary_interval_hits: None,
            multi_valued_doc_range_hits: None,
            elementary_interval_upto: 0,
            missing_count: 0,
        }
    }

    /// Equivalent to `split(int, int, List)`.
    fn split(start: usize, end: usize, elementary_intervals: &[InclusiveRange]) -> LongRangeNode {
        if start == end - 1 {
            // leaf
            let range = elementary_intervals[start];
            LongRangeNode::new(range.start, range.end, None, None, start as i32)
        } else {
            let mid = (start + end) >> 1;
            let left = Self::split(start, mid, elementary_intervals);
            let right = Self::split(mid, end, elementary_intervals);
            LongRangeNode::new(
                left.start,
                right.end,
                Some(Box::new(left)),
                Some(Box::new(right)),
                -1,
            )
        }
    }

    /// Rolls up all the single-valued doc counts.
    ///
    /// Equivalent to `rollupSingleValued(LongRangeNode, boolean)`.
    fn rollup_single_valued(
        node: &LongRangeNode,
        mut saw_outputs: bool,
        counts: &[i32],
        upto: &mut usize,
        missing_count: &mut i32,
        base: &mut LongRangeCounterBase,
    ) -> i32 {
        let mut count;
        saw_outputs |= node.outputs.is_some();
        if let (Some(left), Some(right)) = (&node.left, &node.right) {
            count =
                Self::rollup_single_valued(left, saw_outputs, counts, upto, missing_count, base);
            count +=
                Self::rollup_single_valued(right, saw_outputs, counts, upto, missing_count, base);
        } else {
            // Leaf:
            count = counts[*upto];
            *upto += 1;
            if !saw_outputs {
                // This is a missing count (no output ranges were seen "above"
                // us):
                *missing_count += count;
            }
        }
        if let Some(outputs) = &node.outputs {
            for &range_index in outputs {
                base.increment_by(range_index, count);
            }
        }

        count
    }

    /// Equivalent to `rollupMultiValued(LongRangeNode)`.
    fn rollup_multi_valued(
        node: &LongRangeNode,
        hits: &FixedBitSet,
        upto: &mut usize,
        range_hits: &mut FixedBitSet,
    ) -> bool {
        let mut contained_hit;
        if let (Some(left), Some(right)) = (&node.left, &node.right) {
            contained_hit = Self::rollup_multi_valued(left, hits, upto, range_hits);
            contained_hit |= Self::rollup_multi_valued(right, hits, upto, range_hits);
        } else {
            // Leaf:
            contained_hit = hits.get(*upto);
            *upto += 1;
        }
        if contained_hit {
            if let Some(outputs) = &node.outputs {
                for &range_index in outputs {
                    range_hits.set(range_index as usize);
                }
            }
        }

        contained_hit
    }

    /// Equivalent to `buildElementaryIntervals(LongRange[])`.
    fn build_elementary_intervals(ranges: &[LongRange]) -> Vec<InclusiveRange> {
        // Maps all range inclusive endpoints to int flags; 1 = start of
        // interval, 2 = end of interval. We need to track the start vs end
        // case separately because if a given point is both, then it must be
        // its own elementary interval:
        let mut ends_map: BTreeMap<i64, i32> = BTreeMap::new();

        ends_map.insert(i64::MIN, 1);
        ends_map.insert(i64::MAX, 2);

        for range in ranges {
            *ends_map.entry(range.min).or_insert(0) |= 1;
            *ends_map.entry(range.max).or_insert(0) |= 2;
        }

        let ends_list: Vec<i64> = ends_map.keys().copied().collect();

        // Build elementaryIntervals (a 1D Venn diagram):
        let mut elementary_intervals = Vec::new();
        let mut upto = 1;
        let mut v = ends_list[0];
        let mut prev;
        if ends_map[&v] == 3 {
            elementary_intervals.push(InclusiveRange::new(v, v));
            prev = v.wrapping_add(1);
        } else {
            prev = v;
        }

        while upto < ends_list.len() {
            v = ends_list[upto];
            let flags = ends_map[&v];
            if flags == 3 {
                // This point is both an end and a start; we need to separate
                // it:
                if v > prev {
                    elementary_intervals.push(InclusiveRange::new(prev, v - 1));
                }
                elementary_intervals.push(InclusiveRange::new(v, v));
                prev = v.wrapping_add(1);
            } else if flags == 1 {
                // This point is only the start of an interval; attach it to
                // next interval:
                if v > prev {
                    elementary_intervals.push(InclusiveRange::new(prev, v - 1));
                }
                prev = v;
            } else {
                debug_assert!(flags == 2);
                // This point is only the end of an interval; attach it to last
                // interval:
                elementary_intervals.push(InclusiveRange::new(prev, v));
                prev = v.wrapping_add(1);
            }
            upto += 1;
        }

        elementary_intervals
    }
}

impl LongRangeCounter for OverlappingLongRangeCounter {
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
        // Lazy init a bitset to track the elementary intervals we see of a
        // multi-valued doc:
        match &mut self.multi_valued_doc_elementary_interval_hits {
            None => {
                self.multi_valued_doc_elementary_interval_hits =
                    Some(FixedBitSet::new(self.boundaries.len()));
            }
            Some(hits) => hits.clear_all(),
        }
    }

    fn end_multi_valued_doc(&mut self) -> bool {
        debug_assert!(
            self.multi_valued_doc_elementary_interval_hits.is_some(),
            "must call startDoc() first"
        );

        // Short-circuit if the caller didn't specify any ranges to count:
        if self.base.range_count() == 0 {
            return false;
        }

        // Do the rollup for this doc:

        // Lazy init a bitset to track the requested ranges seen for this
        // multi-valued doc:
        let range_count = self.base.range_count() as usize;
        match &mut self.multi_valued_doc_range_hits {
            None => self.multi_valued_doc_range_hits = Some(FixedBitSet::new(range_count)),
            Some(hits) => hits.clear_all(),
        }
        self.elementary_interval_upto = 0;
        let mut range_hits = self
            .multi_valued_doc_range_hits
            .take()
            .expect("INVARIANT: set above");
        let hits = self
            .multi_valued_doc_elementary_interval_hits
            .as_ref()
            .expect("INVARIANT: must call startDoc() first");
        Self::rollup_multi_valued(
            &self.root,
            hits,
            &mut self.elementary_interval_upto,
            &mut range_hits,
        );

        // Actually increment the count for each matching range, and see if
        // the doc contributed to at least one:
        let mut doc_contributed_to_at_least_one_range = false;
        let length = range_hits.length() as i32;
        let mut i = range_hits.next_set_bit(0);
        while i < length {
            self.base.increment(i);
            doc_contributed_to_at_least_one_range = true;
            i += 1;
            if i < length {
                i = range_hits.next_set_bit(i);
            }
        }
        self.multi_valued_doc_range_hits = Some(range_hits);

        doc_contributed_to_at_least_one_range
    }

    fn finish(&mut self) -> i32 {
        if self.has_unflushed_counts {
            // Rollup any outstanding counts from single-valued cases:
            self.missing_count = 0;
            self.elementary_interval_upto = 0;
            let counts = self
                .single_valued_elementary_interval_counts
                .take()
                .unwrap_or_default();
            Self::rollup_single_valued(
                &self.root,
                false,
                &counts,
                &mut self.elementary_interval_upto,
                &mut self.missing_count,
                &mut self.base,
            );
            self.single_valued_elementary_interval_counts = Some(counts);

            self.missing_count
        } else {
            0
        }
    }

    fn boundaries(&self) -> &[i64] {
        &self.boundaries
    }

    fn process_single_valued_hit(&mut self, elementary_interval_num: i32) {
        // Lazy init:
        let len = self.boundaries.len();
        self.single_valued_elementary_interval_counts
            .get_or_insert_with(|| vec![0; len])[elementary_interval_num as usize] += 1;
        self.has_unflushed_counts = true;
    }

    fn process_multi_valued_hit(&mut self, elementary_interval_num: i32) {
        self.multi_valued_doc_elementary_interval_hits
            .as_mut()
            .expect("must call startDoc() first")
            .set(elementary_interval_num as usize);
    }
}

/// Holds one node of the segment tree.
///
/// Equivalent to `org.apache.lucene.facet.range.OverlappingLongRangeCounter.LongRangeNode`.
#[derive(Debug, Clone)]
pub struct LongRangeNode {
    left: Option<Box<LongRangeNode>>,
    right: Option<Box<LongRangeNode>>,
    // Our range, inclusive:
    start: i64,
    end: i64,
    // Which range indices to output when a query goes through this node:
    outputs: Option<Vec<i32>>,
    // If we are a leaf, the index into elementary ranges that we point to:
    // (assigned and never read, as in Java)
    #[allow(dead_code)]
    elementary_interval_index: i32,
}

impl LongRangeNode {
    /// Equivalent to `LongRangeNode(long, long, LongRangeNode, LongRangeNode,
    /// int)`.
    pub fn new(
        start: i64,
        end: i64,
        left: Option<Box<LongRangeNode>>,
        right: Option<Box<LongRangeNode>>,
        elementary_interval_index: i32,
    ) -> Self {
        Self {
            left,
            right,
            start,
            end,
            outputs: None,
            elementary_interval_index,
        }
    }

    /// Equivalent to `indent(StringBuilder, int)`.
    fn indent(sb: &mut String, depth: usize) {
        for _ in 0..depth {
            sb.push_str("  ");
        }
    }

    /// Recursively assigns range outputs to each node.
    ///
    /// Equivalent to `addOutputs(int, LongRange)`.
    fn add_outputs(&mut self, index: i32, range: &LongRange) {
        if self.start >= range.min && self.end <= range.max {
            // Our range is fully included in the incoming range; add to our
            // output list:
            self.outputs.get_or_insert_with(Vec::new).push(index);
        } else if let (Some(left), Some(right)) = (&mut self.left, &mut self.right) {
            // Recurse:
            left.add_outputs(index, range);
            right.add_outputs(index, range);
        }
    }

    /// Equivalent to `toString(StringBuilder, int)`.
    fn to_string_into(&self, sb: &mut String, depth: usize) {
        Self::indent(sb, depth);
        if self.left.is_none() {
            debug_assert!(self.right.is_none());
            sb.push_str(&format!("leaf: {} to {}", self.start, self.end));
        } else {
            sb.push_str(&format!("node: {} to {}", self.start, self.end));
        }
        if let Some(outputs) = &self.outputs {
            sb.push_str(" outputs=");
            let strs: Vec<String> = outputs.iter().map(|o| o.to_string()).collect();
            sb.push_str(&arrays_to_string(&strs));
        }
        sb.push('\n');

        if let (Some(left), Some(right)) = (&self.left, &self.right) {
            left.to_string_into(sb, depth + 1);
            right.to_string_into(sb, depth + 1);
        }
    }
}

impl fmt::Display for LongRangeNode {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sb = String::new();
        self.to_string_into(&mut sb, 0);
        f.write_str(&sb)
    }
}
