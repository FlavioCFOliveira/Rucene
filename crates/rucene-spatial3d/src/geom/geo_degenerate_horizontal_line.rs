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

//! The types declared in `GeoDegenerateHorizontalLine.java`, of the Java package
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
use super::geo_area::{self, GeoArea};
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

/// Degenerate bounding box limited on two sides (left lon, right lon). The
/// left-right maximum extent for this shape is PI; for anything larger, use
/// [`GeoWideDegenerateHorizontalLine`](super::geo_wide_degenerate_horizontal_line::GeoWideDegenerateHorizontalLine).
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDegenerateHorizontalLine`.
pub(crate) struct GeoDegenerateHorizontalLine {
    planet_model: Arc<PlanetModel>,
    latitude: f64,
    left_lon: f64,
    right_lon: f64,
    lhc: GeoPoint,
    rhc: GeoPoint,
    plane: Plane,
    left_plane: SidedPlane,
    right_plane: SidedPlane,
    plane_points: [GeoPoint; 2],
    center_point: GeoPoint,

    edge_points: [GeoPoint; 1],
}

impl GeoDegenerateHorizontalLine {
    /// `new GeoDegenerateHorizontalLine(PlanetModel, double latitude, double leftLon, double rightLon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for out-of-range or inconsistent bounds.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        latitude: f64,
        left_lon: f64,
        mut right_lon: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        if latitude > PI * 0.5 || latitude < -PI * 0.5 {
            return Err(iae("Latitude out of range"));
        }
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
        if extent > PI {
            return Err(iae("Width of rectangle too great"));
        }
        let stored_right_lon = right_lon;
        let sin_latitude = java_math::sin(latitude);
        let cos_latitude = java_math::cos(latitude);
        let sin_left_lon = java_math::sin(left_lon);
        let cos_left_lon = java_math::cos(left_lon);
        let sin_right_lon = java_math::sin(right_lon);
        let cos_right_lon = java_math::cos(right_lon);
        let lhc = GeoPoint::new_sin_cos_lat_lon(
            pm,
            sin_latitude,
            sin_left_lon,
            cos_latitude,
            cos_left_lon,
            latitude,
            left_lon,
        )?;
        let rhc = GeoPoint::new_sin_cos_lat_lon(
            pm,
            sin_latitude,
            sin_right_lon,
            cos_latitude,
            cos_right_lon,
            latitude,
            right_lon,
        )?;
        let plane = Plane::new_horizontal(pm, sin_latitude);
        while left_lon > right_lon {
            right_lon += PI * 2.0;
        }
        let middle_lon = (left_lon + right_lon) * 0.5;
        let sin_middle_lon = java_math::sin(middle_lon);
        let cos_middle_lon = java_math::cos(middle_lon);
        let center_point = GeoPoint::new_sin_cos(
            pm,
            sin_latitude,
            sin_middle_lon,
            cos_latitude,
            cos_middle_lon,
        );
        let left_plane = SidedPlane::new_vertical(&rhc, cos_left_lon, sin_left_lon)?;
        let right_plane = SidedPlane::new_vertical(&lhc, cos_right_lon, sin_right_lon)?;
        debug_assert!(left_plane.is_within(&center_point));
        debug_assert!(right_plane.is_within(&center_point));
        Ok(Self {
            planet_model: pm.clone(),
            latitude,
            left_lon,
            right_lon: stored_right_lon,
            plane_points: [lhc.clone(), rhc.clone()],
            edge_points: [center_point.clone()],
            lhc,
            rhc,
            plane,
            left_plane,
            right_plane,
            center_point,
        })
    }

    /// `new GeoDegenerateHorizontalLine(PlanetModel, InputStream)`.
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
        so::write_double(output, self.latitude)?;
        so::write_double(output, self.left_lon)?;
        so::write_double(output, self.right_lon)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let distance = ds.compute_plane_distance_xyz(
            &self.planet_model,
            &self.plane,
            x,
            y,
            z,
            &[&self.left_plane, &self.right_plane],
        );
        let lhc_distance = ds.compute_distance_xyz(&self.lhc, x, y, z);
        let rhc_distance = ds.compute_distance_xyz(&self.rhc, x, y, z);
        Ok(java_min(distance, java_min(lhc_distance, rhc_distance)))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.lhc == self.lhc
            && other.rhc == self.rhc
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.lhc.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.rhc.java_hash_code());
        result
    }
}

impl_geo_base_area_shape_common!(
    GeoDegenerateHorizontalLine,
    "org.apache.lucene.spatial3d.geom.GeoDegenerateHorizontalLine",
    geo_b_box
);

impl GeoArea for GeoDegenerateHorizontalLine {
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

impl Membership for GeoDegenerateHorizontalLine {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.plane.evaluate_is_zero_xyz(x, y, z)
            && self.left_plane.is_within_xyz(x, y, z)
            && self.right_plane.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoDegenerateHorizontalLine {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds
            .add_horizontal_plane(
                pm,
                self.latitude,
                &self.plane,
                &[&self.left_plane, &self.right_plane],
            )
            .add_point(&self.lhc)
            .add_point(&self.rhc);
    }
}

impl GeoShape for GeoDegenerateHorizontalLine {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.to_vec()
    }

    fn intersects(
        &self,
        p: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        p.intersects(
            &self.planet_model,
            &self.plane,
            notable_points,
            &self.plane_points,
            bounds,
            &[&self.left_plane, &self.right_plane],
        )
    }
}

impl GeoAreaShape for GeoDegenerateHorizontalLine {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(
            &self.plane,
            &self.plane_points,
            &[&self.left_plane, &self.right_plane],
        )
    }
}

impl GeoSizeable for GeoDegenerateHorizontalLine {
    fn get_radius(&self) -> f64 {
        let top_angle = self.center_point.arc_distance(&self.rhc);
        let bottom_angle = self.center_point.arc_distance(&self.lhc);
        java_max(top_angle, bottom_angle)
    }

    fn get_center(&self) -> GeoPoint {
        self.center_point.clone()
    }
}

impl GeoBBox for GeoDegenerateHorizontalLine {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let new_top_lat = self.latitude + angle;
        let new_bottom_lat = self.latitude - angle;
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
        GeoBBoxFactory::make_geo_bbox(
            &self.planet_model,
            new_top_lat,
            new_bottom_lat,
            new_left_lon,
            new_right_lon,
        )
    }
}

impl fmt::Display for GeoDegenerateHorizontalLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoDegenerateHorizontalLine: {{planetmodel={}, latitude={}({}), leftlon={}({}), rightLon={}({})}}",
            self.planet_model,
            d2s(self.latitude),
            d2s(self.latitude * 180.0 / PI),
            d2s(self.left_lon),
            d2s(self.left_lon * 180.0 / PI),
            d2s(self.right_lon),
            d2s(self.right_lon * 180.0 / PI)
        )
    }
}
