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


//! The types declared in `StandardXYZSolid.java`, of the Java package
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
use super::base_xyz_solid::{X_UNIT_VECTOR, X_VERTICAL_PLANE, Y_UNIT_VECTOR, Y_VERTICAL_PLANE, Z_UNIT_VECTOR};

/// 3D rectangle, bounded on six sides by X,Y,Z limits.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.StandardXYZSolid`. Java's
/// `null` planes and point arrays of the whole-world solid are `None`.
pub(crate) struct StandardXYZSolid {
    planet_model: Arc<PlanetModel>,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    min_z: f64,
    max_z: f64,
    is_whole_world: bool,
    planes: Option<SolidPlanes>,
    min_x_plane_intersects: bool,
    max_x_plane_intersects: bool,
    min_y_plane_intersects: bool,
    max_y_plane_intersects: bool,
    min_z_plane_intersects: bool,
    max_z_plane_intersects: bool,
    edge_points: Option<Vec<GeoPoint>>,
    notable_min_x_points: Vec<GeoPoint>,
    notable_max_x_points: Vec<GeoPoint>,
    notable_min_y_points: Vec<GeoPoint>,
    notable_max_y_points: Vec<GeoPoint>,
    notable_min_z_points: Vec<GeoPoint>,
    notable_max_z_points: Vec<GeoPoint>,
}

/// The six planes of a solid that is not the whole world.
struct SolidPlanes {
    min_x_plane: SidedPlane,
    max_x_plane: SidedPlane,
    min_y_plane: SidedPlane,
    max_y_plane: SidedPlane,
    min_z_plane: SidedPlane,
    max_z_plane: SidedPlane,
}

impl StandardXYZSolid {
    /// `new StandardXYZSolid(PlanetModel, double minX, double maxX, double
    /// minY, double maxY, double minZ, double maxZ)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when values are in the wrong order or
    /// identical.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        planet_model: &Arc<PlanetModel>,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        min_z: f64,
        max_z: f64,
    ) -> Result<Self> {
        let pm = planet_model;
        if max_x - min_x < Vector::MINIMUM_RESOLUTION {
            return Err(iae("X values in wrong order or identical"));
        }
        if max_y - min_y < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Y values in wrong order or identical"));
        }
        if max_z - min_z < Vector::MINIMUM_RESOLUTION {
            return Err(iae("Z values in wrong order or identical"));
        }
        let world_min_x = pm.get_minimum_x_value();
        let world_max_x = pm.get_maximum_x_value();
        let world_min_y = pm.get_minimum_y_value();
        let world_max_y = pm.get_maximum_y_value();
        let world_min_z = pm.get_minimum_z_value();
        let world_max_z = pm.get_maximum_z_value();
        let r = Vector::MINIMUM_RESOLUTION;
        let is_whole_world = (min_x - world_min_x < -r)
            && (max_x - world_max_x > r)
            && (min_y - world_min_y < -r)
            && (max_y - world_max_y > r)
            && (min_z - world_min_z < -r)
            && (max_z - world_max_z > r);
        let mut s = Self {
            planet_model: pm.clone(),
            min_x,
            max_x,
            min_y,
            max_y,
            min_z,
            max_z,
            is_whole_world,
            planes: None,
            min_x_plane_intersects: false,
            max_x_plane_intersects: false,
            min_y_plane_intersects: false,
            max_y_plane_intersects: false,
            min_z_plane_intersects: false,
            max_z_plane_intersects: false,
            edge_points: None,
            notable_min_x_points: Vec::new(),
            notable_max_x_points: Vec::new(),
            notable_min_y_points: Vec::new(),
            notable_max_y_points: Vec::new(),
            notable_min_z_points: Vec::new(),
            notable_max_z_points: Vec::new(),
        };
        if is_whole_world {
            return Ok(s);
        }
        let min_x_plane = SidedPlane::new_xyz_vector_d(max_x, 0.0, 0.0, &X_UNIT_VECTOR, -min_x)?;
        let max_x_plane = SidedPlane::new_xyz_vector_d(min_x, 0.0, 0.0, &X_UNIT_VECTOR, -max_x)?;
        let min_y_plane = SidedPlane::new_xyz_vector_d(0.0, max_y, 0.0, &Y_UNIT_VECTOR, -min_y)?;
        let max_y_plane = SidedPlane::new_xyz_vector_d(0.0, min_y, 0.0, &Y_UNIT_VECTOR, -max_y)?;
        let min_z_plane = SidedPlane::new_xyz_vector_d(0.0, 0.0, max_z, &Z_UNIT_VECTOR, -min_z)?;
        let max_z_plane = SidedPlane::new_xyz_vector_d(0.0, 0.0, min_z, &Z_UNIT_VECTOR, -max_z)?;
        let fi = |a: &SidedPlane, q: &SidedPlane, b: [&SidedPlane; 4]| -> Vec<GeoPoint> {
            a.find_intersections(pm, q, &[b[0], b[1], b[2], b[3]]).unwrap_or_else(|| npe())
        };
        let min_xmin_y = fi(&min_x_plane, &min_y_plane, [&max_x_plane, &max_y_plane, &min_z_plane, &max_z_plane]);
        let min_xmax_y = fi(&min_x_plane, &max_y_plane, [&max_x_plane, &min_y_plane, &min_z_plane, &max_z_plane]);
        let min_xmin_z = fi(&min_x_plane, &min_z_plane, [&max_x_plane, &max_z_plane, &min_y_plane, &max_y_plane]);
        let min_xmax_z = fi(&min_x_plane, &max_z_plane, [&max_x_plane, &min_z_plane, &min_y_plane, &max_y_plane]);
        let max_xmin_y = fi(&max_x_plane, &min_y_plane, [&min_x_plane, &max_y_plane, &min_z_plane, &max_z_plane]);
        let max_xmax_y = fi(&max_x_plane, &max_y_plane, [&min_x_plane, &min_y_plane, &min_z_plane, &max_z_plane]);
        let max_xmin_z = fi(&max_x_plane, &min_z_plane, [&min_x_plane, &max_z_plane, &min_y_plane, &max_y_plane]);
        let max_xmax_z = fi(&max_x_plane, &max_z_plane, [&min_x_plane, &min_z_plane, &min_y_plane, &max_y_plane]);
        let min_ymin_z = fi(&min_y_plane, &min_z_plane, [&max_y_plane, &max_z_plane, &min_x_plane, &max_x_plane]);
        let min_ymax_z = fi(&min_y_plane, &max_z_plane, [&max_y_plane, &min_z_plane, &min_x_plane, &max_x_plane]);
        let max_ymin_z = fi(&max_y_plane, &min_z_plane, [&min_y_plane, &max_z_plane, &min_x_plane, &max_x_plane]);
        let max_ymax_z = fi(&max_y_plane, &max_z_plane, [&min_y_plane, &min_z_plane, &min_x_plane, &max_x_plane]);
        let glue = BaseXYZSolid::glue_together;
        s.notable_min_x_points = glue(&[&min_xmin_y, &min_xmax_y, &min_xmin_z, &min_xmax_z]);
        s.notable_max_x_points = glue(&[&max_xmin_y, &max_xmax_y, &max_xmin_z, &max_xmax_z]);
        s.notable_min_y_points = glue(&[&min_xmin_y, &max_xmin_y, &min_ymin_z, &min_ymax_z]);
        s.notable_max_y_points = glue(&[&min_xmax_y, &max_xmax_y, &max_ymin_z, &max_ymax_z]);
        s.notable_min_z_points = glue(&[&min_xmin_z, &max_xmin_z, &min_ymin_z, &max_ymin_z]);
        s.notable_max_z_points = glue(&[&min_xmax_z, &max_xmax_z, &min_ymax_z, &max_ymax_z]);
        let min_xmin_ymin_z = pm.point_outside_xyz(min_x, min_y, min_z);
        let min_xmin_ymax_z = pm.point_outside_xyz(min_x, min_y, max_z);
        let min_xmax_ymin_z = pm.point_outside_xyz(min_x, max_y, min_z);
        let min_xmax_ymax_z = pm.point_outside_xyz(min_x, max_y, max_z);
        let max_xmin_ymin_z = pm.point_outside_xyz(max_x, min_y, min_z);
        let max_xmin_ymax_z = pm.point_outside_xyz(max_x, min_y, max_z);
        let max_xmax_ymin_z = pm.point_outside_xyz(max_x, max_y, min_z);
        let max_xmax_ymax_z = pm.point_outside_xyz(max_x, max_y, max_z);
        let sample = |plane: &SidedPlane, q: &Plane| -> Vec<GeoPoint> {
            match plane.get_sample_intersection_point(pm, q) {
                Some(p) => vec![p],
                None => Vec::new(),
            }
        };
        let min_x_edges = if min_x - world_min_x >= -r
            && min_x - world_max_x <= r
            && min_y < 0.0
            && max_y > 0.0
            && min_z < 0.0
            && max_z > 0.0
            && min_xmin_ymin_z
            && min_xmin_ymax_z
            && min_xmax_ymin_z
            && min_xmax_ymax_z
        {
            sample(&min_x_plane, &X_VERTICAL_PLANE)
        } else {
            Vec::new()
        };
        let max_x_edges = if max_x - world_min_x >= -r
            && max_x - world_max_x <= r
            && min_y < 0.0
            && max_y > 0.0
            && min_z < 0.0
            && max_z > 0.0
            && max_xmin_ymin_z
            && max_xmin_ymax_z
            && max_xmax_ymin_z
            && max_xmax_ymax_z
        {
            sample(&max_x_plane, &X_VERTICAL_PLANE)
        } else {
            Vec::new()
        };
        let min_y_edges = if min_y - world_min_y >= -r
            && min_y - world_max_y <= r
            && min_x < 0.0
            && max_x > 0.0
            && min_z < 0.0
            && max_z > 0.0
            && min_xmin_ymin_z
            && min_xmin_ymax_z
            && max_xmin_ymin_z
            && max_xmin_ymax_z
        {
            sample(&min_y_plane, &Y_VERTICAL_PLANE)
        } else {
            Vec::new()
        };
        let max_y_edges = if max_y - world_min_y >= -r
            && max_y - world_max_y <= r
            && min_x < 0.0
            && max_x > 0.0
            && min_z < 0.0
            && max_z > 0.0
            && min_xmax_ymin_z
            && min_xmax_ymax_z
            && max_xmax_ymin_z
            && max_xmax_ymax_z
        {
            sample(&max_y_plane, &Y_VERTICAL_PLANE)
        } else {
            Vec::new()
        };
        let min_z_edges = if min_z - world_min_z >= -r
            && min_z - world_max_z <= r
            && min_x < 0.0
            && max_x > 0.0
            && min_y < 0.0
            && max_y > 0.0
            && min_xmin_ymin_z
            && min_xmax_ymin_z
            && max_xmin_ymin_z
            && max_xmax_ymin_z
        {
            sample(&min_z_plane, &X_VERTICAL_PLANE)
        } else {
            Vec::new()
        };
        let max_z_edges = if max_z - world_min_z >= -r
            && max_z - world_max_z <= r
            && min_x < 0.0
            && max_x > 0.0
            && min_y < 0.0
            && max_y > 0.0
            && min_xmin_ymax_z
            && min_xmax_ymax_z
            && max_xmin_ymax_z
            && max_xmax_ymax_z
        {
            sample(&max_z_plane, &X_VERTICAL_PLANE)
        } else {
            Vec::new()
        };
        s.min_x_plane_intersects = s.notable_min_x_points.len() + min_x_edges.len() > 0;
        s.max_x_plane_intersects = s.notable_max_x_points.len() + max_x_edges.len() > 0;
        s.min_y_plane_intersects = s.notable_min_y_points.len() + min_y_edges.len() > 0;
        s.max_y_plane_intersects = s.notable_max_y_points.len() + max_y_edges.len() > 0;
        s.min_z_plane_intersects = s.notable_min_z_points.len() + min_z_edges.len() > 0;
        s.max_z_plane_intersects = s.notable_max_z_points.len() + max_z_edges.len() > 0;
        s.edge_points = Some(glue(&[
            &min_xmin_y, &min_xmax_y, &min_xmin_z, &min_xmax_z, &max_xmin_y, &max_xmax_y, &max_xmin_z, &max_xmax_z,
            &min_ymin_z, &min_ymax_z, &max_ymin_z, &max_ymax_z, &min_x_edges, &max_x_edges, &min_y_edges,
            &max_y_edges, &min_z_edges, &max_z_edges,
        ]));
        s.planes = Some(SolidPlanes {
            min_x_plane,
            max_x_plane,
            min_y_plane,
            max_y_plane,
            min_z_plane,
            max_z_plane,
        });
        Ok(s)
    }

    /// `new StandardXYZSolid(PlanetModel, InputStream)`.
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
        let e = so::read_double(input)?;
        let f = so::read_double(input)?;
        Self::new(planet_model, a, b, c, d, e, f)
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.min_x)?;
        so::write_double(output, self.max_x)?;
        so::write_double(output, self.min_y)?;
        so::write_double(output, self.max_y)?;
        so::write_double(output, self.min_z)?;
        so::write_double(output, self.max_z)
    }

    /// `getEdgePoints()`: `None` for the whole world.
    pub(crate) fn edge_points(&self) -> Option<&[GeoPoint]> {
        self.edge_points.as_deref()
    }

    fn equals_impl(&self, other: &Self) -> bool {
        if !BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            || other.is_whole_world != self.is_whole_world
        {
            return false;
        }
        if !self.is_whole_world {
            let (Some(a), Some(b)) = (&self.planes, &other.planes) else {
                npe()
            };
            return b.min_x_plane == a.min_x_plane
                && b.max_x_plane == a.max_x_plane
                && b.min_y_plane == a.min_y_plane
                && b.max_y_plane == a.max_y_plane
                && b.min_z_plane == a.min_z_plane
                && b.max_z_plane == a.max_z_plane;
        }
        true
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result.wrapping_mul(31).wrapping_add(if self.is_whole_world { 1 } else { 0 });
        if let Some(p) = &self.planes {
            result = result.wrapping_mul(31).wrapping_add(p.min_x_plane.java_hash_code());
            result = result.wrapping_mul(31).wrapping_add(p.max_x_plane.java_hash_code());
            result = result.wrapping_mul(31).wrapping_add(p.min_y_plane.java_hash_code());
            result = result.wrapping_mul(31).wrapping_add(p.max_y_plane.java_hash_code());
            result = result.wrapping_mul(31).wrapping_add(p.min_z_plane.java_hash_code());
            result = result.wrapping_mul(31).wrapping_add(p.max_z_plane.java_hash_code());
        }
        result
    }
}

impl_serializable_object!(StandardXYZSolid, "org.apache.lucene.spatial3d.geom.StandardXYZSolid", planet_object);
impl_planet_object!(StandardXYZSolid);
impl XYZSolid for StandardXYZSolid {}

impl Membership for StandardXYZSolid {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        let Some(p) = &self.planes else {
            return true;
        };
        p.min_x_plane.is_within_xyz(x, y, z)
            && p.max_x_plane.is_within_xyz(x, y, z)
            && p.min_y_plane.is_within_xyz(x, y, z)
            && p.max_y_plane.is_within_xyz(x, y, z)
            && p.min_z_plane.is_within_xyz(x, y, z)
            && p.max_z_plane.is_within_xyz(x, y, z)
    }
}

impl GeoArea for StandardXYZSolid {
    fn get_relationship(&self, path: &dyn GeoShape) -> Result<i32> {
        let Some(p) = &self.planes else {
            if !path.get_edge_points().is_empty() {
                return Ok(geo_area::WITHIN);
            }
            return Ok(geo_area::OVERLAPS);
        };
        let inside_rectangle = BaseXYZSolid::is_shape_inside_area(self, path);
        if inside_rectangle == BaseXYZSolid::SOME_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        let inside_shape = BaseXYZSolid::is_area_inside_shape(self.edge_points().unwrap_or_else(|| npe()), path);
        if inside_shape == BaseXYZSolid::SOME_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        if inside_rectangle == BaseXYZSolid::ALL_INSIDE && inside_shape == BaseXYZSolid::ALL_INSIDE {
            return Ok(geo_area::OVERLAPS);
        }
        if (self.min_x_plane_intersects
            && path.intersects(&p.min_x_plane, &self.notable_min_x_points, &[&p.max_x_plane, &p.min_y_plane, &p.max_y_plane, &p.min_z_plane, &p.max_z_plane]))
            || (self.max_x_plane_intersects
                && path.intersects(&p.max_x_plane, &self.notable_max_x_points, &[&p.min_x_plane, &p.min_y_plane, &p.max_y_plane, &p.min_z_plane, &p.max_z_plane]))
            || (self.min_y_plane_intersects
                && path.intersects(&p.min_y_plane, &self.notable_min_y_points, &[&p.max_y_plane, &p.min_x_plane, &p.max_x_plane, &p.min_z_plane, &p.max_z_plane]))
            || (self.max_y_plane_intersects
                && path.intersects(&p.max_y_plane, &self.notable_max_y_points, &[&p.min_y_plane, &p.min_x_plane, &p.max_x_plane, &p.min_z_plane, &p.max_z_plane]))
            || (self.min_z_plane_intersects
                && path.intersects(&p.min_z_plane, &self.notable_min_z_points, &[&p.max_z_plane, &p.min_x_plane, &p.max_x_plane, &p.min_y_plane, &p.max_y_plane]))
            || (self.max_z_plane_intersects
                && path.intersects(&p.max_z_plane, &self.notable_max_z_points, &[&p.min_z_plane, &p.min_x_plane, &p.max_x_plane, &p.min_y_plane, &p.max_y_plane]))
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

impl fmt::Display for StandardXYZSolid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pl = |sel: fn(&SolidPlanes) -> &SidedPlane| match &self.planes {
            Some(p) => sel(p).to_string(),
            None => "null".to_string(),
        };
        write!(
            f,
            "StandardXYZSolid: {{planetmodel={}, isWholeWorld={}, minXplane={}, maxXplane={}, minYplane={}, maxYplane={}, minZplane={}, maxZplane={}}}",
            self.planet_model,
            self.is_whole_world,
            pl(|p| &p.min_x_plane),
            pl(|p| &p.max_x_plane),
            pl(|p| &p.min_y_plane),
            pl(|p| &p.max_y_plane),
            pl(|p| &p.min_z_plane),
            pl(|p| &p.max_z_plane)
        )
    }
}
