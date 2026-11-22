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

//! The types declared in `BBoxValueSource.java`, of the Java package
//! `org.apache.lucene.spatial.bbox`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::doc_values::DocValues;
use rucene::index::{LeafReaderContext, NumericDocValues};
use rucene::search::SegmentCacheable;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;

use crate::bbox::b_box_strategy::BBoxStrategy;
use crate::jdk::identity_hash;
use crate::shape_values::ShapeValues;
use crate::shape_values_source::ShapeValuesSource;
use crate::spatial_strategy::SpatialStrategy;

/// Equivalent to `org.apache.lucene.spatial.bbox.BBoxValueSource`
/// (package-private `class BBoxValueSource extends ShapeValuesSource`): a
/// `ShapeValuesSource` returning the bounding box of the indexed shapes of
/// a [`BBoxStrategy`].
#[derive(Debug)]
pub(crate) struct BBoxValueSource {
    strategy: Arc<BBoxStrategy>,
}

impl BBoxValueSource {
    /// `BBoxValueSource(BBoxStrategy strategy)`.
    pub(crate) fn new(strategy: Arc<BBoxStrategy>) -> Self {
        Self { strategy }
    }
}

impl fmt::Display for BBoxValueSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bboxShape({})", self.strategy.get_field_name())
    }
}

/// The anonymous `ShapeValues` of `getValues`, which resets one reused
/// rectangle.
struct BBoxShapeValues {
    min_x: Box<dyn NumericDocValues>,
    min_y: Box<dyn NumericDocValues>,
    max_x: Box<dyn NumericDocValues>,
    max_y: Box<dyn NumericDocValues>,
    rect: Arc<dyn Rectangle>, // reused
}

impl ShapeValues for BBoxShapeValues {
    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        Ok(self.min_x.advance_exact(doc)?
            && self.min_y.advance_exact(doc)?
            && self.max_x.advance_exact(doc)?
            && self.max_y.advance_exact(doc)?)
    }

    fn value(&mut self) -> Result<Arc<dyn Shape>> {
        let min_x_value = f64::from_bits(self.min_x.long_value()? as u64);
        let min_y_value = f64::from_bits(self.min_y.long_value()? as u64);
        let max_x_value = f64::from_bits(self.max_x.long_value()? as u64);
        let max_y_value = f64::from_bits(self.max_y.long_value()? as u64);
        self.rect.reset(min_x_value, max_x_value, min_y_value, max_y_value);
        Ok(Arc::clone(&self.rect) as Arc<dyn Shape>)
    }
}

impl SegmentCacheable for BBoxValueSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        let s = &self.strategy;
        DocValues::is_cacheable(ctx, &[&s.field_min_x, &s.field_min_y, &s.field_max_x, &s.field_max_y])
    }
}

impl ShapeValuesSource for BBoxValueSource {
    fn get_values(&self, reader_context: &LeafReaderContext) -> Result<Box<dyn ShapeValues>> {
        let reader = reader_context.leaf_reader();
        let s = &self.strategy;
        let min_x = DocValues::get_numeric(&*reader, &s.field_min_x)?;
        let min_y = DocValues::get_numeric(&*reader, &s.field_min_y)?;
        let max_x = DocValues::get_numeric(&*reader, &s.field_max_x)?;
        let max_y = DocValues::get_numeric(&*reader, &s.field_max_y)?;
        // reused
        let rect = s.get_spatial_context().get_shape_factory().rect(0.0, 0.0, 0.0, 0.0)?;
        Ok(Box::new(BBoxShapeValues {
            min_x,
            min_y,
            max_x,
            max_y,
            rect,
        }))
    }

    fn source_eq(&self, other: &dyn ShapeValuesSource) -> bool {
        match other.as_any().downcast_ref::<BBoxValueSource>() {
            // `strategy.equals` is `Object.equals`: identity.
            Some(that) => Arc::ptr_eq(&self.strategy, &that.strategy),
            None => false,
        }
    }

    /// `strategy.hashCode()`: an identity hash.
    fn source_hash(&self) -> i32 {
        identity_hash("org.apache.lucene.spatial.bbox.BBoxStrategy")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
