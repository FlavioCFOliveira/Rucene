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


//! The types declared in `GeoBaseCompositeMembershipShape.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::error::Result;

use super::distance_style::DistanceStyle;
use super::geo_base_composite_shape::GeoBaseCompositeShape;
use super::geo_membership_shape::GeoMembershipShape;

/// Base class to create a composite of GeoMembershipShapes.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.GeoBaseCompositeMembershipShape`.
///
/// **Divergence.** Rust has no class inheritance. The abstract class is this
/// namespace of its shared behaviour over a
/// [`GeoBaseCompositeShape`] of membership shapes.
pub(crate) struct GeoBaseCompositeMembershipShape;

impl GeoBaseCompositeMembershipShape {
    /// `computeOutsideDistance(DistanceStyle, double, double, double)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the members' distance computations.
    pub(crate) fn compute_outside_distance_xyz<T: ?Sized + GeoMembershipShape>(
        base: &GeoBaseCompositeShape<T>,
        distance_style: &dyn DistanceStyle,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<f64> {
        if base.is_within_xyz(x, y, z) {
            return Ok(0.0);
        }
        let mut distance = f64::INFINITY;
        for shape in &base.shapes {
            let normal_distance = shape.compute_outside_distance_xyz(distance_style, x, y, z)?;
            if normal_distance < distance {
                distance = normal_distance;
            }
        }
        Ok(distance)
    }
}
