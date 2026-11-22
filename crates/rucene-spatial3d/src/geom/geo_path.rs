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


//! The types declared in `GeoPath.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::error::Result;

use super::distance_style::DistanceStyle;
use super::geo_distance_shape::GeoDistanceShape;
use super::geo_point::GeoPoint;

/// Interface describing a path.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoPath`.
pub trait GeoPath: GeoDistanceShape {
    /// `computeNearestDistance(DistanceStyle, GeoPoint)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance computation.
    fn compute_nearest_distance(
        &self,
        distance_style: &dyn DistanceStyle,
        point: &GeoPoint,
    ) -> Result<f64> {
        self.compute_nearest_distance_xyz(distance_style, point.x, point.y, point.z)
    }

    /// `computeNearestDistance(DistanceStyle, double, double, double)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance computation.
    fn compute_nearest_distance_xyz(
        &self,
        distance_style: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64>;

    /// `computePathCenterDistance(DistanceStyle, GeoPoint)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance computation.
    fn compute_path_center_distance(
        &self,
        distance_style: &dyn DistanceStyle,
        point: &GeoPoint,
    ) -> Result<f64> {
        self.compute_path_center_distance_xyz(distance_style, point.x, point.y, point.z)
    }

    /// `computePathCenterDistance(DistanceStyle, double, double, double)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the distance computation.
    fn compute_path_center_distance_xyz(
        &self,
        distance_style: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64>;
}
