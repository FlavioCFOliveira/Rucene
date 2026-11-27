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

//! The types declared in `GeoStandardCircle.java`, of the Java package
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
use super::geo_area::{self, GeoArea};
use super::geo_area_shape::GeoAreaShape;
use super::geo_base_area_shape::GeoBaseAreaShape;
use super::geo_base_bounds::GeoBaseBounds;
use super::geo_circle::GeoCircle;
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::geo_sizeable::GeoSizeable;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;
use super::vector::Vector;
use super::{d2s, iae, npe};

/// Circular area with a center and cutoff angle that represents the latitude
/// and longitude distance from the center where the planet will be cut.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoStandardCircle`.
///
/// Java's class is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub struct GeoStandardCircle {
    planet_model: Arc<PlanetModel>,
    center: GeoPoint,
    cutoff_angle: f64,
    /// `null` (None) when the circle covers the whole planet.
    circle_plane: Option<SidedPlane>,
    edge_points: Vec<GeoPoint>,
}

impl GeoStandardCircle {
    /// `new GeoStandardCircle(PlanetModel, double lat, double lon, double
    /// cutoffAngle)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for out-of-bounds arguments or a circle
    /// too small to construct.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        lat: f64,
        lon: f64,
        cutoff_angle: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        if lat < -PI * 0.5 || lat > PI * 0.5 {
            return Err(iae("Latitude out of bounds"));
        }
        if lon < -PI || lon > PI {
            return Err(iae("Longitude out of bounds"));
        }
        if cutoff_angle < 0.0 || cutoff_angle > PI {
            return Err(iae("Cutoff angle out of bounds"));
        }
        if cutoff_angle < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Cutoff angle cannot be effectively zero"));
        }
        let center = GeoPoint::new_lat_lon(pm, lat, lon)?;
        let mut upper_lat = lat + cutoff_angle;
        let mut upper_lon = lon;
        if upper_lat > PI * 0.5 {
            upper_lon += PI;
            if upper_lon > PI {
                upper_lon -= 2.0 * PI;
            }
            upper_lat = PI - upper_lat;
        }
        let mut lower_lat = lat - cutoff_angle;
        let mut lower_lon = lon;
        if lower_lat < -PI * 0.5 {
            lower_lon += PI;
            if lower_lon > PI {
                lower_lon -= 2.0 * PI;
            }
            lower_lat = -PI - lower_lat;
        }
        let upper_point = GeoPoint::new_lat_lon(pm, upper_lat, upper_lon)?;
        let lower_point = GeoPoint::new_lat_lon(pm, lower_lat, lower_lon)?;
        let circle_plane;
        let edge_points;
        if (cutoff_angle - PI).abs() < Vector::MINIMUM_RESOLUTION {
            circle_plane = None;
            edge_points = Vec::new();
        } else {
            let normal_plane =
                Plane::construct_normalized_z_plane_points(&[&upper_point, &lower_point, &center])
                    .unwrap_or_else(|| npe());
            let Some(cp) = SidedPlane::construct_normalized_perpendicular_sided_plane(
                &center,
                &normal_plane,
                &upper_point,
                &lower_point,
            )?
            else {
                return Err(iae(format!(
                    "Couldn't construct circle plane, probably too small?  Cutoff angle = {}; upperPoint = {}; lowerPoint = {}",
                    d2s(cutoff_angle),
                    upper_point,
                    lower_point
                )));
            };
            let Some(recomputed_intersection_point) =
                cp.get_sample_intersection_point(pm, &normal_plane)
            else {
                return Err(iae(format!(
                    "Couldn't construct intersection point, probably circle too small?  Plane = {}",
                    cp
                )));
            };
            edge_points = vec![recomputed_intersection_point];
            circle_plane = Some(cp);
        }
        Ok(Self {
            planet_model: pm.clone(),
            center,
            cutoff_angle,
            circle_plane,
            edge_points,
        })
    }

    /// `new GeoStandardCircle(PlanetModel, InputStream)`.
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
        so::write_double(output, self.center.get_latitude())?;
        so::write_double(output, self.center.get_longitude())?;
        so::write_double(output, self.cutoff_angle)
    }

    fn distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        Ok(ds.compute_distance_xyz(&self.center, x, y, z))
    }

    fn delta_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        // Inherited `GeoBaseDistanceShape.deltaDistance`.
        super::geo_base_distance_shape::GeoBaseDistanceShape::delta_distance(
            self.distance(ds, x, y, z),
        )
    }

    fn distance_bounds(
        &self,
        bounds: &mut dyn Bounds,
        _ds: &dyn DistanceStyle,
        _distance_value: f64,
    ) -> Result<()> {
        self.get_bounds(bounds);
        Ok(())
    }

    fn outside_distance(&self, ds: &dyn DistanceStyle, x: f64, y: f64, z: f64) -> Result<f64> {
        let circle_plane = self.circle_plane.as_ref().unwrap_or_else(|| npe());
        Ok(ds.compute_plane_distance_xyz(&self.planet_model, circle_plane, x, y, z, &[]))
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            && other.center == self.center
            && other.cutoff_angle == self.cutoff_angle
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.center.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(super::dhash(self.cutoff_angle));
        result
    }
}

impl_geo_base_area_shape_common!(
    GeoStandardCircle,
    "org.apache.lucene.spatial3d.geom.GeoStandardCircle",
    geo_circle
);
impl_geo_base_distance_shape!(GeoStandardCircle);

impl GeoArea for GeoStandardCircle {
    fn get_relationship(&self, geo_shape: &dyn GeoShape) -> Result<i32> {
        if self.circle_plane.is_none() {
            if !geo_shape.get_edge_points().is_empty() {
                return Ok(geo_area::WITHIN);
            }
            return Ok(geo_area::OVERLAPS);
        }
        GeoBaseAreaShape::get_relationship(self, geo_shape)
    }
}

impl Membership for GeoStandardCircle {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        match &self.circle_plane {
            None => true,
            Some(cp) => cp.is_within_xyz(x, y, z),
        }
    }
}

impl Bounded for GeoStandardCircle {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        GeoBaseBounds::get_bounds(self, &self.planet_model, bounds);
        let Some(circle_plane) = &self.circle_plane else {
            return;
        };
        bounds.add_point(&self.center);
        bounds.add_plane(&self.planet_model, circle_plane, &[]);
    }
}

impl GeoShape for GeoStandardCircle {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        self.edge_points.clone()
    }

    fn intersects(
        &self,
        p: &Plane,
        notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
    ) -> bool {
        let Some(circle_plane) = &self.circle_plane else {
            return false;
        };
        circle_plane.intersects(&self.planet_model, p, notable_points, &[], bounds, &[])
    }
}

impl GeoAreaShape for GeoStandardCircle {
    fn intersects_shape(&self, geo_shape: &dyn GeoShape) -> bool {
        let Some(circle_plane) = &self.circle_plane else {
            return false;
        };
        geo_shape.intersects(circle_plane, &[], &[])
    }
}

impl GeoSizeable for GeoStandardCircle {
    fn get_radius(&self) -> f64 {
        self.cutoff_angle
    }

    fn get_center(&self) -> GeoPoint {
        self.center.clone()
    }
}

impl GeoCircle for GeoStandardCircle {}

impl fmt::Display for GeoStandardCircle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GeoStandardCircle: {{planetmodel={}, center={}, radius={}({})}}",
            self.planet_model,
            self.center,
            d2s(self.cutoff_angle),
            d2s(self.cutoff_angle * 180.0 / PI)
        )
    }
}
