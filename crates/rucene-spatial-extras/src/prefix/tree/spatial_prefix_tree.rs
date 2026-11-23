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

//! The types declared in `SpatialPrefixTree.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::shape::Shape;

use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::CellIterator;
use crate::prefix::tree::number_range_prefix_tree::NumberRangePrefixTreeImpl;
use crate::prefix::tree::tree_cell_iterator::TreeCellIterator;

/// The fields of `SpatialPrefixTree`: `maxLevels` and `ctx`.
#[derive(Debug, Clone)]
pub struct SpatialPrefixTreeBase {
    /// `maxLevels`.
    pub max_levels: i32,
    /// `ctx`.
    pub ctx: Arc<SpatialContext>,
}

impl SpatialPrefixTreeBase {
    /// `SpatialPrefixTree(SpatialContext ctx, int maxLevels)`.
    pub fn new(ctx: Arc<SpatialContext>, max_levels: i32) -> Self {
        debug_assert!(max_levels > 0);
        Self { max_levels, ctx }
    }

    /// `toString()`: `getClass().getSimpleName() + "(maxLevels:" + maxLevels
    /// + ",ctx:" + ctx + ")"`, for the class named `simple_name`.
    pub fn to_string_named(&self, simple_name: &str) -> String {
        format!(
            "{simple_name}(maxLevels:{},ctx:{})",
            self.max_levels, self.ctx
        )
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.SpatialPrefixTree`
/// (`public abstract class`): a spatial prefix tree. The tree is shared as
/// `Arc<dyn SpatialPrefixTree>`; its cells refer back to it.
///
/// Java's `toString()` is the [`fmt::Display`] implementation.
pub trait SpatialPrefixTree: Send + Sync + fmt::Display + fmt::Debug + Any {
    /// The `SpatialPrefixTree` fields.
    fn base(&self) -> &SpatialPrefixTreeBase;

    /// `getSpatialContext()`.
    fn get_spatial_context(&self) -> Arc<SpatialContext> {
        Arc::clone(&self.base().ctx)
    }

    /// `getMaxLevels()`.
    fn get_max_levels(&self) -> i32 {
        self.base().max_levels
    }

    /// `getLevelForDistance(double dist)`: the level of the largest cells
    /// (lowest level) whose width is no greater than `dist`.
    fn get_level_for_distance(&self, dist: f64) -> i32;

    /// `getDistanceForLevel(int level)`: the approximate distance to the
    /// furthest corner of a cell at `level`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a level out of range;
    /// `UnsupportedOperationException` where Java throws it.
    fn get_distance_for_level(&self, level: i32) -> Result<f64>;

    /// `getWorldCell()`: the level 0 cell which encompasses all spatial
    /// data.
    fn get_world_cell(&self) -> CellRef;

    /// `readCell(BytesRef term, Cell scratch)`: the cell of `term`, read into
    /// `scratch` when it is given (and returned), else into a new cell.
    fn read_cell(&self, term: &BytesRef, scratch: Option<CellRef>) -> CellRef;

    /// `getTreeCellIterator(Shape shape, int detailLevel)`: the cells
    /// intersecting `shape` down to `detail_level`, parents before children.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `detail_level > maxLevels`.
    fn get_tree_cell_iterator(
        &self,
        shape: Arc<dyn Shape>,
        detail_level: i32,
    ) -> Result<Box<dyn CellIterator>> {
        default_get_tree_cell_iterator(self, shape, detail_level)
    }

    /// This tree as `Any`, for the casts Java writes.
    fn as_any(&self) -> &dyn Any;

    /// `this instanceof NumberRangePrefixTree ? (NumberRangePrefixTree) this
    /// : null`.
    fn as_number_range_prefix_tree(&self) -> Option<&dyn NumberRangePrefixTreeImpl> {
        None
    }
}

/// The body of `SpatialPrefixTree.getTreeCellIterator` (the one a subclass
/// reaches with `super`).
///
/// # Errors
///
/// `IllegalArgumentException("detailLevel > maxLevels")`.
pub fn default_get_tree_cell_iterator<T: SpatialPrefixTree + ?Sized>(
    tree: &T,
    shape: Arc<dyn Shape>,
    detail_level: i32,
) -> Result<Box<dyn CellIterator>> {
    if detail_level > tree.get_max_levels() {
        return Err(LuceneError::illegal_argument("detailLevel > maxLevels"));
    }
    Ok(Box::new(TreeCellIterator::new(
        Some(shape),
        detail_level,
        tree.get_world_cell(),
    )))
}
