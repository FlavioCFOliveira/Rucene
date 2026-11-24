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

//! The types declared in `dXdYdZSolid.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::io::{Read, Write};
use std::sync::Arc;

use rucene::error::Result;

use super::base_planet_object::BasePlanetObject;
use super::base_xyz_solid::BaseXYZSolid;
use super::d2s;
use super::geo_area::{self, GeoArea};
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::membership::Membership;
use super::planet_model::PlanetModel;
use super::serializable_object as so;
use super::xyz_solid::XYZSolid;

/// 3D rectangle, bounded on six sides by X,Y,Z limits, degenerate in all
/// dimensions.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.dXdYdZSolid`.
#[allow(non_camel_case_types)]
pub(crate) struct dXdYdZSolid {
    planet_model: Arc<PlanetModel>,
    x: f64,
    y: f64,
    z: f64,
    is_on_surface: bool,
    the_point: Option<GeoPoint>,
    edge_points: Vec<GeoPoint>,
}

impl dXdYdZSolid {
    /// `new dXdYdZSolid(PlanetModel, double X, double Y, double Z)`.
    pub fn new(planet_model: &Arc<PlanetModel>, x: f64, y: f64, z: f64) -> Self {
        let is_on_surface = planet_model.point_on_surface_xyz(x, y, z);
        let (the_point, edge_points) = if is_on_surface {
            let p = GeoPoint::new(x, y, z);
            (Some(p.clone()), vec![p])
        } else {
            (None, Vec::new())
        };
        Self {
            planet_model: planet_model.clone(),
            x,
            y,
            z,
            is_on_surface,
            the_point,
            edge_points,
        }
    }

    /// `new dXdYdZSolid(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the stream.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, input: &mut dyn Read) -> Result<Self> {
        let a = so::read_double(input)?;
        let b = so::read_double(input)?;
        let c = so::read_double(input)?;
        Ok(Self::new(planet_model, a, b, c))
    }

    fn write_impl(&self, output: &mut dyn Write) -> Result<()> {
        so::write_double(output, self.x)?;
        so::write_double(output, self.y)?;
        so::write_double(output, self.z)
    }

    /// `getEdgePoints()`.
    pub(crate) fn edge_points(&self) -> &[GeoPoint] {
        &self.edge_points
    }

    fn equals_impl(&self, other: &Self) -> bool {
        if !BasePlanetObject::equals(&self.planet_model, &other.planet_model)
            || other.is_on_surface != self.is_on_surface
        {
            return false;
        }
        if self.is_on_surface {
            return other.the_point == self.the_point;
        }
        true
    }

    fn hash_code_impl(&self) -> i32 {
        let mut result = BasePlanetObject::hash_code(&self.planet_model);
        result = result
            .wrapping_mul(31)
            .wrapping_add(if self.is_on_surface { 1 } else { 0 });
        if let Some(p) = &self.the_point {
            result = result.wrapping_mul(31).wrapping_add(p.java_hash_code());
        }
        result
    }
}

impl_serializable_object!(
    dXdYdZSolid,
    "org.apache.lucene.spatial3d.geom.dXdYdZSolid",
    planet_object
);
impl_planet_object!(dXdYdZSolid);
impl XYZSolid for dXdYdZSolid {}

impl Membership for dXdYdZSolid {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        match &self.the_point {
            None => false,
            Some(p) => p.is_identical_xyz(x, y, z),
        }
    }
}

impl GeoArea for dXdYdZSolid {
    fn get_relationship(&self, path: &dyn GeoShape) -> Result<i32> {
        if !self.is_on_surface {
            return Ok(geo_area::DISJOINT);
        }
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

impl fmt::Display for dXdYdZSolid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = match &self.the_point {
            Some(p) => p.to_string(),
            None => "null".to_string(),
        };
        let _ = d2s;
        write!(
            f,
            "dXdYdZSolid: {{planetmodel={}, isOnSurface={}, thePoint={}}}",
            self.planet_model, self.is_on_surface, p
        )
    }
}
