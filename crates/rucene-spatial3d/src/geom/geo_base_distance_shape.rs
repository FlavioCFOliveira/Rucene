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

//! The types declared in `GeoBaseDistanceShape.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

/// Distance shapes have capabilities of both geohashing and distance
/// computation (which also includes point membership determination).
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoBaseDistanceShape`.
///
/// **Divergence.** Rust has no class inheritance. The abstract class is this
/// namespace of its shared behaviour, which each concrete class (a struct)
/// reaches through the `impl_*` macros of [`super`] and through these
/// associated functions where Java calls `super`.
/// `computeDistance`, `computeDeltaDistance` and `getDistanceBounds` are
/// implemented by `impl_geo_base_distance_shape!` over each class's inherent
/// `distance`, `delta_distance` (Java's default: `distance * 2.0`) and
/// `distance_bounds`.
///
/// Equivalent to `GeoBaseDistanceShape.isWithin(Vector)`: the provided
/// [`Membership::is_within`](super::membership::Membership::is_within), which
/// is `isWithin(point.x, point.y, point.z)` as this override is.
/// Equivalent to `GeoBaseDistanceShape.distanceBounds(Bounds, DistanceStyle, double)`:
/// the abstract method is each class's inherent `distance_bounds`, which
/// `impl_geo_base_distance_shape!` calls from `getDistanceBounds`.
pub struct GeoBaseDistanceShape;

impl GeoBaseDistanceShape {
    /// Called by a `computeDeltaDistance` method if X/Y/Z is within this
    /// shape: Java's default body, `distance(...) * 2.0`, given the result of
    /// the class's `distance(distanceStyle, x, y, z)`. The classes that do not
    /// override `deltaDistance` (`GeoStandardCircle`, `GeoExactCircle`) call
    /// it from their inherent `delta_distance`.
    ///
    /// Equivalent to `GeoBaseDistanceShape.deltaDistance(DistanceStyle, double, double, double)`.
    pub fn delta_distance(distance: rucene::error::Result<f64>) -> rucene::error::Result<f64> {
        Ok(distance? * 2.0)
    }
}
