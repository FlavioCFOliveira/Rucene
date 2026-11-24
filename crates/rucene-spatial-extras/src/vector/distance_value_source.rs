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

//! The types declared in `DistanceValueSource.java`, of the Java package
//! `org.apache.lucene.spatial.vector`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::doc_values::DocValues;
use rucene::index::{LeafReaderContext, NumericDocValues};
use rucene::search::double_values::{with_default, DoubleValues};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{IndexSearcher, SegmentCacheable};
use spatial4j::distance::distance_calculator::DistanceCalculator;
use spatial4j::shape::point::Point;

use crate::spatial_strategy::SpatialStrategy;
use crate::vector::point_vector_strategy::PointVectorStrategy;

/// Equivalent to `org.apache.lucene.spatial.vector.DistanceValueSource`
/// (`public class DistanceValueSource extends DoubleValuesSource`): an
/// implementation of the Lucene ValueSource model that returns the distance
/// for a [`PointVectorStrategy`].
#[derive(Debug)]
pub struct DistanceValueSource {
    strategy: Arc<PointVectorStrategy>,
    from: Arc<dyn Point>,
    multiplier: f64,
    null_value: f64,
}

impl DistanceValueSource {
    /// `DistanceValueSource(PointVectorStrategy strategy, Point from, double
    /// multiplier)`: constructor.
    pub fn new(strategy: Arc<PointVectorStrategy>, from: Arc<dyn Point>, multiplier: f64) -> Self {
        Self {
            strategy,
            from,
            multiplier,
            null_value: 180.0 * multiplier,
        }
    }
}

struct PointDistanceValues {
    pt_x: Box<dyn NumericDocValues>,
    pt_y: Box<dyn NumericDocValues>,
    from: Arc<dyn Point>,
    calculator: Arc<dyn DistanceCalculator>,
    multiplier: f64,
}

impl DoubleValues for PointDistanceValues {
    fn double_value(&mut self) -> Result<f64> {
        let x = f64::from_bits(self.pt_x.long_value()? as u64);
        let y = f64::from_bits(self.pt_y.long_value()? as u64);
        Ok(self.calculator.distance_xy(&*self.from, x, y) * self.multiplier)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        Ok(self.pt_x.advance_exact(doc)? && self.pt_y.advance_exact(doc)?)
    }
}

impl SegmentCacheable for DistanceValueSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        DocValues::is_cacheable(
            ctx,
            &[
                self.strategy.get_field_name_x(),
                self.strategy.get_field_name_y(),
            ],
        )
    }
}

impl DoubleValuesSource for DistanceValueSource {
    /// Returns the `DoubleValues` used by the function query.
    fn get_values<'a>(
        self: Arc<Self>,
        reader_context: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let reader = reader_context.leaf_reader();
        let pt_x = DocValues::get_numeric(&*reader, self.strategy.get_field_name_x())?;
        let pt_y = DocValues::get_numeric(&*reader, self.strategy.get_field_name_y())?;
        Ok(Box::new(with_default(
            PointDistanceValues {
                pt_x,
                pt_y,
                from: Arc::clone(&self.from),
                calculator: self.strategy.get_spatial_context().get_dist_calc(),
                multiplier: self.multiplier,
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
        let Some(that) = other.as_any().downcast_ref::<DistanceValueSource>() else {
            return false;
        };
        if std::ptr::eq(self, that) {
            return true;
        }
        if !self.from.equals(that.from.as_shape()) {
            return false;
        }
        // `strategy.equals` is `Object.equals`: identity.
        if !Arc::ptr_eq(&self.strategy, &that.strategy) {
            return false;
        }
        self.multiplier == that.multiplier
    }

    fn source_hash(&self) -> i32 {
        self.from.hash_code()
    }

    fn to_source_string(&self) -> String {
        format!("DistanceValueSource({}, {})", self.strategy, self.from)
    }
}
