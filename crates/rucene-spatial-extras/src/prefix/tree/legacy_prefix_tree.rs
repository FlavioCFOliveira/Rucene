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

//! The types declared in `LegacyPrefixTree.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::{CellIterator, ListCellIterator};
use crate::prefix::tree::filter_cell_iterator::FilterCellIterator;
use crate::prefix::tree::legacy_cell::as_legacy_cell;
use crate::prefix::tree::spatial_prefix_tree::{default_get_tree_cell_iterator, SpatialPrefixTree};

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.LegacyPrefixTree`
/// (package-private `abstract class LegacyPrefixTree extends
/// SpatialPrefixTree`): the base of the geohash and quad trees, whose cells
/// are [`LegacyCell`](crate::prefix::tree::legacy_cell::LegacyCell)s. Its
/// method bodies are the `legacy_*` functions of this module, which the
/// subclasses' [`SpatialPrefixTree`] implementations call.
pub(crate) trait LegacyPrefixTree: SpatialPrefixTree {
    /// `getCell(Point p, int level)`: the cell at `level` containing `p`.
    fn get_cell(&self, p: &dyn Point, level: i32) -> CellRef;
}

/// The body of `LegacyPrefixTree.getDistanceForLevel(int level)`.
///
/// # Errors
///
/// `IllegalArgumentException("Level must be in 1 to maxLevels range")`.
pub(crate) fn legacy_get_distance_for_level<T: LegacyPrefixTree + ?Sized>(tree: &T, level: i32) -> Result<f64> {
    if level < 1 || level > tree.get_max_levels() {
        return Err(LuceneError::illegal_argument("Level must be in 1 to maxLevels range"));
    }
    // TODO cache for each level
    let center = tree.get_spatial_context().get_world_bounds().get_center();
    let cell = tree.get_cell(&*center, level);
    let bbox = cell.get_shape().get_bounding_box();
    let width = bbox.get_width();
    let height = bbox.get_height();
    // Use standard cartesian hypotenuse. For geospatial, this answer is larger
    // than the correct one but it's okay to over-estimate.
    Ok((width * width + height * height).sqrt())
}

/// The body of `LegacyPrefixTree.readCell(BytesRef term, Cell scratch)`.
pub(crate) fn legacy_read_cell<T: LegacyPrefixTree + ?Sized>(tree: &T, term: &BytesRef, scratch: Option<CellRef>) -> CellRef {
    let cell = scratch.unwrap_or_else(|| tree.get_world_cell());
    as_legacy_cell(&*cell).read_cell(term);
    cell
}

/// The body of `LegacyPrefixTree.getTreeCellIterator(Shape shape, int
/// detailLevel)`.
///
/// # Errors
///
/// `IllegalArgumentException` from the default implementation.
pub(crate) fn legacy_get_tree_cell_iterator<T: LegacyPrefixTree + ?Sized>(
    tree: &T,
    shape: Arc<dyn Shape>,
    detail_level: i32,
) -> Result<Box<dyn CellIterator>> {
    let Some(point) = shape.as_point() else {
        return default_get_tree_cell_iterator(tree, shape, detail_level);
    };
    // This specialization is here because the legacy implementations don't have a fast
    // implementation of cell.getSubCells(point). It's fastest here to encode the full bytes for
    // detailLevel, and create subcells from the bytesRef in a loop. This avoids an O(N^2) encode,
    // and we have O(N) instead.
    let cell = tree.get_cell(point, detail_level);
    let mut full_bytes = cell.get_token_bytes_no_leaf(None);
    // fill in reverse order to be sorted
    let mut cells: Vec<CellRef> = Vec::with_capacity(detail_level.max(0) as usize);
    for i in 1..detail_level {
        full_bytes.length = i as usize;
        let parent_cell = tree.read_cell(&full_bytes, None);
        cells.push(parent_cell);
    }
    cells.push(cell);
    Ok(Box::new(FilterCellIterator::new(Box::new(ListCellIterator::new(cells)), None))) // null filter
}
