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


//! The types declared in `GeoWorld.java`, of the Java package
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
use super::geo_area_shape::GeoAreaShape;
use super::geo_b_box::GeoBBox;
use super::geo_base_bounds::GeoBaseBounds;
use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::geo_sizeable::GeoSizeable;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::geo_area::{self, GeoArea};

/// Bounding box including the entire world.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoWorld`.
pub(crate) struct GeoWorld {
    planet_model: Arc<PlanetModel>,
    /// Origin point.
    origin_point: GeoPoint,
}

impl GeoWorld {
    /// `new GeoWorld(PlanetModel)`.
    pub fn new(planet_model: &Arc<PlanetModel>) -> Self {
        Self {
            planet_model: planet_model.clone(),
            origin_point: GeoPoint::new_magnitude(planet_model.xy_scaling, 1.0, 0.0, 0.0),
        }
    }

    /// `new GeoWorld(PlanetModel, InputStream)`.
    ///
    /// # Errors
    ///
    /// Never: nothing is read.
    pub fn from_stream(planet_model: &Arc<PlanetModel>, _input: &mut dyn Read) -> Result<Self> {
        Ok(Self::new(planet_model))
    }

    fn write_impl(&self, _output: &mut dyn Write) -> Result<()> {
        Ok(())
    }

    fn outside_distance(&self, _ds: &dyn DistanceStyle, _x: f64, _y: f64, _z: f64) -> Result<f64> {
        Ok(0.0)
    }

    fn equals_impl(&self, other: &Self) -> bool {
        BasePlanetObject::equals(&self.planet_model, &other.planet_model)
    }

    fn hash_code_impl(&self) -> i32 {
        BasePlanetObject::hash_code(&self.planet_model)
    }
}

impl_geo_base_area_shape_common!(GeoWorld, "org.apache.lucene.spatial3d.geom.GeoWorld", geo_b_box);

impl GeoArea for GeoWorld {
    fn get_relationship(&self, path: &dyn GeoShape) -> Result<i32> {
        if !path.get_edge_points().is_empty() {
            return Ok(geo_area::WITHIN);
        }
        Ok(geo_area::OVERLAPS)
    }
}

impl Membership for GeoWorld {
    fn is_within_xyz(&self, _x: f64, _y: f64, _z: f64) -> bool {
        true
    }
}

impl Bounded for GeoWorld {
    fn get_bounds(&self, bounds: &mut dyn Bounds) {
        GeoBaseBounds::get_bounds(self, &self.planet_model, bounds);
    }
}

impl GeoShape for GeoWorld {
    fn get_edge_points(&self) -> Vec<GeoPoint> {
        Vec::new()
    }

    fn intersects(&self, _p: &Plane, _notable_points: &[GeoPoint], _bounds: &[&dyn Membership]) -> bool {
        false
    }
}

impl GeoAreaShape for GeoWorld {
    fn intersects_shape(&self, _geo_shape: &dyn GeoShape) -> bool {
        false
    }
}

impl GeoSizeable for GeoWorld {
    fn get_radius(&self) -> f64 {
        PI
    }

    fn get_center(&self) -> GeoPoint {
        self.origin_point.clone()
    }
}

impl GeoBBox for GeoWorld {
    fn expand(self: Arc<Self>, _angle: f64) -> Result<Arc<dyn GeoBBox>> {
        Ok(self)
    }
}

impl fmt::Display for GeoWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GeoWorld: {{planetmodel={}}}", self.planet_model)
    }
}
