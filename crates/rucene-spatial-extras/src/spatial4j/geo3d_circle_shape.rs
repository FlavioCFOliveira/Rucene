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

//! The types declared in `Geo3dCircleShape.java`, of the Java package
//! `org.apache.lucene.spatial.spatial4j`.

use std::fmt;
use std::sync::{Arc, RwLock};

use rucene::error::Result;
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::geo_circle::GeoCircle;
use rucene_spatial3d::geom::geo_circle_factory::GeoCircleFactory;
use rucene_spatial3d::geom::geo_point_shape_factory::GeoPointShapeFactory;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::circle::Circle;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::jdk::{panic_unsupported, unsupported};
use crate::spatial4j::geo3d_point_shape::Geo3dPointShape;
use crate::spatial4j::geo3d_shape::{lock, Geo3dShape};

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dCircleShape`
/// (`public class Geo3dCircleShape extends Geo3dShape<GeoCircle> implements
/// Circle`): specialization of a [`Geo3dShape`] which represents a
/// [`Circle`].
pub struct Geo3dCircleShape {
    /// The `Geo3dShape` fields.
    pub(crate) base: Geo3dShape,
    /// `shape`, typed.
    circle: RwLock<Arc<dyn GeoCircle>>,
}

impl Geo3dCircleShape {
    /// `Geo3dCircleShape(GeoCircle shape, SpatialContext spatialcontext)`.
    pub fn new(shape: Arc<dyn GeoCircle>, spatialcontext: Arc<SpatialContext>) -> Arc<Geo3dCircleShape> {
        let area: Arc<dyn GeoAreaShape> = shape.clone();
        Arc::new(Geo3dCircleShape {
            base: Geo3dShape::new(area, spatialcontext),
            circle: RwLock::new(shape),
        })
    }

    /// `shape`, as a `GeoCircle`.
    pub fn circle_shape(&self) -> Arc<dyn GeoCircle> {
        Arc::clone(&self.circle.read().unwrap_or_else(|e| e.into_inner()))
    }
}

impl fmt::Display for Geo3dCircleShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_shape())
    }
}

impl fmt::Debug for Geo3dCircleShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_shape())
    }
}

impl Circle for Geo3dCircleShape {
    fn reset(&self, x: f64, y: f64, radius_deg: f64) {
        let shape = GeoCircleFactory::make_geo_circle(
            self.circle_shape().get_planet_model(),
            y * DistanceUtils::DEGREES_TO_RADIANS,
            x * DistanceUtils::DEGREES_TO_RADIANS,
            radius_deg * DistanceUtils::DEGREES_TO_RADIANS,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let area: Arc<dyn GeoAreaShape> = shape.clone();
        self.base.set_shape(area);
        *self.circle.write().unwrap_or_else(|e| e.into_inner()) = shape;
        *lock(&self.base.center) = None;
        *lock(&self.base.bounding_box) = None;
    }

    fn get_radius(&self) -> f64 {
        self.circle_shape().get_radius() * DistanceUtils::RADIANS_TO_DEGREES
    }
}

impl Shape for Geo3dCircleShape {
    spatial4j::spatial4j_shape_boilerplate!("org.apache.lucene.spatial.spatial4j.Geo3dCircleShape");

    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        self.base.relate_shape(other)
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.base.bounding_box_default()
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
        let shape = self.circle_shape();
        let center: Arc<dyn Point> = Geo3dPointShape::new(
            GeoPointShapeFactory::make_geo_point_shape(
                shape.get_planet_model(),
                shape.get_center().get_latitude(),
                shape.get_center().get_longitude(),
            )
            .unwrap_or_else(|e| panic!("{e}")),
            Arc::clone(&self.base.spatialcontext),
        );
        *lock(&self.base.center) = Some(Arc::clone(&center));
        center
    }

    fn get_buffered(&self, _distance: f64, _ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        Err(unsupported())
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

    fn as_circle(&self) -> Option<&dyn Circle> {
        Some(self)
    }
}
