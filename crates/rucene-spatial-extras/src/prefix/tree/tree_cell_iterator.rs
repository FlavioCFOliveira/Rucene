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

//! The types declared in `TreeCellIterator.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::sync::Arc;

use spatial4j::shape::shape::Shape;

use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::{CellIterator, CellIteratorBase};

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.TreeCellIterator`
/// (package-private `class TreeCellIterator extends CellIterator`): navigates
/// a [`SpatialPrefixTree`](crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree)
/// from a given cell (typically the world cell) down to a maximum number of
/// configured levels, filtered by a given shape. Intermediate non-leaf cells
/// are returned. It supports [`remove`](CellIterator::remove) for skipping
/// traversal of children of the current cell.
pub(crate) struct TreeCellIterator {
    base: CellIteratorBase,
    shape_filter: Option<Arc<dyn Shape>>, // possibly null
    iter_stack: Vec<Option<Box<dyn CellIterator>>>, // starts at level 1
    stack_idx: i32,                                 // -1 when done
    descend: bool,
}

impl TreeCellIterator {
    /// `TreeCellIterator(Shape shapeFilter, int detailLevel, Cell parentCell)`.
    pub(crate) fn new(shape_filter: Option<Arc<dyn Shape>>, detail_level: i32, parent_cell: CellRef) -> Self {
        debug_assert_eq!(parent_cell.get_level(), 0);
        let mut iter_stack: Vec<Option<Box<dyn CellIterator>>> = (0..detail_level.max(0)).map(|_| None).collect();
        iter_stack[0] = Some(parent_cell.get_next_level_cells(shape_filter.clone()));
        Self {
            base: CellIteratorBase::default(),
            shape_filter,
            iter_stack,
            stack_idx: 0, // always points to an iter (non-null)
            // note: not obvious but needed to visit the first cell before trying to descend
            descend: false,
        }
    }

    fn iter(&mut self, idx: i32) -> &mut Box<dyn CellIterator> {
        self.iter_stack[idx as usize].as_mut().expect("iterStack[stackIdx] is non-null")
    }
}

impl CellIterator for TreeCellIterator {
    fn has_next(&mut self) -> bool {
        if self.base.next_cell.is_some() {
            return true;
        }
        let len = self.iter_stack.len() as i32;
        loop {
            if self.stack_idx == -1 {
                // the only condition in which we return false
                return false;
            }
            // If we can descend...
            if self.descend
                && !(self.stack_idx == len - 1 || self.iter(self.stack_idx).this_cell().is_leaf())
            {
                let this_cell = self.iter(self.stack_idx).this_cell();
                let next_iter = this_cell.get_next_level_cells(self.shape_filter.clone());
                // push stack
                self.stack_idx += 1;
                self.iter_stack[self.stack_idx as usize] = Some(next_iter);
            }
            // Get sibling...
            let idx = self.stack_idx;
            if self.iter(idx).has_next() {
                let next_cell = self.iter(idx).next();
                // at detailLevel
                if self.stack_idx == len - 1
                    && !self.shape_filter.as_ref().is_some_and(|s| s.as_point().is_some())
                {
                    // point check is a kludge
                    next_cell.set_leaf(); // because at bottom
                }
                self.base.next_cell = Some(next_cell);
                break;
            }
            // Couldn't get next; go up...
            // pop stack
            self.iter_stack[self.stack_idx as usize] = None;
            self.stack_idx -= 1;
            self.descend = false; // so that we don't re-descend where we just were
        }
        debug_assert!(self.base.next_cell.is_some());
        self.descend = true; // reset
        true
    }

    fn next(&mut self) -> CellRef {
        crate::cell_iterator_next!(self, base)
    }

    fn this_cell(&self) -> CellRef {
        self.base.this_cell()
    }

    fn remove(&mut self) {
        debug_assert!(self.base.this_cell.is_some() && self.base.next_cell.is_none());
        self.descend = false;
    }
}
