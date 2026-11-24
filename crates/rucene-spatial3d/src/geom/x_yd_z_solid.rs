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

//! The types declared in `XYdZSolid.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::base_xyz_solid::BaseXYZSolid;
use super::base_xyz_solid::{X_UNIT_VECTOR, X_VERTICAL_PLANE, Y_UNIT_VECTOR, Z_UNIT_VECTOR};
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

/// 3D rectangle, bounded on six sides by X,Y,Z limits, degenerate in Z.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.XYdZSolid`.
#[allow(non_camel_case_types)]
pub(crate) struct XYdZSolid {
    planet_model: Arc<PlanetModel>,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    z: f64,
    min_x_plane: SidedPlane,
    max_x_plane: SidedPlane,
    min_y_plane: SidedPlane,
    max_y_plane: SidedPlane,
    z_plane: Plane,
    edge_points: Vec<GeoPoint>,
    notable_z_points: Vec<GeoPoint>,
}

impl XYdZSolid {
    /// `new XYdZSolid(PlanetModel, double min_x, double max_x, double min_y, double max_y, double z)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when values are in the wrong order or
    /// identical.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        z: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        if max_x - min_x < Vector::MINIMUM_RESOLUTION {
            return Err(iae("X values in wrong order or identical"));
        }
        if max_y - min_y < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Y values in wrong order or identical"));
        }
        let world_min_z = pm.get_minimum_z_value();
        let world_max_z = pm.get_maximum_z_value();
        let min_x_plane = SidedPlane::new_xyz_vector_d(max_x, 0.0, 0.0, &X_UNIT_VECTOR, -min_x)?;
        let max_x_plane = SidedPlane::new_xyz_vector_d(min_x, 0.0, 0.0, &X_UNIT_VECTOR, -max_x)?;
        let min_y_plane = SidedPlane::new_xyz_vector_d(0.0, max_y, 0.0, &Y_UNIT_VECTOR, -min_y)?;
        let max_y_plane = SidedPlane::new_xyz_vector_d(0.0, min_y, 0.0, &Y_UNIT_VECTOR, -max_y)?;
        let z_plane = Plane::new_vector_d(&Z_UNIT_VECTOR, -z);
        let min_xz = min_x_plane
            .find_intersections(pm, &z_plane, &[&max_x_plane, &min_y_plane, &max_y_plane])
            .unwrap_or_else(|| npe());
        let max_xz = max_x_plane
            .find_intersections(pm, &z_plane, &[&min_x_plane, &min_y_plane, &max_y_plane])
            .unwrap_or_else(|| npe());
        let min_yz = min_y_plane
            .find_intersections(pm, &z_plane, &[&max_y_plane, &min_x_plane, &max_x_plane])
            .unwrap_or_else(|| npe());
        let max_yz = max_y_plane
            .find_intersections(pm, &z_plane, &[&min_y_plane, &min_x_plane, &max_x_plane])
            .unwrap_or_else(|| npe());
        let notable_z_points = BaseXYZSolid::glue_together(&[&min_xz, &max_xz, &min_yz, &max_yz]);
        let f_min_x_min_y_z = pm.point_outside_xyz(min_x, min_y, z);
        let f_min_x_max_y_z = pm.point_outside_xyz(min_x, max_y, z);
        let f_max_x_min_y_z = pm.point_outside_xyz(max_x, min_y, z);
        let f_max_x_max_y_z = pm.point_outside_xyz(max_x, max_y, z);
        let z_edges: Vec<GeoPoint> = if z - world_min_z >= -Vector::MINIMUM_RESOLUTION
            && z - world_max_z <= Vector::MINIMUM_RESOLUTION
            && min_x < 0.0
            && max_x > 0.0
            && min_y < 0.0
            && max_y > 0.0
            && f_min_x_min_y_z
            && f_min_x_max_y_z
            && f_max_x_min_y_z
            && f_max_x_max_y_z
        {
            match z_plane.get_sample_intersection_point(pm, &X_VERTICAL_PLANE) {
                Some(int_point) => vec![int_point],
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let edge_points =
            BaseXYZSolid::glue_together(&[&min_xz, &max_xz, &min_yz, &max_yz, &z_edges]);
        Ok(Self {
            planet_model: pm.clone(),
            min_x,
            max_x,
            min_y,
            max_y,
            z,
            min_x_plane,
            max_x_plane,
            min_y_plane,
            max_y_plane,
            z_plane,
            edge_points,
            notable_z_points,
        })
    }

    /// `new XYdZSolid(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let min_x = so::read_double(input)?;
        let max_x = so::read_double(input)?;
        let min_y = so::read_double(input)?;
        let max_y = so::read_double(input)?;
        let z = so::read_double(input)?;
        Self::new(planet_model, min_x, max_x, min_y, max_y, z)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.min_x)?;
        so::write_double(output, self.max_x)?;
        so::write_double(output, self.min_y)?;
        so::write_double(output, self.max_y)?;
        so::write_double(output, self.z)
    }

    /// `getEdgePoints()`.
    pub(crate) fn edge_points(&self) -> &[GeoPoint] {
        &self.edge_points
    }

    fn equals_impl(&self, other: &Self) -> bool {
        if !BasePlanetObject::equals(&self.planet_model, &other.planet_model) {
            return false;
        }
        other.min_x_plane == self.min_x_plane
            && other.max_x_plane == self.max_x_plane
            && other.min_y_plane == self.min_y_plane
            && other.max_y_plane == self.max_y_plane
            && other.z_plane == self.z_plane
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.min_x_plane.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.max_x_plane.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.min_y_plane.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.max_y_plane.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.z_plane.java_hash_code());
        result
    }
}

impl_serializable_object!(
    XYdZSolid,
    "org.apache.lucene.spatial3d.geom.XYdZSolid",
    planet_object
);
impl_planet_object!(XYdZSolid);
impl XYZSolid for XYdZSolid {}

impl Membership for XYdZSolid {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.min_x_plane.is_within_xyz(x, y, z)
            && self.max_x_plane.is_within_xyz(x, y, z)
            && self.min_y_plane.is_within_xyz(x, y, z)
            && self.max_y_plane.is_within_xyz(x, y, z)
            && self.z_plane.evaluate_is_zero_xyz(x, y, z)
    }
}

impl GeoArea for XYdZSolid {
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
        if path.intersects(
            &self.z_plane,
            &self.notable_z_points,
            &[
                &self.min_x_plane,
                &self.max_x_plane,
                &self.min_y_plane,
                &self.max_y_plane,
            ],
        ) {
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

impl fmt::Display for XYdZSolid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "XYdZSolid: {{planetmodel={}, minXplane={}, maxXplane={}, minYplane={}, maxYplane={}, zplane={}}}",
            self.planet_model,
            self.min_x_plane, self.max_x_plane, self.min_y_plane, self.max_y_plane, self.z_plane
        )
    }
}
