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

//! The types declared in `NormalDistance.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::error::Result;

use super::distance_style::DistanceStyle;
use super::geo_point::GeoPoint;
use super::ise;
use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;

/// Normal distance computation style.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.NormalDistance`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NormalDistance;

impl NormalDistance {
    /// A convenient instance.
    pub const INSTANCE: NormalDistance = NormalDistance;

    /// `new NormalDistance()`.
    pub fn new() -> Self {
        NormalDistance
    }
}

const NO_REVERSE: &str = "Reverse mapping not implemented for this distance metric";

impl DistanceStyle for NormalDistance {
    fn compute_distance(&self, point1: &GeoPoint, point2: &GeoPoint) -> f64 {
        point1.normal_distance(point2)
    }

    fn compute_distance_xyz(&self, point1: &GeoPoint, x2: f64, y2: f64, z2: f64) -> f64 {
        point1.normal_distance_xyz(x2, y2, z2)
    }

    #[allow(unused_variables)]
    fn compute_plane_distance(
        &self,
        planet_model: &PlanetModel,
        plane: &Plane,
        point: &GeoPoint,
        bounds: &[&dyn Membership],
    ) -> f64 {
        plane.normal_distance(point, bounds)
    }

    #[allow(unused_variables)]
    fn compute_plane_distance_xyz(
        &self,
        planet_model: &PlanetModel,
        plane: &Plane,
        x: f64,
        y: f64,
        z: f64,
        bounds: &[&dyn Membership],
    ) -> f64 {
        plane.normal_distance_xyz(x, y, z, bounds)
    }

    fn find_distance_points(
        &self,
        _planet_model: &PlanetModel,
        _distance_value: f64,
        _start_point: &GeoPoint,
        _plane: &Plane,
        _bounds: &[&dyn Membership],
    ) -> Result<Vec<GeoPoint>> {
        Err(ise(NO_REVERSE))
    }

    fn find_minimum_arc_distance(
        &self,
        _planet_model: &PlanetModel,
        _distance_value: f64,
    ) -> Result<f64> {
        Err(ise(NO_REVERSE))
    }

    fn find_maximum_arc_distance(
        &self,
        _planet_model: &PlanetModel,
        _distance_value: f64,
    ) -> Result<f64> {
        Err(ise(NO_REVERSE))
    }
}
