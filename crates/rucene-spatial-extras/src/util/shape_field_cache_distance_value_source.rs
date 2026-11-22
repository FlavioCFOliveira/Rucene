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

//! The types declared in `ShapeFieldCacheDistanceValueSource.java`, of the
//! Java package `org.apache.lucene.spatial.util`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::double_values::{with_default, DoubleValues};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{IndexSearcher, SegmentCacheable};
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_calculator::DistanceCalculator;
use spatial4j::shape::point::Point;

use crate::jdk::addr;
use crate::util::shape_field_cache::ShapeFieldCache;
use crate::util::shape_field_cache_provider::ShapeFieldCacheProvider;

/// Equivalent to `org.apache.lucene.spatial.util.ShapeFieldCacheDistanceValueSource`
/// (`public class ShapeFieldCacheDistanceValueSource extends
/// DoubleValuesSource`): a DoubleValuesSource that returns the spatial
/// distance between an input point and a document's points in a
/// [`ShapeFieldCacheProvider`]. The shortest distance is returned if a
/// document has more than one point.
pub struct ShapeFieldCacheDistanceValueSource {
    ctx: Arc<SpatialContext>,
    from: Arc<dyn Point>,
    provider: Arc<dyn ShapeFieldCacheProvider<dyn Point>>,
    multiplier: f64,
}

impl fmt::Debug for ShapeFieldCacheDistanceValueSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_source_string())
    }
}

impl ShapeFieldCacheDistanceValueSource {
    /// `ShapeFieldCacheDistanceValueSource(SpatialContext ctx,
    /// ShapeFieldCacheProvider<Point> provider, Point from, double
    /// multiplier)`.
    pub fn new(
        ctx: Arc<SpatialContext>,
        provider: Arc<dyn ShapeFieldCacheProvider<dyn Point>>,
        from: Arc<dyn Point>,
        multiplier: f64,
    ) -> Self {
        Self {
            ctx,
            from,
            provider,
            multiplier,
        }
    }
}

struct CacheDistanceValues {
    cache: Arc<ShapeFieldCache<dyn Point>>,
    from: Arc<dyn Point>,
    calculator: Arc<dyn DistanceCalculator>,
    current_doc: Option<i32>,
    multiplier: f64,
}

impl DoubleValues for CacheDistanceValues {
    fn double_value(&mut self) -> Result<f64> {
        let current_vals = self
            .current_doc
            .and_then(|doc| self.cache.get_shapes(doc))
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: currentVals"));
        let mut v = self.calculator.distance(&*self.from, &*current_vals[0]);
        for p in &current_vals[1..] {
            v = v.min(self.calculator.distance(&*self.from, &**p));
        }
        Ok(v * self.multiplier)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        let has = self.cache.get_shapes(doc).is_some();
        self.current_doc = if has { Some(doc) } else { None };
        Ok(has)
    }
}

impl SegmentCacheable for ShapeFieldCacheDistanceValueSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl DoubleValuesSource for ShapeFieldCacheDistanceValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        reader_context: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let null_value = if self.ctx.is_geo() { 180.0 * self.multiplier } else { f64::MAX };
        Ok(Box::new(with_default(
            CacheDistanceValues {
                cache: self.provider.get_cache(&reader_context.leaf_reader())?,
                from: Arc::clone(&self.from),
                calculator: self.ctx.get_dist_calc(),
                current_doc: None,
                multiplier: self.multiplier,
            },
            null_value,
        )))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        let Some(that) = other.as_any().downcast_ref::<ShapeFieldCacheDistanceValueSource>() else {
            return false;
        };
        if std::ptr::eq(self, that) {
            return true;
        }
        // `ctx.equals` and `provider.equals` are `Object.equals`: identity.
        if !Arc::ptr_eq(&self.ctx, &that.ctx) {
            return false;
        }
        if !self.from.equals(that.from.as_shape()) {
            return false;
        }
        if addr(&*self.provider) != addr(&*that.provider) {
            return false;
        }
        self.multiplier == that.multiplier
    }

    fn source_hash(&self) -> i32 {
        self.from.hash_code()
    }

    fn to_source_string(&self) -> String {
        format!(
            "ShapeFieldCacheDistanceValueSource({}, {})",
            self.provider.provider_to_string(),
            self.from
        )
    }
}
