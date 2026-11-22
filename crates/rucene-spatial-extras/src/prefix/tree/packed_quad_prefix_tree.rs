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

//! The types declared in `PackedQuadPrefixTree.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::any::Any;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;
use spatial4j::context::spatial_context::{SpatialContext, SpatialContextRef};
use spatial4j::shape::point::Point;
use spatial4j::shape::r#impl::rectangle_impl::RectangleImpl;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::tree::cell::{Cell, CellRef};
use crate::prefix::tree::cell_can_prune::CellCanPrune;
use crate::prefix::tree::cell_iterator::{CellIterator, CellIteratorBase};
use crate::prefix::tree::legacy_cell::{self, LegacyCell, LegacyCellState};
use crate::prefix::tree::legacy_prefix_tree::{self, LegacyPrefixTree};
use crate::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use crate::prefix::tree::spatial_prefix_tree::{SpatialPrefixTree, SpatialPrefixTreeBase};
use crate::prefix::tree::spatial_prefix_tree_factory::{SpatialPrefixTreeFactory, SpatialPrefixTreeFactoryBase};

/// `QUAD`.
const QUAD: [u8; 4] = [0x00, 0x01, 0x02, 0x03];

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.PackedQuadPrefixTree.Factory`
/// (`public static class Factory extends QuadPrefixTree.Factory`): factory
/// for creating [`PackedQuadPrefixTree`] instances with useful defaults.
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

    /// Inherited from `QuadPrefixTree.Factory`: `newSPT().getLevelForDistance(degrees)`.
    fn get_level_for_distance(&self, degrees: f64) -> Result<i32> {
        Ok(self.new_spt()?.get_level_for_distance(degrees))
    }

    fn new_spt(&self) -> Result<Arc<dyn SpatialPrefixTree>> {
        Ok(PackedQuadPrefixTree::new(
            Arc::clone(self.base.ctx()),
            self.base.max_levels.unwrap_or(PackedQuadPrefixTree::MAX_LEVELS_POSSIBLE),
        )?)
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.PackedQuadPrefixTree`
/// (`public class PackedQuadPrefixTree extends QuadPrefixTree`): uses a
/// compact binary representation of 8 bytes to encode a spatial quad trie.
///
/// The binary representation is as follows:
///
/// ```text
/// CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCDDDDDL
///
/// Where C = Cell bits (2 per quad)
///       D = Depth bits (5 with max of 29 levels)
///       L = isLeaf bit
/// ```
///
/// It includes a built-in "pruneLeafyBranches" setting (true by default)
/// similar to `RecursivePrefixTreeStrategy.setPruneLeafyBranches(boolean)`
/// although this one only prunes at the target detail level (where it has
/// the most effect).
///
/// The `QuadPrefixTree` superclass is the embedded [`quad`](Self::quad)
/// fields.
#[derive(Debug)]
pub struct PackedQuadPrefixTree {
    /// The `QuadPrefixTree` fields.
    pub quad: QuadPrefixTree,
    leafy_prune: AtomicBool,
    me: Weak<PackedQuadPrefixTree>,
}

impl PackedQuadPrefixTree {
    /// `MAX_LEVELS_POSSIBLE`.
    pub const MAX_LEVELS_POSSIBLE: i32 = 29;

    /// `PackedQuadPrefixTree(SpatialContext ctx, int maxLevels)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `maxLevels` exceeds
    /// `MAX_LEVELS_POSSIBLE`.
    pub fn new(ctx: Arc<SpatialContext>, max_levels: i32) -> Result<Arc<PackedQuadPrefixTree>> {
        let bounds = ctx.get_world_bounds();
        let quad = QuadPrefixTree::fields(ctx, &*bounds, max_levels, Weak::new());
        if max_levels > Self::MAX_LEVELS_POSSIBLE {
            return Err(LuceneError::illegal_argument(format!(
                "maxLevels of {max_levels} exceeds limit of {}",
                Self::MAX_LEVELS_POSSIBLE
            )));
        }
        Ok(Arc::new_cyclic(|me| PackedQuadPrefixTree {
            quad,
            leafy_prune: AtomicBool::new(true),
            me: me.clone(),
        }))
    }

    fn me(&self) -> Arc<PackedQuadPrefixTree> {
        self.me.upgrade().expect("a PackedQuadPrefixTree is shared as an Arc")
    }

    fn max_levels(&self) -> i32 {
        self.quad.base.max_levels
    }

    fn cell_term(&self, p: &dyn Point, level: i32) -> i64 {
        let q = &self.quad;
        let mut current_xmid = q.xmid;
        let mut current_ymid = q.ymid;
        let xp = p.get_x();
        let yp = p.get_y();
        let mut term: i64 = 0;
        let level_limit = if level > self.max_levels() { self.max_levels() } else { level };
        for lvl in 0..level_limit.max(0) {
            let quad = q.battenberg(current_xmid, current_ymid, xp, yp);
            let half_width = q.level_w[(lvl + 1) as usize];
            let half_height = q.level_h[(lvl + 1) as usize];
            match quad {
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
            // set bits for next level
            term |= (i64::from(quad)).wrapping_shl((64 - ((lvl + 1) << 1)) as u32);
            // increment level
            term = ((((term as u64) >> 1) as i64).wrapping_add(1)).wrapping_shl(1);
        }
        term
    }

    /// `buildNotRobustly(double x, double y, int level, List<Cell> matches,
    /// long term, Shape shape, int maxLevel)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the shape factory.
    #[allow(clippy::too_many_arguments)]
    pub fn build_not_robustly(
        &self,
        x: f64,
        y: f64,
        level: i32,
        matches: &mut Vec<CellRef>,
        term: i64,
        shape: &dyn Shape,
        max_level: i32,
    ) -> Result<()> {
        let w = self.quad.level_w[level as usize] / 2.0;
        let h = self.quad.level_h[level as usize] / 2.0;
        // Z-Order
        // http://en.wikipedia.org/wiki/Z-order_%28curve%29
        self.check_battenberg_not_robustly(QUAD[0], x - w, y + h, level, matches, term, shape, max_level)?;
        self.check_battenberg_not_robustly(QUAD[1], x + w, y + h, level, matches, term, shape, max_level)?;
        self.check_battenberg_not_robustly(QUAD[2], x - w, y - h, level, matches, term, shape, max_level)?;
        self.check_battenberg_not_robustly(QUAD[3], x + w, y - h, level, matches, term, shape, max_level)
    }

    /// `checkBattenbergNotRobustly(byte quad, double cx, double cy, int level,
    /// List<Cell> matches, long term, Shape shape, int maxLevel)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the shape factory.
    #[allow(clippy::too_many_arguments)]
    pub fn check_battenberg_not_robustly(
        &self,
        quad: u8,
        cx: f64,
        cy: f64,
        mut level: i32,
        matches: &mut Vec<CellRef>,
        mut term: i64,
        shape: &dyn Shape,
        max_level: i32,
    ) -> Result<()> {
        // short-circuit if we find a match for the point (no need to continue recursion)
        if shape.as_point().is_some() && !matches.is_empty() {
            return Ok(());
        }
        let w = self.quad.level_w[level as usize] / 2.0;
        let h = self.quad.level_h[level as usize] / 2.0;
        let rect = self.quad.base.ctx.get_shape_factory().rect(cx - w, cx + w, cy - h, cy + h)?;
        let v = shape.relate(rect.as_shape());
        if SpatialRelation::Disjoint == v {
            return Ok(());
        }
        // set bits for next level
        level += 1;
        term |= (i64::from(quad)).wrapping_shl((64 - (level << 1)) as u32);
        // increment level
        term = ((((term as u64) >> 1) as i64).wrapping_add(1)).wrapping_shl(1);
        if SpatialRelation::Contains == v || (level >= max_level) {
            matches.push(PackedQuadCell::with_rel(&self.me(), term, Some(v.transpose())));
        } else {
            // SpatialRelation.WITHIN, SpatialRelation.INTERSECTS
            self.build_not_robustly(cx, cy, level, matches, term, shape, max_level)?;
        }
        Ok(())
    }

    /// `isPruneLeafyBranches()`.
    pub fn is_prune_leafy_branches(&self) -> bool {
        self.leafy_prune.load(Ordering::Relaxed)
    }

    /// `setPruneLeafyBranches(boolean pruneLeafyBranches)`: like
    /// `RecursivePrefixTreeStrategy.setPruneLeafyBranches(boolean)` but more
    /// memory efficient and only applies to the detailLevel, where it has
    /// the most effect.
    pub fn set_prune_leafy_branches(&self, prune_leafy_branches: bool) {
        self.leafy_prune.store(prune_leafy_branches, Ordering::Relaxed);
    }
}

impl fmt::Display for PackedQuadPrefixTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PackedQuadPrefixTree(maxLevels:{},ctx:{},prune:{})",
            self.max_levels(),
            self.quad.base.ctx,
            self.is_prune_leafy_branches()
        )
    }
}

impl SpatialPrefixTree for PackedQuadPrefixTree {
    fn base(&self) -> &SpatialPrefixTreeBase {
        &self.quad.base
    }

    fn get_level_for_distance(&self, dist: f64) -> i32 {
        self.quad.quad_level_for_distance(dist)
    }

    fn get_distance_for_level(&self, level: i32) -> Result<f64> {
        legacy_prefix_tree::legacy_get_distance_for_level(self, level)
    }

    fn get_world_cell(&self) -> CellRef {
        PackedQuadCell::new_arc(&self.me(), 0x0)
    }

    fn read_cell(&self, term: &BytesRef, scratch: Option<CellRef>) -> CellRef {
        let cell = scratch.unwrap_or_else(|| self.get_world_cell());
        let packed = cell
            .as_any()
            .downcast_ref::<PackedQuadCell>()
            .unwrap_or_else(|| panic!("java.lang.ClassCastException: {cell:?} cannot be cast to PackedQuadCell"));
        packed.read_cell(term);
        cell
    }

    fn get_tree_cell_iterator(&self, shape: Arc<dyn Shape>, detail_level: i32) -> Result<Box<dyn CellIterator>> {
        if detail_level > self.max_levels() {
            return Err(LuceneError::illegal_argument(format!(
                "detailLevel:{detail_level} exceed max: {}",
                self.max_levels()
            )));
        }
        Ok(Box::new(PrefixTreeIterator::new(self, shape, detail_level as i16)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LegacyPrefixTree for PackedQuadPrefixTree {
    fn get_cell(&self, p: &dyn Point, level: i32) -> CellRef {
        let term = self.cell_term(p, level);
        PackedQuadCell::with_rel(&self.me(), term, Some(SpatialRelation::Contains))
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.PackedQuadPrefixTree.PackedQuadCell`
/// (`protected class PackedQuadCell extends QuadCell`): PackedQuad specific
/// cell.
#[derive(Debug)]
pub struct PackedQuadCell {
    tree: Arc<PackedQuadPrefixTree>,
    state: Mutex<LegacyCellState>,
    /// `term`.
    term: AtomicI64,
}

/// `longToByteArray(long value, byte[] result)`.
fn long_to_byte_array(mut value: i64, result: &mut [u8]) {
    for i in (0..8).rev() {
        result[i] = (value & 255) as u8;
        value >>= 8;
    }
}

/// `fromBytes(byte b1, ..., byte b8)` over `longFromByteArray(byte[] bytes,
/// int ofs)`.
fn long_from_byte_array(bytes: &[u8], ofs: usize) -> i64 {
    debug_assert!(bytes.len() >= 8);
    let mut v: i64 = 0;
    for i in 0..8 {
        v = (v << 8) | i64::from(bytes[i + ofs]);
    }
    v
}

impl PackedQuadCell {
    fn build(tree: &Arc<PackedQuadPrefixTree>, term: i64) -> PackedQuadCell {
        let mut bytes = vec![0u8; 8];
        long_to_byte_array(term, &mut bytes);
        let cell = PackedQuadCell {
            tree: Arc::clone(tree),
            state: Mutex::new(LegacyCellState::new(bytes, 0, 8)),
            term: AtomicI64::new(term),
        };
        cell.read_leaf_adjust();
        cell
    }

    /// `PackedQuadCell(long term)`.
    fn new_arc(tree: &Arc<PackedQuadPrefixTree>, term: i64) -> CellRef {
        Arc::new(Self::build(tree, term))
    }

    /// `PackedQuadCell(long term, SpatialRelation shapeRel)`.
    fn with_rel(tree: &Arc<PackedQuadPrefixTree>, term: i64, shape_rel: Option<SpatialRelation>) -> CellRef {
        let cell = Self::build(tree, term);
        cell.state().shape_rel = shape_rel;
        Arc::new(cell)
    }

    fn term(&self) -> i64 {
        self.term.load(Ordering::Relaxed)
    }

    fn get_shift_for_level(level: i32) -> i32 {
        64 - (level << 1)
    }

    /// `isEnd(int level, int shift)`.
    pub fn is_end(&self, level: i32, shift: i32) -> bool {
        let term = self.term();
        term != 0x0
            && ((1i64.wrapping_shl((level << 1) as u32)).wrapping_sub(1))
                .wrapping_sub(((term as u64).wrapping_shr(shift as u32)) as i64)
                == 0x0
    }

    /// `nextCell(boolean descend)`: get the next cell in the tree without
    /// using recursion. `descend` determines whether or not to traverse the
    /// children of this cell. `None` (Java `null`) at the end.
    pub fn next_cell(&self, descend: bool) -> Option<PackedQuadCell> {
        let level = self.get_level();
        let shift = Self::get_shift_for_level(level);
        let max_levels = self.tree.max_levels();
        // base case: can't go further
        if (!descend && self.is_end(level, shift)) || self.is_end(max_levels, Self::get_shift_for_level(max_levels)) {
            return None;
        }
        let term = self.term();
        let new_term;
        let is_leaf = (term & 0x1) == 0x1;
        // if descend requested && we're not at the maxLevel
        if (descend && !is_leaf && (level != max_levels)) || level == 0 {
            // simple case: increment level bits (next level)
            new_term = (((term as u64 >> 1) as i64).wrapping_add(0x1)).wrapping_shl(1);
        } else {
            // we're not descending or we can't descend
            let mut t = term.wrapping_add(1i64.wrapping_shl(shift as u32));
            // we're at the last sibling...force descend
            if ((term as u64).wrapping_shr(shift as u32) & 0x3) == 0x3 {
                // adjust level for number popping up
                let tz = ((t as u64).wrapping_shr(shift as u32)).trailing_zeros() as i64;
                t = (((t as u64 >> 1) as i64).wrapping_sub(((tz as u64) >> 1) as i64)).wrapping_shl(1);
            }
            new_term = t;
        }
        Some(Self::build(&self.tree, new_term))
    }

    /// `isInternalPrefix(PackedQuadCell c)`.
    fn is_internal_prefix(&self, c: &PackedQuadCell) -> bool {
        let shift = 64 - (self.get_level() << 1);
        ((self.term() as u64).wrapping_shr(shift as u32)).wrapping_sub((c.term() as u64).wrapping_shr(shift as u32)) == 0x0
    }

    /// `concat(byte postfix)`.
    pub fn concat(&self, postfix: u8) -> i64 {
        // extra leaf bit
        self.term()
            | (i64::from(postfix)).wrapping_shl((((self.get_max_levels() - self.get_level()) << 1) + 6) as u32)
    }

    /// `makeShape()`.
    fn make_shape(&self) -> Arc<dyn Rectangle> {
        let q = &self.tree.quad;
        let mut xmin = q.xmin;
        let mut ymin = q.ymin;
        let level = self.get_level();
        let term = self.term();
        let mut l: i16 = 0;
        let mut i: i16 = 1;
        while i32::from(l) < level {
            let b = ((term as u64).wrapping_shr((64 - (i32::from(i) << 1)) as u32) & 0x3) as u8;
            match b {
                0x00 => ymin += q.level_h[l as usize],
                0x01 => {
                    xmin += q.level_w[l as usize];
                    ymin += q.level_h[l as usize];
                }
                0x02 => {} // nothing really
                0x03 => xmin += q.level_w[l as usize],
                _ => panic!("java.lang.RuntimeException: unexpected quadrant"),
            }
            l += 1;
            i += 1;
        }
        let (width, height) = if level > 0 {
            (q.level_w[(level - 1) as usize], q.level_h[(level - 1) as usize])
        } else {
            (q.grid_w, q.grid_h)
        };
        RectangleImpl::new(
            xmin,
            xmin + width,
            ymin,
            ymin + height,
            Some(SpatialContextRef::Strong(Arc::clone(&q.base.ctx))),
        )
    }

    /// `toString()`: the 64-bit binary representation of the term, with its
    /// leading zeros.
    fn to_binary(&self) -> String {
        let term = self.term();
        let mut s = String::with_capacity(64);
        let number_of_leading_zeros = term.leading_zeros();
        for _ in 0..number_of_leading_zeros {
            s.push('0');
        }
        if term != 0 {
            s.push_str(&format!("{:b}", term as u64));
        }
        s
    }
}

impl fmt::Display for PackedQuadCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_binary())
    }
}

impl LegacyCell for PackedQuadCell {
    fn state(&self) -> MutexGuard<'_, LegacyCellState> {
        legacy_cell::lock(&self.state)
    }

    fn get_max_levels(&self) -> i32 {
        self.tree.max_levels()
    }

    fn read_cell(&self, bytes: &BytesRef) {
        {
            let mut s = self.state();
            s.shape_rel = None;
            s.shape = None;
            s.bytes = bytes.bytes.clone();
            s.b_off = bytes.offset;
            s.b_len = bytes.length as i16 as u16 as usize; // (short) bytes.length
            self.term.store(long_from_byte_array(&s.bytes, bytes.offset), Ordering::Relaxed);
        }
        self.read_leaf_adjust();
    }

    fn read_leaf_adjust(&self) {
        let level = self.get_level();
        let max_levels = self.get_max_levels();
        let term = self.term();
        let mut s = self.state();
        s.is_leaf = (0x1 & term) == 0x1;
        if level == max_levels {
            s.is_leaf = true;
        }
    }

    fn get_sub_cell(&self, p: &dyn Point) -> CellRef {
        self.tree.get_cell(p, self.get_level() + 1) // not performant!
    }

    fn get_sub_cells(&self) -> Vec<CellRef> {
        let term = self.term();
        let start = PackedQuadCell::build(&self.tree, if (term & 0x1) == 0x1 { term - 1 } else { term });
        let mut cells: Vec<CellRef> = Vec::with_capacity(4);
        let mut pqc = start.next_cell(true).expect("a sub-cell");
        for _ in 0..3 {
            let next = pqc.next_cell(false).expect("a sibling sub-cell");
            cells.push(Arc::new(pqc));
            pqc = next;
        }
        cells.push(Arc::new(pqc));
        cells
    }
}

impl CellCanPrune for PackedQuadCell {
    /// Inherited from `QuadCell`.
    fn get_sub_cells_size(&self) -> i32 {
        4
    }
}

impl Cell for PackedQuadCell {
    crate::legacy_cell_common!();

    fn get_token_bytes_with_leaf(&self, result: Option<BytesRef>) -> BytesRef {
        let mut result = self.get_token_bytes_no_leaf(result);
        if self.is_leaf() {
            result.bytes[8 - 1] |= 0x1; // set leaf
        }
        result
    }

    fn get_token_bytes_no_leaf(&self, result: Option<BytesRef>) -> BytesRef {
        let mut result = result.unwrap_or_else(|| BytesRef::new(vec![0u8; 8]));
        if result.bytes.len() < 8 {
            result.bytes = vec![0u8; 8];
        }
        long_to_byte_array(self.term(), &mut result.bytes);
        result.offset = 0;
        result.length = 8;
        // no leaf
        result.bytes[8 - 1] &= !1; // clear last bit (leaf bit)
        result
    }

    fn get_level(&self) -> i32 {
        (((self.term() as u64) >> 1) & 0x1F) as i32
    }

    /// Inherited from `QuadCell.getShape()`, over this class's `makeShape()`.
    fn get_shape(&self) -> Arc<dyn Shape> {
        if let Some(shape) = &self.state().shape {
            return Arc::clone(shape);
        }
        let shape: Arc<dyn Shape> = self.make_shape();
        self.state().shape = Some(Arc::clone(&shape));
        shape
    }

    fn is_prefix_of(&self, c: &dyn Cell) -> bool {
        let cell = c
            .as_any()
            .downcast_ref::<PackedQuadCell>()
            .unwrap_or_else(|| panic!("java.lang.ClassCastException: {c:?} cannot be cast to PackedQuadCell"));
        (self.term() == 0x0) || self.is_internal_prefix(cell)
    }

    fn compare_to_no_leaf(&self, from_cell: &dyn Cell) -> i32 {
        let b = from_cell
            .as_any()
            .downcast_ref::<PackedQuadCell>()
            .unwrap_or_else(|| panic!("java.lang.ClassCastException: {from_cell:?} cannot be cast to PackedQuadCell"));
        // TODO clear last bit without the condition
        let term = self.term();
        let b_term = b.term();
        let this_term = if (0x1 & term) == 0x1 { term - 1 } else { term };
        let from_term = if (0x1 & b_term) == 0x1 { b_term - 1 } else { b_term };
        match (this_term as u64).cmp(&(from_term as u64)) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.PackedQuadPrefixTree.PrefixTreeIterator`
/// (`protected class PrefixTreeIterator extends CellIterator`): this is a
/// streamlined version of `TreeCellIterator`, with built-in support to
/// prune at detailLevel (but not recursively upwards).
///
/// Java's class declares its own private `thisCell` and `nextCell`, which
/// shadow `CellIterator`'s; the inherited `thisCell()` reads the
/// superclass's field, which this class never sets, so
/// [`this_cell`](CellIterator::this_cell) fails as Java's does.
pub struct PrefixTreeIterator {
    super_base: CellIteratorBase,
    shape: Arc<dyn Shape>,
    this_cell: Option<Arc<PackedQuadCell>>,
    next_cell: Option<Arc<PackedQuadCell>>,
    level: i16,
    detail_level: i16,
    prune_iter: Option<Box<dyn CellIterator>>,
    leafy_prune: bool,
}

impl PrefixTreeIterator {
    /// `PrefixTreeIterator(Shape shape, short detailLevel)`.
    fn new(tree: &PackedQuadPrefixTree, shape: Arc<dyn Shape>, detail_level: i16) -> Self {
        let world = PackedQuadCell::build(&tree.me(), 0x0);
        Self {
            super_base: CellIteratorBase::default(),
            shape,
            this_cell: world.next_cell(true).map(Arc::new),
            next_cell: None,
            level: 0,
            detail_level,
            prune_iter: None,
            leafy_prune: tree.is_prune_leafy_branches(),
        }
    }

    fn pruned(&mut self, rel: SpatialRelation) -> bool {
        if rel == SpatialRelation::Intersects && self.leafy_prune && self.level == self.detail_level - 1 {
            let this_cell = self.this_cell.as_ref().expect("thisCell is set");
            let mut prune_iter = this_cell.get_next_level_cells(Some(Arc::clone(&self.shape)));
            let mut leaves = 0;
            while prune_iter.has_next() {
                prune_iter.next();
                leaves += 1;
            }
            self.prune_iter = Some(prune_iter);
            return leaves == 4;
        }
        false
    }
}

impl CellIterator for PrefixTreeIterator {
    fn has_next(&mut self) -> bool {
        if self.next_cell.is_some() {
            return true;
        }
        // loop until we're at the end of the quad tree or we hit a relation
        while let Some(this_cell) = self.this_cell.clone() {
            let rel = this_cell.get_shape().relate(&*self.shape);
            if rel == SpatialRelation::Disjoint {
                self.this_cell = this_cell.next_cell(false).map(Arc::new);
            } else {
                // within || intersects || contains
                this_cell.set_shape_rel(Some(rel));
                self.next_cell = Some(Arc::clone(&this_cell));
                if rel == SpatialRelation::Within {
                    this_cell.set_leaf();
                    self.this_cell = this_cell.next_cell(false).map(Arc::new);
                } else {
                    // intersects || contains
                    self.level = this_cell.get_level() as i16;
                    if self.level == self.detail_level || self.pruned(rel) {
                        this_cell.set_leaf();
                        if self.shape.as_point().is_some() {
                            this_cell.set_shape_rel(Some(SpatialRelation::Within));
                            self.this_cell = None;
                        } else {
                            self.this_cell = this_cell.next_cell(false).map(Arc::new);
                        }
                        break;
                    }
                    self.this_cell = this_cell.next_cell(true).map(Arc::new);
                }
                break;
            }
        }
        self.next_cell.is_some()
    }

    fn next(&mut self) -> CellRef {
        if self.next_cell.is_none() && !self.has_next() {
            panic!("java.util.NoSuchElementException");
        }
        // overriding since this implementation sets thisCell in hasNext
        let temp = self.next_cell.take().expect("hasNext() set nextCell");
        temp
    }

    fn this_cell(&self) -> CellRef {
        self.super_base.this_cell()
    }

    fn remove(&mut self) {
        // no-op
    }
}
