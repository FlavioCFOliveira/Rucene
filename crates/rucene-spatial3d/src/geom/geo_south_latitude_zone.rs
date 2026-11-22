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


//! The types declared in `GeoSouthLatitudeZone.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

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
use super::d2s;

/// This GeoBBox represents an area rectangle limited only in top latitude (including the south pole).
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoSouthLatitudeZone`.
pub(crate) struct GeoSouthLatitudeZone {
    planet_model: Arc<PlanetModel>,
    top_lat: f64,
    cos_top_lat: f64,
    top_plane: SidedPlane,
    interior_point: GeoPoint,
    top_boundary_point: GeoPoint,
    edge_points: [GeoPoint; 1],
}

impl GeoSouthLatitudeZone {
    /// `new GeoSouthLatitudeZone(PlanetModel, double topLat)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the plane cannot be sided.
    pub fn new(planet_model: &Arc<PlanetModel>, top_lat: f64) -> Result<Self> {
        let pm = planet_model;
        let sin_top_lat = top_lat.sin();
        let cos_top_lat = top_lat.cos();
        let middle_lat = (top_lat - PI * 0.5) * 0.5;
        let sin_middle_lat = middle_lat.sin();
        let interior_point = GeoPoint::new_sin_cos(pm, sin_middle_lat, 0.0, (1.0 - sin_middle_lat * sin_middle_lat).sqrt(), 1.0);
        let top_boundary_point = GeoPoint::new_sin_cos(pm, sin_top_lat, 0.0, (1.0 - sin_top_lat * sin_top_lat).sqrt(), 1.0);
        let top_plane = SidedPlane::new_horizontal(&interior_point, pm, sin_top_lat)?;
        Ok(Self {
            planet_model: pm.clone(),
            top_lat,
            cos_top_lat,
            top_plane,
            interior_point,
            edge_points: [top_boundary_point.clone()],
            top_boundary_point,
        })
    }

    /// `new GeoSouthLatitudeZone(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        Self::new(planet_model, a)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.top_lat)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(ds.compute_plane_distance_xyz(&self.planet_model, &self.top_plane, x, y, z, &[]))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.top_boundary_point == self.top_boundary_point
    }

    fn hash_code_impl(&self) -> i32 {
        let result = BasePlanetObject::hash_code(&self.planet_model);
        result.wrapping_mul(31).wrapping_add(self.top_boundary_point.java_hash_code())
    }
}

impl_geo_base_area_shape_common!(GeoSouthLatitudeZone, "org.apache.lucene.spatial3d.geom.GeoSouthLatitudeZone", geo_b_box);
impl_geo_base_area_relationship!(GeoSouthLatitudeZone);

impl Membership for GeoSouthLatitudeZone {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.top_plane.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoSouthLatitudeZone {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds.add_horizontal_plane(pm, self.top_lat, &self.top_plane, &[]);
    }
}

impl GeoShape for GeoSouthLatitudeZone {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(&self, p: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        p.intersects(&self.planet_model, &self.top_plane, notable_points, &[], bounds, &[])
    }
}

impl GeoAreaShape for GeoSouthLatitudeZone {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(&self.top_plane, &[], &[])
    }
}

impl GeoSizeable for GeoSouthLatitudeZone {
    fn get_radius(&self) -> f64 {
        if self.top_lat > 0.0 {
            return PI;
        }
        let max_cos_lat = self.cos_top_lat;
        max_cos_lat * PI
    }

    fn get_center(&self) -> GeoPoint {
        self.interior_point.clone()
    }
}

impl GeoBBox for GeoSouthLatitudeZone {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let new_top_lat = self.top_lat + angle;
        let new_bottom_lat = -PI * 0.5;
        GeoBBoxFactory::make_geo_bbox(&self.planet_model, new_top_lat, new_bottom_lat, -PI, PI)
    }
}

impl fmt::Display for GeoSouthLatitudeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoSouthLatitudeZone: {{planetmodel={}, toplat={}({})}}",
            self.planet_model,
            d2s(self.top_lat),
            d2s(self.top_lat * 180.0 / PI)
        )
    }
}
