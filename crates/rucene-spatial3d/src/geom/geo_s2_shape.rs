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


//! The types declared in `GeoS2Shape.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::bounded::Bounded;
use super::bounds::Bounds;
use super::distance_style::DistanceStyle;
use super::geo_area_shape::GeoAreaShape;
use super::geo_base_bounds::GeoBaseBounds;
use super::geo_point::GeoPoint;
use super::geo_polygon::GeoPolygon;
use super::geo_shape::GeoShape;
use super::java_min;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;

/// Fast implementation of a polygon representing S2 geometry cell. There are
/// no checks validating that points are convex therefore users must be
/// provide four points in CCW or the logic will fail.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoS2Shape`.
pub(crate) struct GeoS2Shape {
    planet_model: Arc<PlanetModel>,
    point1: GeoPoint,
    point2: GeoPoint,
    point3: GeoPoint,
    point4: GeoPoint,
    plane1: SidedPlane,
    plane2: SidedPlane,
    plane3: SidedPlane,
    plane4: SidedPlane,
    plane1_points: [GeoPoint; 2],
    plane2_points: [GeoPoint; 2],
    plane3_points: [GeoPoint; 2],
    plane4_points: [GeoPoint; 2],
    edge_points: [GeoPoint; 1],
}

impl GeoS2Shape {
    /// `new GeoS2Shape(PlanetModel, GeoPoint, GeoPoint, GeoPoint, GeoPoint)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a side plane cannot be built.
    pub fn new(planet_model: &Arc<PlanetModel>, point1: GeoPoint, point2: GeoPoint, point3: GeoPoint, point4: GeoPoint) -> Result<Self> {
        let plane1 = SidedPlane::new_cross(&point4, &point1, &point2)?;
        let plane2 = SidedPlane::new_cross(&point1, &point2, &point3)?;
        let plane3 = SidedPlane::new_cross(&point2, &point3, &point4)?;
        let plane4 = SidedPlane::new_cross(&point3, &point4, &point1)?;
        Ok(Self {
            planet_model: planet_model.clone(),
            plane1_points: [point1.clone(), point2.clone()],
            plane2_points: [point2.clone(), point3.clone()],
            plane3_points: [point3.clone(), point4.clone()],
            plane4_points: [point4.clone(), point1.clone()],
            edge_points: [point1.clone()],
            point1,
            point2,
            point3,
            point4,
            plane1,
            plane2,
            plane3,
            plane4,
        })
    }

    /// `new GeoS2Shape(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `ClassCastException` when an object is
    /// not a point; `IllegalArgumentException` from the constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let p1 = so::downcast_point(so::read_object(input)?)?;
        let p2 = so::downcast_point(so::read_object(input)?)?;
        let p3 = so::downcast_point(so::read_object(input)?)?;
        let p4 = so::downcast_point(so::read_object(input)?)?;
        Self::new(planet_model, p1, p2, p3, p4)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_object(output, &self.point1)?;
        so::write_object(output, &self.point2)?;
        so::write_object(output, &self.point3)?;
        so::write_object(output, &self.point4)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let pm = &self.planet_model;
        let plane_distance1 = ds.compute_plane_distance_xyz(pm, &self.plane1, x, y, z, &[&self.plane2, &self.plane4]);
        let plane_distance2 = ds.compute_plane_distance_xyz(pm, &self.plane2, x, y, z, &[&self.plane3, &self.plane1]);
        let plane_distance3 = ds.compute_plane_distance_xyz(pm, &self.plane3, x, y, z, &[&self.plane4, &self.plane2]);
        let plane_distance4 = ds.compute_plane_distance_xyz(pm, &self.plane4, x, y, z, &[&self.plane1, &self.plane3]);
        let point_distance1 = ds.compute_distance_xyz(&self.point1, x, y, z);
        let point_distance2 = ds.compute_distance_xyz(&self.point2, x, y, z);
        let point_distance3 = ds.compute_distance_xyz(&self.point3, x, y, z);
        let point_distance4 = ds.compute_distance_xyz(&self.point4, x, y, z);
        Ok(java_min(
            java_min(java_min(plane_distance1, plane_distance2), java_min(plane_distance3, plane_distance4)),
            java_min(java_min(point_distance1, point_distance2), java_min(point_distance3, point_distance4)),
        ))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.point1 == self.point1
            && other.point2 == self.point2
            && other.point3 == self.point3
            && other.point4 == self.point4
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result.wrapping_mul(31).wrapping_add(self.point1.java_hash_code());
        result = result.wrapping_mul(31).wrapping_add(self.point2.java_hash_code());
        result = result.wrapping_mul(31).wrapping_add(self.point3.java_hash_code());
        result = result.wrapping_mul(31).wrapping_add(self.point4.java_hash_code());
        result
    }
}

impl_geo_base_area_shape_common!(GeoS2Shape, "org.apache.lucene.spatial3d.geom.GeoS2Shape", geo_polygon);
impl_geo_base_area_relationship!(GeoS2Shape);

impl GeoPolygon for GeoS2Shape {}

impl Membership for GeoS2Shape {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.plane1.is_within_xyz(x, y, z)
            && self.plane2.is_within_xyz(x, y, z)
            && self.plane3.is_within_xyz(x, y, z)
            && self.plane4.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoS2Shape {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds
            .add_plane(pm, &self.plane1, &[&self.plane2, &self.plane4])
            .add_plane(pm, &self.plane2, &[&self.plane3, &self.plane1])
            .add_plane(pm, &self.plane3, &[&self.plane4, &self.plane2])
            .add_plane(pm, &self.plane4, &[&self.plane1, &self.plane3])
            .add_point(&self.point1)
            .add_point(&self.point2)
            .add_point(&self.point3)
            .add_point(&self.point4);
    }
}

impl GeoShape for GeoS2Shape {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(&self, p: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        let pm = &self.planet_model;
        p.intersects(pm, &self.plane1, notable_points, &self.plane1_points, bounds, &[&self.plane2, &self.plane4])
            || p.intersects(pm, &self.plane2, notable_points, &self.plane2_points, bounds, &[&self.plane3, &self.plane1])
            || p.intersects(pm, &self.plane3, notable_points, &self.plane3_points, bounds, &[&self.plane4, &self.plane2])
            || p.intersects(pm, &self.plane4, notable_points, &self.plane4_points, bounds, &[&self.plane1, &self.plane3])
    }
}

impl GeoAreaShape for GeoS2Shape {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(&self.plane1, &self.plane1_points, &[&self.plane2, &self.plane4])
            || geo_shape.intersects(&self.plane2, &self.plane2_points, &[&self.plane3, &self.plane1])
            || geo_shape.intersects(&self.plane3, &self.plane3_points, &[&self.plane4, &self.plane2])
            || geo_shape.intersects(&self.plane4, &self.plane4_points, &[&self.plane1, &self.plane3])
    }
}

impl fmt::Display for GeoS2Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoS2Shape: {{planetmodel={}, point1={}, point2={}, point3={}, point4={}}}",
            self.planet_model, self.point1, self.point2, self.point3, self.point4
        )
    }
}
