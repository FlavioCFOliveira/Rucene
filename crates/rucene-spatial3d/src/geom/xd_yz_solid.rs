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

//! The types declared in `XdYZSolid.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::base_xyz_solid::BaseXYZSolid;
use super::base_xyz_solid::{X_UNIT_VECTOR, Y_UNIT_VECTOR, Y_VERTICAL_PLANE, Z_UNIT_VECTOR};
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

/// 3D rectangle, bounded on six sides by X,Y,Z limits, degenerate in Y.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.XdYZSolid`.
#[allow(non_camel_case_types)]
pub(crate) struct XdYZSolid {
    planet_model: Arc<PlanetModel>,
    min_x: f64,
    max_x: f64,
    y: f64,
    min_z: f64,
    max_z: f64,
    min_x_plane: SidedPlane,
    max_x_plane: SidedPlane,
    y_plane: Plane,
    min_z_plane: SidedPlane,
    max_z_plane: SidedPlane,
    edge_points: Vec<GeoPoint>,
    notable_y_points: Vec<GeoPoint>,
}

impl XdYZSolid {
    /// `new XdYZSolid(PlanetModel, double min_x, double max_x, double y, double min_z, double max_z)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when values are in the wrong order or
    /// identical.
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        min_x: f64,
        max_x: f64,
        y: f64,
        min_z: f64,
        max_z: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        if max_x - min_x < Vector::MINIMUM_RESOLUTION {
            return Err(iae("X values in wrong order or identical"));
        }
        if max_z - min_z < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Z values in wrong order or identical"));
        }
        let world_min_y = pm.get_minimum_y_value();
        let world_max_y = pm.get_maximum_y_value();
        let min_x_plane = SidedPlane::new_xyz_vector_d(max_x, 0.0, 0.0, &X_UNIT_VECTOR, -min_x)?;
        let max_x_plane = SidedPlane::new_xyz_vector_d(min_x, 0.0, 0.0, &X_UNIT_VECTOR, -max_x)?;
        let y_plane = Plane::new_vector_d(&Y_UNIT_VECTOR, -y);
        let min_z_plane = SidedPlane::new_xyz_vector_d(0.0, 0.0, max_z, &Z_UNIT_VECTOR, -min_z)?;
        let max_z_plane = SidedPlane::new_xyz_vector_d(0.0, 0.0, min_z, &Z_UNIT_VECTOR, -max_z)?;
        let min_xy = min_x_plane
            .find_intersections(pm, &y_plane, &[&max_x_plane, &min_z_plane, &max_z_plane])
            .unwrap_or_else(|| npe());
        let max_xy = max_x_plane
            .find_intersections(pm, &y_plane, &[&min_x_plane, &min_z_plane, &max_z_plane])
            .unwrap_or_else(|| npe());
        let ymin_z = y_plane
            .find_intersections(
                pm,
                &min_z_plane,
                &[&max_z_plane, &min_x_plane, &max_x_plane],
            )
            .unwrap_or_else(|| npe());
        let ymax_z = y_plane
            .find_intersections(
                pm,
                &max_z_plane,
                &[&min_z_plane, &min_x_plane, &max_x_plane],
            )
            .unwrap_or_else(|| npe());
        let notable_y_points = BaseXYZSolid::glue_together(&[&min_xy, &max_xy, &ymin_z, &ymax_z]);
        let f_min_x_y_min_z = pm.point_outside_xyz(min_x, y, min_z);
        let f_min_x_y_max_z = pm.point_outside_xyz(min_x, y, max_z);
        let f_max_x_y_min_z = pm.point_outside_xyz(max_x, y, min_z);
        let f_max_x_y_max_z = pm.point_outside_xyz(max_x, y, max_z);
        let y_edges: Vec<GeoPoint> = if y - world_min_y >= -Vector::MINIMUM_RESOLUTION
            && y - world_max_y <= Vector::MINIMUM_RESOLUTION
            && min_x < 0.0
            && max_x > 0.0
            && min_z < 0.0
            && max_z > 0.0
            && f_min_x_y_min_z
            && f_min_x_y_max_z
            && f_max_x_y_min_z
            && f_max_x_y_max_z
        {
            match y_plane.get_sample_intersection_point(pm, &Y_VERTICAL_PLANE) {
                Some(int_point) => vec![int_point],
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let edge_points =
            BaseXYZSolid::glue_together(&[&min_xy, &max_xy, &ymin_z, &ymax_z, &y_edges]);
        Ok(Self {
            planet_model: pm.clone(),
            min_x,
            max_x,
            y,
            min_z,
            max_z,
            min_x_plane,
            max_x_plane,
            y_plane,
            min_z_plane,
            max_z_plane,
            edge_points,
            notable_y_points,
        })
    }

    /// `new XdYZSolid(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream; `IllegalArgumentException` from the
    /// constructor.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let min_x = so::read_double(input)?;
        let max_x = so::read_double(input)?;
        let y = so::read_double(input)?;
        let min_z = so::read_double(input)?;
        let max_z = so::read_double(input)?;
        Self::new(planet_model, min_x, max_x, y, min_z, max_z)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.min_x)?;
        so::write_double(output, self.max_x)?;
        so::write_double(output, self.y)?;
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
        other.min_x_plane == self.min_x_plane
            && other.max_x_plane == self.max_x_plane
            && other.y_plane == self.y_plane
            && other.min_z_plane == self.min_z_plane
            && other.max_z_plane == self.max_z_plane
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
            .wrapping_add(self.y_plane.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.min_z_plane.java_hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.max_z_plane.java_hash_code());
        result
    }
}

impl_serializable_object!(
    XdYZSolid,
    "org.apache.lucene.spatial3d.geom.XdYZSolid",
    planet_object
);
impl_planet_object!(XdYZSolid);
impl XYZSolid for XdYZSolid {}

impl Membership for XdYZSolid {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.min_x_plane.is_within_xyz(x, y, z)
            && self.max_x_plane.is_within_xyz(x, y, z)
            && self.y_plane.evaluate_is_zero_xyz(x, y, z)
            && self.min_z_plane.is_within_xyz(x, y, z)
            && self.max_z_plane.is_within_xyz(x, y, z)
    }
}

impl GeoArea for XdYZSolid {
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
            &self.y_plane,
            &self.notable_y_points,
            &[
                &self.min_x_plane,
                &self.max_x_plane,
                &self.min_z_plane,
                &self.max_z_plane,
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

impl fmt::Display for XdYZSolid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "XdYZSolid: {{planetmodel={}, minXplane={}, maxXplane={}, yplane={}, minZplane={}, maxZplane={}}}",
            self.planet_model,
            self.min_x_plane, self.max_x_plane, self.y_plane, self.min_z_plane, self.max_z_plane
        )
    }
}
