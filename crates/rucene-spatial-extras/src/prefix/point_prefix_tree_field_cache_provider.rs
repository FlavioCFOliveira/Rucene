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

//! The types declared in `PointPrefixTreeFieldCacheProvider.java`, of the
//! Java package `org.apache.lucene.spatial.prefix`.

use std::fmt;
use std::sync::{Arc, Mutex};

use rucene::util::BytesRef;
use spatial4j::shape::point::Point;

use crate::jdk::identity_hash;
use crate::prefix::tree::cell::CellRef;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use crate::util::shape_field_cache_provider::{
    ShapeFieldCacheProvider, ShapeFieldCacheProviderBase,
};

const CLASS_NAME: &str = "org.apache.lucene.spatial.prefix.PointPrefixTreeFieldCacheProvider";

/// Equivalent to `org.apache.lucene.spatial.prefix.PointPrefixTreeFieldCacheProvider`
/// (`public class PointPrefixTreeFieldCacheProvider extends
/// ShapeFieldCacheProvider<Point>`): implementation of
/// `ShapeFieldCacheProvider` designed for `PrefixTreeStrategy`s that index
/// points (AND ONLY POINTS!).
pub struct PointPrefixTreeFieldCacheProvider {
    base: ShapeFieldCacheProviderBase<dyn Point>,
    grid: Arc<dyn SpatialPrefixTree>,
    scan_cell: Mutex<Option<CellRef>>, // re-used in readShape to save GC
}

impl fmt::Debug for PointPrefixTreeFieldCacheProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.provider_to_string())
    }
}

impl PointPrefixTreeFieldCacheProvider {
    /// `PointPrefixTreeFieldCacheProvider(SpatialPrefixTree grid, String
    /// shapeField, int defaultSize)`.
    pub fn new(grid: Arc<dyn SpatialPrefixTree>, shape_field: &str, default_size: i32) -> Self {
        Self {
            base: ShapeFieldCacheProviderBase::new(shape_field, default_size),
            grid,
            scan_cell: Mutex::new(None),
        }
    }
}

impl ShapeFieldCacheProvider<dyn Point> for PointPrefixTreeFieldCacheProvider {
    fn provider_base(&self) -> &ShapeFieldCacheProviderBase<dyn Point> {
        &self.base
    }

    fn read_shape(&self, term: &BytesRef) -> Option<Arc<dyn Point>> {
        let mut scan = self.scan_cell.lock().unwrap_or_else(|e| e.into_inner());
        let scan_cell = self.grid.read_cell(term, scan.take());
        *scan = Some(Arc::clone(&scan_cell));
        if scan_cell.get_level() == self.grid.get_max_levels() {
            return Some(scan_cell.get_shape().get_center());
        }
        None
    }

    fn provider_to_string(&self) -> String {
        format!("{CLASS_NAME}@{:x}", identity_hash(CLASS_NAME))
    }
}
