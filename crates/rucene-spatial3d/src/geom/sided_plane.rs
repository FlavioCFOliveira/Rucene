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


//! The types declared in `SidedPlane.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use rucene::error::Result;

use super::membership::Membership;
use super::plane::Plane;
use super::planet_model::PlanetModel;
use super::vector::Vector;
use super::{d2s, dhash, double_compare_eq, iae, java_signum};

/// Combination of a plane, and a sign value indicating what evaluation
/// values are on the correct side of the plane.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.SidedPlane`. Java's
/// `extends Plane` is a [`Deref`] to the plane.
#[derive(Clone, Debug)]
pub struct SidedPlane {
    plane: Plane,
    /// The sign value for evaluation of a point on the correct side of the
    /// plane.
    pub sig_num: f64,
}

impl Deref for SidedPlane {
    type Target = Plane;
    #[inline]
    fn deref(&self) -> &Plane {
        &self.plane
    }
}

const SIDEDNESS: &str = "Cannot determine sidedness because check point is on plane.";

impl SidedPlane {
    fn checked(plane: Plane, sig_num: f64) -> Result<Self> {
        if sig_num == 0.0 {
            return Err(iae(SIDEDNESS));
        }
        Ok(Self { plane, sig_num })
    }

    /// `new SidedPlane(SidedPlane)`: the same plane with the opposite side.
    pub fn new_inverse(sided_plane: &SidedPlane) -> Self {
        Self {
            plane: Plane::new_vector_d(sided_plane.as_vector(), sided_plane.d),
            sig_num: -sided_plane.sig_num,
        }
    }

    /// `new SidedPlane(double pX, double pY, double pZ, Vector A, Vector B)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel or the check
    /// point is on the plane.
    pub fn new_xyz_cross(px: f64, py: f64, pz: f64, a: &Vector, b: &Vector) -> Result<Self> {
        let plane = Plane::new_cross(a, b)?;
        let sig_num = java_signum(plane.evaluate_xyz(px, py, pz));
        Self::checked(plane, sig_num)
    }

    /// `new SidedPlane(Vector p, Vector A, Vector B)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel or the check
    /// point is on the plane.
    pub fn new_cross(p: &Vector, a: &Vector, b: &Vector) -> Result<Self> {
        let plane = Plane::new_cross(a, b)?;
        let sig_num = java_signum(plane.evaluate(p));
        Self::checked(plane, sig_num)
    }

    /// `new SidedPlane(Vector A, Vector B)`: the positive side.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel.
    pub fn new_cross_positive(a: &Vector, b: &Vector) -> Result<Self> {
        Ok(Self {
            plane: Plane::new_cross(a, b)?,
            sig_num: 1.0,
        })
    }

    /// `new SidedPlane(Vector p, Vector A, double BX, double BY, double BZ)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel or the check
    /// point is on the plane.
    pub fn new_cross_xyz(p: &Vector, a: &Vector, bx: f64, by: f64, bz: f64) -> Result<Self> {
        let plane = Plane::new_cross_xyz(a, bx, by, bz)?;
        let sig_num = java_signum(plane.evaluate(p));
        Self::checked(plane, sig_num)
    }

    /// `new SidedPlane(Vector p, boolean onSide, Vector A, Vector B)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel or the check
    /// point is on the plane.
    pub fn new_cross_on_side(p: &Vector, on_side: bool, a: &Vector, b: &Vector) -> Result<Self> {
        let plane = Plane::new_cross(a, b)?;
        let sig_num = if on_side {
            java_signum(plane.evaluate(p))
        } else {
            -java_signum(plane.evaluate(p))
        };
        Self::checked(plane, sig_num)
    }

    /// `new SidedPlane(Vector p, PlanetModel planetModel, double sinLat)`: a
    /// horizontal plane.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the check point is on the plane.
    pub fn new_horizontal(p: &Vector, planet_model: &PlanetModel, sin_lat: f64) -> Result<Self> {
        let plane = Plane::new_horizontal(planet_model, sin_lat);
        let sig_num = java_signum(plane.evaluate(p));
        Self::checked(plane, sig_num)
    }

    /// `new SidedPlane(Vector p, double x, double y)`: a vertical plane.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the check point is on the plane.
    pub fn new_vertical(p: &Vector, x: f64, y: f64) -> Result<Self> {
        let plane = Plane::new_vertical(x, y);
        let sig_num = java_signum(plane.evaluate(p));
        Self::checked(plane, sig_num)
    }

    /// `new SidedPlane(Vector p, double vX, double vY, double vZ, double D)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the check point is on the plane.
    pub fn new_abcd(p: &Vector, vx: f64, vy: f64, vz: f64, d: f64) -> Result<Self> {
        let plane = Plane::new(vx, vy, vz, d);
        let sig_num = java_signum(plane.evaluate(p));
        Self::checked(plane, sig_num)
    }

    /// `new SidedPlane(Vector p, Vector v, double D)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the check point is on the plane.
    pub fn new_vector_d(p: &Vector, v: &Vector, d: f64) -> Result<Self> {
        let plane = Plane::new_vector_d(v, d);
        let sig_num = java_signum(plane.evaluate(p));
        Self::checked(plane, sig_num)
    }

    /// `new SidedPlane(double pX, double pY, double pZ, Vector v, double D)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the check point is on the plane.
    pub fn new_xyz_vector_d(px: f64, py: f64, pz: f64, v: &Vector, d: f64) -> Result<Self> {
        let plane = Plane::new_vector_d(v, d);
        let sig_num = java_signum(plane.evaluate_xyz(px, py, pz));
        Self::checked(plane, sig_num)
    }

    /// The plane (Java's `super`).
    #[inline]
    pub fn as_plane(&self) -> &Plane {
        &self.plane
    }

    /// `constructNormalizedPerpendicularSidedPlane(Vector, Vector, Vector,
    /// Vector)`: `None` when the construction fails.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the normal and the points vector are
    /// parallel (Java does not catch that one).
    pub fn construct_normalized_perpendicular_sided_plane(
        inside_point: &Vector,
        normal_vector: &Vector,
        point1: &Vector,
        point2: &Vector,
    ) -> Result<Option<SidedPlane>> {
        let points_vector = Vector::new(
            point1.x - point2.x,
            point1.y - point2.y,
            point1.z - point2.z,
        );
        let new_normal_vector = Vector::new_cross(normal_vector, &points_vector)?;
        Ok(SidedPlane::new_vector_d(
            inside_point,
            &new_normal_vector,
            -new_normal_vector.dot_product(point1),
        )
        .ok())
    }

    /// `constructSidedPlaneFromTwoPoints(Vector, Vector, Vector)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the construction.
    pub fn construct_sided_plane_from_two_points(
        inside_point: &Vector,
        upper_point: &Vector,
        lower_point: &Vector,
    ) -> Result<SidedPlane> {
        let plane = Plane::construct_perpendicular_center_plane_two_points(upper_point, lower_point)?;
        SidedPlane::new_abcd(inside_point, plane.x, plane.y, plane.z, plane.d)
    }

    /// `constructSidedPlaneFromOnePoint(Vector, Plane, Vector)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from the construction.
    pub fn construct_sided_plane_from_one_point(
        inside_point: &Vector,
        plane: &Plane,
        intersection_point: &Vector,
    ) -> Result<SidedPlane> {
        let new_plane = Plane::construct_perpendicular_center_plane_one_point(plane, intersection_point)?;
        SidedPlane::new_abcd(inside_point, new_plane.x, new_plane.y, new_plane.z, new_plane.d)
    }

    /// `constructNormalizedThreePointSidedPlane(Vector, Vector, Vector,
    /// Vector)`: `None` when no construction succeeds.
    pub fn construct_normalized_three_point_sided_plane(
        inside_point: &Vector,
        point1: &Vector,
        point2: &Vector,
        point3: &Vector,
    ) -> Option<SidedPlane> {
        let attempt = |p1: &Vector, p2: &Vector, p3: &Vector, base: &Vector| -> Result<SidedPlane> {
            let plane_normal = Vector::new_cross_components(
                p1.x - p2.x,
                p1.y - p2.y,
                p1.z - p2.z,
                p2.x - p3.x,
                p2.y - p3.y,
                p2.z - p3.z,
            )?;
            SidedPlane::new_vector_d(inside_point, &plane_normal, -plane_normal.dot_product(base))
        };
        if let Ok(rval) = attempt(point1, point2, point3, point2) {
            return Some(rval);
        }
        // new Vector(p1 - p3, p3 - p2), D from point3
        if let Ok(rval) = attempt(point1, point3, point2, point3) {
            return Some(rval);
        }
        // new Vector(p3 - p1, p1 - p2), D from point1
        attempt(point3, point1, point2, point1).ok()
    }

    /// `strictlyWithin(Vector)`.
    pub fn strictly_within(&self, v: &Vector) -> bool {
        let eval_result = self.evaluate_xyz(v.x, v.y, v.z);
        let sig_num = java_signum(eval_result);
        sig_num == 0.0 || sig_num == self.sig_num
    }

    /// `strictlyWithin(double, double, double)`.
    pub fn strictly_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        let eval_result = self.evaluate_xyz(x, y, z);
        let sig_num = java_signum(eval_result);
        sig_num == 0.0 || sig_num == self.sig_num
    }

    /// `SidedPlane.equals(Object)` against another sided plane.
    pub fn java_equals(&self, other: &SidedPlane) -> bool {
        std::ptr::eq(self, other)
            || (self.plane.java_equals(&other.plane) && double_compare_eq(other.sig_num, self.sig_num))
    }

    /// `SidedPlane.hashCode()`.
    pub fn java_hash_code(&self) -> i32 {
        self.plane
            .java_hash_code()
            .wrapping_mul(31)
            .wrapping_add(dhash(self.sig_num))
    }
}

impl Membership for SidedPlane {
    fn is_within_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        let eval_result = self.evaluate_xyz(x, y, z);
        if eval_result.abs() < Vector::MINIMUM_RESOLUTION {
            return true;
        }
        let sig_num = java_signum(eval_result);
        sig_num == self.sig_num
    }
}

impl PartialEq for SidedPlane {
    fn eq(&self, other: &Self) -> bool {
        self.java_equals(other)
    }
}

impl Eq for SidedPlane {}

impl Hash for SidedPlane {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.java_hash_code());
    }
}

impl fmt::Display for SidedPlane {
    /// `SidedPlane.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[A={}, B={}, C={}, D={}, side={}]",
            d2s(self.x),
            d2s(self.y),
            d2s(self.z),
            d2s(self.d),
            d2s(self.sig_num)
        )
    }
}
