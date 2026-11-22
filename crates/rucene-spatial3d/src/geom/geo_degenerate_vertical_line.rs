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


//! The types declared in `GeoDegenerateVerticalLine.java`, of the Java package
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
use super::{d2s, iae, java_max, java_min};
use super::geo_area::{self, GeoArea};

/// Degenerate bounding box limited on two sides (top lat, bottom lat).
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDegenerateVerticalLine`.
pub struct GeoDegenerateVerticalLine {
    planet_model: Arc<PlanetModel>,
    top_lat: f64,
    bottom_lat: f64,
    longitude: f64,
    uhc: GeoPoint,
    lhc: GeoPoint,
    top_plane: SidedPlane,
    bottom_plane: SidedPlane,
    bounding_plane: SidedPlane,
    plane: Plane,
    plane_points: [GeoPoint; 2],
    center_point: GeoPoint,
    edge_points: [GeoPoint; 1],
}

impl GeoDegenerateVerticalLine {
    /// `new GeoDegenerateVerticalLine(PlanetModel, double topLat, double
    /// bottomLat, double longitude)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for out-of-range or inconsistent bounds.
    pub fn new(planet_model: &Arc<PlanetModel>, top_lat: f64, bottom_lat: f64, longitude: f64) -> Result<Self> {
        let pm = planet_model;
        if top_lat > PI * 0.5 || top_lat < -PI * 0.5 {
            return Err(iae("Top latitude out of range"));
        }
        if bottom_lat > PI * 0.5 || bottom_lat < -PI * 0.5 {
            return Err(iae("Bottom latitude out of range"));
        }
        if top_lat < bottom_lat {
            return Err(iae("Top latitude less than bottom latitude"));
        }
        if longitude < -PI || longitude > PI {
            return Err(iae("Longitude out of range"));
        }
        let sin_top_lat = top_lat.sin();
        let cos_top_lat = top_lat.cos();
        let sin_bottom_lat = bottom_lat.sin();
        let cos_bottom_lat = bottom_lat.cos();
        let sin_longitude = longitude.sin();
        let cos_longitude = longitude.cos();
        let uhc = GeoPoint::new_sin_cos_lat_lon(pm, sin_top_lat, sin_longitude, cos_top_lat, cos_longitude, top_lat, longitude)?;
        let lhc = GeoPoint::new_sin_cos_lat_lon(pm, sin_bottom_lat, sin_longitude, cos_bottom_lat, cos_longitude, bottom_lat, longitude)?;
        let plane = Plane::new_vertical(cos_longitude, sin_longitude);
        let middle_lat = (top_lat + bottom_lat) * 0.5;
        let sin_middle_lat = middle_lat.sin();
        let cos_middle_lat = middle_lat.cos();
        let center_point = GeoPoint::new_sin_cos(pm, sin_middle_lat, sin_longitude, cos_middle_lat, cos_longitude);
        let top_plane = SidedPlane::new_horizontal(&lhc, pm, sin_top_lat)?;
        let bottom_plane = SidedPlane::new_horizontal(&uhc, pm, sin_bottom_lat)?;
        debug_assert!(top_plane.is_within(&center_point));
        debug_assert!(bottom_plane.is_within(&center_point));
        let bounding_plane = SidedPlane::new_vertical(&center_point, -sin_longitude, cos_longitude)?;
        Ok(Self {
            planet_model: pm.clone(),
            top_lat,
            bottom_lat,
            longitude,
            plane_points: [uhc.clone(), lhc.clone()],
            edge_points: [center_point.clone()],
            uhc,
            lhc,
            top_plane,
            bottom_plane,
            bounding_plane,
            plane,
            center_point,
        })
    }

    /// `new GeoDegenerateVerticalLine(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        let b = so::read_double(input)?;
        let c = so::read_double(input)?;
        Self::new(planet_model, a, b, c)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.top_lat)?;
        so::write_double(output, self.bottom_lat)?;
        so::write_double(output, self.longitude)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let distance = ds.compute_plane_distance_xyz(&self.planet_model, &self.plane, x, y, z, &[&self.top_plane, &self.bottom_plane, &self.bounding_plane]);
        let uhc_distance = ds.compute_distance_xyz(&self.uhc, x, y, z);
        let lhc_distance = ds.compute_distance_xyz(&self.lhc, x, y, z);
        Ok(java_min(distance, java_min(uhc_distance, lhc_distance)))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.uhc == self.uhc
            && other.lhc == self.lhc
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result.wrapping_mul(31).wrapping_add(self.uhc.java_hash_code());
        result = result.wrapping_mul(31).wrapping_add(self.lhc.java_hash_code());
        result
    }
}

impl_geo_base_area_shape_common!(GeoDegenerateVerticalLine, "org.apache.lucene.spatial3d.geom.GeoDegenerateVerticalLine", geo_b_box);

impl GeoArea for GeoDegenerateVerticalLine {
    fn get_relationship(&self, path: &dyn GeoShape) -> Result<i32> {
        if self.intersects_shape(path) {
            return Ok(geo_area::OVERLAPS);
        }
        if path.is_within(&self.center_point) {
            return Ok(geo_area::CONTAINS);
        }
        Ok(geo_area::DISJOINT)
    }
}

impl Membership for GeoDegenerateVerticalLine {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.plane.evaluate_is_zero_xyz(x, y, z)
            && self.bounding_plane.is_within_xyz(x, y, z)
            && self.top_plane.is_within_xyz(x, y, z)
            && self.bottom_plane.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoDegenerateVerticalLine {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds
            .add_vertical_plane(pm, self.longitude, &self.plane, &[&self.bounding_plane, &self.top_plane, &self.bottom_plane])
            .add_point(&self.uhc)
            .add_point(&self.lhc);
    }
}

impl GeoShape for GeoDegenerateVerticalLine {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(&self, p: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        p.intersects(&self.planet_model, &self.plane, notable_points, &self.plane_points, bounds, &[&self.bounding_plane, &self.top_plane, &self.bottom_plane])
    }
}

impl GeoAreaShape for GeoDegenerateVerticalLine {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(&self.plane, &self.plane_points, &[&self.bounding_plane, &self.top_plane, &self.bottom_plane])
    }
}

impl GeoSizeable for GeoDegenerateVerticalLine {
    fn get_radius(&self) -> f64 {
        let top_angle = self.center_point.arc_distance(&self.uhc);
        let bottom_angle = self.center_point.arc_distance(&self.lhc);
        java_max(top_angle, bottom_angle)
    }

    fn get_center(&self) -> GeoPoint {
        self.center_point.clone()
    }
}

impl GeoBBox for GeoDegenerateVerticalLine {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let new_top_lat = self.top_lat + angle;
        let new_bottom_lat = self.bottom_lat - angle;
        let mut new_left_lon = self.longitude - angle;
        let mut new_right_lon = self.longitude + angle;
        let current_lon_span = 2.0 * angle;
        if current_lon_span + 2.0 * angle >= PI * 2.0 {
            new_left_lon = -PI;
            new_right_lon = PI;
        }
        GeoBBoxFactory::make_geo_bbox(&self.planet_model, new_top_lat, new_bottom_lat, new_left_lon, new_right_lon)
    }
}

impl fmt::Display for GeoDegenerateVerticalLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoDegenerateVerticalLine: {{longitude={}({}), toplat={}({}), bottomlat={}({})}}",
            d2s(self.longitude),
            d2s(self.longitude * 180.0 / PI),
            d2s(self.top_lat),
            d2s(self.top_lat * 180.0 / PI),
            d2s(self.bottom_lat),
            d2s(self.bottom_lat * 180.0 / PI)
        )
    }
}
