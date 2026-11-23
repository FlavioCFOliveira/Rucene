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

//! The types declared in `HeatmapFacetCounter.java`, of the Java package
//! `org.apache.lucene.spatial.prefix`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::IndexReaderContext;
use rucene::util::{ArrayUtil, Bits, MatchNoBits};
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::jdk::java_round;
use crate::prefix::prefix_tree_facet_counter::{self, FacetVisitor};
use crate::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use crate::prefix::tree::cell::CellRef;

/// `MAX_ROWS_OR_COLUMNS`: maximum number of supported rows (or columns).
pub const MAX_ROWS_OR_COLUMNS: i32 = 46340; // (int) Math.sqrt(ArrayUtil.MAX_ARRAY_LENGTH)

const _: () = {
    // `Math.multiplyExact(MAX_ROWS_OR_COLUMNS, MAX_ROWS_OR_COLUMNS)`: will
    // fail if it doesn't stay within integer
    assert!((MAX_ROWS_OR_COLUMNS as i64) * (MAX_ROWS_OR_COLUMNS as i64) <= i32::MAX as i64);
};

/// Equivalent to `org.apache.lucene.spatial.prefix.HeatmapFacetCounter`
/// (`public class HeatmapFacetCounter`): computes spatial facets in two
/// dimensions as a grid of numbers. The data is often visualized as a
/// so-called "heatmap", hence the name. This is a static utility class;
/// its method is [`calc_facets`].
#[derive(Debug)]
pub struct HeatmapFacetCounter;

/// Equivalent to `org.apache.lucene.spatial.prefix.HeatmapFacetCounter.Heatmap`
/// (`public static class Heatmap`): response structure.
pub struct Heatmap {
    /// `columns`.
    pub columns: i32,
    /// `rows`.
    pub rows: i32,
    /// `counts`: in order of 1st column (all rows) then 2nd column (all
    /// rows) etc.
    pub counts: Vec<i32>,
    /// `region`.
    pub region: Arc<dyn Rectangle>,
}

impl Heatmap {
    /// `Heatmap(int columns, int rows, Rectangle region)`.
    pub fn new(columns: i32, rows: i32, region: Arc<dyn Rectangle>) -> Self {
        Self {
            columns,
            rows,
            counts: vec![0; (columns * rows).max(0) as usize],
            region,
        }
    }

    /// `getCount(int x, int y)`.
    pub fn get_count(&self, x: i32, y: i32) -> i32 {
        self.counts[(x * self.rows + y) as usize]
    }
}

impl fmt::Display for Heatmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Heatmap{{{}x{} {}}}", self.columns, self.rows, self.region.as_shape())
    }
}

impl fmt::Debug for Heatmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// `calcFacets(PrefixTreeStrategy strategy, IndexReaderContext context,
/// Bits topAcceptDocs, Shape inputShape, int facetLevel, int maxCells)`:
/// calculates spatial 2D facets (aggregated counts) in a grid, sometimes
/// called a heatmap. Facet computation is implemented by navigating the
/// underlying indexed terms efficiently. If you don't know exactly what
/// facetLevel to go to for a given input box but you have some sense of
/// how many cells there should be relative to the size of the shape, then
/// consider using the logic that Solr does.
///
/// The returned heatmap will start at the upper left -- min X and max Y
/// within the specified inputShape (or world).
///
/// # Errors
///
/// `IllegalArgumentException` for too many cells; the errors of the facet
/// computation.
pub fn calc_facets(
    strategy: &dyn PrefixTreeStrategy,
    context: &Arc<dyn IndexReaderContext>,
    top_accept_docs: Option<&dyn Bits>,
    input_shape: Option<Arc<dyn Shape>>,
    facet_level: i32,
    max_cells: i32,
) -> Result<Heatmap> {
    debug_assert_eq!(MAX_ROWS_OR_COLUMNS, (f64::from(ArrayUtil::MAX_ARRAY_LENGTH as i32)).sqrt() as i32);
    if i64::from(max_cells) > i64::from(MAX_ROWS_OR_COLUMNS) * i64::from(MAX_ROWS_OR_COLUMNS) {
        return Err(LuceneError::illegal_argument(format!(
            "maxCells ({max_cells}) should be <= {MAX_ROWS_OR_COLUMNS}"
        )));
    }
    let input_shape: Arc<dyn Shape> = match input_shape {
        Some(s) => s,
        None => strategy.get_spatial_context().get_world_bounds(),
    };
    let input_rect = input_shape.get_bounding_box();
    // First get the rect of the cell at the bottom-left at depth facetLevel
    let grid = strategy.get_grid();
    let ctx = grid.get_spatial_context();
    let corner_pt = ctx.get_shape_factory().point_xy(input_rect.get_min_x(), input_rect.get_min_y())?;
    let mut cell_iterator = grid.get_tree_cell_iterator(corner_pt, facet_level)?;
    let mut corner_cell: Option<CellRef> = None;
    while cell_iterator.has_next() {
        corner_cell = Some(cell_iterator.next());
    }
    let corner_cell = corner_cell.unwrap_or_else(|| panic!("java.lang.NullPointerException: Cell not at target level"));
    debug_assert_eq!(corner_cell.get_level(), facet_level, "Cell not at target level: {corner_cell}");
    let corner_shape = corner_cell.get_shape();
    let corner_rect = corner_shape.as_rectangle().unwrap_or_else(|| {
        panic!("java.lang.ClassCastException: {corner_shape} cannot be cast to Rectangle")
    });
    debug_assert!(corner_rect.has_area());
    // Now calculate the number of columns and rows necessary to cover the inputRect
    let mut heat_min_x = corner_rect.get_min_x(); // note: we might change this below...
    let cell_width = corner_rect.get_width();
    let world_rect = ctx.get_world_bounds();
    let columns = calc_rows_or_cols(
        cell_width,
        heat_min_x,
        input_rect.get_width(),
        input_rect.get_min_x(),
        world_rect.get_width(),
    );
    let heat_min_y = corner_rect.get_min_y();
    let cell_height = corner_rect.get_height();
    let rows = calc_rows_or_cols(
        cell_height,
        heat_min_y,
        input_rect.get_height(),
        input_rect.get_min_y(),
        world_rect.get_height(),
    );
    debug_assert!(rows > 0 && columns > 0);
    if columns > MAX_ROWS_OR_COLUMNS || rows > MAX_ROWS_OR_COLUMNS || columns.wrapping_mul(rows) > max_cells {
        return Err(LuceneError::illegal_argument(format!(
            "Too many cells ({columns} x {rows}) for level {facet_level} shape {}",
            input_rect.as_shape()
        )));
    }

    // Create resulting heatmap bounding rectangle & Heatmap object.
    let half_cell_width = cell_width / 2.0;
    // if X world-wraps, use world bounds' range
    if f64::from(columns) * cell_width + half_cell_width > world_rect.get_width() {
        heat_min_x = world_rect.get_min_x();
    }
    let mut heat_max_x = heat_min_x + f64::from(columns) * cell_width;
    if (heat_max_x - world_rect.get_max_x()).abs() < half_cell_width {
        // numeric conditioning issue
        heat_max_x = world_rect.get_max_x();
    } else if heat_max_x > world_rect.get_max_x() {
        // wraps dateline (won't happen if !geo)
        heat_max_x = heat_max_x - world_rect.get_max_x() + world_rect.get_min_x();
    }
    let half_cell_height = cell_height / 2.0;
    let mut heat_max_y = heat_min_y + f64::from(rows) * cell_height;
    if (heat_max_y - world_rect.get_max_y()).abs() < half_cell_height {
        // numeric conditioning issue
        heat_max_y = world_rect.get_max_y();
    }

    let mut heatmap = Heatmap::new(
        columns,
        rows,
        ctx.get_shape_factory().rect(heat_min_x, heat_max_x, heat_min_y, heat_max_y)?,
    );
    if top_accept_docs.is_some_and(is_match_no_bits) {
        return Ok(heatmap); // short-circuit
    }

    // All ancestor cell counts (of facetLevel) will be captured during facet visiting and applied
    // later. If the data is
    // just points then there won't be any ancestors.
    let mut visitor = HeatmapVisitor {
        heatmap: &mut heatmap,
        facet_level,
        cell_width,
        cell_height,
        heat_min_y,
        all_cells_ancestor_count: 0,
        ancestors: AncestorMap::default(),
    };
    // Now lets count some facets!
    prefix_tree_facet_counter::compute(strategy, context, top_accept_docs, &input_shape, facet_level, &mut visitor)?;
    let all_cells_ancestor_count = visitor.all_cells_ancestor_count;
    let ancestors = std::mem::take(&mut visitor.ancestors);

    // Update the heatmap counts with ancestor counts

    // Apply allCellsAncestorCount
    if all_cells_ancestor_count > 0 {
        for c in heatmap.counts.iter_mut() {
            *c += all_cells_ancestor_count;
        }
    }

    // Apply ancestors
    //  note: This approach isn't optimized for a ton of ancestor cells. We'll potentially increment
    // the same cells
    //    multiple times in separate passes if any ancestors overlap. IF this poses a problem, we
    // could optimize it
    //    with additional complication by keeping track of intervals in a sorted tree structure
    // (possible TreeMap/Set)
    //    and iterate them cleverly such that we just make one pass at this stage.

    let mut pair = [0i32; 2]; // output of intersectInterval
    for (rect, count) in ancestors.entries() {
        // from a cell (thus doesn't cross DL)
        // note: we approach this in a way that eliminates int overflow/underflow (think huge cell,
        // tiny heatmap)
        intersect_interval(heat_min_y, heat_max_y, cell_height, rows, rect.get_min_y(), rect.get_max_y(), &mut pair);
        let start_row = pair[0];
        let end_row = pair[1];

        if !heatmap.region.get_crosses_date_line() {
            intersect_interval(heat_min_x, heat_max_x, cell_width, columns, rect.get_min_x(), rect.get_max_x(), &mut pair);
            let start_col = pair[0];
            let end_col = pair[1];
            increment_range(&mut heatmap, start_col, end_col, start_row, end_row, count);
        } else {
            // note: the cell rect might intersect 2 disjoint parts of the heatmap, so we do the left &
            // right separately
            let left_columns = java_round((180.0 - heat_min_x) / cell_width) as i32;
            let right_columns = heatmap.columns - left_columns;
            // left half of dateline:
            if rect.get_max_x() > heat_min_x {
                intersect_interval(heat_min_x, 180.0, cell_width, left_columns, rect.get_min_x(), rect.get_max_x(), &mut pair);
                let start_col = pair[0];
                let end_col = pair[1];
                increment_range(&mut heatmap, start_col, end_col, start_row, end_row, count);
            }
            // right half of dateline
            if rect.get_min_x() < heat_max_x {
                intersect_interval(-180.0, heat_max_x, cell_width, right_columns, rect.get_min_x(), rect.get_max_x(), &mut pair);
                let start_col = pair[0] + left_columns;
                let end_col = pair[1] + left_columns;
                increment_range(&mut heatmap, start_col, end_col, start_row, end_row, count);
            }
        }
    }

    Ok(heatmap)
}

/// `topAcceptDocs instanceof Bits.MatchNoBits` (`HeatmapFacetCounter.java:174`).
fn is_match_no_bits(bits: &dyn Bits) -> bool {
    bits.bits_as_any().is_some_and(|any| any.is::<MatchNoBits>())
}

/// `Map<Rectangle, Integer> ancestors`: a `HashMap` keyed by
/// `Rectangle.equals`/`hashCode`. Its iteration order only changes the
/// order of additions, which commute.
#[derive(Default)]
struct AncestorMap {
    buckets: HashMap<i32, Vec<(Arc<dyn Rectangle>, i32)>>,
}

impl AncestorMap {
    /// `put(rect, count)` then, when there was an existing count, `put(rect,
    /// count + existingCount)`.
    fn add(&mut self, rect: Arc<dyn Rectangle>, count: i32) {
        let bucket = self.buckets.entry(rect.hash_code()).or_default();
        match bucket.iter_mut().find(|(r, _)| r.equals(rect.as_shape())) {
            Some((_, c)) => *c += count,
            None => bucket.push((rect, count)),
        }
    }

    fn entries(&self) -> Vec<(Arc<dyn Rectangle>, i32)> {
        self.buckets.values().flatten().map(|(r, c)| (Arc::clone(r), *c)).collect()
    }
}

/// The anonymous `FacetVisitor` of `calcFacets`.
struct HeatmapVisitor<'a> {
    heatmap: &'a mut Heatmap,
    facet_level: i32,
    cell_width: f64,
    cell_height: f64,
    heat_min_y: f64,
    /// `allCellsAncestorCount[0]`: facet count of ancestors covering all of
    /// the heatmap.
    all_cells_ancestor_count: i32,
    /// `ancestors`: all other ancestors.
    ancestors: AncestorMap,
}

impl FacetVisitor for HeatmapVisitor<'_> {
    fn visit(&mut self, cell: &CellRef, count: i32) {
        let heat_min_x = self.heatmap.region.get_min_x();
        let shape = cell.get_shape();
        let rect = shape
            .as_rectangle()
            .unwrap_or_else(|| panic!("java.lang.ClassCastException: {shape} cannot be cast to Rectangle"));
        if cell.get_level() == self.facet_level {
            // heatmap level; count it directly
            // convert to col & row
            let column = if rect.get_min_x() >= heat_min_x {
                java_round((rect.get_min_x() - heat_min_x) / self.cell_width) as i32
            } else {
                // due to dateline wrap
                java_round((rect.get_min_x() + 360.0 - heat_min_x) / self.cell_width) as i32
            };
            let row = java_round((rect.get_min_y() - self.heat_min_y) / self.cell_height) as i32;
            // note: unfortunately, it's possible for us to visit adjacent cells to the heatmap
            // (if the SpatialPrefixTree
            // allows adjacent cells to overlap on the seam), so we need to skip them
            if column < 0 || column >= self.heatmap.columns || row < 0 || row >= self.heatmap.rows {
                return;
            }
            // increment
            self.heatmap.counts[(column * self.heatmap.rows + row) as usize] += count;
        } else if rect.relate(self.heatmap.region.as_shape()) == SpatialRelation::Contains {
            self.all_cells_ancestor_count += count;
        } else {
            // ancestor
            // note: not particularly efficient (possible put twice, and Integer wrapper); oh well
            self.ancestors.add(shape.get_bounding_box(), count);
        }
    }
}

/// `intersectInterval(double heatMin, double heatMax, double heatCellLen,
/// int numCells, double cellMin, double cellMax, int[] out)`.
fn intersect_interval(
    heat_min: f64,
    heat_max: f64,
    heat_cell_len: f64,
    num_cells: i32,
    cell_min: f64,
    cell_max: f64,
    out: &mut [i32; 2],
) {
    debug_assert!(heat_min < heat_max && cell_min < cell_max);
    // precondition: we know there's an intersection
    if heat_min >= cell_min {
        out[0] = 0;
    } else {
        out[0] = java_round((cell_min - heat_min) / heat_cell_len) as i32;
    }
    if heat_max <= cell_max {
        out[1] = num_cells - 1;
    } else {
        out[1] = java_round((cell_max - heat_min) / heat_cell_len) as i32 - 1;
    }
}

/// `incrementRange(Heatmap heatmap, int startColumn, int endColumn, int
/// startRow, int endRow, int count)`.
fn increment_range(heatmap: &mut Heatmap, mut start_column: i32, mut end_column: i32, mut start_row: i32, mut end_row: i32, count: i32) {
    // startColumn & startRow are not necessarily within the heatmap range; likewise numRows/columns
    // may overlap.
    if start_column < 0 {
        end_column += start_column;
        start_column = 0;
    }
    end_column = (heatmap.columns - 1).min(end_column);
    if start_row < 0 {
        end_row += start_row;
        start_row = 0;
    }
    end_row = (heatmap.rows - 1).min(end_row);
    if start_row > end_row {
        return; // short-circuit
    }
    for c in start_column..=end_column {
        let c_base = c * heatmap.rows;
        for r in start_row..=end_row {
            heatmap.counts[(c_base + r) as usize] += count;
        }
    }
}

/// `calcRowsOrCols(double cellRange, double cellMin, double requestRange,
/// double requestMin, double worldRange)`.
fn calc_rows_or_cols(cell_range: f64, cell_min: f64, request_range: f64, request_min: f64, world_range: f64) -> i32 {
    debug_assert!(request_min >= cell_min);
    // Idealistically this wouldn't be so complicated but we concern ourselves with overflow and
    // edge cases
    let range = request_range + (request_min - cell_min);
    if range == 0.0 {
        return 1;
    }
    let intervals = (range / cell_range).ceil();
    if intervals > f64::from(i32::MAX) {
        return i32::MAX; // should result in an error soon (exceed thresholds)
    }
    // ensures we don't have more intervals than world bounds (possibly due to rounding/edge issue)
    let intervals_max = java_round(world_range / cell_range);
    if intervals_max > i64::from(i32::MAX) {
        // just return intervals
        return intervals as i32;
    }
    (intervals_max as i32).min(intervals as i32)
}
