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

//! Port of `org.apache.lucene.tests.index.PointsStackTracker`: a simple
//! utility class to track the current BKD stack based solely on calls to
//! `IntersectVisitor.compare`.

use randomizedtesting::jdk::exception::Throwable;

/// Port of `org.apache.lucene.tests.index.PointsStackTracker`.
#[derive(Debug, Clone)]
pub struct PointsStackTracker {
    num_dims: i32,
    bytes_per_dim: i32,
    /// The stack of cells, outermost first.
    pub stack: Vec<Cell>,
}

/// Port of the inner class `PointsStackTracker.Cell`: an individual BKD
/// stack frame. Java's inner instance reaches the tracker's `numDims` and
/// `bytesPerDim`; the cell keeps a copy of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// A copy of the cell's minimum packed value.
    pub min_packed_value: Vec<u8>,
    /// A copy of the cell's maximum packed value.
    pub max_packed_value: Vec<u8>,
    num_dims: i32,
    bytes_per_dim: i32,
}

impl Cell {
    /// Whether `other` lies within this cell in every dimension: neither
    /// `other.min < min` nor `other.max > max`, comparing the dimension's
    /// bytes unsigned (`Arrays.compareUnsigned`).
    pub fn contains(&self, other: &Cell) -> bool {
        for dim in 0..self.num_dims {
            let offset = dim.wrapping_mul(self.bytes_per_dim);
            let end = offset.wrapping_add(self.bytes_per_dim);
            // other.min < min?
            if compare_unsigned(
                &other.min_packed_value,
                offset,
                end,
                &self.min_packed_value,
                offset,
                end,
            ) < 0
            {
                return false;
            }
            // other.max > max?
            if compare_unsigned(
                &other.max_packed_value,
                offset,
                end,
                &self.max_packed_value,
                offset,
                end,
            ) > 0
            {
                return false;
            }
        }
        true
    }
}

/// `Arrays.compareUnsigned(a, aFromIndex, aToIndex, b, bFromIndex, bToIndex)`
/// per the JDK 25 specification: both ranges are checked first (the
/// `IllegalArgumentException` of a reversed range, the
/// `ArrayIndexOutOfBoundsException` of an index outside the array, thrown),
/// then the ranges compare lexicographically as unsigned bytes, a proper
/// prefix first.
fn compare_unsigned(a: &[u8], a_from: i32, a_to: i32, b: &[u8], b_from: i32, b_to: i32) -> i32 {
    range_check(a.len(), a_from, a_to);
    range_check(b.len(), b_from, b_to);
    let a = &a[a_from as usize..a_to as usize];
    let b = &b[b_from as usize..b_to as usize];
    match a.iter().zip(b).find(|(x, y)| x != y) {
        Some((x, y)) => i32::from(*x) - i32::from(*y),
        None => a.len() as i32 - b.len() as i32,
    }
}

/// `Arrays.rangeCheck(arrayLength, fromIndex, toIndex)`.
fn range_check(array_length: usize, from_index: i32, to_index: i32) {
    if from_index > to_index {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            format!("fromIndex({from_index}) > toIndex({to_index})"),
        )
        .throw();
    }
    if from_index < 0 {
        Throwable::new(
            "java.lang.ArrayIndexOutOfBoundsException",
            format!("Array index out of range: {from_index}"),
        )
        .throw();
    }
    if to_index as usize > array_length {
        Throwable::new(
            "java.lang.ArrayIndexOutOfBoundsException",
            format!("Array index out of range: {to_index}"),
        )
        .throw();
    }
}

impl PointsStackTracker {
    /// `new PointsStackTracker(numDims, bytesPerDim)`.
    pub fn new(num_dims: i32, bytes_per_dim: i32) -> Self {
        PointsStackTracker {
            num_dims,
            bytes_per_dim,
            stack: Vec::new(),
        }
    }

    /// `tracker.new Cell(minPackedValue, maxPackedValue)`: a cell holding
    /// copies of the two values.
    pub fn new_cell(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Cell {
        Cell {
            min_packed_value: min_packed_value.to_vec(),
            max_packed_value: max_packed_value.to_vec(),
            num_dims: self.num_dims,
            bytes_per_dim: self.bytes_per_dim,
        }
    }

    /// `onCompare(minPackedValue, maxPackedValue)`: pops every cell that does
    /// not contain the compared one, then pushes it.
    pub fn on_compare(&mut self, min_packed_value: &[u8], max_packed_value: &[u8]) {
        let cell = self.new_cell(min_packed_value, max_packed_value);
        // Pop stack:
        while self.stack.last().is_some_and(|top| !top.contains(&cell)) {
            self.stack.pop();
        }
        // Push stack:
        self.stack.push(cell);
    }

    // TODO: expose other details about the stack...
}

#[cfg(test)]
mod tests {
    use super::*;
    use randomizedtesting::jdk::exception::QuietPanics;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// `Cell.contains` reads each dimension through `Arrays.compareUnsigned`,
    /// whose range check throws `ArrayIndexOutOfBoundsException` ("Array
    /// index out of range: 8") for a dimension past the packed values
    /// (`PointsStackTracker.java:50-68`).
    #[test]
    fn contains_past_the_packed_values_throws_the_jdk_range_exception() {
        let mut tracker = PointsStackTracker::new(2, 4);
        tracker.on_compare(&[0, 0, 0, 1], &[0, 0, 0, 9]);
        let payload = {
            let _quiet = QuietPanics::new();
            catch_unwind(AssertUnwindSafe(|| {
                tracker.on_compare(&[0, 0, 0, 1], &[0, 0, 0, 9])
            }))
            .expect_err("the ArrayIndexOutOfBoundsException")
        };
        let thrown = Throwable::from_panic(payload);
        assert_eq!(
            thrown.class_name(),
            "java.lang.ArrayIndexOutOfBoundsException"
        );
        assert_eq!(thrown.message(), Some("Array index out of range: 8"));
    }

    /// Unsigned byte order: `0x80` sorts after `0x7f`.
    #[test]
    fn contains_compares_unsigned() {
        let tracker = PointsStackTracker::new(1, 1);
        let outer = tracker.new_cell(&[0x00], &[0xff]);
        let inner = tracker.new_cell(&[0x7f], &[0x80]);
        assert!(outer.contains(&inner));
        assert!(!inner.contains(&outer));
    }
}
