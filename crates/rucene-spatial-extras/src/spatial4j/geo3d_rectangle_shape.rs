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

//! The types declared in `Geo3dRectangleShape.java`, of the Java package
//! `org.apache.lucene.spatial.spatial4j`.

use std::fmt;
use std::sync::{Arc, RwLock, Weak};

use rucene::error::Result;
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::geo_b_box::GeoBBox;
use rucene_spatial3d::geom::geo_b_box_factory::GeoBBoxFactory;
use rucene_spatial3d::geom::geo_point_shape_factory::GeoPointShapeFactory;
use rucene_spatial3d::geom::lat_lon_bounds::LatLonBounds;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::jdk::panic_unsupported;
use crate::spatial4j::geo3d_point_shape::Geo3dPointShape;
use crate::spatial4j::geo3d_shape::{lock, Geo3dShape};

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dRectangleShape`
/// (`public class Geo3dRectangleShape extends Geo3dShape<GeoBBox> implements
/// Rectangle`): specialization of a [`Geo3dShape`] which represents a
/// [`Rectangle`]. Its bounding box is itself.
///
/// As in Java, `reset` replaces the wrapped shape but not the `minX`,
/// `maxX`, `minY`, `maxY` fields.
pub struct Geo3dRectangleShape {
    /// The `Geo3dShape` fields.
    pub(crate) base: Geo3dShape,
    /// `shape`, typed.
    bbox: RwLock<Arc<dyn GeoBBox>>,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    me: Weak<Geo3dRectangleShape>,
}

impl Geo3dRectangleShape {
    /// `Geo3dRectangleShape(GeoBBox shape, SpatialContext spatialcontext,
    /// double minX, double maxX, double minY, double maxY)`.
    pub fn with_bounds(
        shape: Arc<dyn GeoBBox>,
        spatialcontext: Arc<SpatialContext>,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
    ) -> Arc<Geo3dRectangleShape> {
        let area: Arc<dyn GeoAreaShape> = shape.clone();
        Arc::new_cyclic(|me| Geo3dRectangleShape {
            base: Geo3dShape::new(area, spatialcontext),
            bbox: RwLock::new(shape),
            min_x,
            max_x,
            min_y,
            max_y,
            me: me.clone(),
        })
    }

    /// `Geo3dRectangleShape(GeoBBox shape, SpatialContext spatialcontext)`:
    /// the bounds are those of the shape (`setBoundsFromshape()`).
    pub fn new(shape: Arc<dyn GeoBBox>, spatialcontext: Arc<SpatialContext>) -> Arc<Geo3dRectangleShape> {
        let mut bounds = LatLonBounds::new();
        shape.get_bounds(&mut bounds);
        let min_x = if bounds.check_no_longitude_bound() {
            -180.0
        } else {
            unbox(bounds.get_left_longitude()) * DistanceUtils::RADIANS_TO_DEGREES
        };
        let min_y = if bounds.check_no_bottom_latitude_bound() {
            -90.0
        } else {
            unbox(bounds.get_min_latitude()) * DistanceUtils::RADIANS_TO_DEGREES
        };
        let max_x = if bounds.check_no_longitude_bound() {
            180.0
        } else {
            unbox(bounds.get_right_longitude()) * DistanceUtils::RADIANS_TO_DEGREES
        };
        let max_y = if bounds.check_no_top_latitude_bound() {
            90.0
        } else {
            unbox(bounds.get_max_latitude()) * DistanceUtils::RADIANS_TO_DEGREES
        };
        Self::with_bounds(shape, spatialcontext, min_x, max_x, min_y, max_y)
    }

    /// `shape`, as a `GeoBBox`.
    pub fn bbox_shape(&self) -> Arc<dyn GeoBBox> {
        Arc::clone(&self.bbox.read().unwrap_or_else(|e| e.into_inner()))
    }

    fn me(&self) -> Arc<Geo3dRectangleShape> {
        self.me.upgrade().expect("a Geo3dRectangleShape is shared as an Arc")
    }
}

/// Java's unboxing of a `Double` bound: `NullPointerException` for `null`.
fn unbox(v: Option<f64>) -> f64 {
    v.unwrap_or_else(|| panic!("java.lang.NullPointerException"))
}

impl fmt::Display for Geo3dRectangleShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_shape())
    }
}

impl fmt::Debug for Geo3dRectangleShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_shape())
    }
}

impl Rectangle for Geo3dRectangleShape {
    fn reset(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) {
        let shape = GeoBBoxFactory::make_geo_bbox(
            self.bbox_shape().get_planet_model(),
            max_y * DistanceUtils::DEGREES_TO_RADIANS,
            min_y * DistanceUtils::DEGREES_TO_RADIANS,
            min_x * DistanceUtils::DEGREES_TO_RADIANS,
            max_x * DistanceUtils::DEGREES_TO_RADIANS,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let area: Arc<dyn GeoAreaShape> = shape.clone();
        self.base.set_shape(area);
        *self.bbox.write().unwrap_or_else(|e| e.into_inner()) = shape;
        *lock(&self.base.center) = None;
        *lock(&self.base.bounding_box) = None;
    }

    fn get_width(&self) -> f64 {
        let mut result = self.get_max_x() - self.get_min_x();
        if result < 0.0 {
            result += 360.0;
        }
        result
    }

    fn get_height(&self) -> f64 {
        self.get_max_y() - self.get_min_y()
    }

    fn get_min_x(&self) -> f64 {
        self.min_x
    }

    fn get_min_y(&self) -> f64 {
        self.min_y
    }

    fn get_max_x(&self) -> f64 {
        self.max_x
    }

    fn get_max_y(&self) -> f64 {
        self.max_y
    }

    fn get_crosses_date_line(&self) -> bool {
        self.get_max_x() > 0.0 && self.get_min_x() < 0.0
    }

    fn relate_y_range(&self, min_y: f64, max_y: f64) -> SpatialRelation {
        let r = self
            .base
            .spatialcontext
            .get_shape_factory()
            .rect(-180.0, 180.0, min_y, max_y)
            .unwrap_or_else(|e| panic!("{e}"));
        self.relate(r.as_shape())
    }

    fn relate_x_range(&self, min_x: f64, max_x: f64) -> SpatialRelation {
        let r = self
            .base
            .spatialcontext
            .get_shape_factory()
            .rect(min_x, max_x, -90.0, 90.0)
            .unwrap_or_else(|e| panic!("{e}"));
        self.relate(r.as_shape())
    }
}

impl Shape for Geo3dRectangleShape {
    spatial4j::spatial4j_shape_boilerplate!("org.apache.lucene.spatial.spatial4j.Geo3dRectangleShape");

    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        self.base.relate_shape(other)
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.me()
    }

    fn has_area(&self) -> bool {
        true
    }

    fn get_area(&self, _ctx: Option<&SpatialContext>) -> f64 {
        panic_unsupported()
    }

    fn get_center(&self) -> Arc<dyn Point> {
        if let Some(c) = &*lock(&self.base.center) {
            // volatile read once
            return Arc::clone(c);
        }
        let shape = self.bbox_shape();
        let point = shape.get_center();
        let center: Arc<dyn Point> = Geo3dPointShape::new(
            GeoPointShapeFactory::make_geo_point_shape(shape.get_planet_model(), point.get_latitude(), point.get_longitude())
                .unwrap_or_else(|e| panic!("{e}")),
            Arc::clone(&self.base.spatialcontext),
        );
        *lock(&self.base.center) = Some(Arc::clone(&center));
        center
    }

    fn get_buffered(&self, distance: f64, spatial_context: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let b_box = self.bbox_shape().expand(distance * DistanceUtils::DEGREES_TO_RADIANS)?;
        Ok(Geo3dRectangleShape::new(b_box, spatial_context.self_arc()))
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

    fn as_rectangle(&self) -> Option<&dyn Rectangle> {
        Some(self)
    }
}
