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

//! The types declared in `RecursivePrefixTreeStrategy.java`, of the Java
//! package `org.apache.lucene.spatial.prefix`.

use std::any::Any;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::index::{IndexableField, Term};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{Query, TermQuery};
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::prefix::contains_prefix_tree_query::ContainsPrefixTreeQuery;
use crate::prefix::intersects_prefix_tree_query::IntersectsPrefixTreeQuery;
use crate::prefix::prefix_tree_strategy::{
    prefix_create_cell_iterator_to_index, prefix_create_indexable_fields,
    prefix_make_distance_value_source, PrefixTreeStrategy, PrefixTreeStrategyBase,
};
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::{CellIterator, ListCellIterator};
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use crate::prefix::within_prefix_tree_query::WithinPrefixTreeQuery;
use crate::query::spatial_args::SpatialArgs;
use crate::query::spatial_operation::SpatialOperation;
use crate::query::unsupported_spatial_operation::UnsupportedSpatialOperation;
use crate::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};

/// The fields of `RecursivePrefixTreeStrategy` (with those of
/// `PrefixTreeStrategy`).
#[derive(Debug)]
pub struct RecursivePrefixTreeStrategyBase {
    /// The `PrefixTreeStrategy` fields.
    pub prefix: PrefixTreeStrategyBase,
    prefix_grid_scan_level: AtomicI32,
    // Formerly known as simplifyIndexedCells. Eventually will be removed. Only compatible with RPT
    // and cells implementing CellCanPrune, otherwise ignored.
    prune_leafy_branches: AtomicBool,
    multi_overlapping_indexed_shapes: AtomicBool,
}

impl RecursivePrefixTreeStrategyBase {
    /// `RecursivePrefixTreeStrategy(SpatialPrefixTree grid, String
    /// fieldName)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(grid: Arc<dyn SpatialPrefixTree>, field_name: &str) -> Result<Self> {
        // TODO this default constant is dependent on the prefix grid size
        let prefix_grid_scan_level = grid.get_max_levels() - 4;
        Ok(Self {
            prefix: PrefixTreeStrategyBase::new(grid, field_name)?,
            prefix_grid_scan_level: AtomicI32::new(prefix_grid_scan_level),
            prune_leafy_branches: AtomicBool::new(true),
            multi_overlapping_indexed_shapes: AtomicBool::new(true),
        })
    }
}

/// The methods of `RecursivePrefixTreeStrategy` its subclass
/// (`NumberRangePrefixTreeStrategy`) inherits or overrides: Java's
/// `RecursivePrefixTreeStrategy` type, as `Arc<dyn
/// RecursivePrefixTreeStrategyOps>`.
pub trait RecursivePrefixTreeStrategyOps: PrefixTreeStrategy {
    /// The `RecursivePrefixTreeStrategy` fields.
    fn rpt_base(&self) -> &RecursivePrefixTreeStrategyBase;

    /// `getClass().getSimpleName()`.
    fn simple_name(&self) -> &'static str;

    /// `getPrefixGridScanLevel()`.
    fn get_prefix_grid_scan_level(&self) -> i32 {
        self.rpt_base()
            .prefix_grid_scan_level
            .load(Ordering::Relaxed)
    }

    /// `setPrefixGridScanLevel(int prefixGridScanLevel)`: sets the grid
    /// level \[1-maxLevels\] at which indexed terms are scanned brute-force
    /// instead of by grid decomposition. By default this is maxLevels - 4.
    /// The final level, maxLevels, is always scanned.
    fn set_prefix_grid_scan_level(&self, prefix_grid_scan_level: i32) {
        // TODO if negative then subtract from maxlevels
        self.rpt_base()
            .prefix_grid_scan_level
            .store(prefix_grid_scan_level, Ordering::Relaxed);
    }

    /// `isMultiOverlappingIndexedShapes()`.
    fn is_multi_overlapping_indexed_shapes(&self) -> bool {
        self.rpt_base()
            .multi_overlapping_indexed_shapes
            .load(Ordering::Relaxed)
    }

    /// `setMultiOverlappingIndexedShapes(boolean
    /// multiOverlappingIndexedShapes)`: see `ContainsPrefixTreeQuery`.
    fn set_multi_overlapping_indexed_shapes(&self, multi_overlapping_indexed_shapes: bool) {
        self.rpt_base()
            .multi_overlapping_indexed_shapes
            .store(multi_overlapping_indexed_shapes, Ordering::Relaxed);
    }

    /// `isPruneLeafyBranches()`.
    fn is_prune_leafy_branches(&self) -> bool {
        self.rpt_base().prune_leafy_branches.load(Ordering::Relaxed)
    }

    /// `setPruneLeafyBranches(boolean pruneLeafyBranches)`: an option to
    /// reduce the number of indexed terms by pruning leafy branches, when
    /// all of a cell's children are leaves (which is true for
    /// `CellCanPrune` cells).
    fn set_prune_leafy_branches(&self, prune_leafy_branches: bool) {
        self.rpt_base()
            .prune_leafy_branches
            .store(prune_leafy_branches, Ordering::Relaxed);
    }

    /// `isGridAlignedShape(Shape shape)`: a quick check of the shape to see
    /// if it is perfectly aligned to a grid. Points always are as they are
    /// indivisible. It's okay to return false if the shape actually is
    /// aligned; this is an optimization hint.
    fn is_grid_aligned_shape(&self, shape: &dyn Shape) -> bool {
        self.is_point_shape(shape)
    }

    /// `makeGridShapeIntersectsQuery(Shape gridShape)`: `makeQuery`
    /// optimization when the shape is grid-aligned: a `TermQuery` when the
    /// data is only points, else an `IntersectsPrefixTreeQuery` at the
    /// maximum levels.
    ///
    /// # Errors
    ///
    /// The errors of `getTreeCellIterator`.
    fn make_grid_shape_intersects_query(
        &self,
        grid_shape: &Arc<dyn Shape>,
    ) -> Result<Arc<dyn Query>> {
        debug_assert!(self.is_grid_aligned_shape(&**grid_shape));
        let grid = self.get_grid();
        if self.is_points_only() {
            // Awesome; this will be equivalent to a TermQuery.
            let mut cell_iterator =
                grid.get_tree_cell_iterator(Arc::clone(grid_shape), grid.get_max_levels())?;
            // get last cell
            let mut cell = cell_iterator.next();
            while cell_iterator.has_next() {
                let prev_level = cell.get_level();
                cell = cell_iterator.next();
                debug_assert!(prev_level < cell.get_level());
            }
            debug_assert!(cell.is_leaf());
            Ok(Arc::new(TermQuery::new(Term::new(
                self.get_field_name().to_string(),
                cell.get_token_bytes_with_leaf(None),
            ))))
        } else {
            // Well there could be parent cells. But we can reduce the "scan level" which will be slower
            // for a point query.
            // TODO: AVPTQ will still scan the bottom nonetheless; file an issue to eliminate that
            Ok(Arc::new(IntersectsPrefixTreeQuery::new(
                Arc::clone(grid_shape),
                self.get_field_name(),
                Arc::clone(&grid),
                grid.get_max_levels(),
                grid.get_max_levels() + 1,
            )))
        }
    }
}

/// The body of `RecursivePrefixTreeStrategy.toString()`.
pub fn rpt_to_string<S: RecursivePrefixTreeStrategyOps + ?Sized>(s: &S) -> String {
    let mut str = String::from(s.simple_name());
    str.push('(');
    str.push_str("SPG:(");
    str.push_str(&s.get_grid().to_string());
    str.push(')');
    if s.is_points_only() {
        str.push_str(",pointsOnly");
    }
    if s.is_prune_leafy_branches() {
        str.push_str(",pruneLeafyBranches");
    }
    if s.get_prefix_grid_scan_level() != s.get_grid().get_max_levels() - 4 {
        str.push_str(&format!(
            ",prefixGridScanLevel:{}",
            s.get_prefix_grid_scan_level()
        ));
    }
    if !s.is_multi_overlapping_indexed_shapes() {
        str.push_str(",!multiOverlappingIndexedShapes");
    }
    str.push(')');
    str
}

/// The body of `RecursivePrefixTreeStrategy.createCellIteratorToIndex`.
///
/// # Errors
///
/// The errors of the cell iterator.
pub fn rpt_create_cell_iterator_to_index<S: RecursivePrefixTreeStrategyOps + ?Sized>(
    s: &S,
    shape: &Arc<dyn Shape>,
    detail_level: i32,
) -> Result<Box<dyn CellIterator>> {
    if !s.is_prune_leafy_branches() || s.is_grid_aligned_shape(&**shape) {
        return prefix_create_cell_iterator_to_index(s, shape, detail_level);
    }
    let mut cells: Vec<CellRef> = Vec::with_capacity(4096);
    recursive_traverse_and_prune(
        s.get_grid().get_world_cell(),
        shape,
        detail_level,
        &mut cells,
    );
    Ok(Box::new(ListCellIterator::new(cells)))
}

/// `recursiveTraverseAndPrune(Cell cell, Shape shape, int detailLevel,
/// List<Cell> result)`: returns true if cell was added as a leaf. If it
/// wasn't it recursively descends.
fn recursive_traverse_and_prune(
    cell: CellRef,
    shape: &Arc<dyn Shape>,
    detail_level: i32,
    result: &mut Vec<CellRef>,
) -> bool {
    if cell.get_level() == detail_level {
        cell.set_leaf(); // FYI might already be a leaf
    }
    if cell.is_leaf() {
        result.push(cell);
        return true;
    }
    if cell.get_level() != 0 {
        result.push(Arc::clone(&cell));
    }

    let mut leaves = 0;
    let mut sub_cells = cell.get_next_level_cells(Some(Arc::clone(shape)));
    while sub_cells.has_next() {
        let sub_cell = sub_cells.next();
        if recursive_traverse_and_prune(sub_cell, shape, detail_level, result) {
            leaves += 1;
        }
    }

    let Some(can_prune) = cell.as_cell_can_prune() else {
        // Cannot prune so return false
        return false;
    };

    // can we prune?
    if leaves == can_prune.get_sub_cells_size() && cell.get_level() != 0 {
        // Optimization: substitute the parent as a leaf instead of adding all
        // children as leaves

        // remove the leaves
        loop {
            result.pop(); // remove last
            leaves -= 1;
            if leaves <= 0 {
                break;
            }
        }
        // add cell as the leaf
        cell.set_leaf();
        return true;
    }
    false
}

/// The body of `RecursivePrefixTreeStrategy.makeQuery(SpatialArgs args)`.
///
/// # Errors
///
/// `UnsupportedSpatialOperation` for an operation other than `Intersects`,
/// `IsWithin` and `Contains`; the errors of `resolveDistErr`.
pub fn rpt_make_query<S: RecursivePrefixTreeStrategyOps + ?Sized>(
    s: &S,
    args: &SpatialArgs,
) -> Result<Arc<dyn Query>> {
    let op = args.get_operation();
    let shape = &args.get_shape();
    let grid = s.get_grid();
    let ctx = s.get_spatial_context();
    let detail_level =
        grid.get_level_for_distance(args.resolve_dist_err(&ctx, s.get_dist_err_pct())?);

    if op == SpatialOperation::Intersects {
        if s.is_grid_aligned_shape(&*args.get_shape()) {
            return s.make_grid_shape_intersects_query(&args.get_shape());
        }
        Ok(Arc::new(IntersectsPrefixTreeQuery::new(
            Arc::clone(shape),
            s.get_field_name(),
            grid,
            detail_level,
            s.get_prefix_grid_scan_level(),
        )))
    } else if op == SpatialOperation::IsWithin {
        Ok(Arc::new(WithinPrefixTreeQuery::new(
            Arc::clone(shape),
            s.get_field_name(),
            grid,
            detail_level,
            s.get_prefix_grid_scan_level(),
            -1.0, // -1 flag is slower but ensures correct results
        )?))
    } else if op == SpatialOperation::Contains {
        Ok(Arc::new(ContainsPrefixTreeQuery::new(
            Arc::clone(shape),
            s.get_field_name(),
            grid,
            detail_level,
            s.is_multi_overlapping_indexed_shapes(),
        )))
    } else {
        Err(UnsupportedSpatialOperation::new(op))
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.RecursivePrefixTreeStrategy`
/// (`public class RecursivePrefixTreeStrategy extends PrefixTreeStrategy`):
/// a [`PrefixTreeStrategy`] which uses `AbstractVisitingPrefixTreeQuery`.
/// This strategy has support for searching non-point shapes (note: not
/// tested). Even a query shape with distErrPct=0 (fully precise to the
/// grid) should have good performance for typical data, unless there is a
/// lot of indexed data coincident with the shape's edge.
#[derive(Debug)]
pub struct RecursivePrefixTreeStrategy {
    base: RecursivePrefixTreeStrategyBase,
    me: Weak<RecursivePrefixTreeStrategy>,
}

impl RecursivePrefixTreeStrategy {
    /// `RecursivePrefixTreeStrategy(SpatialPrefixTree grid, String
    /// fieldName)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(
        grid: Arc<dyn SpatialPrefixTree>,
        field_name: &str,
    ) -> Result<Arc<RecursivePrefixTreeStrategy>> {
        let base = RecursivePrefixTreeStrategyBase::new(grid, field_name)?;
        Ok(Arc::new_cyclic(|me| RecursivePrefixTreeStrategy {
            base,
            me: me.clone(),
        }))
    }
}

impl fmt::Display for RecursivePrefixTreeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rpt_to_string(self))
    }
}

impl SpatialStrategy for RecursivePrefixTreeStrategy {
    fn as_recursive_prefix_tree_strategy(&self) -> Option<&dyn RecursivePrefixTreeStrategyOps> {
        Some(self)
    }

    fn strategy_base(&self) -> &SpatialStrategyBase {
        &self.base.prefix.strategy
    }

    fn create_indexable_fields(
        &self,
        shape: &Arc<dyn Shape>,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        prefix_create_indexable_fields(self, shape)
    }

    fn make_distance_value_source(
        &self,
        query_point: &Arc<dyn Point>,
        multiplier: f64,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(prefix_make_distance_value_source(
            self,
            query_point,
            multiplier,
        ))
    }

    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>> {
        rpt_make_query(self, args)
    }
}

impl PrefixTreeStrategy for RecursivePrefixTreeStrategy {
    fn prefix_base(&self) -> &PrefixTreeStrategyBase {
        &self.base.prefix
    }

    fn me_prefix(&self) -> Arc<dyn PrefixTreeStrategy> {
        self.me
            .upgrade()
            .expect("a RecursivePrefixTreeStrategy is shared as an Arc")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_cell_iterator_to_index(
        &self,
        shape: &Arc<dyn Shape>,
        detail_level: i32,
    ) -> Result<Box<dyn CellIterator>> {
        rpt_create_cell_iterator_to_index(self, shape, detail_level)
    }
}

impl RecursivePrefixTreeStrategyOps for RecursivePrefixTreeStrategy {
    fn rpt_base(&self) -> &RecursivePrefixTreeStrategyBase {
        &self.base
    }

    fn simple_name(&self) -> &'static str {
        "RecursivePrefixTreeStrategy"
    }
}
