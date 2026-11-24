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

//! The types declared in `S2PrefixTree.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;
use s2_geometry::s2_cell_id::S2CellId;
use s2_geometry::s2_lat_lng::S2LatLng;
use s2_geometry::s2_projections::S2Projections;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::shape::Shape;

use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::cell_iterator::{CellIterator, ListCellIterator};
use crate::prefix::tree::filter_cell_iterator::FilterCellIterator;
use crate::prefix::tree::s2_prefix_tree_cell::S2PrefixTreeCell;
use crate::prefix::tree::s2_shape_factory::{as_s2_shape_factory, S2ShapeFactory};
use crate::prefix::tree::spatial_prefix_tree::{
    default_get_tree_cell_iterator, SpatialPrefixTree, SpatialPrefixTreeBase,
};
use crate::prefix::tree::spatial_prefix_tree_factory::{
    SpatialPrefixTreeFactory, SpatialPrefixTreeFactoryBase,
};

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.S2PrefixTree.Factory`
/// (`protected static class Factory extends SpatialPrefixTreeFactory`):
/// factory for creating [`S2PrefixTree`] instances with useful defaults.
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
        let grid = S2PrefixTree::new(
            Arc::clone(self.base.ctx()),
            S2PrefixTree::get_max_levels_for_arity(1),
        )?;
        Ok(grid.get_level_for_distance(degrees))
    }

    fn new_spt(&self) -> Result<Arc<dyn SpatialPrefixTree>> {
        Ok(S2PrefixTree::new(
            Arc::clone(self.base.ctx()),
            self.base
                .max_levels
                .unwrap_or_else(|| S2PrefixTree::get_max_levels_for_arity(1)),
        )?)
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.S2PrefixTree`
/// (`public class S2PrefixTree extends SpatialPrefixTree`): spatial prefix
/// tree for S2 Geometry. Shape factories of the given `SpatialContext` must
/// implement the interface [`S2ShapeFactory`].
///
/// The tree can be configured on how it divided itself by providing an
/// arity. The default arity is 1 which divided every sub-cell in 4 (except
/// the first level that is always divided by 6). Arity 2 divides sub-cells
/// in 16 and arity 3 in 64 sub-cells.
pub struct S2PrefixTree {
    base: SpatialPrefixTreeBase,
    /// `s2ShapeFactory`: factory to generate S2 cell shapes (the context's
    /// shape factory).
    s2_shape_factory: Arc<dyn S2ShapeFactory>,
    /// `arity`.
    pub(crate) arity: i32,
    me: Weak<S2PrefixTree>,
}

impl S2PrefixTree {
    /// `S2PrefixTree(SpatialContext ctx, int maxLevels)`.
    ///
    /// # Errors
    ///
    /// As [`with_arity`](Self::with_arity).
    pub fn new(ctx: Arc<SpatialContext>, max_levels: i32) -> Result<Arc<S2PrefixTree>> {
        Self::with_arity(ctx, max_levels, 1)
    }

    /// `S2PrefixTree(SpatialContext ctx, int maxLevels, int arity)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the context's shape factory is not
    /// an `S2ShapeFactory`, or for an arity out of `[1, 3]`.
    pub fn with_arity(
        ctx: Arc<SpatialContext>,
        max_levels: i32,
        arity: i32,
    ) -> Result<Arc<S2PrefixTree>> {
        let base = SpatialPrefixTreeBase::new(Arc::clone(&ctx), max_levels);
        let Some(shape_factory) = as_s2_shape_factory(&ctx.get_shape_factory()) else {
            return Err(LuceneError::illegal_argument(
                "Spatial context does not support S2 spatial index.",
            ));
        };
        if !(1..=3).contains(&arity) {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid value for S2 tree arity. Possible values are 1, 2 or 3. Provided value is {arity}."
            )));
        }
        Ok(Arc::new_cyclic(|me| S2PrefixTree {
            base,
            s2_shape_factory: shape_factory,
            arity,
            me: me.clone(),
        }))
    }

    /// `getMaxLevels(int arity)`: get max levels for this spatial tree.
    pub fn get_max_levels_for_arity(arity: i32) -> i32 {
        S2CellId::MAX_LEVEL / arity + 1
    }

    pub(crate) fn me(&self) -> Arc<S2PrefixTree> {
        self.me
            .upgrade()
            .expect("an S2PrefixTree is shared as an Arc")
    }

    /// `s2ShapeFactory`.
    pub(crate) fn s2_shape_factory(&self) -> &dyn S2ShapeFactory {
        &*self.s2_shape_factory
    }
}

impl fmt::Debug for S2PrefixTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S2PrefixTree")
            .field("base", &self.base)
            .field("arity", &self.arity)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for S2PrefixTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_named("S2PrefixTree"))
    }
}

impl SpatialPrefixTree for S2PrefixTree {
    fn base(&self) -> &SpatialPrefixTreeBase {
        &self.base
    }

    fn get_level_for_distance(&self, dist: f64) -> i32 {
        if dist == 0.0 {
            return self.base.max_levels;
        }
        let mut level =
            S2Projections::MAX_WIDTH.get_min_level(dist * DistanceUtils::DEGREES_TO_RADIANS);
        let round_level = if level % self.arity != 0 { 1 } else { 0 };
        level = level / self.arity + round_level;
        self.base.max_levels.min(level + 1)
    }

    fn get_distance_for_level(&self, level: i32) -> Result<f64> {
        if level == 0 {
            return Ok(180.0);
        }
        Ok(S2Projections::MAX_WIDTH.get_value(self.arity * (level - 1))
            * DistanceUtils::RADIANS_TO_DEGREES)
    }

    fn get_world_cell(&self) -> CellRef {
        Arc::new(S2PrefixTreeCell::new(&self.me(), None))
    }

    fn read_cell(&self, term: &BytesRef, scratch: Option<CellRef>) -> CellRef {
        let cell = scratch.unwrap_or_else(|| self.get_world_cell());
        let s2 = cell
            .as_any()
            .downcast_ref::<S2PrefixTreeCell>()
            .unwrap_or_else(|| {
                panic!("java.lang.ClassCastException: {cell:?} cannot be cast to S2PrefixTreeCell")
            });
        s2.read_cell(&self.me(), term);
        cell
    }

    fn get_tree_cell_iterator(
        &self,
        shape: Arc<dyn Shape>,
        detail_level: i32,
    ) -> Result<Box<dyn CellIterator>> {
        let Some(p) = shape.as_point() else {
            return default_get_tree_cell_iterator(self, shape, detail_level);
        };
        let id = S2CellId::from_lat_lng(&S2LatLng::from_degrees(p.get_y(), p.get_x()))
            .parent_at(self.arity * (detail_level - 1));
        let me = self.me();
        let mut cells: Vec<CellRef> = Vec::with_capacity(detail_level.max(0) as usize);
        for i in 0..(detail_level - 1).max(0) {
            cells.push(Arc::new(S2PrefixTreeCell::new(
                &me,
                Some(id.parent_at(i * self.arity)),
            )));
        }
        cells.push(Arc::new(S2PrefixTreeCell::new(&me, Some(id))));
        Ok(Box::new(FilterCellIterator::new(
            Box::new(ListCellIterator::new(cells)),
            None,
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
