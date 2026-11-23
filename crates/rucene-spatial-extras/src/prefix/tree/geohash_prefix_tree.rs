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

//! The types declared in `GeohashPrefixTree.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::io::geohash_utils::GeohashUtils;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::prefix::tree::cell::{Cell, CellRef};
use crate::prefix::tree::cell_can_prune::CellCanPrune;
use crate::prefix::tree::cell_iterator::CellIterator;
use crate::prefix::tree::legacy_cell::{self, LegacyCell, LegacyCellState};
use crate::prefix::tree::legacy_prefix_tree::{self, LegacyPrefixTree};
use crate::prefix::tree::spatial_prefix_tree::{SpatialPrefixTree, SpatialPrefixTreeBase};
use crate::prefix::tree::spatial_prefix_tree_factory::{
    SpatialPrefixTreeFactory, SpatialPrefixTreeFactoryBase,
};

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.GeohashPrefixTree.Factory`
/// (`public static class Factory extends SpatialPrefixTreeFactory`): factory
/// for creating [`GeohashPrefixTree`] instances with useful defaults.
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
        let grid = GeohashPrefixTree::new(
            Arc::clone(self.base.ctx()),
            GeohashPrefixTree::get_max_levels_possible(),
        )?;
        Ok(grid.get_level_for_distance(degrees))
    }

    fn new_spt(&self) -> Result<Arc<dyn SpatialPrefixTree>> {
        Ok(GeohashPrefixTree::new(
            Arc::clone(self.base.ctx()),
            self.base
                .max_levels
                .unwrap_or_else(GeohashPrefixTree::get_max_levels_possible),
        )?)
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.GeohashPrefixTree`
/// (`public class GeohashPrefixTree extends LegacyPrefixTree`): a
/// [`SpatialPrefixTree`] based on Geohashes. Uses `GeohashUtils` to do all
/// the geohash work.
#[derive(Debug)]
pub struct GeohashPrefixTree {
    base: SpatialPrefixTreeBase,
    me: Weak<GeohashPrefixTree>,
}

impl GeohashPrefixTree {
    /// `GeohashPrefixTree(SpatialContext ctx, int maxLevels)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for world bounds that are not lat-lon, or
    /// `maxLevels` out of `[1, getMaxLevelsPossible()]`.
    pub fn new(ctx: Arc<SpatialContext>, max_levels: i32) -> Result<Arc<GeohashPrefixTree>> {
        let base = SpatialPrefixTreeBase::new(Arc::clone(&ctx), max_levels);
        let bounds = ctx.get_world_bounds();
        if bounds.get_min_x() != -180.0 {
            return Err(LuceneError::illegal_argument(format!(
                "Geohash only supports lat-lon world bounds. Got {bounds}"
            )));
        }
        let maxp = Self::get_max_levels_possible();
        if max_levels <= 0 || max_levels > maxp {
            return Err(LuceneError::illegal_argument(format!(
                "maxLevels must be [1-{maxp}] but got {max_levels}"
            )));
        }
        Ok(Arc::new_cyclic(|me| GeohashPrefixTree {
            base,
            me: me.clone(),
        }))
    }

    /// `getMaxLevelsPossible()`: any more than this and there's no point
    /// (double lat and lon are the same).
    pub fn get_max_levels_possible() -> i32 {
        GeohashUtils::MAX_PRECISION
    }

    fn me(&self) -> Arc<GeohashPrefixTree> {
        self.me
            .upgrade()
            .expect("a GeohashPrefixTree is shared as an Arc")
    }

    /// `stringToBytesPlus1(String token)`: copies an ASCII token to a byte
    /// array with one extra spot for an eventual `LEAF_BYTE` if needed.
    fn string_to_bytes_plus1(token: &str) -> Vec<u8> {
        let units: Vec<u16> = token.encode_utf16().collect();
        let mut bytes = vec![0u8; units.len() + 1];
        for (i, u) in units.iter().enumerate() {
            bytes[i] = *u as u8;
        }
        bytes
    }
}

impl fmt::Display for GeohashPrefixTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_named("GeohashPrefixTree"))
    }
}

impl SpatialPrefixTree for GeohashPrefixTree {
    fn base(&self) -> &SpatialPrefixTreeBase {
        &self.base
    }

    fn get_level_for_distance(&self, dist: f64) -> i32 {
        if dist == 0.0 {
            return self.base.max_levels; // short circuit
        }
        let level = GeohashUtils::lookup_hash_len_for_width_height(dist, dist);
        level.min(self.base.max_levels).max(1)
    }

    fn get_distance_for_level(&self, level: i32) -> Result<f64> {
        legacy_prefix_tree::legacy_get_distance_for_level(self, level)
    }

    fn get_world_cell(&self) -> CellRef {
        GhCell::from_bytes(&self.me(), Vec::new(), 0, 0)
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

impl LegacyPrefixTree for GeohashPrefixTree {
    fn get_cell(&self, p: &dyn Point, level: i32) -> CellRef {
        // args are lat,lon (y,x)
        GhCell::from_geohash(
            &self.me(),
            &GeohashUtils::encode_lat_lon_with(p.get_y(), p.get_x(), level),
        )
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.GeohashPrefixTree.GhCell`
/// (`private class GhCell extends LegacyCell`): a cell of a
/// [`GeohashPrefixTree`].
#[derive(Debug)]
pub(crate) struct GhCell {
    tree: Arc<GeohashPrefixTree>,
    state: Mutex<LegacyCellState>,
    /// `geohash`: cache; never has leaf byte, simply a geohash.
    geohash: Mutex<Option<String>>,
}

impl GhCell {
    /// `GhCell(String geohash)`.
    fn from_geohash(tree: &Arc<GeohashPrefixTree>, geohash: &str) -> CellRef {
        let len = geohash.encode_utf16().count();
        let cell = GhCell {
            tree: Arc::clone(tree),
            state: Mutex::new(LegacyCellState::new(
                GeohashPrefixTree::string_to_bytes_plus1(geohash),
                0,
                len,
            )),
            geohash: Mutex::new(Some(geohash.to_string())),
        };
        cell.read_leaf_adjust();
        if cell.is_leaf() && cell.get_level() < cell.get_max_levels() {
            // we don't have a leaf byte at max levels (an opt)
            let units: Vec<u16> = geohash.encode_utf16().collect();
            *cell.geohash.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(String::from_utf16_lossy(&units[..units.len() - 1]));
        }
        Arc::new(cell)
    }

    /// `GhCell(byte[] bytes, int off, int len)`.
    fn from_bytes(
        tree: &Arc<GeohashPrefixTree>,
        bytes: Vec<u8>,
        off: usize,
        len: usize,
    ) -> CellRef {
        let cell = GhCell {
            tree: Arc::clone(tree),
            state: Mutex::new(LegacyCellState::new(bytes, off, len)),
            geohash: Mutex::new(None),
        };
        cell.read_leaf_adjust();
        Arc::new(cell)
    }

    /// `getGeohash()`.
    fn get_geohash(&self) -> String {
        let cached = self
            .geohash
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(geohash) = cached {
            return geohash;
        }
        let token = self.get_token_bytes_no_leaf(None);
        let geohash = token.utf8_to_string().unwrap_or_else(|_| {
            String::from_utf8_lossy(&token.bytes[token.offset..token.offset + token.length])
                .into_owned()
        });
        *self.geohash.lock().unwrap_or_else(|e| e.into_inner()) = Some(geohash.clone());
        geohash
    }
}

impl fmt::Display for GhCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&legacy_cell::legacy_to_string(self))
    }
}

impl LegacyCell for GhCell {
    fn state(&self) -> MutexGuard<'_, LegacyCellState> {
        legacy_cell::lock(&self.state)
    }

    fn get_max_levels(&self) -> i32 {
        self.tree.base.max_levels
    }

    fn read_cell(&self, bytes: &BytesRef) {
        legacy_cell::legacy_read_cell(self, bytes);
        *self.geohash.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn get_sub_cell(&self, p: &dyn Point) -> CellRef {
        self.tree.get_cell(p, self.get_level() + 1) // not performant!
    }

    fn get_sub_cells(&self) -> Vec<CellRef> {
        let hashes = GeohashUtils::get_sub_geohashes(&self.get_geohash()); // sorted
        hashes
            .iter()
            .map(|hash| GhCell::from_geohash(&self.tree, hash))
            .collect()
    }
}

impl CellCanPrune for GhCell {
    fn get_sub_cells_size(&self) -> i32 {
        32 // 8x4
    }
}

impl Cell for GhCell {
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
        let shape: Arc<dyn Shape> =
            GeohashUtils::decode_boundary(&self.get_geohash(), &self.tree.base.ctx)
                .unwrap_or_else(|e| panic!("{e}"));
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
