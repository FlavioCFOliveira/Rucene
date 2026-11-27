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

//! The types declared in `DistanceStyle.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::any::Any;

use rucene::error::Result;

use super::arc_distance::ArcDistance;
use super::geo_point::GeoPoint;
use super::linear_distance::LinearDistance;
use super::linear_squared_distance::LinearSquaredDistance;
use super::membership::Membership;
use super::normal_distance::NormalDistance;
use super::normal_squared_distance::NormalSquaredDistance;
use super::plane::Plane;
use super::planet_model::PlanetModel;

/// Distance computation styles, supporting various ways of computing
/// distance to shapes.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.DistanceStyle`.
pub trait DistanceStyle: Any + Send + Sync {
    /// `computeDistance(GeoPoint, GeoPoint)`.
    fn compute_distance(&self, point1: &GeoPoint, point2: &GeoPoint) -> f64 {
        self.compute_distance_xyz(point1, point2.x, point2.y, point2.z)
    }

    /// `computeDistance(GeoPoint, double, double, double)`.
    fn compute_distance_xyz(&self, point1: &GeoPoint, x2: f64, y2: f64, z2: f64) -> f64;

    /// `computeDistance(PlanetModel, Plane, GeoPoint, Membership...)`.
    fn compute_plane_distance(
        &self,
        planet_model: &PlanetModel,
        plane: &Plane,
        point: &GeoPoint,
        bounds: &[&dyn Membership],
    ) -> f64 {
        self.compute_plane_distance_xyz(planet_model, plane, point.x, point.y, point.z, bounds)
    }

    /// `computeDistance(PlanetModel, Plane, double, double, double,
    /// Membership...)`.
    fn compute_plane_distance_xyz(
        &self,
        planet_model: &PlanetModel,
        plane: &Plane,
        x: f64,
        y: f64,
        z: f64,
        bounds: &[&dyn Membership],
    ) -> f64;

    /// `toAggregationForm(double)`.
    fn to_aggregation_form(&self, distance: f64) -> f64 {
        distance
    }

    /// `aggregateDistances(double...)`.
    fn aggregate_distances(&self, distances: &[f64]) -> f64 {
        let mut rval = 0.0;
        for distance in distances {
            rval += distance;
        }
        rval
    }

    /// `fromAggregationForm(double)`.
    // Java's method name `fromAggregationForm`.
    #[allow(clippy::wrong_self_convention)]
    fn from_aggregation_form(&self, aggregate_distance: f64) -> f64 {
        aggregate_distance
    }

    /// `findDistancePoints(PlanetModel, double, GeoPoint, Plane,
    /// Membership...)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when the style has no reverse mapping.
    fn find_distance_points(
        &self,
        planet_model: &PlanetModel,
        distance_value: f64,
        start_point: &GeoPoint,
        plane: &Plane,
        bounds: &[&dyn Membership],
    ) -> Result<Vec<GeoPoint>>;

    /// `findMinimumArcDistance(PlanetModel, double)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when the style has no reverse mapping.
    fn find_minimum_arc_distance(
        &self,
        planet_model: &PlanetModel,
        distance_value: f64,
    ) -> Result<f64>;

    /// `findMaximumArcDistance(PlanetModel, double)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when the style has no reverse mapping.
    fn find_maximum_arc_distance(
        &self,
        planet_model: &PlanetModel,
        distance_value: f64,
    ) -> Result<f64>;
}

/// `DistanceStyle.ARC`: arc distance calculator.
pub static ARC: ArcDistance = ArcDistance::INSTANCE;
/// `DistanceStyle.LINEAR`: linear distance calculator.
pub static LINEAR: LinearDistance = LinearDistance::INSTANCE;
/// `DistanceStyle.LINEAR_SQUARED`: linear distance squared calculator.
pub static LINEAR_SQUARED: LinearSquaredDistance = LinearSquaredDistance::INSTANCE;
/// `DistanceStyle.NORMAL`: normal distance calculator.
pub static NORMAL: NormalDistance = NormalDistance::INSTANCE;
/// `DistanceStyle.NORMAL_SQUARED`: normal distance squared calculator.
pub static NORMAL_SQUARED: NormalSquaredDistance = NormalSquaredDistance::INSTANCE;

/// The key of a per-style cache: Java keys a `ConcurrentHashMap` by the
/// style object; the styles are stateless singletons, so their type
/// identifies them.
pub(crate) fn style_key(distance_style: &dyn DistanceStyle) -> std::any::TypeId {
    let any: &dyn Any = distance_style;
    any.type_id()
}
