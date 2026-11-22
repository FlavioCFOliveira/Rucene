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


//! The types declared in `GeoWideLongitudeSlice.java`, of the Java package
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
use super::geo_wide_rectangle::GeoWideRectangle;

/// Bounding box wider than PI but limited on left and right sides (left lon,
/// right lon).
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoWideLongitudeSlice`.
pub(crate) struct GeoWideLongitudeSlice {
    planet_model: Arc<PlanetModel>,
    left_lon: f64,
    right_lon: f64,
    left_plane: SidedPlane,
    right_plane: SidedPlane,

    plane_points: [GeoPoint; 2],
    center_point: GeoPoint,
    edge_points: [GeoPoint; 1],
}

impl GeoWideLongitudeSlice {
    /// `new GeoWideLongitudeSlice(PlanetModel, double leftLon, double rightLon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for out-of-range or inconsistent bounds.
    pub fn new(planet_model: &Arc<PlanetModel>, left_lon: f64, mut right_lon: f64) -> Result<Self> {
        let pm = planet_model;
        if left_lon < -PI || left_lon > PI {
            return Err(iae("Left longitude out of range"));
        }
        if right_lon < -PI || right_lon > PI {
            return Err(iae("Right longitude out of range"));
        }
        let mut extent = right_lon - left_lon;
        if extent < 0.0 {
            extent += 2.0 * PI;
        }
        if extent < GeoWideRectangle::MIN_WIDE_EXTENT {
            return Err(iae("Width of rectangle too small"));
        }
        let stored_right_lon = right_lon;
        let sin_left_lon = left_lon.sin();
        let cos_left_lon = left_lon.cos();
        let sin_right_lon = right_lon.sin();
        let cos_right_lon = right_lon.cos();
        while left_lon > right_lon {
            right_lon += PI * 2.0;
        }
        let mut middle_lon = (left_lon + right_lon) * 0.5;
        while middle_lon > PI {
            middle_lon -= PI * 2.0;
        }
        while middle_lon < -PI {
            middle_lon += PI * 2.0;
        }
        let center_point = GeoPoint::new_lat_lon(pm, 0.0, middle_lon)?;
        let left_plane = SidedPlane::new_vertical(&center_point, cos_left_lon, sin_left_lon)?;
        let right_plane = SidedPlane::new_vertical(&center_point, cos_right_lon, sin_right_lon)?;

        Ok(Self {
            planet_model: pm.clone(),
            left_lon,
            right_lon: stored_right_lon,
            left_plane,
            right_plane,

            plane_points: [pm.north_pole.clone(), pm.south_pole.clone()],
            center_point,
            edge_points: [pm.north_pole.clone()],
        })
    }

    /// `new GeoWideLongitudeSlice(PlanetModel, InputStream)`.
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
        so::write_double(output, self.left_lon)?;
        so::write_double(output, self.right_lon)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let pm = &self.planet_model;
        let left_distance = ds.compute_plane_distance_xyz(pm, &self.left_plane, x, y, z, &[]);
        let right_distance = ds.compute_plane_distance_xyz(pm, &self.right_plane, x, y, z, &[]);
        let north_distance = ds.compute_distance_xyz(&pm.north_pole, x, y, z);
        let south_distance = ds.compute_distance_xyz(&pm.south_pole, x, y, z);
        Ok(java_min(java_min(left_distance, right_distance), java_min(north_distance, south_distance)))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.left_lon == self.left_lon
            && other.right_lon == self.right_lon
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result.wrapping_mul(31).wrapping_add(super::dhash(self.left_lon));
        result = result.wrapping_mul(31).wrapping_add(super::dhash(self.right_lon));
        result
    }
}

impl_geo_base_area_shape_common!(GeoWideLongitudeSlice, "org.apache.lucene.spatial3d.geom.GeoWideLongitudeSlice", geo_b_box);
impl_geo_base_area_relationship!(GeoWideLongitudeSlice);

impl Membership for GeoWideLongitudeSlice {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.left_plane.is_within_xyz(x, y, z) || self.right_plane.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoWideLongitudeSlice {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds
            .is_wide()
            .add_vertical_plane(pm, self.left_lon, &self.left_plane, &[])
            .add_vertical_plane(pm, self.right_lon, &self.right_plane, &[])
            .add_intersection(pm, &self.left_plane, &self.right_plane, &[])
            .add_point(&pm.north_pole)
            .add_point(&pm.south_pole);
    }
}

impl GeoShape for GeoWideLongitudeSlice {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(&self, p: &Plane, notable_points: &[GeoPoint], bounds: &[&dyn Membership]) -> bool {
        let pm = &self.planet_model;
        p.intersects(pm, &self.left_plane, notable_points, &self.plane_points, bounds, &[])
            || p.intersects(pm, &self.right_plane, notable_points, &self.plane_points, bounds, &[])
    }
}

impl GeoAreaShape for GeoWideLongitudeSlice {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(&self.left_plane, &self.plane_points, &[])
            || geo_shape.intersects(&self.right_plane, &self.plane_points, &[])
    }
}

impl GeoSizeable for GeoWideLongitudeSlice {
    fn get_radius(&self) -> f64 {
        let mut extent = self.right_lon - self.left_lon;
        if extent < 0.0 {
            extent += PI * 2.0;
        }
        java_max(PI * 0.5, extent * 0.5)
    }

    fn get_center(&self) -> GeoPoint {
        self.center_point.clone()
    }
}

impl GeoBBox for GeoWideLongitudeSlice {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let mut current_lon_span = self.right_lon - self.left_lon;
        if current_lon_span < 0.0 {
            current_lon_span += PI * 2.0;
        }
        let mut new_left_lon = self.left_lon - angle;
        let mut new_right_lon = self.right_lon + angle;
        if current_lon_span + 2.0 * angle >= PI * 2.0 {
            new_left_lon = -PI;
            new_right_lon = PI;
        }
        GeoBBoxFactory::make_geo_bbox(&self.planet_model, PI * 0.5, -PI * 0.5, new_left_lon, new_right_lon)
    }
}

impl fmt::Display for GeoWideLongitudeSlice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoWideLongitudeSlice: {{planetmodel={}, leftlon={}({}), rightlon={}({})}}",
            self.planet_model,
            d2s(self.left_lon),
            d2s(self.left_lon * 180.0 / PI),
            d2s(self.right_lon),
            d2s(self.right_lon * 180.0 / PI)
        )
    }
}
