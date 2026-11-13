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

//! Base utility class for implementing a `CharFilter`, ported from
//! `org.apache.lucene.analysis.charfilter.BaseCharFilter`.

use rucene::analysis::CharReader;
use rucene::util::ArrayUtil;

/// Base utility class for implementing a `CharFilter`. You subclass this, and
/// then record mappings by calling
/// [`add_off_correct_map`](Self::add_off_correct_map), and then invoke the
/// correct method to correct an offset.
///
/// Equivalent to `org.apache.lucene.analysis.charfilter.BaseCharFilter`, an
/// abstract `CharFilter`. Rust has no class inheritance: a char filter
/// "subclass" holds a `BaseCharFilter`, which carries the inherited state
/// (`CharFilter.input` and the correction map), and implements
/// [`CharFilter::correct`](rucene::analysis::CharFilter::correct) with
/// [`correct`](Self::correct).
#[derive(Debug)]
pub struct BaseCharFilter {
    /// `CharFilter.input`: the wrapped reader.
    input: Box<dyn CharReader>,
    offsets: Option<Vec<i32>>,
    diffs: Option<Vec<i32>>,
    size: usize,
}

/// `Arrays.binarySearch(int[] a, int fromIndex, int toIndex, int key)`: the
/// index of `key`, or `-(insertion point) - 1`.
fn java_binary_search(a: &[i32], from_index: usize, to_index: usize, key: i32) -> i32 {
    let mut low = from_index as i32;
    let mut high = to_index as i32 - 1;
    while low <= high {
        let mid = ((low as u32 + high as u32) >> 1) as i32;
        let mid_val = a[mid as usize];
        if mid_val < key {
            low = mid + 1;
        } else if mid_val > key {
            high = mid - 1;
        } else {
            return mid; // key found
        }
    }
    -(low + 1) // key not found.
}

/// `Arrays.toString(int[])`: `[a, b, c]`.
fn java_array_to_string(a: &[i32]) -> String {
    let items: Vec<String> = a.iter().map(i32::to_string).collect();
    format!("[{}]", items.join(", "))
}

impl BaseCharFilter {
    /// Equivalent to `BaseCharFilter(Reader)`, `super(in)`: `input` is the
    /// reader the filter wraps.
    pub fn new(input: Box<dyn CharReader>) -> Self {
        Self {
            input,
            offsets: None,
            diffs: None,
            size: 0,
        }
    }

    /// The wrapped reader, Java's `CharFilter.input`.
    pub fn input(&self) -> &dyn CharReader {
        self.input.as_ref()
    }

    /// The wrapped reader, Java's `CharFilter.input`, for reading.
    pub fn input_mut(&mut self) -> &mut dyn CharReader {
        self.input.as_mut()
    }

    /// Retrieve the corrected offset.
    ///
    /// Equivalent to `BaseCharFilter.correct(int)`.
    pub fn correct(&self, current_off: i32) -> i32 {
        let (Some(offsets), Some(diffs)) = (&self.offsets, &self.diffs) else {
            return current_off;
        };

        let mut index = java_binary_search(offsets, 0, self.size, current_off);
        if index < -1 {
            index = -2 - index;
        }

        let diff = if index < 0 { 0 } else { diffs[index as usize] };
        current_off + diff
    }

    /// Equivalent to `BaseCharFilter.getLastCumulativeDiff()`.
    pub fn get_last_cumulative_diff(&self) -> i32 {
        match &self.diffs {
            None => 0,
            Some(diffs) => diffs[self.size - 1],
        }
    }

    /// Adds an offset correction mapping at the given output stream offset.
    ///
    /// Assumption: the offset given with each successive call to this method
    /// will not be smaller than the offset given at the previous invocation.
    ///
    /// * `off` The output stream offset at which to apply the correction
    /// * `cumulative_diff` The input offset is given by adding this to the
    ///   output offset
    ///
    /// Equivalent to `BaseCharFilter.addOffCorrectMap(int, int)`; Java's
    /// `assert` is a debug assertion, with Java's message.
    pub fn add_off_correct_map(&mut self, off: i32, cumulative_diff: i32) {
        let offsets = self.offsets.get_or_insert_with(|| vec![0; 64]);
        let diffs = self.diffs.get_or_insert_with(|| vec![0; 64]);
        if self.size == offsets.len() {
            *offsets = ArrayUtil::grow_int(offsets, offsets.len() + 1);
            *diffs = ArrayUtil::grow_int(diffs, diffs.len() + 1);
        }

        debug_assert!(
            self.size == 0 || off >= offsets[self.size - 1],
            "Offset #{}({}) is less than the last recorded offset {}\n{}\n{}",
            self.size,
            off,
            offsets[self.size - 1],
            java_array_to_string(offsets),
            java_array_to_string(diffs)
        );

        if self.size == 0 || off != offsets[self.size - 1] {
            offsets[self.size] = off;
            diffs[self.size] = cumulative_diff;
            self.size += 1;
        } else {
            // Overwrite the diff at the last recorded offset
            diffs[self.size - 1] = cumulative_diff;
        }
    }
}
