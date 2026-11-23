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

//! The types declared in `BaseXYZSolid.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use super::geo_point::GeoPoint;
use super::geo_shape::GeoShape;
use super::membership::Membership;
use super::plane::Plane;
use super::vector::Vector;

/// Base class of 3D rectangles bounded by X, Y, Z limits.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.BaseXYZSolid`.
///
/// **Divergence.** Rust has no class inheritance. The abstract class is this
/// namespace of its shared behaviour; each solid (a struct) implements the
/// interfaces and calls these associated functions where Java calls the
/// inherited methods. `getEdgePoints()` is each solid's inherent
/// `edge_points()`.
pub(crate) struct BaseXYZSolid;

/// Unit vector in x.
pub(crate) static X_UNIT_VECTOR: Vector = Vector::new(1.0, 0.0, 0.0);
/// Unit vector in y.
pub(crate) static Y_UNIT_VECTOR: Vector = Vector::new(0.0, 1.0, 0.0);
/// Unit vector in z.
pub(crate) static Z_UNIT_VECTOR: Vector = Vector::new(0.0, 0.0, 1.0);
/// Vertical plane normal to x unit vector passing through origin.
pub(crate) static X_VERTICAL_PLANE: Plane = Plane::new(0.0, 1.0, 0.0, 0.0);
/// Vertical plane normal to y unit vector passing through origin.
pub(crate) static Y_VERTICAL_PLANE: Plane = Plane::new(1.0, 0.0, 0.0, 0.0);

impl BaseXYZSolid {
    /// All edgepoints inside shape.
    pub(crate) const ALL_INSIDE: i32 = 0;
    /// Some edgepoints inside shape.
    pub(crate) const SOME_INSIDE: i32 = 1;
    /// No edgepoints inside shape.
    pub(crate) const NONE_INSIDE: i32 = 2;
    /// No edgepoints at all (means a shape that is the whole world).
    pub(crate) const NO_EDGEPOINTS: i32 = 3;

    /// `glueTogether(GeoPoint[]...)`.
    pub(crate) fn glue_together(point_arrays: &[&[GeoPoint]]) -> Vec<GeoPoint> {
        let mut count = 0;
        for point_array in point_arrays {
            count += point_array.len();
        }
        let mut rval = Vec::with_capacity(count);
        for point_array in point_arrays {
            for point in point_array.iter() {
                rval.push(point.clone());
            }
        }
        rval
    }

    /// `isShapeInsideArea(GeoShape)`.
    pub(crate) fn is_shape_inside_area(this: &dyn Membership, path: &dyn GeoShape) -> i32 {
        let path_points = path.get_edge_points();
        if path_points.is_empty() {
            return Self::NO_EDGEPOINTS;
        }
        let mut found_outside = false;
        let mut found_inside = false;
        for p in &path_points {
            if this.is_within(p) {
                found_inside = true;
            } else {
                found_outside = true;
            }
            if found_inside && found_outside {
                return Self::SOME_INSIDE;
            }
        }
        if !found_inside && !found_outside {
            return Self::NONE_INSIDE;
        }
        if found_inside && !found_outside {
            return Self::ALL_INSIDE;
        }
        if found_outside && !found_inside {
            return Self::NONE_INSIDE;
        }
        Self::SOME_INSIDE
    }

    /// `isAreaInsideShape(GeoShape)` over the solid's edge points.
    pub(crate) fn is_area_inside_shape(edge_points: &[GeoPoint], path: &dyn GeoShape) -> i32 {
        if edge_points.is_empty() {
            return Self::NO_EDGEPOINTS;
        }
        let mut found_outside = false;
        let mut found_inside = false;
        for p in edge_points {
            if path.is_within(p) {
                found_inside = true;
            } else {
                found_outside = true;
            }
            if found_inside && found_outside {
                return Self::SOME_INSIDE;
            }
        }
        if !found_inside && !found_outside {
            return Self::NONE_INSIDE;
        }
        if found_inside && !found_outside {
            return Self::ALL_INSIDE;
        }
        if found_outside && !found_inside {
            return Self::NONE_INSIDE;
        }
        Self::SOME_INSIDE
    }
}
