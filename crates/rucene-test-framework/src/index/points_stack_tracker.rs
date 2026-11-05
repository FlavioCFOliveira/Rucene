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
            let offset = (dim * self.bytes_per_dim) as usize;
            let range = offset..offset + self.bytes_per_dim as usize;
            // other.min < min?
            if other.min_packed_value[range.clone()] < self.min_packed_value[range.clone()] {
                return false;
            }
            // other.max > max?
            if other.max_packed_value[range.clone()] > self.max_packed_value[range] {
                return false;
            }
        }
        true
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
