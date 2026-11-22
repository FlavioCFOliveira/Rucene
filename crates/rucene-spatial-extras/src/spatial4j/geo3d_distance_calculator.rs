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

//! The types declared in `Geo3dDistanceCalculator.java`, of the Java package
//! `org.apache.lucene.spatial.spatial4j`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::planet_model::PlanetModel;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_calculator::DistanceCalculator;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::circle::Circle;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;

use crate::jdk::{addr, identity_hash, panic_unsupported};
use crate::spatial4j::geo3d_point_shape::Geo3dPointShape;

const CLASS_NAME: &str = "org.apache.lucene.spatial.spatial4j.Geo3dDistanceCalculator";

/// Equivalent to `org.apache.lucene.spatial.spatial4j.Geo3dDistanceCalculator`
/// (`public class Geo3dDistanceCalculator implements DistanceCalculator`):
/// geo3d implementation of `DistanceCalculator`.
///
/// It does not override `equals`/`hashCode`/`toString`: equality is
/// identity, the hash is the core's identity-hash stand-in, and the string
/// is `Object.toString()` over that hash.
#[derive(Debug)]
pub struct Geo3dDistanceCalculator {
    /// `planetModel`.
    pub(crate) planet_model: Arc<PlanetModel>,
}

impl Geo3dDistanceCalculator {
    /// `Geo3dDistanceCalculator(PlanetModel planetModel)`.
    pub fn new(planet_model: Arc<PlanetModel>) -> Self {
        Self { planet_model }
    }

    fn geo_point(&self, lat: f64, lon: f64) -> GeoPoint {
        GeoPoint::new_lat_lon(&self.planet_model, lat, lon).unwrap_or_else(|e| panic!("{e}"))
    }
}

impl fmt::Display for Geo3dDistanceCalculator {
    /// `Object.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{CLASS_NAME}@{:x}", identity_hash(CLASS_NAME))
    }
}

impl DistanceCalculator for Geo3dDistanceCalculator {
    fn distance(&self, from: &dyn Point, to: &dyn Point) -> f64 {
        let from3d = from.as_any().downcast_ref::<Geo3dPointShape>();
        let to3d = to.as_any().downcast_ref::<Geo3dPointShape>();
        if let (Some(from3d), Some(to3d)) = (from3d, to3d) {
            let point_shape1 = from3d.point_shape();
            let point_shape2 = to3d.point_shape();
            return self
                .planet_model
                .surface_distance(&point_shape1.get_center(), &point_shape2.get_center())
                * DistanceUtils::RADIANS_TO_DEGREES;
        }
        self.distance_xy(from, to.get_x(), to.get_y())
    }

    fn distance_xy(&self, from: &dyn Point, to_x: f64, to_y: f64) -> f64 {
        let from_geo_point = match from.as_any().downcast_ref::<Geo3dPointShape>() {
            Some(from3d) => from3d.point_shape().get_center(),
            None => self.geo_point(
                from.get_y() * DistanceUtils::DEGREES_TO_RADIANS,
                from.get_x() * DistanceUtils::DEGREES_TO_RADIANS,
            ),
        };
        let to_geo_point = self.geo_point(to_y * DistanceUtils::DEGREES_TO_RADIANS, to_x * DistanceUtils::DEGREES_TO_RADIANS);
        self.planet_model.surface_distance(&from_geo_point, &to_geo_point) * DistanceUtils::RADIANS_TO_DEGREES
    }

    fn within(&self, from: &dyn Point, to_x: f64, to_y: f64, distance: f64) -> bool {
        distance < self.distance_xy(from, to_x, to_y)
    }

    fn point_on_bearing(
        &self,
        from: &Arc<dyn Point>,
        dist_deg: f64,
        bearing_deg: f64,
        ctx: &SpatialContext,
        reuse: Option<Arc<dyn Point>>,
    ) -> Result<Arc<dyn Point>> {
        let geo_from = from.as_any().downcast_ref::<Geo3dPointShape>().ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ClassCastException",
                format!("class {} cannot be cast to class {}", from.java_class_name(), "org.apache.lucene.spatial.spatial4j.Geo3dPointShape"),
            )
        })?;
        let point = geo_from.point_shape().get_center();
        let dist = DistanceUtils::DEGREES_TO_RADIANS * dist_deg;
        let bearing = DistanceUtils::DEGREES_TO_RADIANS * bearing_deg;
        let new_point = self.planet_model.surface_point_on_bearing(&point, dist, bearing)?;
        let new_lat = new_point.get_latitude() * DistanceUtils::RADIANS_TO_DEGREES;
        let new_lon = new_point.get_longitude() * DistanceUtils::RADIANS_TO_DEGREES;
        match reuse {
            Some(reuse) => {
                reuse.reset(new_lon, new_lat);
                Ok(reuse)
            }
            None => ctx.get_shape_factory().point_xy(new_lon, new_lat),
        }
    }

    fn calc_box_by_dist_from_pt(
        &self,
        from: &dyn Point,
        dist_deg: f64,
        ctx: &SpatialContext,
        _reuse: Option<Arc<dyn Rectangle>>,
    ) -> Result<Arc<dyn Rectangle>> {
        // `ctx.getShapeFactory().circle(from, distDEG)`, which reads the point's x and y
        let circle = ctx.get_shape_factory().circle(from.get_x(), from.get_y(), dist_deg)?;
        Ok(circle.get_bounding_box())
    }

    fn calc_box_by_dist_from_pt_y_horiz_axis_deg(&self, _from: &dyn Point, _dist_deg: f64, _ctx: &SpatialContext) -> f64 {
        panic_unsupported()
    }

    fn area_rect(&self, _rect: &dyn Rectangle) -> f64 {
        panic_unsupported()
    }

    fn area_circle(&self, _circle: &dyn Circle) -> f64 {
        panic_unsupported()
    }

    /// `equals(Object)`: identity.
    fn equals(&self, other: &dyn DistanceCalculator) -> bool {
        addr(self) == addr(other)
    }

    /// `hashCode()`: an identity hash.
    fn hash_code(&self) -> i32 {
        identity_hash(CLASS_NAME)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
