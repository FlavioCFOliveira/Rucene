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

//! The types declared in `XdYdZSolid.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::base_xyz_solid::BaseXYZSolid;
use super::base_xyz_solid::{X_UNIT_VECTOR, Y_UNIT_VECTOR, Z_UNIT_VECTOR};
use super::geo_area::{self, GeoArea};
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::sided_plane::SidedPlane;
use super::vector::Vector;
use super::xyz_solid::XYZSolid;
use super::{iae, npe};

/// 3D rectangle, bounded on six sides by X,Y,Z limits, degenerate in Y and Z.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.XdYdZSolid`.
#[allow(non_camel_case_types)]
pub(crate) struct XdYdZSolid {
    planet_model: Arc<PlanetModel>,
    min_x: f64,
    max_x: f64,
    y: f64,
    z: f64,
    surface_points: Vec<GeoPoint>,
}

impl XdYdZSolid {
    /// `new XdYdZSolid(PlanetModel, double minX, double maxX, double Y, double
    /// Z)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the X values are in the wrong order.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        min_x: f64,
        max_x: f64,
        y: f64,
        z: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        if max_x - min_x < Vector::MINIMUM_RESOLUTION {
            return Err(iae("X values in wrong order or identical"));
        }
        let y_plane = Plane::new_vector_d(&Y_UNIT_VECTOR, -y);
        let z_plane = Plane::new_vector_d(&Z_UNIT_VECTOR, -z);
        let min_x_plane = SidedPlane::new_xyz_vector_d(max_x, 0.0, 0.0, &X_UNIT_VECTOR, -min_x)?;
        let max_x_plane = SidedPlane::new_xyz_vector_d(min_x, 0.0, 0.0, &X_UNIT_VECTOR, -max_x)?;
        let surface_points = y_plane
            .find_intersections(pm, &z_plane, &[&min_x_plane, &max_x_plane])
            .unwrap_or_else(|| npe());
        Ok(Self {
            planet_model: pm.clone(),
            min_x,
            max_x,
            y,
            z,
            surface_points,
        })
    }

    /// `new XdYdZSolid(PlanetModel, InputStream)`.
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
        so::write_double(output, self.min_x)?;
        so::write_double(output, self.max_x)?;
        so::write_double(output, self.y)?;
        so::write_double(output, self.z)
    }

    /// `getEdgePoints()`.
    pub(crate) fn edge_points(&self) -> &[GeoPoint] {
        &self.surface_points
    }

    fn equals_impl(&self, other: &Self) -> bool {
        if !BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            || self.surface_points.len() != other.surface_points.len()
        {
            return false;
        }
        self.surface_points
            .iter()
            .zip(&other.surface_points)
            .all(|(a, b)| a == b)
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        for p in &self.surface_points {
            result = result.wrapping_mul(31).wrapping_add(p.java_hash_code());
        }
        result
    }
}

impl_serializable_object!(
    XdYdZSolid,
    "org.apache.lucene.spatial3d.geom.XdYdZSolid",
    planet_object
);
impl_planet_object!(XdYdZSolid);
impl XYZSolid for XdYdZSolid {}

impl Membership for XdYdZSolid {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        for p in &self.surface_points {
            if p.is_identical_xyz(x, y, z) {
                return true;
            }
        }
        false
    }
}

impl GeoArea for XdYdZSolid {
    fn get_relationship(&self, path: &dyn GeoShape) -> Result<i32> {
        let inside_rectangle = BaseXYZSolid::is_shape_inside_area(self, path);
        if inside_rectangle == BaseXYZSolid::SOME_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        let inside_shape = BaseXYZSolid::is_area_inside_shape(self.edge_points(), path);
        if inside_shape == BaseXYZSolid::SOME_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        if inside_rectangle == BaseXYZSolid::ALL_INSIDE && inside_shape == BaseXYZSolid::ALL_INSIDE
        {
            return Ok(geo_area::OVERLAPS);
        }
        if inside_rectangle == BaseXYZSolid::ALL_INSIDE {
            return Ok(geo_area::WITHIN);
        }
        if inside_shape == BaseXYZSolid::ALL_INSIDE {
            return Ok(geo_area::CONTAINS);
        }
        Ok(geo_area::DISJOINT)
    }
}

impl fmt::Display for XdYdZSolid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sb = String::new();
        for p in &self.surface_points {
            sb.push(' ');
            sb.push_str(&p.to_string());
            sb.push(' ');
        }
        write!(
            f,
            "XdYdZSolid: {{planetmodel={}, {}}}",
            self.planet_model, sb
        )
    }
}
