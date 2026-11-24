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

//! The types declared in `Cell.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::util::BytesRef;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::tree::cell_can_prune::CellCanPrune;
use crate::prefix::tree::cell_iterator::CellIterator;

/// A shared [`Cell`], as Java shares cell objects by reference.
pub type CellRef = Arc<dyn Cell>;

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.Cell` (`public
/// interface`): represents a grid cell. Cell instances are generally very
/// transient and may be re-used internally.
///
/// Java's cells are mutable objects shared by reference (`setShapeRel`,
/// `setLeaf`, `SpatialPrefixTree.readCell` into a scratch cell); here a cell
/// is shared as [`CellRef`] and keeps its mutable state behind a lock, so the
/// mutators take `&self` and every holder of the `Arc` sees the change, as in
/// Java. Java's `equals(Object)`, `hashCode()` and `toString()` are
/// [`cell_equals`](Self::cell_equals), [`cell_hash_code`](Self::cell_hash_code)
/// and the [`fmt::Display`] implementation.
///
/// Java's `getTokenBytes*(BytesRef result)` fill and return `result`, which
/// may then share the cell's own byte array; this port's [`BytesRef`] owns its
/// bytes, so the returned value is a copy of the same bytes (`result` is
/// reused as the allocation when given).
pub trait Cell: Send + Sync + fmt::Display + fmt::Debug + Any {
    /// `getShapeRel()`: the relationship of this cell to the shape filter
    /// given when it was created (`None` for `null`).
    fn get_shape_rel(&self) -> Option<SpatialRelation>;

    /// `setShapeRel(SpatialRelation rel)`.
    fn set_shape_rel(&self, rel: Option<SpatialRelation>);

    /// `isLeaf()`.
    fn is_leaf(&self) -> bool;

    /// `setLeaf()`.
    fn set_leaf(&self);

    /// `getTokenBytesWithLeaf(BytesRef result)`: the bytes of this cell,
    /// with a leaf byte when it is a leaf.
    fn get_token_bytes_with_leaf(&self, result: Option<BytesRef>) -> BytesRef;

    /// `getTokenBytesNoLeaf(BytesRef result)`: the bytes of this cell,
    /// without a leaf byte.
    fn get_token_bytes_no_leaf(&self, result: Option<BytesRef>) -> BytesRef;

    /// `getLevel()`: level 0 is the world (and has no parent).
    fn get_level(&self) -> i32;

    /// `getNextLevelCells(Shape shapeFilter)`: the cells at the next level,
    /// filtered by `shape_filter` (`None` for no filter).
    fn get_next_level_cells(&self, shape_filter: Option<Arc<dyn Shape>>) -> Box<dyn CellIterator>;

    /// `getShape()`: the shape of this cell.
    fn get_shape(&self) -> Arc<dyn Shape>;

    /// `isPrefixOf(Cell c)`: whether this cell is a prefix of (or equal to)
    /// `c`, disregarding leaf status.
    fn is_prefix_of(&self, c: &dyn Cell) -> bool;

    /// `compareToNoLeaf(Cell fromCell)`: the order of the token bytes
    /// without the leaf byte.
    fn compare_to_no_leaf(&self, from_cell: &dyn Cell) -> i32;

    /// `equals(Object obj)` against another cell.
    fn cell_equals(&self, other: &dyn Cell) -> bool;

    /// `hashCode()`.
    fn cell_hash_code(&self) -> i32;

    /// This cell as `Any`, for the casts Java writes (`(LegacyCell) c`).
    fn as_any(&self) -> &dyn Any;

    /// `this instanceof CellCanPrune ? (CellCanPrune) this : null`.
    fn as_cell_can_prune(&self) -> Option<&dyn CellCanPrune> {
        None
    }
}
