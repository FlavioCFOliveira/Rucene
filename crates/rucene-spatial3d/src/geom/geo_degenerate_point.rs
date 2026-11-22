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


//! The types declared in `GeoDegeneratePoint.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::f64::consts::PI;
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::bounded::Bounded;
use super::bounds::Bounds;
use super::distance_style::DistanceStyle;
use super::geo_area_shape::GeoAreaShape;
use super::geo_b_box::GeoBBox;
use super::geo_b_box_factory::GeoBBoxFactory;
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::geo_sizeable::GeoSizeable;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::d2s;
use std::any::Any;
use std::ops::Deref;

use super::geo_area::{self, GeoArea};
use super::geo_circle::GeoCircle;
use super::geo_distance::GeoDistance;
use super::geo_distance_shape::GeoDistanceShape;
use super::geo_membership_shape::GeoMembershipShape;
use super::geo_outside_distance::GeoOutsideDistance;
use super::geo_point_shape::GeoPointShape;
use super::planet_object::PlanetObject;
use super::serializable_object::SerializableObject;

/// This class represents a degenerate point bounding box. It is not a simple
/// [`GeoPoint`] because we must have the latitude and longitude.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDegeneratePoint`.
/// Java's `extends GeoPoint` is a [`Deref`] to the point.
pub(crate) struct GeoDegeneratePoint {
    point: GeoPoint,
    planet_model: Arc<PlanetModel>,
}

impl Deref for GeoDegeneratePoint {
    type Target = GeoPoint;
    fn deref(&self) -> &GeoPoint {
        &self.point
    }
}

impl GeoDegeneratePoint {
    /// `new GeoDegeneratePoint(PlanetModel, double lat, double lon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the latitude or longitude is out of
    /// range.
    pub fn new(planet_model: &Arc<PlanetModel>, lat: f64, lon: f64) -> Result<Self> {
        Ok(Self {
            point: GeoPoint::new_lat_lon(planet_model, lat, lon)?,
            planet_model: planet_model.clone(),
        })
    }

    /// `new GeoDegeneratePoint(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            point: GeoPoint::from_stream_with_model(planet_model, input)?,
            planet_model: planet_model.clone(),
        })
    }

    fn latitude(&self) -> f64 {
        self.point.latitude_field()
    }

    fn longitude(&self) -> f64 {
        self.point.longitude_field()
    }
}

impl SerializableObject for GeoDegeneratePoint {
    fn write(&self, output: &mut dyn Write) -> Result<()> {
        self.point.write(output)
    }
    fn class_name(&self) -> &'static str {
        "org.apache.lucene.spatial3d.geom.GeoDegeneratePoint"
    }
    fn java_equals(&self, o: &dyn Any) -> bool {
        o.downcast_ref::<GeoDegeneratePoint>().is_some_and(|other| {
            self.point.as_vector().java_equals(other.point.as_vector())
                && other.latitude() == self.latitude()
                && other.longitude() == self.longitude()
        })
    }
    fn java_hash_code(&self) -> i32 {
        self.point.as_vector().java_hash_code()
    }
    fn into_planet_object(self: Arc<Self>) -> Option<Arc<dyn PlanetObject>> {
        Some(self)
    }
    fn into_geo_membership_shape(self: Arc<Self>) -> Option<Arc<dyn GeoMembershipShape>> {
        Some(self)
    }
    fn into_geo_area_shape(self: Arc<Self>) -> Option<Arc<dyn GeoAreaShape>> {
        Some(self)
    }
    fn into_geo_b_box(self: Arc<Self>) -> Option<Arc<dyn super::geo_b_box::GeoBBox>> {
        Some(self)
    }
    fn into_geo_circle(self: Arc<Self>) -> Option<Arc<dyn super::geo_circle::GeoCircle>> {
        Some(self)
    }
    fn into_geo_point_shape(self: Arc<Self>) -> Option<Arc<dyn super::geo_point_shape::GeoPointShape>> {
        Some(self)
    }
}

impl_planet_object!(GeoDegeneratePoint);
impl_geo_bounds!(GeoDegeneratePoint);

impl Membership for GeoDegeneratePoint {
    fn is_within(&self, point: &super::vector::Vector) -> bool {
        self.is_within_xyz(point.x, point.y, point.z)
    }

    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.point.is_identical_xyz(x, y, z)
    }
}

impl Bounded for GeoDegeneratePoint {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        bounds.add_point(&self.point);
    }
}

impl GeoShape for GeoDegeneratePoint {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        vec![self.point.clone()]
    }

    fn intersects(&self, plane: &Plane, _notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        if !plane.evaluate_is_zero(&self.point) {
            return false;
        }
        for m in bounds {
            if !m.is_within(&self.point) {
                return false;
            }
        }
        true
    }
}

impl GeoOutsideDistance for GeoDegeneratePoint {
    fn compute_outside_distance(&self, distance_style: &dyn DistanceStyle, point: &GeoPoint) -> Result<f64> {
        Ok(distance_style.compute_distance(&self.point, point))
    }

    fn compute_outside_distance_xyz(&self, distance_style: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(distance_style.compute_distance_xyz(&self.point, x, y, z))
    }
}

impl GeoMembershipShape for GeoDegeneratePoint {}

impl GeoArea for GeoDegeneratePoint {
    fn get_relationship(&self, shape: &dyn GeoShape) -> Result<i32> {
        if shape.is_within(&self.point) {
            return Ok(geo_area::CONTAINS);
        }
        Ok(geo_area::DISJOINT)
    }
}

impl GeoAreaShape for GeoDegeneratePoint {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.is_within(&self.point)
    }
}

impl GeoDistance for GeoDegeneratePoint {
    fn compute_distance_xyz(&self, _distance_style: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        if self.is_within_xyz(x, y, z) {
            return Ok(0.0);
        }
        Ok(f64::INFINITY)
    }
}

impl GeoDistanceShape for GeoDegeneratePoint {
    fn get_distance_bounds(&self, bounds: &mut dyn Bounds, _distance_style: &dyn DistanceStyle, _distance_value: f64) -> Result<()> {
        self.get_bounds(bounds);
        Ok(())
    }
}

impl GeoSizeable for GeoDegeneratePoint {
    fn get_radius(&self) -> f64 {
        0.0
    }

    fn get_center(&self) -> GeoPoint {
        self.point.clone()
    }
}

impl GeoCircle for GeoDegeneratePoint {}

impl GeoBBox for GeoDegeneratePoint {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let new_top_lat = self.latitude() + angle;
        let new_bottom_lat = self.latitude() - angle;
        let new_left_lon = self.longitude() - angle;
        let new_right_lon = self.longitude() + angle;
        GeoBBoxFactory::make_geo_bbox(&self.planet_model, new_top_lat, new_bottom_lat, new_left_lon, new_right_lon)
    }
}

impl GeoPointShape for GeoDegeneratePoint {}

impl fmt::Display for GeoDegeneratePoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoDegeneratePoint: {{planetmodel={}, lat={}({}), lon={}({})}}",
            self.planet_model,
            d2s(self.latitude()),
            d2s(self.latitude() * 180.0 / PI),
            d2s(self.longitude()),
            d2s(self.longitude() * 180.0 / PI)
        )
    }
}
