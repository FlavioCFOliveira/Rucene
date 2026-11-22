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

//! The types declared in `LegacyCell.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::sync::{Arc, Mutex, MutexGuard};

use rucene::util::BytesRef;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::tree::cell::{Cell, CellRef};
use crate::prefix::tree::cell_can_prune::CellCanPrune;
use crate::prefix::tree::cell_iterator::{CellIterator, ListCellIterator};
use crate::prefix::tree::filter_cell_iterator::FilterCellIterator;
use crate::prefix::tree::geohash_prefix_tree::GhCell;
use crate::prefix::tree::packed_quad_prefix_tree::PackedQuadCell;
use crate::prefix::tree::quad_prefix_tree::QuadCell;
use crate::prefix::tree::singleton_cell_iterator::SingletonCellIterator;

/// `LEAF_BYTE`: must sort before letters & numbers.
pub(crate) const LEAF_BYTE: u8 = b'+';

/// The fields of `LegacyCell`. A `LegacyCell` doesn't share state for
/// `getNextLevelCells()`, and the legacy trees assume this in their
/// simplify-tree logic.
#[derive(Debug, Clone, Default)]
pub struct LegacyCellState {
    /// `bytes`: generally bigger to potentially hold a leaf.
    pub bytes: Vec<u8>,
    /// `b_off`.
    pub b_off: usize,
    /// `b_len`: doesn't reflect leaf; same as `getLevel()`.
    pub b_len: usize,
    /// `isLeaf`.
    pub is_leaf: bool,
    /// `shapeRel`.
    pub shape_rel: Option<SpatialRelation>,
    /// `shape`: cached.
    pub shape: Option<Arc<dyn Shape>>,
}

impl LegacyCellState {
    /// `LegacyCell(byte[] bytes, int off, int len)` before `readLeafAdjust()`.
    pub fn new(bytes: Vec<u8>, off: usize, len: usize) -> Self {
        Self {
            bytes,
            b_off: off,
            b_len: len,
            is_leaf: false,
            shape_rel: None,
            shape: None,
        }
    }
}

/// The lock of a cell's mutable state.
pub(crate) type StateCell = Mutex<LegacyCellState>;

/// Locks a cell state; a poisoned lock is recovered (its data stays valid).
pub(crate) fn lock(state: &StateCell) -> MutexGuard<'_, LegacyCellState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.LegacyCell`
/// (`public abstract class LegacyCell implements CellCanPrune`): the base
/// for the cell implementations of the trees that use a whole number of
/// bytes per level (geohash and quad). Public for RPT `pruneLeafyBranches`
/// code.
///
/// The fields are the [`LegacyCellState`] behind [`state`](Self::state);
/// the shared method bodies are the `legacy_*` functions of this module,
/// which the subclasses' [`Cell`] implementations call.
pub trait LegacyCell: CellCanPrune {
    /// The `LegacyCell` fields.
    fn state(&self) -> MutexGuard<'_, LegacyCellState>;

    /// `getMaxLevels()`.
    fn get_max_levels(&self) -> i32;

    /// `getGrid().getMaxLevels()`.
    fn get_grid_max_levels(&self) -> i32 {
        self.get_max_levels()
    }

    /// `readCell(BytesRef bytes)`.
    fn read_cell(&self, bytes: &BytesRef) {
        legacy_read_cell(self, bytes);
    }

    /// `readLeafAdjust()`.
    fn read_leaf_adjust(&self) {
        legacy_read_leaf_adjust(self);
    }

    /// `getSubCell(Point p)`: performant implementations are expected to
    /// implement this efficiently by considering the current cell's
    /// boundary.
    fn get_sub_cell(&self, p: &dyn Point) -> CellRef;

    /// `getSubCells()`: the sub-cells of this cell, sorted.
    fn get_sub_cells(&self) -> Vec<CellRef>;
}

/// `(LegacyCell) c`: the legacy cell types of this crate.
///
/// # Panics
///
/// `java.lang.ClassCastException` for another cell type.
pub(crate) fn as_legacy_cell(c: &dyn Cell) -> &dyn LegacyCell {
    let any = c.as_any();
    if let Some(cell) = any.downcast_ref::<QuadCell>() {
        return cell;
    }
    if let Some(cell) = any.downcast_ref::<PackedQuadCell>() {
        return cell;
    }
    if let Some(cell) = any.downcast_ref::<GhCell>() {
        return cell;
    }
    panic!("java.lang.ClassCastException: {c:?} cannot be cast to class org.apache.lucene.spatial.prefix.tree.LegacyCell")
}

/// The body of `LegacyCell.readCell(BytesRef bytes)`.
pub fn legacy_read_cell<C: LegacyCell + ?Sized>(cell: &C, bytes: &BytesRef) {
    {
        let mut s = cell.state();
        s.shape_rel = None;
        s.shape = None;
        s.bytes = bytes.bytes.clone();
        s.b_off = bytes.offset;
        s.b_len = bytes.length as i16 as u16 as usize; // (short) bytes.length
    }
    cell.read_leaf_adjust();
}

/// The body of `LegacyCell.readLeafAdjust()`.
pub fn legacy_read_leaf_adjust<C: LegacyCell + ?Sized>(cell: &C) {
    let max_levels = cell.get_max_levels();
    let mut s = cell.state();
    s.is_leaf = s.b_len > 0 && s.bytes[s.b_off + s.b_len - 1] == LEAF_BYTE;
    if s.is_leaf {
        s.b_len -= 1;
    }
    if s.b_len as i32 == max_levels {
        s.is_leaf = true;
    }
}

/// The body of `LegacyCell.getTokenBytesWithLeaf(BytesRef result)`.
pub fn legacy_get_token_bytes_with_leaf<C: LegacyCell + ?Sized>(cell: &C, result: Option<BytesRef>) -> BytesRef {
    let mut result = legacy_get_token_bytes_no_leaf(cell, result);
    let (is_leaf, level) = {
        let s = cell.state();
        (s.is_leaf, s.b_len as i32)
    };
    if !is_leaf || level == cell.get_max_levels() {
        return result;
    }
    // Java writes LEAF_BYTE into the cell's own array, just past its bytes
    // (which never changes what the cell reads); the copy gets it appended.
    let end = result.offset + result.length;
    if result.bytes.len() < end + 1 {
        result.bytes.resize(end + 1, 0);
    }
    result.bytes[end] = LEAF_BYTE;
    result.length += 1;
    result
}

/// The body of `LegacyCell.getTokenBytesNoLeaf(BytesRef result)`.
pub fn legacy_get_token_bytes_no_leaf<C: LegacyCell + ?Sized>(cell: &C, result: Option<BytesRef>) -> BytesRef {
    let s = cell.state();
    let mut result = result.unwrap_or_else(|| BytesRef::new(Vec::new()));
    result.bytes.clear();
    result.bytes.extend_from_slice(&s.bytes);
    result.offset = s.b_off;
    result.length = s.b_len;
    result
}

/// The body of `LegacyCell.getNextLevelCells(Shape shapeFilter)`.
pub fn legacy_get_next_level_cells<C: LegacyCell + ?Sized>(
    cell: &C,
    shape_filter: Option<Arc<dyn Shape>>,
) -> Box<dyn CellIterator> {
    debug_assert!(cell.get_level() < cell.get_grid_max_levels());
    match &shape_filter {
        Some(filter) if filter.as_point().is_some() => {
            let sub_cell = cell.get_sub_cell(filter.as_point().expect("checked"));
            sub_cell.set_shape_rel(Some(SpatialRelation::Contains));
            Box::new(SingletonCellIterator::new(sub_cell))
        }
        _ => Box::new(FilterCellIterator::new(
            Box::new(ListCellIterator::new(cell.get_sub_cells())),
            shape_filter,
        )),
    }
}

/// The body of `LegacyCell.isPrefixOf(Cell c)`. Note: this only works when
/// each level uses a whole number of bytes.
pub fn legacy_is_prefix_of<C: LegacyCell + ?Sized>(this: &C, c: &dyn Cell) -> bool {
    let cell = as_legacy_cell(c);
    let (o_bytes, o_off, o_len) = {
        let s = cell.state();
        (s.bytes.clone(), s.b_off, s.b_len)
    };
    let s = this.state();
    slice_equals(&o_bytes, o_off, o_len, &s.bytes, s.b_off, s.b_len)
}

fn slice_equals(
    slice_to_test_bytes: &[u8],
    slice_to_test_offset: usize,
    slice_to_test_length: usize,
    other_bytes: &[u8],
    other_offset: usize,
    other_length: usize,
) -> bool {
    if slice_to_test_length < other_length {
        return false;
    }
    let mut i = slice_to_test_offset;
    let mut j = other_offset;
    let k = other_offset + other_length;
    while j < k {
        if slice_to_test_bytes[i] != other_bytes[j] {
            return false;
        }
        i += 1;
        j += 1;
    }
    true
}

/// The body of `LegacyCell.compareToNoLeaf(Cell fromCell)`.
pub fn legacy_compare_to_no_leaf<C: LegacyCell + ?Sized>(this: &C, from_cell: &dyn Cell) -> i32 {
    let b = as_legacy_cell(from_cell);
    let (b_bytes, b_off, b_len) = {
        let s = b.state();
        (s.bytes.clone(), s.b_off, s.b_len)
    };
    let s = this.state();
    compare(&s.bytes, s.b_off, s.b_len, &b_bytes, b_off, b_len)
}

/// `LegacyCell.compare(byte[] aBytes, int aUpto, int a_length, byte[] bBytes,
/// int bUpto, int b_length)`.
pub fn compare(a_bytes: &[u8], mut a_upto: usize, a_length: usize, b_bytes: &[u8], mut b_upto: usize, b_length: usize) -> i32 {
    let a_stop = a_upto + a_length.min(b_length);
    while a_upto < a_stop {
        let a_byte = i32::from(a_bytes[a_upto]);
        let b_byte = i32::from(b_bytes[b_upto]);
        a_upto += 1;
        b_upto += 1;
        let diff = a_byte - b_byte;
        if diff != 0 {
            return diff;
        }
    }
    // One is a prefix of the other, or, they are equal:
    a_length as i32 - b_length as i32
}

/// The body of `LegacyCell.equals(Object obj)`: this method isn't
/// "normally" called; just in asserts/tests.
pub fn legacy_equals<C: LegacyCell + ?Sized>(this: &C, cell: &dyn Cell) -> bool {
    this.get_token_bytes_with_leaf(None).bytes_equals(&cell.get_token_bytes_with_leaf(None))
}

/// The body of `LegacyCell.hashCode()`.
pub fn legacy_hash_code<C: LegacyCell + ?Sized>(this: &C) -> i32 {
    this.get_token_bytes_with_leaf(None).hash_code()
}

/// The body of `LegacyCell.toString()`: this method isn't "normally"
/// called; just in asserts/tests.
pub fn legacy_to_string<C: LegacyCell + ?Sized>(this: &C) -> String {
    let b = this.get_token_bytes_with_leaf(None);
    b.utf8_to_string()
        .unwrap_or_else(|_| String::from_utf8_lossy(&b.bytes[b.offset..b.offset + b.length]).into_owned())
}

/// Implements the [`Cell`] methods every legacy cell shares with
/// `LegacyCell`, over the `legacy_*` bodies.
#[macro_export]
#[doc(hidden)]
macro_rules! legacy_cell_common {
    () => {
        fn get_shape_rel(&self) -> Option<spatial4j::shape::spatial_relation::SpatialRelation> {
            $crate::prefix::tree::legacy_cell::LegacyCell::state(self).shape_rel
        }

        fn set_shape_rel(&self, rel: Option<spatial4j::shape::spatial_relation::SpatialRelation>) {
            $crate::prefix::tree::legacy_cell::LegacyCell::state(self).shape_rel = rel;
        }

        fn is_leaf(&self) -> bool {
            $crate::prefix::tree::legacy_cell::LegacyCell::state(self).is_leaf
        }

        fn set_leaf(&self) {
            $crate::prefix::tree::legacy_cell::LegacyCell::state(self).is_leaf = true;
        }

        fn get_next_level_cells(
            &self,
            shape_filter: Option<std::sync::Arc<dyn spatial4j::shape::shape::Shape>>,
        ) -> Box<dyn $crate::prefix::tree::cell_iterator::CellIterator> {
            $crate::prefix::tree::legacy_cell::legacy_get_next_level_cells(self, shape_filter)
        }

        fn cell_equals(&self, other: &dyn $crate::prefix::tree::cell::Cell) -> bool {
            $crate::prefix::tree::legacy_cell::legacy_equals(self, other)
        }

        fn cell_hash_code(&self) -> i32 {
            $crate::prefix::tree::legacy_cell::legacy_hash_code(self)
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_cell_can_prune(&self) -> Option<&dyn $crate::prefix::tree::cell_can_prune::CellCanPrune> {
            Some(self)
        }
    };
}
