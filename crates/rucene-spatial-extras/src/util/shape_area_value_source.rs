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

//! The types declared in `ShapeAreaValueSource.java`, of the Java package
//! `org.apache.lucene.spatial.util`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::double_values::{with_default, DoubleValues};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{IndexSearcher, SegmentCacheable};
use spatial4j::context::spatial_context::SpatialContext;

use crate::shape_values::ShapeValues;
use crate::shape_values_source::ShapeValuesSource;

/// Equivalent to `org.apache.lucene.spatial.util.ShapeAreaValueSource`
/// (`public class ShapeAreaValueSource extends DoubleValuesSource`): the
/// area of a Shape retrieved from a `ShapeValuesSource`.
#[derive(Debug)]
pub struct ShapeAreaValueSource {
    shape_value_source: Arc<dyn ShapeValuesSource>,
    // not part of identity; should be associated with shapeValueSource indirectly
    ctx: Arc<SpatialContext>,
    geo_area: bool,
    multiplier: f64,
}

impl ShapeAreaValueSource {
    /// `ShapeAreaValueSource(ShapeValuesSource shapeValueSource,
    /// SpatialContext ctx, boolean geoArea, double multiplier)`.
    pub fn new(
        shape_value_source: Arc<dyn ShapeValuesSource>,
        ctx: Arc<SpatialContext>,
        geo_area: bool,
        multiplier: f64,
    ) -> Self {
        Self {
            shape_value_source,
            ctx,
            geo_area,
            multiplier,
        }
    }
}

struct AreaValues {
    shape_values: Box<dyn ShapeValues>,
    ctx: Option<Arc<SpatialContext>>,
    multiplier: f64,
}

impl DoubleValues for AreaValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(self.shape_values.value()?.get_area(self.ctx.as_deref()) * self.multiplier)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.shape_values.advance_exact(doc)
    }
}

impl SegmentCacheable for ShapeAreaValueSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.shape_value_source.is_cacheable(ctx)
    }
}

impl DoubleValuesSource for ShapeAreaValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let shape_values = self.shape_value_source.get_values(ctx)?;
        Ok(Box::new(with_default(
            AreaValues {
                shape_values,
                ctx: if self.geo_area {
                    Some(Arc::clone(&self.ctx))
                } else {
                    None
                },
                multiplier: self.multiplier,
            },
            0.0,
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
        let Some(that) = other.as_any().downcast_ref::<ShapeAreaValueSource>() else {
            return false;
        };
        if std::ptr::eq(self, that) {
            return true;
        }
        if self.geo_area != that.geo_area {
            return false;
        }
        self.shape_value_source.source_eq(&*that.shape_value_source)
    }

    fn source_hash(&self) -> i32 {
        let mut result = self.shape_value_source.source_hash();
        result = result
            .wrapping_mul(31)
            .wrapping_add(if self.geo_area { 1 } else { 0 });
        result
    }

    fn to_source_string(&self) -> String {
        format!("area({},geo={})", self.shape_value_source, self.geo_area)
    }
}
