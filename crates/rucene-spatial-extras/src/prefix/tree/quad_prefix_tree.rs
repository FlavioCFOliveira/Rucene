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

//! The types declared in `QuadPrefixTree.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use rucene::error::{LuceneError, Result};
use rucene::util::java_fmt::DecimalFormat;
use rucene::util::BytesRef;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::tree::cell::{Cell, CellRef};
use crate::prefix::tree::cell_can_prune::CellCanPrune;
use crate::prefix::tree::cell_iterator::CellIterator;
use crate::prefix::tree::legacy_cell::{self, LegacyCell, LegacyCellState};
use crate::prefix::tree::legacy_prefix_tree::{self, LegacyPrefixTree};
use crate::prefix::tree::spatial_prefix_tree::{SpatialPrefixTree, SpatialPrefixTreeBase};
use crate::prefix::tree::spatial_prefix_tree_factory::{
    SpatialPrefixTreeFactory, SpatialPrefixTreeFactoryBase,
};

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.QuadPrefixTree.Factory`
/// (`public static class Factory extends SpatialPrefixTreeFactory`): factory
/// for creating [`QuadPrefixTree`] instances with useful defaults.
#[derive(Debug, Default)]
pub struct Factory {
    base: SpatialPrefixTreeFactoryBase,
}

impl SpatialPrefixTreeFactory for Factory {
    fn base(&self) -> &SpatialPrefixTreeFactoryBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut SpatialPrefixTreeFactoryBase {
        &mut self.base
    }

    fn get_level_for_distance(&self, degrees: f64) -> Result<i32> {
        Ok(self.new_spt()?.get_level_for_distance(degrees))
    }

    fn new_spt(&self) -> Result<Arc<dyn SpatialPrefixTree>> {
        Ok(QuadPrefixTree::new(
            Arc::clone(self.base.ctx()),
            self.base
                .max_levels
                .unwrap_or(QuadPrefixTree::MAX_LEVELS_POSSIBLE),
        ))
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.QuadPrefixTree`
/// (`public class QuadPrefixTree extends LegacyPrefixTree`): a
/// [`SpatialPrefixTree`] which uses a quad tree in which an indexed term
/// will be generated for each cell, 'A', 'B', 'C', 'D'.
#[derive(Debug)]
pub struct QuadPrefixTree {
    pub(crate) base: SpatialPrefixTreeBase,
    pub(crate) xmin: f64,
    #[allow(dead_code)] // Java field, read by no method.
    pub(crate) xmax: f64,
    pub(crate) ymin: f64,
    #[allow(dead_code)] // Java field, read by no method.
    pub(crate) ymax: f64,
    pub(crate) xmid: f64,
    pub(crate) ymid: f64,
    pub(crate) grid_w: f64,
    /// `gridH` (public in Java).
    pub grid_h: f64,
    pub(crate) level_w: Vec<f64>,
    pub(crate) level_h: Vec<f64>,
    me: Weak<QuadPrefixTree>,
}

impl QuadPrefixTree {
    /// `MAX_LEVELS_POSSIBLE`: not really sure how big this should be.
    pub const MAX_LEVELS_POSSIBLE: i32 = 50;
    /// `DEFAULT_MAX_LEVELS`.
    pub const DEFAULT_MAX_LEVELS: i32 = 12;

    /// The fields of `QuadPrefixTree(SpatialContext ctx, Rectangle bounds, int
    /// maxLevels)`, without the self reference (for `PackedQuadPrefixTree`,
    /// which makes its own cells).
    pub(crate) fn fields(
        ctx: Arc<SpatialContext>,
        bounds: &dyn Rectangle,
        max_levels: i32,
        me: Weak<QuadPrefixTree>,
    ) -> Self {
        let xmin = bounds.get_min_x();
        let xmax = bounds.get_max_x();
        let ymin = bounds.get_min_y();
        let ymax = bounds.get_max_y();
        let n = (max_levels + 1).max(0) as usize;
        let mut level_w = vec![0.0; n];
        let mut level_h = vec![0.0; n];
        let grid_w = xmax - xmin;
        let grid_h = ymax - ymin;
        let xmid = xmin + grid_w / 2.0;
        let ymid = ymin + grid_h / 2.0;
        if n > 0 {
            level_w[0] = grid_w / 2.0;
            level_h[0] = grid_h / 2.0;
        }
        for i in 1..n {
            level_w[i] = level_w[i - 1] / 2.0;
            level_h[i] = level_h[i - 1] / 2.0;
        }
        Self {
            base: SpatialPrefixTreeBase::new(ctx, max_levels),
            xmin,
            xmax,
            ymin,
            ymax,
            xmid,
            ymid,
            grid_w,
            grid_h,
            level_w,
            level_h,
            me,
        }
    }

    /// `QuadPrefixTree(SpatialContext ctx, Rectangle bounds, int maxLevels)`.
    pub fn with_bounds(
        ctx: Arc<SpatialContext>,
        bounds: &dyn Rectangle,
        max_levels: i32,
    ) -> Arc<QuadPrefixTree> {
        Arc::new_cyclic(|me| Self::fields(ctx, bounds, max_levels, me.clone()))
    }

    /// `QuadPrefixTree(SpatialContext ctx)`.
    pub fn with_default_levels(ctx: Arc<SpatialContext>) -> Arc<QuadPrefixTree> {
        Self::new(ctx, Self::DEFAULT_MAX_LEVELS)
    }

    /// `QuadPrefixTree(SpatialContext ctx, int maxLevels)`.
    pub fn new(ctx: Arc<SpatialContext>, max_levels: i32) -> Arc<QuadPrefixTree> {
        let bounds = ctx.get_world_bounds();
        Self::with_bounds(ctx, &*bounds, max_levels)
    }

    fn me(&self) -> Arc<QuadPrefixTree> {
        self.me
            .upgrade()
            .expect("a QuadPrefixTree is shared as an Arc")
    }

    /// `printInfo(PrintStream out)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of `out`.
    pub fn print_info(&self, out: &mut dyn std::io::Write) -> Result<()> {
        // NumberFormat.getNumberInstance(Locale.ROOT) with 5 fraction digits
        // (minimum and maximum) and at least 3 integer digits.
        let nf = DecimalFormat::new("#,000.00000")?;
        for i in 0..self.base.max_levels.max(0) as usize {
            writeln!(
                out,
                "{i}]\t{}\t{}",
                nf.format(self.level_w[i]),
                nf.format(self.level_h[i])
            )
            .map_err(LuceneError::from)?;
        }
        Ok(())
    }

    /// `getLevelForDistance(double dist)`.
    pub(crate) fn quad_level_for_distance(&self, dist: f64) -> i32 {
        let max_levels = self.base.max_levels;
        if dist == 0.0 {
            // short circuit
            return max_levels;
        }
        for i in 0..(max_levels - 1).max(0) as usize {
            // note: level[i] is actually a lookup for level i+1
            if dist > self.level_w[i] && dist > self.level_h[i] {
                return i as i32 + 1;
            }
        }
        max_levels
    }

    /// `getCell(Point p, int level)` (the cell bytes; the caller makes the
    /// cell).
    fn cell_bytes(&self, p: &dyn Point, level: i32) -> BytesRef {
        let max_levels = self.base.max_levels;
        let mut current_xmid = self.xmid;
        let mut current_ymid = self.ymid;
        let xp = p.get_x();
        let yp = p.get_y();
        let mut str = BytesRef::with_capacity((max_levels + 1) as usize);
        let level_limit = if level > max_levels {
            max_levels
        } else {
            level
        };
        for lvl in 0..level_limit.max(0) as usize {
            let c = self.battenberg(current_xmid, current_ymid, xp, yp);
            let half_width = self.level_w[lvl + 1];
            let half_height = self.level_h[lvl + 1];
            match c {
                0 => {
                    current_xmid -= half_width;
                    current_ymid += half_height;
                }
                1 => {
                    current_xmid += half_width;
                    current_ymid += half_height;
                }
                2 => {
                    current_xmid -= half_width;
                    current_ymid -= half_height;
                }
                3 => {
                    current_xmid += half_width;
                    current_ymid -= half_height;
                }
                _ => {}
            }
            push_byte(&mut str, b'A' + c as u8);
        }
        str
    }

    /// `buildNotRobustly(double x, double y, int level, List<Cell> matches,
    /// BytesRef str, Shape shape, int maxLevel)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the shape factory.
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Java's legacy private method; no caller in Lucene.
    fn build_not_robustly(
        &self,
        x: f64,
        y: f64,
        level: i32,
        matches: &mut Vec<CellRef>,
        str: &mut BytesRef,
        shape: &dyn Shape,
        max_level: i32,
    ) -> Result<()> {
        debug_assert_eq!(str.length as i32, level);
        let w = self.level_w[level as usize] / 2.0;
        let h = self.level_h[level as usize] / 2.0;
        // Z-Order
        // http://en.wikipedia.org/wiki/Z-order_%28curve%29
        self.check_battenberg_not_robustly(
            b'A',
            x - w,
            y + h,
            level,
            matches,
            str,
            shape,
            max_level,
        )?;
        self.check_battenberg_not_robustly(
            b'B',
            x + w,
            y + h,
            level,
            matches,
            str,
            shape,
            max_level,
        )?;
        self.check_battenberg_not_robustly(
            b'C',
            x - w,
            y - h,
            level,
            matches,
            str,
            shape,
            max_level,
        )?;
        self.check_battenberg_not_robustly(
            b'D',
            x + w,
            y - h,
            level,
            matches,
            str,
            shape,
            max_level,
        )
        // possibly consider hilbert curve
        // http://en.wikipedia.org/wiki/Hilbert_curve
        // http://blog.notdot.net/2009/11/Damn-Cool-Algorithms-Spatial-indexing-with-Quadtrees-and-Hilbert-Curves
        // if we actually use the range property in the query, this could be useful
    }

    /// `checkBattenbergNotRobustly(char c, double cx, double cy, int level,
    /// List<Cell> matches, BytesRef str, Shape shape, int maxLevel)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the shape factory.
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Java legacy method; only buildNotRobustly calls it.
    pub(crate) fn check_battenberg_not_robustly(
        &self,
        c: u8,
        cx: f64,
        cy: f64,
        level: i32,
        matches: &mut Vec<CellRef>,
        str: &mut BytesRef,
        shape: &dyn Shape,
        max_level: i32,
    ) -> Result<()> {
        debug_assert_eq!(str.length as i32, level);
        debug_assert_eq!(str.offset, 0);
        let w = self.level_w[level as usize] / 2.0;
        let h = self.level_h[level as usize] / 2.0;
        let strlen = str.length;
        let rectangle = self
            .base
            .ctx
            .get_shape_factory()
            .rect(cx - w, cx + w, cy - h, cy + h)?;
        let v = shape.relate(rectangle.as_shape());
        if SpatialRelation::Contains == v {
            push_byte(str, c); // append
                               // str.append(SpatialPrefixGrid.COVER);
            matches.push(QuadCell::from_bytes_ref(
                &self.me(),
                &BytesRef::deep_copy_of(str),
                Some(v.transpose()),
            ));
        } else if SpatialRelation::Disjoint == v {
            // nothing
        } else {
            // SpatialRelation.WITHIN, SpatialRelation.INTERSECTS
            push_byte(str, c); // append
            let next_level = level + 1;
            if next_level >= max_level {
                // str.append(SpatialPrefixGrid.INTERSECTS);
                matches.push(QuadCell::from_bytes_ref(
                    &self.me(),
                    &BytesRef::deep_copy_of(str),
                    Some(v.transpose()),
                ));
            } else {
                self.build_not_robustly(cx, cy, next_level, matches, str, shape, max_level)?;
            }
        }
        str.length = strlen;
        Ok(())
    }

    /// `battenberg(double xmid, double ymid, double xp, double yp)`.
    pub(crate) fn battenberg(&self, xmid: f64, ymid: f64, xp: f64, yp: f64) -> i32 {
        // http://en.wikipedia.org/wiki/Z-order_%28curve%29
        if ymid <= yp {
            if xmid >= xp {
                return 0;
            }
            1
        } else {
            if xmid >= xp {
                return 2;
            }
            3
        }
        // possibly consider hilbert curve
        // http://en.wikipedia.org/wiki/Hilbert_curve
        // http://blog.notdot.net/2009/11/Damn-Cool-Algorithms-Spatial-indexing-with-Quadtrees-and-Hilbert-Curves
        // if we actually use the range property in the query, this could be useful
    }
}

/// `str.bytes[str.length++] = b`, growing the copy where Java's array was
/// sized for it.
pub(crate) fn push_byte(str: &mut BytesRef, b: u8) {
    let end = str.offset + str.length;
    if str.bytes.len() <= end {
        str.bytes.resize(end + 1, 0);
    }
    str.bytes[end] = b;
    str.length += 1;
}

impl fmt::Display for QuadPrefixTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_named("QuadPrefixTree"))
    }
}

impl SpatialPrefixTree for QuadPrefixTree {
    fn base(&self) -> &SpatialPrefixTreeBase {
        &self.base
    }

    fn get_level_for_distance(&self, dist: f64) -> i32 {
        self.quad_level_for_distance(dist)
    }

    fn get_distance_for_level(&self, level: i32) -> Result<f64> {
        legacy_prefix_tree::legacy_get_distance_for_level(self, level)
    }

    fn get_world_cell(&self) -> CellRef {
        QuadCell::new(&self.me(), Vec::new(), 0, 0)
    }

    fn read_cell(&self, term: &BytesRef, scratch: Option<CellRef>) -> CellRef {
        legacy_prefix_tree::legacy_read_cell(self, term, scratch)
    }

    fn get_tree_cell_iterator(
        &self,
        shape: Arc<dyn Shape>,
        detail_level: i32,
    ) -> Result<Box<dyn CellIterator>> {
        legacy_prefix_tree::legacy_get_tree_cell_iterator(self, shape, detail_level)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LegacyPrefixTree for QuadPrefixTree {
    fn get_cell(&self, p: &dyn Point, level: i32) -> CellRef {
        let str = self.cell_bytes(p, level);
        QuadCell::from_bytes_ref(&self.me(), &str, Some(SpatialRelation::Contains))
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.QuadPrefixTree.QuadCell`
/// (`protected class QuadCell extends LegacyCell`): a cell of a
/// [`QuadPrefixTree`].
#[derive(Debug)]
pub struct QuadCell {
    tree: Arc<QuadPrefixTree>,
    state: Mutex<LegacyCellState>,
}

impl QuadCell {
    /// `QuadCell(byte[] bytes, int off, int len)`: the new cell, shared as
    /// the trees share their cells.
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(
        tree: &Arc<QuadPrefixTree>,
        bytes: Vec<u8>,
        off: usize,
        len: usize,
    ) -> CellRef {
        let cell = QuadCell {
            tree: Arc::clone(tree),
            state: Mutex::new(LegacyCellState::new(bytes, off, len)),
        };
        cell.read_leaf_adjust();
        Arc::new(cell)
    }

    /// `QuadCell(BytesRef str, SpatialRelation shapeRel)`.
    pub(crate) fn from_bytes_ref(
        tree: &Arc<QuadPrefixTree>,
        str: &BytesRef,
        shape_rel: Option<SpatialRelation>,
    ) -> CellRef {
        let cell = QuadCell::new(tree, str.bytes.clone(), str.offset, str.length);
        cell.set_shape_rel(shape_rel);
        cell
    }

    /// `concat(BytesRef source, byte b)`.
    pub(crate) fn concat(source: &BytesRef, b: u8) -> BytesRef {
        // +2 for new char + potential leaf
        let mut buffer = vec![0u8; source.length + 2];
        buffer[..source.length]
            .copy_from_slice(&source.bytes[source.offset..source.offset + source.length]);
        let mut target = BytesRef::new(buffer);
        target.length = source.length;
        target.bytes[target.length] = b;
        target.length += 1;
        target
    }

    /// `makeShape()`.
    fn make_shape(&self) -> Arc<dyn Rectangle> {
        let token = self.get_token_bytes_no_leaf(None);
        let t = &self.tree;
        let mut xmin = t.xmin;
        let mut ymin = t.ymin;
        for i in 0..token.length {
            let c = token.bytes[token.offset + i];
            match c {
                b'A' => ymin += t.level_h[i],
                b'B' => {
                    xmin += t.level_w[i];
                    ymin += t.level_h[i];
                }
                b'C' => {} // nothing really
                b'D' => xmin += t.level_w[i],
                _ => panic!("java.lang.RuntimeException: unexpected char: {}", c as i8),
            }
        }
        let len = token.length;
        let (width, height) = if len > 0 {
            (t.level_w[len - 1], t.level_h[len - 1])
        } else {
            (t.grid_w, t.grid_h)
        };
        t.base
            .ctx
            .get_shape_factory()
            .rect(xmin, xmin + width, ymin, ymin + height)
            .unwrap_or_else(|e| panic!("{e}"))
    }
}

impl fmt::Display for QuadCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&legacy_cell::legacy_to_string(self))
    }
}

impl LegacyCell for QuadCell {
    fn state(&self) -> MutexGuard<'_, LegacyCellState> {
        legacy_cell::lock(&self.state)
    }

    fn get_max_levels(&self) -> i32 {
        self.tree.base.max_levels
    }

    fn get_sub_cell(&self, p: &dyn Point) -> CellRef {
        self.tree.get_cell(p, self.get_level() + 1) // not performant!
    }

    fn get_sub_cells(&self) -> Vec<CellRef> {
        let source = self.get_token_bytes_no_leaf(None);
        vec![
            QuadCell::from_bytes_ref(&self.tree, &QuadCell::concat(&source, b'A'), None),
            QuadCell::from_bytes_ref(&self.tree, &QuadCell::concat(&source, b'B'), None),
            QuadCell::from_bytes_ref(&self.tree, &QuadCell::concat(&source, b'C'), None),
            QuadCell::from_bytes_ref(&self.tree, &QuadCell::concat(&source, b'D'), None),
        ]
    }
}

impl CellCanPrune for QuadCell {
    fn get_sub_cells_size(&self) -> i32 {
        4
    }
}

impl Cell for QuadCell {
    crate::legacy_cell_common!();

    fn get_token_bytes_with_leaf(&self, result: Option<BytesRef>) -> BytesRef {
        legacy_cell::legacy_get_token_bytes_with_leaf(self, result)
    }

    fn get_token_bytes_no_leaf(&self, result: Option<BytesRef>) -> BytesRef {
        legacy_cell::legacy_get_token_bytes_no_leaf(self, result)
    }

    fn get_level(&self) -> i32 {
        self.state().b_len as i32
    }

    fn get_shape(&self) -> Arc<dyn Shape> {
        if let Some(shape) = &self.state().shape {
            return Arc::clone(shape);
        }
        let shape: Arc<dyn Shape> = self.make_shape();
        self.state().shape = Some(Arc::clone(&shape));
        shape
    }

    fn is_prefix_of(&self, c: &dyn Cell) -> bool {
        legacy_cell::legacy_is_prefix_of(self, c)
    }

    fn compare_to_no_leaf(&self, from_cell: &dyn Cell) -> i32 {
        legacy_cell::legacy_compare_to_no_leaf(self, from_cell)
    }
}
