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

//! The types declared in `Geo3dPointShape.java`, of the Java package
//! `org.apache.lucene.spatial.spatial4j`.

use std::fmt;
use std::sync::{Arc, RwLock, Weak};

use rucene::error::Result;
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::geo_point_shape::GeoPointShape;
use rucene_spatial3d::geom::geo_point_shape_factory::GeoPointShapeFactory;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::jdk::panic_unsupported;
use crate::spatial4j::geo3d_rectangle_shape::Geo3dRectangleShape;
use crate::spatial4j::geo3d_shape::{lock, Geo3dShape};

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dPointShape`
/// (`public class Geo3dPointShape extends Geo3dShape<GeoPointShape>
/// implements Point`): specialization of a [`Geo3dShape`] which represents
/// a [`Point`]. Its center is itself.
pub struct Geo3dPointShape {
    /// The `Geo3dShape` fields.
    pub(crate) base: Geo3dShape,
    /// `shape`, typed.
    point: RwLock<Arc<dyn GeoPointShape>>,
    me: Weak<Geo3dPointShape>,
}

impl Geo3dPointShape {
    /// `Geo3dPointShape(GeoPointShape shape, SpatialContext spatialcontext)`.
    pub fn new(shape: Arc<dyn GeoPointShape>, spatialcontext: Arc<SpatialContext>) -> Arc<Geo3dPointShape> {
        let area: Arc<dyn GeoAreaShape> = shape.clone();
        Arc::new_cyclic(|me| Geo3dPointShape {
            base: Geo3dShape::new(area, spatialcontext),
            point: RwLock::new(shape),
            me: me.clone(),
        })
    }

    /// `shape`, as a `GeoPointShape`.
    pub fn point_shape(&self) -> Arc<dyn GeoPointShape> {
        Arc::clone(&self.point.read().unwrap_or_else(|e| e.into_inner()))
    }

    fn me(&self) -> Arc<Geo3dPointShape> {
        self.me.upgrade().expect("a Geo3dPointShape is shared as an Arc")
    }
}

impl fmt::Display for Geo3dPointShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_shape())
    }
}

impl fmt::Debug for Geo3dPointShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_shape())
    }
}

impl Point for Geo3dPointShape {
    fn reset(&self, x: f64, y: f64) {
        let shape = GeoPointShapeFactory::make_geo_point_shape(
            self.point_shape().get_planet_model(),
            y * DistanceUtils::DEGREES_TO_RADIANS,
            x * DistanceUtils::DEGREES_TO_RADIANS,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let area: Arc<dyn GeoAreaShape> = shape.clone();
        self.base.set_shape(area);
        *self.point.write().unwrap_or_else(|e| e.into_inner()) = shape;
        // center = this (the center of a point is the point itself)
        *lock(&self.base.bounding_box) = None;
    }

    fn get_x(&self) -> f64 {
        self.point_shape().get_center().get_longitude() * DistanceUtils::RADIANS_TO_DEGREES
    }

    fn get_y(&self) -> f64 {
        self.point_shape().get_center().get_latitude() * DistanceUtils::RADIANS_TO_DEGREES
    }
}

impl Shape for Geo3dPointShape {
    spatial4j::spatial4j_shape_boilerplate!("org.apache.lucene.spatial.spatial4j.Geo3dPointShape");

    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        self.base.relate_shape(other)
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        let mut bbox = lock(&self.base.bounding_box); // volatile read once
        if let Some(b) = &*bbox {
            return Arc::clone(b);
        }
        let b: Arc<dyn Rectangle> = Geo3dRectangleShape::new(self.point_shape(), Arc::clone(&self.base.spatialcontext));
        *bbox = Some(Arc::clone(&b));
        b
    }

    fn has_area(&self) -> bool {
        false
    }

    fn get_area(&self, _ctx: Option<&SpatialContext>) -> f64 {
        panic_unsupported()
    }

    /// Inherited from `Geo3dShape`; `center` is `this`.
    fn get_center(&self) -> Arc<dyn Point> {
        self.me()
    }

    fn get_buffered(&self, distance: f64, spatial_context: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let circle: Arc<dyn Shape> = spatial_context.get_shape_factory().circle(self.get_x(), self.get_y(), distance)?;
        Ok(circle)
    }

    fn is_empty(&self) -> bool {
        false
    }

    fn equals(&self, other: &dyn Shape) -> bool {
        self.base.equals_shape(other)
    }

    fn hash_code(&self) -> i32 {
        self.base.hash_code_shape()
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        Some(Arc::clone(&self.base.spatialcontext))
    }

    fn as_point(&self) -> Option<&dyn Point> {
        Some(self)
    }
}
