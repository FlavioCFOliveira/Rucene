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


//! The types declared in `dXYZSolid.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::base_xyz_solid::BaseXYZSolid;
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
use super::base_xyz_solid::{X_UNIT_VECTOR, Y_UNIT_VECTOR, Z_UNIT_VECTOR, X_VERTICAL_PLANE};

/// 3D rectangle, bounded on six sides by X,Y,Z limits, degenerate in X.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.dXYZSolid`.
#[allow(non_camel_case_types)]
pub(crate) struct dXYZSolid {
    planet_model: Arc<PlanetModel>,
    x: f64,
    min_y: f64,
    max_y: f64,
    min_z: f64,
    max_z: f64,
    x_plane: Plane,
    min_y_plane: SidedPlane,
    max_y_plane: SidedPlane,
    min_z_plane: SidedPlane,
    max_z_plane: SidedPlane,
    edge_points: Vec<GeoPoint>,
    notable_x_points: Vec<GeoPoint>,
}

impl dXYZSolid {
    /// `new dXYZSolid(PlanetModel, double x, double min_y, double max_y, double min_z, double max_z)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when values are in the wrong order or
    /// identical.
    pub fn new(planet_model: &Arc<PlanetModel>, x: f64, min_y: f64, max_y: f64, min_z: f64, max_z: f64) -> Result<Self> {
        let pm = planet_model;
        if max_y - min_y < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Y values in wrong order or identical"));
        }
        if max_z - min_z < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Z values in wrong order or identical"));
        }
        let world_min_x = pm.get_minimum_x_value();
        let world_max_x = pm.get_maximum_x_value();
        let x_plane = Plane::new_vector_d(&X_UNIT_VECTOR, -x);
        let min_y_plane = SidedPlane::new_xyz_vector_d(0.0, max_y, 0.0, &Y_UNIT_VECTOR, -min_y)?;
        let max_y_plane = SidedPlane::new_xyz_vector_d(0.0, min_y, 0.0, &Y_UNIT_VECTOR, -max_y)?;
        let min_z_plane = SidedPlane::new_xyz_vector_d(0.0, 0.0, max_z, &Z_UNIT_VECTOR, -min_z)?;
        let max_z_plane = SidedPlane::new_xyz_vector_d(0.0, 0.0, min_z, &Z_UNIT_VECTOR, -max_z)?;
        let xmin_y = x_plane.find_intersections(pm, &min_y_plane, &[&max_y_plane, &min_z_plane, &max_z_plane]).unwrap_or_else(|| npe());
        let xmax_y = x_plane.find_intersections(pm, &max_y_plane, &[&min_y_plane, &min_z_plane, &max_z_plane]).unwrap_or_else(|| npe());
        let xmin_z = x_plane.find_intersections(pm, &min_z_plane, &[&max_z_plane, &min_y_plane, &max_y_plane]).unwrap_or_else(|| npe());
        let xmax_z = x_plane.find_intersections(pm, &max_z_plane, &[&min_z_plane, &min_y_plane, &max_y_plane]).unwrap_or_else(|| npe());
        let notable_x_points = BaseXYZSolid::glue_together(&[&xmin_y, &xmax_y, &xmin_z, &xmax_z]);
        let f_x_min_y_min_z = pm.point_outside_xyz(x, min_y, min_z);
        let f_x_min_y_max_z = pm.point_outside_xyz(x, min_y, max_z);
        let f_x_max_y_min_z = pm.point_outside_xyz(x, max_y, min_z);
        let f_x_max_y_max_z = pm.point_outside_xyz(x, max_y, max_z);
        let x_edges: Vec<GeoPoint> = if x - world_min_x >= -Vector::MINIMUM_RESOLUTION
            && x - world_max_x <= Vector::MINIMUM_RESOLUTION
            && min_y < 0.0
            && max_y > 0.0
            && min_z < 0.0
            && max_z > 0.0
            && f_x_min_y_min_z
            && f_x_min_y_max_z
            && f_x_max_y_min_z
            && f_x_max_y_max_z
        {
            match x_plane.get_sample_intersection_point(pm, &X_VERTICAL_PLANE) {
                Some(int_point) => vec![int_point],
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let edge_points = BaseXYZSolid::glue_together(&[&xmin_y, &xmax_y, &xmin_z, &xmax_z, &x_edges]);
        Ok(Self {
            planet_model: pm.clone(),
            x,
            min_y,
            max_y,
            min_z,
            max_z,
            x_plane,
            min_y_plane,
            max_y_plane,
            min_z_plane,
            max_z_plane,
            edge_points,
            notable_x_points,
        })
    }

    /// `new dXYZSolid(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let x = so::read_double(input)?;
        let min_y = so::read_double(input)?;
        let max_y = so::read_double(input)?;
        let min_z = so::read_double(input)?;
        let max_z = so::read_double(input)?;
        Self::new(planet_model, x, min_y, max_y, min_z, max_z)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.x)?;
        so::write_double(output, self.min_y)?;
        so::write_double(output, self.max_y)?;
        so::write_double(output, self.min_z)?;
        so::write_double(output, self.max_z)
    }

    /// `getEdgePoints()`.
    pub(crate) fn edge_points(&self) -> &[GeoPoint] {
        &self.edge_points
    }

    fn equals_impl(&self, other: &Self) -> bool {
        if !BasePlanetObject::equals(&self.planet_model, &other.planet_model) {
            return false;
        }
        other.x_plane == self.x_plane
            && other.min_y_plane == self.min_y_plane
            && other.max_y_plane == self.max_y_plane
            && other.min_z_plane == self.min_z_plane
            && other.max_z_plane == self.max_z_plane
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result.wrapping_mul(31).wrapping_add(self.x_plane.java_hash_code());
        result = result.wrapping_mul(31).wrapping_add(self.min_y_plane.java_hash_code());
        result = result.wrapping_mul(31).wrapping_add(self.max_y_plane.java_hash_code());
        result = result.wrapping_mul(31).wrapping_add(self.min_z_plane.java_hash_code());
        result = result.wrapping_mul(31).wrapping_add(self.max_z_plane.java_hash_code());
        result
    }
}

impl_serializable_object!(dXYZSolid, "org.apache.lucene.spatial3d.geom.dXYZSolid", planet_object);
impl_planet_object!(dXYZSolid);
impl XYZSolid for dXYZSolid {}

impl Membership for dXYZSolid {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.x_plane.evaluate_is_zero_xyz(x, y, z)
            && self.min_y_plane.is_within_xyz(x, y, z)
            && self.max_y_plane.is_within_xyz(x, y, z)
            && self.min_z_plane.is_within_xyz(x, y, z)
            && self.max_z_plane.is_within_xyz(x, y, z)
    }
}

impl GeoArea for dXYZSolid {
    fn get_relationship(&self, path: &dyn GeoShape) -> Result<i32> {
        let inside_rectangle = BaseXYZSolid::is_shape_inside_area(self, path);
        if inside_rectangle == BaseXYZSolid::SOME_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        let inside_shape = BaseXYZSolid::is_area_inside_shape(self.edge_points(), path);
        if inside_shape == BaseXYZSolid::SOME_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        if inside_rectangle == BaseXYZSolid::ALL_INSIDE && inside_shape == BaseXYZSolid::ALL_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        if path.intersects(&self.x_plane, &self.notable_x_points, &[&self.min_y_plane, &self.max_y_plane, &self.min_z_plane, &self.max_z_plane]) {
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

impl fmt::Display for dXYZSolid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dXYZSolid: {{planetmodel={}, xplane={}, minYplane={}, maxYplane={}, minZplane={}, maxZplane={}}}",
            self.planet_model,
            self.x_plane, self.min_y_plane, self.max_y_plane, self.min_z_plane, self.max_z_plane
        )
    }
}
