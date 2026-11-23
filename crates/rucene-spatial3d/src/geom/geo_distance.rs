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

//! The types declared in `GeoDistance.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::error::Result;

use super::distance_style::DistanceStyle;
use super::geo_point::GeoPoint;
use super::membership::Membership;

/// An implementer of this interface is capable of computing the described
/// "distance" values.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoDistance`.
pub trait GeoDistance: Membership {
    /// `computeDistance(DistanceStyle, GeoPoint)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance computation.
    fn compute_distance(
        &self,
        distance_style: &dyn DistanceStyle,
        point: &GeoPoint,
    ) -> Result<f64> {
        self.compute_distance_xyz(distance_style, point.x, point.y, point.z)
    }

    /// `computeDistance(DistanceStyle, double, double, double)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance computation.
    fn compute_distance_xyz(
        &self,
        distance_style: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64>;

    /// `computeDeltaDistance(DistanceStyle, GeoPoint)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance computation.
    fn compute_delta_distance(
        &self,
        distance_style: &dyn DistanceStyle,
        point: &GeoPoint,
    ) -> Result<f64> {
        self.compute_delta_distance_xyz(distance_style, point.x, point.y, point.z)
    }

    /// `computeDeltaDistance(DistanceStyle, double, double, double)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance computation.
    fn compute_delta_distance_xyz(
        &self,
        distance_style: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64> {
        Ok(self.compute_distance_xyz(distance_style, x, y, z)? * 2.0)
    }
}
