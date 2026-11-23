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

//! The types declared in `GeoLatitudeZone.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::util::java_math;
use std::f64::consts::PI;
use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::bounded::Bounded;
use super::bounds::Bounds;
use super::distance_style::DistanceStyle;
use super::geo_area_shape::GeoAreaShape;
use super::geo_b_box::GeoBBox;
use super::geo_b_box_factory::GeoBBoxFactory;
use super::geo_base_bounds::GeoBaseBounds;
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::geo_sizeable::GeoSizeable;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;
use super::{d2s, java_min};

/// This GeoBBox represents an area rectangle limited only in latitude.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoLatitudeZone`.
pub(crate) struct GeoLatitudeZone {
    planet_model: Arc<PlanetModel>,
    top_lat: f64,
    bottom_lat: f64,
    cos_top_lat: f64,
    cos_bottom_lat: f64,
    top_plane: SidedPlane,
    bottom_plane: SidedPlane,
    interior_point: GeoPoint,
    top_boundary_point: GeoPoint,
    bottom_boundary_point: GeoPoint,
    edge_points: [GeoPoint; 2],
}

impl GeoLatitudeZone {
    /// `new GeoLatitudeZone(PlanetModel, double topLat, double bottomLat)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a plane cannot be sided.
    pub fn new(planet_model: &Arc<PlanetModel>, top_lat: f64, bottom_lat: f64) -> Result<Self> {
        let pm = planet_model;
        let sin_top_lat = java_math::sin(top_lat);
        let sin_bottom_lat = java_math::sin(bottom_lat);
        let cos_top_lat = java_math::cos(top_lat);
        let cos_bottom_lat = java_math::cos(bottom_lat);
        let middle_lat = (top_lat + bottom_lat) * 0.5;
        let sin_middle_lat = java_math::sin(middle_lat);
        let interior_point = GeoPoint::new_sin_cos(
            pm,
            sin_middle_lat,
            0.0,
            java_math::sqrt(1.0 - sin_middle_lat * sin_middle_lat),
            1.0,
        );
        let top_boundary_point = GeoPoint::new_sin_cos(
            pm,
            sin_top_lat,
            0.0,
            java_math::sqrt(1.0 - sin_top_lat * sin_top_lat),
            1.0,
        );
        let bottom_boundary_point = GeoPoint::new_sin_cos(
            pm,
            sin_bottom_lat,
            0.0,
            java_math::sqrt(1.0 - sin_bottom_lat * sin_bottom_lat),
            1.0,
        );
        let top_plane = SidedPlane::new_horizontal(&interior_point, pm, sin_top_lat)?;
        let bottom_plane = SidedPlane::new_horizontal(&interior_point, pm, sin_bottom_lat)?;
        Ok(Self {
            planet_model: pm.clone(),
            top_lat,
            bottom_lat,
            cos_top_lat,
            cos_bottom_lat,
            top_plane,
            bottom_plane,
            interior_point,
            edge_points: [top_boundary_point.clone(), bottom_boundary_point.clone()],
            top_boundary_point,
            bottom_boundary_point,
        })
    }

    /// `new GeoLatitudeZone(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        let b = so::read_double(input)?;
        Self::new(planet_model, a, b)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.top_lat)?;
        so::write_double(output, self.bottom_lat)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let pm = &self.planet_model;
        let top_distance =
            ds.compute_plane_distance_xyz(pm, &self.top_plane, x, y, z, &[&self.bottom_plane]);
        let bottom_distance =
            ds.compute_plane_distance_xyz(pm, &self.bottom_plane, x, y, z, &[&self.top_plane]);
        Ok(java_min(top_distance, bottom_distance))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.top_boundary_point == self.top_boundary_point
            && other.bottom_boundary_point == self.bottom_boundary_point
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.top_boundary_point.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.bottom_boundary_point.java_hash_code());
        result
    }
}

impl_geo_base_area_shape_common!(
    GeoLatitudeZone,
    "org.apache.lucene.spatial3d.geom.GeoLatitudeZone",
    geo_b_box
);
impl_geo_base_area_relationship!(GeoLatitudeZone);

impl Membership for GeoLatitudeZone {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.top_plane.is_within_xyz(x, y, z) && self.bottom_plane.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoLatitudeZone {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds
            .no_longitude_bound()
            .add_horizontal_plane(pm, self.top_lat, &self.top_plane, &[])
            .add_horizontal_plane(pm, self.bottom_lat, &self.bottom_plane, &[]);
    }
}

impl GeoShape for GeoLatitudeZone {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(
        &self,
        p: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        let pm = &self.planet_model;
        p.intersects(
            pm,
            &self.top_plane,
            notable_points,
            &[],
            bounds,
            &[&self.bottom_plane],
        ) || p.intersects(
            pm,
            &self.bottom_plane,
            notable_points,
            &[],
            bounds,
            &[&self.top_plane],
        )
    }
}

impl GeoAreaShape for GeoLatitudeZone {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(&self.top_plane, &[], &[&self.bottom_plane])
            || geo_shape.intersects(&self.bottom_plane, &[], &[&self.top_plane])
    }
}

impl GeoSizeable for GeoLatitudeZone {
    fn get_radius(&self) -> f64 {
        if self.top_lat > 0.0 && self.bottom_lat < 0.0 {
            return PI;
        }
        let mut max_cos_lat = self.cos_top_lat;
        if max_cos_lat < self.cos_bottom_lat {
            max_cos_lat = self.cos_bottom_lat;
        }
        max_cos_lat * PI
    }

    fn get_center(&self) -> GeoPoint {
        self.interior_point.clone()
    }
}

impl GeoBBox for GeoLatitudeZone {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let new_top_lat = self.top_lat + angle;
        let new_bottom_lat = self.bottom_lat - angle;
        GeoBBoxFactory::make_geo_bbox(&self.planet_model, new_top_lat, new_bottom_lat, -PI, PI)
    }
}

impl fmt::Display for GeoLatitudeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoLatitudeZone: {{planetmodel={}, toplat={}({}), bottomlat={}({})}}",
            self.planet_model,
            d2s(self.top_lat),
            d2s(self.top_lat * 180.0 / PI),
            d2s(self.bottom_lat),
            d2s(self.bottom_lat * 180.0 / PI)
        )
    }
}
