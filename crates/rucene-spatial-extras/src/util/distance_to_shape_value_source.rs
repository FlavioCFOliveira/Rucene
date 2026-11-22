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

//! The types declared in `DistanceToShapeValueSource.java`, of the Java
//! package `org.apache.lucene.spatial.util`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::double_values::{with_default, DoubleValues};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{IndexSearcher, SegmentCacheable};
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_calculator::DistanceCalculator;
use spatial4j::shape::point::Point;

use crate::jdk::{d2s, double_compare, hash_double};
use crate::shape_values::ShapeValues;
use crate::shape_values_source::ShapeValuesSource;

/// Equivalent to `org.apache.lucene.spatial.util.DistanceToShapeValueSource`
/// (`public class DistanceToShapeValueSource extends DoubleValuesSource`):
/// the distance from a provided Point to a Point retrieved from a
/// `ShapeValuesSource`. The distance is calculated via a
/// `DistanceCalculator`.
#[derive(Debug)]
pub struct DistanceToShapeValueSource {
    shape_value_source: Arc<dyn ShapeValuesSource>,
    query_point: Arc<dyn Point>,
    multiplier: f64,
    dist_calc: Arc<dyn DistanceCalculator>,
    // TODO if DoubleValues returns NaN; will things be ok?
    null_value: f64,
}

impl DistanceToShapeValueSource {
    /// `DistanceToShapeValueSource(ShapeValuesSource shapeValueSource, Point
    /// queryPoint, double multiplier, SpatialContext ctx)`.
    pub fn new(
        shape_value_source: Arc<dyn ShapeValuesSource>,
        query_point: Arc<dyn Point>,
        multiplier: f64,
        ctx: &SpatialContext,
    ) -> Self {
        Self {
            shape_value_source,
            query_point,
            multiplier,
            dist_calc: ctx.get_dist_calc(),
            null_value: if ctx.is_geo() { 180.0 * multiplier } else { f64::MAX },
        }
    }
}

struct DistanceValues {
    shape_values: Box<dyn ShapeValues>,
    query_point: Arc<dyn Point>,
    multiplier: f64,
    dist_calc: Arc<dyn DistanceCalculator>,
}

impl DoubleValues for DistanceValues {
    fn double_value(&mut self) -> Result<f64> {
        let center = self.shape_values.value()?.get_center();
        Ok(self.dist_calc.distance(&*self.query_point, &*center) * self.multiplier)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.shape_values.advance_exact(doc)
    }
}

impl SegmentCacheable for DistanceToShapeValueSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.shape_value_source.is_cacheable(ctx)
    }
}

impl DoubleValuesSource for DistanceToShapeValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let shape_values = self.shape_value_source.get_values(ctx)?;
        Ok(Box::new(with_default(
            DistanceValues {
                shape_values,
                query_point: Arc::clone(&self.query_point),
                multiplier: self.multiplier,
                dist_calc: Arc::clone(&self.dist_calc),
            },
            self.null_value,
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
        let Some(that) = other.as_any().downcast_ref::<DistanceToShapeValueSource>() else {
            return false;
        };
        if std::ptr::eq(self, that) {
            return true;
        }
        if !self.query_point.equals(that.query_point.as_shape()) {
            return false;
        }
        if double_compare(that.multiplier, self.multiplier) != 0 {
            return false;
        }
        if !self.shape_value_source.source_eq(&*that.shape_value_source) {
            return false;
        }
        self.dist_calc.equals(&*that.dist_calc)
    }

    fn source_hash(&self) -> i32 {
        let mut result = self.shape_value_source.source_hash();
        result = result.wrapping_mul(31).wrapping_add(self.query_point.hash_code());
        result = result.wrapping_mul(31).wrapping_add(hash_double(self.multiplier));
        result
    }

    fn to_source_string(&self) -> String {
        format!(
            "distance({} to {})*{})",
            self.query_point,
            self.shape_value_source,
            d2s(self.multiplier)
        )
    }
}
