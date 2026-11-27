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

//! The types declared in `dXYdZSolid.java`, of the Java package
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

/// 3D rectangle, bounded on six sides by X,Y,Z limits, degenerate in X and Z.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.dXYdZSolid`.
#[allow(non_camel_case_types)]
pub(crate) struct dXYdZSolid {
    planet_model: Arc<PlanetModel>,
    x: f64,
    min_y: f64,
    max_y: f64,
    z: f64,
    surface_points: Vec<GeoPoint>,
}

impl dXYdZSolid {
    /// `new dXYdZSolid(PlanetModel, double X, double minY, double maxY, double
    /// Z)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the Y values are in the wrong order.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        x: f64,
        min_y: f64,
        max_y: f64,
        z: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        if max_y - min_y < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Y values in wrong order or identical"));
        }
        let x_plane = Plane::new_vector_d(&X_UNIT_VECTOR, -x);
        let z_plane = Plane::new_vector_d(&Z_UNIT_VECTOR, -z);
        let min_y_plane = SidedPlane::new_xyz_vector_d(0.0, max_y, 0.0, &Y_UNIT_VECTOR, -min_y)?;
        let max_y_plane = SidedPlane::new_xyz_vector_d(0.0, min_y, 0.0, &Y_UNIT_VECTOR, -max_y)?;
        let surface_points = x_plane
            .find_intersections(pm, &z_plane, &[&min_y_plane, &max_y_plane])
            .unwrap_or_else(|| npe());
        Ok(Self {
            planet_model: pm.clone(),
            x,
            min_y,
            max_y,
            z,
            surface_points,
        })
    }

    /// `new dXYdZSolid(PlanetModel, InputStream)`.
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
        so::write_double(output, self.x)?;
        so::write_double(output, self.min_y)?;
        so::write_double(output, self.max_y)?;
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
    dXYdZSolid,
    "org.apache.lucene.spatial3d.geom.dXYdZSolid",
    planet_object
);
impl_planet_object!(dXYdZSolid);
impl XYZSolid for dXYdZSolid {}

impl Membership for dXYdZSolid {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        for p in &self.surface_points {
            if p.is_identical_xyz(x, y, z) {
                return true;
            }
        }
        false
    }
}

impl GeoArea for dXYdZSolid {
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

impl fmt::Display for dXYdZSolid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sb = String::new();
        for p in &self.surface_points {
            sb.push(' ');
            sb.push_str(&p.to_string());
            sb.push(' ');
        }
        write!(
            f,
            "dXYdZSolid: {{planetmodel={}, {}}}",
            self.planet_model, sb
        )
    }
}
