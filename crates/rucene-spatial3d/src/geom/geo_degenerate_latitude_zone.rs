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


//! The types declared in `GeoDegenerateLatitudeZone.java`, of the Java package
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
use super::d2s;
use super::geo_area::{self, GeoArea};

/// This GeoBBox represents an area rectangle of one specific latitude with
/// no longitude bounds.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDegenerateLatitudeZone`.
pub(crate) struct GeoDegenerateLatitudeZone {
    planet_model: Arc<PlanetModel>,
    latitude: f64,
    sin_latitude: f64,
    plane: Plane,
    interior_point: GeoPoint,
    edge_points: [GeoPoint; 1],
}

impl GeoDegenerateLatitudeZone {
    /// `new GeoDegenerateLatitudeZone(PlanetModel, double latitude)`.
    ///
    /// # Errors
    ///
    /// Never; the `Result` keeps the constructors uniform.
    pub fn new(planet_model: &Arc<PlanetModel>, latitude: f64) -> Result<Self> {
        let pm = planet_model;
        let sin_latitude = latitude.sin();
        let cos_latitude = latitude.cos();
        let plane = Plane::new_horizontal(pm, sin_latitude);
        let interior_point = GeoPoint::new_sin_cos(pm, sin_latitude, 0.0, cos_latitude, 1.0);
        Ok(Self {
            planet_model: pm.clone(),
            latitude,
            sin_latitude,
            plane,
            edge_points: [interior_point.clone()],
            interior_point,
        })
    }

    /// `new GeoDegenerateLatitudeZone(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        Self::new(planet_model, a)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.latitude)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(ds.compute_plane_distance_xyz(&self.planet_model, &self.plane, x, y, z, &[]))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model) && other.latitude == self.latitude
    }

    fn hash_code_impl(&self) -> i32 {
        let result = BasePlanetObject::hash_code(&self.planet_model);
        result.wrapping_mul(31).wrapping_add(super::dhash(self.latitude))
    }
}

impl_geo_base_area_shape_common!(GeoDegenerateLatitudeZone, "org.apache.lucene.spatial3d.geom.GeoDegenerateLatitudeZone", geo_b_box);

impl GeoArea for GeoDegenerateLatitudeZone {
    fn get_relationship(&self, path: &dyn GeoShape) -> Result<i32> {
        if self.intersects_shape(path) {
            return Ok(geo_area::OVERLAPS);
        }
        if path.is_within(&self.interior_point) {
            return Ok(geo_area::CONTAINS);
        }
        Ok(geo_area::DISJOINT)
    }
}

impl Membership for GeoDegenerateLatitudeZone {
    fn is_within_xyz(&self, _x: f64, _y: f64, z: f64) -> bool {
        (z - self.sin_latitude).abs() < 1e-10
    }
}

impl Bounded for GeoDegenerateLatitudeZone {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds
            .no_longitude_bound()
            .add_horizontal_plane(pm, self.latitude, &self.plane, &[]);
    }
}

impl GeoShape for GeoDegenerateLatitudeZone {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(&self, p: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        p.intersects(&self.planet_model, &self.plane, notable_points, &[], bounds, &[])
    }
}

impl GeoAreaShape for GeoDegenerateLatitudeZone {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(&self.plane, &[], &[])
    }
}

impl GeoSizeable for GeoDegenerateLatitudeZone {
    fn get_radius(&self) -> f64 {
        PI
    }

    fn get_center(&self) -> GeoPoint {
        self.interior_point.clone()
    }
}

impl GeoBBox for GeoDegenerateLatitudeZone {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let new_top_lat = self.latitude + angle;
        let new_bottom_lat = self.latitude - angle;
        GeoBBoxFactory::make_geo_bbox(&self.planet_model, new_top_lat, new_bottom_lat, -PI, PI)
    }
}

impl fmt::Display for GeoDegenerateLatitudeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoDegenerateLatitudeZone: {{planetmodel={}, lat={}({})}}",
            self.planet_model,
            d2s(self.latitude),
            d2s(self.latitude * 180.0 / PI)
        )
    }
}
