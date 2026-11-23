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

//! The types declared in `GeoRectangle.java`, of the Java package
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
use super::{d2s, iae, java_max, java_min};

/// Bounding box limited on four sides (top lat, bottom lat, left lon, right
/// lon). The left-right maximum extent for this shape is PI; for anything
/// larger, use [`GeoWideRectangle`](super::geo_wide_rectangle::GeoWideRectangle).
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoRectangle`.
///
/// Java's class is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub struct GeoRectangle {
    planet_model: Arc<PlanetModel>,
    top_lat: f64,
    bottom_lat: f64,
    left_lon: f64,
    right_lon: f64,
    cos_middle_lat: f64,
    ulhc: GeoPoint,
    urhc: GeoPoint,
    lrhc: GeoPoint,
    llhc: GeoPoint,
    top_plane: SidedPlane,
    bottom_plane: SidedPlane,
    left_plane: SidedPlane,
    right_plane: SidedPlane,
    backing_plane: SidedPlane,
    top_plane_points: [GeoPoint; 2],
    bottom_plane_points: [GeoPoint; 2],
    left_plane_points: [GeoPoint; 2],
    right_plane_points: [GeoPoint; 2],
    center_point: GeoPoint,
    edge_points: [GeoPoint; 1],
}

impl GeoRectangle {
    /// `new GeoRectangle(PlanetModel, double topLat, double bottomLat, double
    /// leftLon, double rightLon)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for out-of-range or inconsistent bounds.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        top_lat: f64,
        bottom_lat: f64,
        left_lon: f64,
        mut right_lon: f64,
    ) -> Result<Self> {
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
        let sin_top_lat = java_math::sin(top_lat);
        let cos_top_lat = java_math::cos(top_lat);
        let sin_bottom_lat = java_math::sin(bottom_lat);
        let cos_bottom_lat = java_math::cos(bottom_lat);
        let sin_left_lon = java_math::sin(left_lon);
        let cos_left_lon = java_math::cos(left_lon);
        let sin_right_lon = java_math::sin(right_lon);
        let cos_right_lon = java_math::cos(right_lon);
        let ulhc = GeoPoint::new_sin_cos_lat_lon(
            pm,
            sin_top_lat,
            sin_left_lon,
            cos_top_lat,
            cos_left_lon,
            top_lat,
            left_lon,
        )?;
        let urhc = GeoPoint::new_sin_cos_lat_lon(
            pm,
            sin_top_lat,
            sin_right_lon,
            cos_top_lat,
            cos_right_lon,
            top_lat,
            right_lon,
        )?;
        let lrhc = GeoPoint::new_sin_cos_lat_lon(
            pm,
            sin_bottom_lat,
            sin_right_lon,
            cos_bottom_lat,
            cos_right_lon,
            bottom_lat,
            right_lon,
        )?;
        let llhc = GeoPoint::new_sin_cos_lat_lon(
            pm,
            sin_bottom_lat,
            sin_left_lon,
            cos_bottom_lat,
            cos_left_lon,
            bottom_lat,
            left_lon,
        )?;
        let middle_lat = (top_lat + bottom_lat) * 0.5;
        let sin_middle_lat = java_math::sin(middle_lat);
        let cos_middle_lat = java_math::cos(middle_lat);
        while left_lon > right_lon {
            right_lon += PI * 2.0;
        }
        let middle_lon = (left_lon + right_lon) * 0.5;
        let sin_middle_lon = java_math::sin(middle_lon);
        let cos_middle_lon = java_math::cos(middle_lon);
        let center_point = GeoPoint::new_sin_cos(
            pm,
            sin_middle_lat,
            sin_middle_lon,
            cos_middle_lat,
            cos_middle_lon,
        );
        let top_plane = SidedPlane::new_horizontal(&llhc, pm, sin_top_lat)?;
        let bottom_plane = SidedPlane::new_horizontal(&urhc, pm, sin_bottom_lat)?;
        let left_plane = SidedPlane::new_vertical(&urhc, cos_left_lon, sin_left_lon)?;
        let right_plane = SidedPlane::new_vertical(&llhc, cos_right_lon, sin_right_lon)?;
        debug_assert!(top_plane.is_within(&center_point));
        debug_assert!(bottom_plane.is_within(&center_point));
        debug_assert!(left_plane.is_within(&center_point));
        debug_assert!(right_plane.is_within(&center_point));
        let backing_plane =
            SidedPlane::new_abcd(&center_point, cos_middle_lon, sin_middle_lon, 0.0, 0.0)?;
        Ok(Self {
            planet_model: pm.clone(),
            top_lat,
            bottom_lat,
            left_lon,
            right_lon: stored_right_lon,
            cos_middle_lat,
            top_plane_points: [ulhc.clone(), urhc.clone()],
            bottom_plane_points: [llhc.clone(), lrhc.clone()],
            left_plane_points: [ulhc.clone(), llhc.clone()],
            right_plane_points: [urhc.clone(), lrhc.clone()],
            edge_points: [ulhc.clone()],
            ulhc,
            urhc,
            lrhc,
            llhc,
            top_plane,
            bottom_plane,
            left_plane,
            right_plane,
            backing_plane,
            center_point,
        })
    }

    /// `new GeoRectangle(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        let b = so::read_double(input)?;
        let c = so::read_double(input)?;
        let d = so::read_double(input)?;
        Self::new(planet_model, a, b, c, d)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.top_lat)?;
        so::write_double(output, self.bottom_lat)?;
        so::write_double(output, self.left_lon)?;
        so::write_double(output, self.right_lon)
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let pm = &self.planet_model;
        let top_distance = ds.compute_plane_distance_xyz(
            pm,
            &self.top_plane,
            x,
            y,
            z,
            &[&self.bottom_plane, &self.left_plane, &self.right_plane],
        );
        let bottom_distance = ds.compute_plane_distance_xyz(
            pm,
            &self.bottom_plane,
            x,
            y,
            z,
            &[&self.top_plane, &self.left_plane, &self.right_plane],
        );
        let left_distance = ds.compute_plane_distance_xyz(
            pm,
            &self.left_plane,
            x,
            y,
            z,
            &[&self.right_plane, &self.top_plane, &self.bottom_plane],
        );
        let right_distance = ds.compute_plane_distance_xyz(
            pm,
            &self.right_plane,
            x,
            y,
            z,
            &[&self.left_plane, &self.top_plane, &self.bottom_plane],
        );
        let ulhc_distance = ds.compute_distance_xyz(&self.ulhc, x, y, z);
        let urhc_distance = ds.compute_distance_xyz(&self.urhc, x, y, z);
        let lrhc_distance = ds.compute_distance_xyz(&self.lrhc, x, y, z);
        let llhc_distance = ds.compute_distance_xyz(&self.llhc, x, y, z);
        Ok(java_min(
            java_min(
                java_min(top_distance, bottom_distance),
                java_min(left_distance, right_distance),
            ),
            java_min(
                java_min(ulhc_distance, urhc_distance),
                java_min(lrhc_distance, llhc_distance),
            ),
        ))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.ulhc == self.ulhc
            && other.lrhc == self.lrhc
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.ulhc.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.lrhc.java_hash_code());
        result
    }
}

impl_geo_base_area_shape_common!(
    GeoRectangle,
    "org.apache.lucene.spatial3d.geom.GeoRectangle",
    geo_b_box
);
impl_geo_base_area_relationship!(GeoRectangle);

impl Membership for GeoRectangle {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.backing_plane.is_within_xyz(x, y, z)
            && self.top_plane.is_within_xyz(x, y, z)
            && self.bottom_plane.is_within_xyz(x, y, z)
            && self.left_plane.is_within_xyz(x, y, z)
            && self.right_plane.is_within_xyz(x, y, z)
    }
}

impl Bounded for GeoRectangle {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        let pm = &self.planet_model;
        GeoBaseBounds::get_bounds(self, pm, bounds);
        bounds
            .add_horizontal_plane(
                pm,
                self.top_lat,
                &self.top_plane,
                &[&self.bottom_plane, &self.left_plane, &self.right_plane],
            )
            .add_vertical_plane(
                pm,
                self.right_lon,
                &self.right_plane,
                &[&self.top_plane, &self.bottom_plane, &self.left_plane],
            )
            .add_horizontal_plane(
                pm,
                self.bottom_lat,
                &self.bottom_plane,
                &[&self.top_plane, &self.left_plane, &self.right_plane],
            )
            .add_vertical_plane(
                pm,
                self.left_lon,
                &self.left_plane,
                &[&self.top_plane, &self.bottom_plane, &self.right_plane],
            )
            .add_point(&self.ulhc)
            .add_point(&self.urhc)
            .add_point(&self.llhc)
            .add_point(&self.lrhc);
    }
}

impl GeoShape for GeoRectangle {
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
            &self.top_plane_points,
            bounds,
            &[&self.bottom_plane, &self.left_plane, &self.right_plane],
        ) || p.intersects(
            pm,
            &self.bottom_plane,
            notable_points,
            &self.bottom_plane_points,
            bounds,
            &[&self.top_plane, &self.left_plane, &self.right_plane],
        ) || p.intersects(
            pm,
            &self.left_plane,
            notable_points,
            &self.left_plane_points,
            bounds,
            &[&self.right_plane, &self.top_plane, &self.bottom_plane],
        ) || p.intersects(
            pm,
            &self.right_plane,
            notable_points,
            &self.right_plane_points,
            bounds,
            &[&self.left_plane, &self.top_plane, &self.bottom_plane],
        )
    }
}

impl GeoAreaShape for GeoRectangle {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        geo_shape.intersects(
            &self.top_plane,
            &self.top_plane_points,
            &[&self.bottom_plane, &self.left_plane, &self.right_plane],
        ) || geo_shape.intersects(
            &self.bottom_plane,
            &self.bottom_plane_points,
            &[&self.top_plane, &self.left_plane, &self.right_plane],
        ) || geo_shape.intersects(
            &self.left_plane,
            &self.left_plane_points,
            &[&self.right_plane, &self.top_plane, &self.bottom_plane],
        ) || geo_shape.intersects(
            &self.right_plane,
            &self.right_plane_points,
            &[&self.left_plane, &self.top_plane, &self.bottom_plane],
        )
    }
}

impl GeoSizeable for GeoRectangle {
    fn get_radius(&self) -> f64 {
        let center_angle =
            (self.right_lon - (self.right_lon + self.left_lon) * 0.5) * self.cos_middle_lat;
        let top_angle = self.center_point.arc_distance(&self.urhc);
        let bottom_angle = self.center_point.arc_distance(&self.llhc);
        java_max(center_angle, java_max(top_angle, bottom_angle))
    }

    fn get_center(&self) -> GeoPoint {
        self.center_point.clone()
    }
}

impl GeoBBox for GeoRectangle {
    fn expand(self: Arc<Self>, angle: f64) -> Result<Arc<dyn GeoBBox>> {
        let new_top_lat = self.top_lat + angle;
        let new_bottom_lat = self.bottom_lat - angle;
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

impl fmt::Display for GeoRectangle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoRectangle: {{planetmodel={}, toplat={}({}), bottomlat={}({}), leftlon={}({}), rightlon={}({})}}",
            self.planet_model,
            d2s(self.top_lat),
            d2s(self.top_lat * 180.0 / PI),
            d2s(self.bottom_lat),
            d2s(self.bottom_lat * 180.0 / PI),
            d2s(self.left_lon),
            d2s(self.left_lon * 180.0 / PI),
            d2s(self.right_lon),
            d2s(self.right_lon * 180.0 / PI)
        )
    }
}
