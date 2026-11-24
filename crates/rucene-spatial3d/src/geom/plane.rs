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

//! The types declared in `Plane.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use rucene::util::java_math;
use std::f64::consts::PI;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use rucene::error::Result;

use super::bounds::Bounds;
use super::geo_point::GeoPoint;
use super::lat_lon_bounds::LatLonBounds;
use super::membership::Membership;
use super::planet_model::PlanetModel;
use super::vector::Vector;
use super::xyz_bounds::XYZBounds;
use super::{d2s, dhash, iae, ise, npe};

const MINIMUM_RESOLUTION: f64 = Vector::MINIMUM_RESOLUTION;
const MINIMUM_RESOLUTION_SQUARED: f64 = Vector::MINIMUM_RESOLUTION_SQUARED;
const MINIMUM_RESOLUTION_CUBED: f64 = Vector::MINIMUM_RESOLUTION_CUBED;

/// We know about three kinds of planes. First kind: general plane through
/// two points and origin. Second kind: horizontal plane at specified height.
/// Third kind: vertical plane with specified x and y value, through origin.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.Plane`. Java's `extends
/// Vector` is a [`Deref`] to the normal vector; an overload of an inherited
/// `Vector` method keeps its name, and Java's calls of the inherited form go
/// through [`Plane::as_vector`].
#[derive(Clone, Debug)]
pub struct Plane {
    v: Vector,
    /// Ax + By + Cz + D = 0
    pub d: f64,
}

impl Deref for Plane {
    type Target = Vector;
    #[inline]
    fn deref(&self) -> &Vector {
        &self.v
    }
}

/// `Plane.NO_POINTS`: an array with no points in it.
pub const NO_POINTS: &[GeoPoint] = &[];
/// `Plane.NO_BOUNDS`: an array with no bounds in it.
pub const NO_BOUNDS: &[&dyn Membership] = &[];
/// `Plane.normalYPlane`: a vertical plane normal to the Y axis.
pub static NORMAL_Y_PLANE: Plane = Plane::new(0.0, 1.0, 0.0, 0.0);
/// `Plane.normalXPlane`: a vertical plane normal to the X axis.
pub static NORMAL_X_PLANE: Plane = Plane::new(1.0, 0.0, 0.0, 0.0);
/// `Plane.normalZPlane`: a vertical plane normal to the Z axis.
pub static NORMAL_Z_PLANE: Plane = Plane::new(0.0, 0.0, 1.0, 0.0);

/// The line of intersection of two planes: its direction and a point on it.
struct Line {
    vx: f64,
    vy: f64,
    vz: f64,
    x0: f64,
    y0: f64,
    z0: f64,
}

impl Plane {
    /// `new Plane(double A, double B, double C, double D)`.
    pub const fn new(a: f64, b: f64, c: f64, d: f64) -> Self {
        Self {
            v: Vector::new(a, b, c),
            d,
        }
    }

    /// `new Plane(Vector A, double BX, double BY, double BZ)`: a plane through
    /// the origin and two points.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel.
    pub fn new_cross_xyz(a: &Vector, bx: f64, by: f64, bz: f64) -> Result<Self> {
        Ok(Self {
            v: Vector::new_cross_xyz(a, bx, by, bz)?,
            d: 0.0,
        })
    }

    /// `new Plane(Vector A, Vector B)`: a plane through the origin and two
    /// points.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel.
    pub fn new_cross(a: &Vector, b: &Vector) -> Result<Self> {
        Ok(Self {
            v: Vector::new_cross(a, b)?,
            d: 0.0,
        })
    }

    /// `new Plane(PlanetModel, double sinLat)`: a horizontal plane.
    pub fn new_horizontal(planet_model: &PlanetModel, sin_lat: f64) -> Self {
        Self {
            v: Vector::new(0.0, 0.0, 1.0),
            d: -sin_lat * Vector::compute_desired_ellipsoid_magnitude_z(planet_model, sin_lat),
        }
    }

    /// `new Plane(double x, double y)`: a vertical plane through the origin.
    pub fn new_vertical(x: f64, y: f64) -> Self {
        Self {
            v: Vector::new(y, -x, 0.0),
            d: 0.0,
        }
    }

    /// `new Plane(Vector v, double D)`.
    pub fn new_vector_d(v: &Vector, d: f64) -> Self {
        Self {
            v: Vector::new(v.x, v.y, v.z),
            d,
        }
    }

    /// `new Plane(Plane basePlane, boolean above)`: a plane parallel to the
    /// base one, just above or below it.
    pub fn new_offset(base_plane: &Plane, above: bool) -> Self {
        Self::new(
            base_plane.x,
            base_plane.y,
            base_plane.z,
            if above {
                (base_plane.d + MINIMUM_RESOLUTION).next_up()
            } else {
                (base_plane.d - MINIMUM_RESOLUTION).next_down()
            },
        )
    }

    /// The normal vector (Java's `super`).
    #[inline]
    pub fn as_vector(&self) -> &Vector {
        &self.v
    }

    /// `constructPerpendicularCenterPlaneOnePoint(Plane, Vector)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when no perpendicular plane exists.
    pub fn construct_perpendicular_center_plane_one_point(
        plane: &Plane,
        m: &Vector,
    ) -> Result<Plane> {
        debug_assert!(plane.evaluate_is_zero(m));
        let a0 = plane.x;
        let b0 = plane.y;
        let c0 = plane.z;
        let a1_denom = c0 * m.y - b0 * m.z;
        let b1_denom = c0 * m.x - a0 * m.z;
        let c1_denom = b0 * m.x - a0 * m.y;
        let a1;
        let b1;
        let c1;
        if a1_denom.abs() >= b1_denom.abs() && a1_denom.abs() >= c1_denom.abs() {
            a1 = 1.0;
            if m.y.abs() >= m.z.abs() {
                c1 = (b0 * m.x - a0 * m.y) / a1_denom;
                b1 = (-m.x - c1 * m.z) / m.y;
            } else {
                b1 = (a0 * m.z - c0 * m.x) / a1_denom;
                c1 = (-m.x - b1 * m.y) / m.z;
            }
        } else if b1_denom.abs() >= a1_denom.abs() && b1_denom.abs() >= c1_denom.abs() {
            b1 = 1.0;
            if m.x.abs() >= m.z.abs() {
                c1 = (a0 * m.y - b0 * m.x) / b1_denom;
                a1 = (-m.y - c1 * m.z) / m.x;
            } else {
                a1 = (b0 * m.z - c0 * m.y) / b1_denom;
                c1 = (-m.y - a1 * m.x) / m.z;
            }
        } else if c1_denom.abs() >= a1_denom.abs() && c1_denom.abs() >= b1_denom.abs() {
            c1 = 1.0;
            if m.x.abs() >= m.y.abs() {
                b1 = (a0 * m.z - c0 * m.x) / c1_denom;
                a1 = (-m.z - b1 * m.y) / m.x;
            } else {
                a1 = (c0 * m.y - b0 * m.z) / c1_denom;
                b1 = (-m.z - a1 * m.x) / m.y;
            }
        } else {
            return Err(iae("Cannot find perpendicular plane as requested"));
        }
        let norm_factor = 1.0 / java_math::sqrt(a1 * a1 + b1 * b1 + c1 * c1);
        let v = Vector::new(a1 * norm_factor, b1 * norm_factor, c1 * norm_factor);
        Ok(Plane::new_vector_d(
            &v,
            -(v.x * m.x + v.y * m.y + v.z * m.z),
        ))
    }

    /// `constructPerpendicularCenterPlaneTwoPoints(Vector, Vector)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the points are identical or the
    /// equation is unsolvable.
    pub fn construct_perpendicular_center_plane_two_points(
        m: &Vector,
        n: &Vector,
    ) -> Result<Plane> {
        let center_plane = Plane::new_cross(m, n)?;
        let a0 = center_plane.x;
        let b0 = center_plane.y;
        let c0 = center_plane.z;
        let x_diff = m.x - n.x;
        let y_diff = m.y - n.y;
        let z_diff = m.z - n.z;
        if x_diff * x_diff + y_diff * y_diff + z_diff * z_diff < MINIMUM_RESOLUTION_SQUARED {
            return Err(iae("Chosen points are numerically identical"));
        }
        let a1;
        let b1;
        let c1;
        let a1choice = c0 * y_diff - b0 * z_diff;
        let b1choice = c0 * x_diff - a0 * z_diff;
        let c1choice = b0 * x_diff - a0 * y_diff;
        if a1choice.abs() >= b1choice.abs() && a1choice.abs() >= c1choice.abs() {
            a1 = 1.0;
            if y_diff.abs() >= z_diff.abs() {
                c1 = (b0 * x_diff - a0 * y_diff) / a1choice;
                b1 = (-c1 * z_diff - x_diff) / y_diff;
            } else {
                b1 = (a0 * z_diff - c0 * x_diff) / a1choice;
                c1 = (-b1 * y_diff - x_diff) / z_diff;
            }
        } else if b1choice.abs() >= a1choice.abs() && b1choice.abs() >= c1choice.abs() {
            b1 = 1.0;
            if x_diff.abs() >= z_diff.abs() {
                c1 = (a0 * y_diff - b0 * x_diff) / b1choice;
                a1 = (-c1 * z_diff - y_diff) / x_diff;
            } else {
                a1 = (b0 * z_diff - c0 * y_diff) / b1choice;
                c1 = (-a1 * x_diff - y_diff) / z_diff;
            }
        } else if c1choice.abs() >= a1choice.abs() && c1choice.abs() >= b1choice.abs() {
            c1 = 1.0;
            if x_diff.abs() >= y_diff.abs() {
                b1 = (a0 * z_diff - c0 * x_diff) / c1choice;
                a1 = (-b1 * y_diff - z_diff) / x_diff;
            } else {
                a1 = (c0 * y_diff - b0 * z_diff) / c1choice;
                b1 = (-a1 * x_diff - z_diff) / y_diff;
            }
        } else {
            return Err(iae("Equation appears to be unsolveable"));
        }
        let norm_factor = 1.0 / java_math::sqrt(a1 * a1 + b1 * b1 + c1 * c1);
        let v = Vector::new(a1 * norm_factor, b1 * norm_factor, c1 * norm_factor);
        let rval = Plane::new_vector_d(&v, -(v.x * m.x + v.y * m.y + v.z * m.z));
        debug_assert!(rval.evaluate_is_zero(n));
        debug_assert!(
            (rval.x * center_plane.x + rval.y * center_plane.y + rval.z * center_plane.z).abs()
                < MINIMUM_RESOLUTION
        );
        Ok(rval)
    }

    /// `constructNormalizedZPlane(Vector...)`: `None` when the farthest point
    /// is on the z axis.
    pub fn construct_normalized_z_plane_points(plane_points: &[&Vector]) -> Option<Plane> {
        let mut best_distance = 0.0;
        let mut best_point: Option<&Vector> = None;
        for point in plane_points {
            let point_dist = point.x * point.x + point.y * point.y;
            if point_dist > best_distance {
                best_distance = point_dist;
                best_point = Some(point);
            }
        }
        let best_point = best_point.unwrap_or_else(|| npe());
        Self::construct_normalized_z_plane(best_point.x, best_point.y)
    }

    /// `constructNormalizedYPlane(Vector...)`.
    pub fn construct_normalized_y_plane_points(plane_points: &[&Vector]) -> Option<Plane> {
        let mut best_distance = 0.0;
        let mut best_point: Option<&Vector> = None;
        for point in plane_points {
            let point_dist = point.x * point.x + point.z * point.z;
            if point_dist > best_distance {
                best_distance = point_dist;
                best_point = Some(point);
            }
        }
        let best_point = best_point.unwrap_or_else(|| npe());
        Self::construct_normalized_y_plane(best_point.x, best_point.z, 0.0)
    }

    /// `constructNormalizedXPlane(Vector...)`.
    pub fn construct_normalized_x_plane_points(plane_points: &[&Vector]) -> Option<Plane> {
        let mut best_distance = 0.0;
        let mut best_point: Option<&Vector> = None;
        for point in plane_points {
            let point_dist = point.y * point.y + point.z * point.z;
            if point_dist > best_distance {
                best_distance = point_dist;
                best_point = Some(point);
            }
        }
        let best_point = best_point.unwrap_or_else(|| npe());
        Self::construct_normalized_x_plane(best_point.y, best_point.z, 0.0)
    }

    /// `constructNormalizedZPlane(double, double)`.
    pub fn construct_normalized_z_plane(x: f64, y: f64) -> Option<Plane> {
        if x.abs() < MINIMUM_RESOLUTION && y.abs() < MINIMUM_RESOLUTION {
            return None;
        }
        let denom = 1.0 / java_math::sqrt(x * x + y * y);
        Some(Plane::new(y * denom, -x * denom, 0.0, 0.0))
    }

    /// `constructNormalizedYPlane(double, double, double)`.
    pub fn construct_normalized_y_plane(x: f64, z: f64, d_value: f64) -> Option<Plane> {
        if x.abs() < MINIMUM_RESOLUTION && z.abs() < MINIMUM_RESOLUTION {
            return None;
        }
        let denom = 1.0 / java_math::sqrt(x * x + z * z);
        Some(Plane::new(z * denom, 0.0, -x * denom, d_value))
    }

    /// `constructNormalizedXPlane(double, double, double)`.
    pub fn construct_normalized_x_plane(y: f64, z: f64, d_value: f64) -> Option<Plane> {
        if y.abs() < MINIMUM_RESOLUTION && z.abs() < MINIMUM_RESOLUTION {
            return None;
        }
        let denom = 1.0 / java_math::sqrt(y * y + z * z);
        Some(Plane::new(0.0, z * denom, -y * denom, d_value))
    }

    /// `evaluate(Vector)`.
    #[inline]
    pub fn evaluate(&self, v: &Vector) -> f64 {
        self.v.dot_product(v) + self.d
    }

    /// `evaluate(double, double, double)`.
    #[inline]
    pub fn evaluate_xyz(&self, x: f64, y: f64, z: f64) -> f64 {
        self.v.dot_product_xyz(x, y, z) + self.d
    }

    /// `evaluateIsZero(Vector)`.
    #[inline]
    pub fn evaluate_is_zero(&self, v: &Vector) -> bool {
        self.evaluate(v).abs() < MINIMUM_RESOLUTION
    }

    /// `evaluateIsZero(double, double, double)`.
    #[inline]
    pub fn evaluate_is_zero_xyz(&self, x: f64, y: f64, z: f64) -> bool {
        self.evaluate_xyz(x, y, z).abs() < MINIMUM_RESOLUTION
    }

    /// `normalize()`: `None` when the normal is too small.
    pub fn normalize(&self) -> Option<Plane> {
        let norm_vect = self.v.normalize()?;
        Some(Plane::new_vector_d(&norm_vect, self.d))
    }

    /// `arcDistance(PlanetModel, GeoPoint, Membership...)`.
    pub fn arc_distance(
        &self,
        planet_model: &PlanetModel,
        v: &Vector,
        bounds: &[&dyn Membership],
    ) -> f64 {
        self.arc_distance_xyz(planet_model, v.x, v.y, v.z, bounds)
    }

    /// `arcDistance(PlanetModel, double, double, double, Membership...)`.
    pub fn arc_distance_xyz(
        &self,
        planet_model: &PlanetModel,
        x: f64,
        y: f64,
        z: f64,
        bounds: &[&dyn Membership],
    ) -> f64 {
        if self.evaluate_is_zero_xyz(x, y, z) {
            if Self::meets_all_bounds_xyz(x, y, z, bounds) {
                return 0.0;
            }
            return f64::INFINITY;
        }
        let perp_plane = Plane::new(
            self.y * z - self.z * y,
            self.z * x - self.x * z,
            self.x * y - self.y * x,
            0.0,
        );
        let intersection_points = self
            .find_intersections(planet_model, &perp_plane, NO_BOUNDS)
            .unwrap_or_else(|| npe());
        let mut min_distance = f64::INFINITY;
        for intersection_point in &intersection_points {
            if Self::meets_all_bounds(intersection_point, bounds) {
                let the_distance = intersection_point.arc_distance_xyz(x, y, z);
                if the_distance < min_distance {
                    min_distance = the_distance;
                }
            }
        }
        min_distance
    }

    /// `normalDistance(Vector, Membership...)`.
    pub fn normal_distance(&self, v: &Vector, bounds: &[&dyn Membership]) -> f64 {
        self.normal_distance_xyz(v.x, v.y, v.z, bounds)
    }

    /// `normalDistance(double, double, double, Membership...)`.
    pub fn normal_distance_xyz(&self, x: f64, y: f64, z: f64, bounds: &[&dyn Membership]) -> f64 {
        let dist = self.evaluate_xyz(x, y, z);
        let perp_x = x - dist * self.x;
        let perp_y = y - dist * self.y;
        let perp_z = z - dist * self.z;
        if !Self::meets_all_bounds_xyz(perp_x, perp_y, perp_z, bounds) {
            return f64::INFINITY;
        }
        dist.abs()
    }

    /// `normalDistanceSquared(Vector, Membership...)`.
    pub fn normal_distance_squared(&self, v: &Vector, bounds: &[&dyn Membership]) -> f64 {
        self.normal_distance_squared_xyz(v.x, v.y, v.z, bounds)
    }

    /// `normalDistanceSquared(double, double, double, Membership...)`.
    pub fn normal_distance_squared_xyz(
        &self,
        x: f64,
        y: f64,
        z: f64,
        bounds: &[&dyn Membership],
    ) -> f64 {
        let normal = self.normal_distance_xyz(x, y, z, bounds);
        if normal == f64::INFINITY {
            return normal;
        }
        normal * normal
    }

    /// `linearDistance(PlanetModel, GeoPoint, Membership...)`.
    pub fn linear_distance(
        &self,
        planet_model: &PlanetModel,
        v: &Vector,
        bounds: &[&dyn Membership],
    ) -> f64 {
        self.linear_distance_xyz(planet_model, v.x, v.y, v.z, bounds)
    }

    /// `linearDistance(PlanetModel, double, double, double, Membership...)`.
    pub fn linear_distance_xyz(
        &self,
        planet_model: &PlanetModel,
        x: f64,
        y: f64,
        z: f64,
        bounds: &[&dyn Membership],
    ) -> f64 {
        if self.evaluate_is_zero_xyz(x, y, z) {
            if Self::meets_all_bounds_xyz(x, y, z, bounds) {
                return 0.0;
            }
            return f64::INFINITY;
        }
        let perp_plane = Plane::new(
            self.y * z - self.z * y,
            self.z * x - self.x * z,
            self.x * y - self.y * x,
            0.0,
        );
        let intersection_points = self
            .find_intersections(planet_model, &perp_plane, NO_BOUNDS)
            .unwrap_or_else(|| npe());
        let mut min_distance = f64::INFINITY;
        for intersection_point in &intersection_points {
            if Self::meets_all_bounds(intersection_point, bounds) {
                let the_distance = intersection_point.linear_distance_xyz(x, y, z);
                if the_distance < min_distance {
                    min_distance = the_distance;
                }
            }
        }
        min_distance
    }

    /// `linearDistanceSquared(PlanetModel, GeoPoint, Membership...)`.
    pub fn linear_distance_squared(
        &self,
        planet_model: &PlanetModel,
        v: &Vector,
        bounds: &[&dyn Membership],
    ) -> f64 {
        self.linear_distance_squared_xyz(planet_model, v.x, v.y, v.z, bounds)
    }

    /// `linearDistanceSquared(PlanetModel, double, double, double,
    /// Membership...)`.
    pub fn linear_distance_squared_xyz(
        &self,
        planet_model: &PlanetModel,
        x: f64,
        y: f64,
        z: f64,
        bounds: &[&dyn Membership],
    ) -> f64 {
        let linear_distance = self.linear_distance_xyz(planet_model, x, y, z, bounds);
        linear_distance * linear_distance
    }

    /// `interpolate(PlanetModel, GeoPoint, GeoPoint, double[])`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a point is not on the plane.
    // Java's late-assigned local.
    #[allow(clippy::needless_late_init)]
    pub fn interpolate(
        &self,
        planet_model: &PlanetModel,
        start: &GeoPoint,
        end: &GeoPoint,
        proportions: &[f64],
    ) -> Result<Vec<GeoPoint>> {
        let mut a = self.x;
        let mut b = self.y;
        let mut c = self.z;
        let trans_x = -self.d * a;
        let trans_y = -self.d * b;
        let trans_z = -self.d * c;
        let cos_ra;
        let sin_ra;
        let cos_ha;
        let sin_ha;
        let magnitude = self.magnitude();
        if magnitude >= MINIMUM_RESOLUTION {
            let denom = 1.0 / magnitude;
            a *= denom;
            b *= denom;
            c *= denom;
            let xy_magnitude = java_math::sqrt(a * a + b * b);
            if xy_magnitude >= MINIMUM_RESOLUTION {
                let xy_denom = 1.0 / xy_magnitude;
                cos_ra = a * xy_denom;
                sin_ra = -b * xy_denom;
            } else {
                cos_ra = 1.0;
                sin_ra = 0.0;
            }
            sin_ha = xy_magnitude;
            cos_ha = c;
        } else {
            cos_ra = 1.0;
            sin_ra = 0.0;
            cos_ha = 1.0;
            sin_ha = 0.0;
        }
        let modified_start = Self::modify(
            start, trans_x, trans_y, trans_z, sin_ra, cos_ra, sin_ha, cos_ha,
        );
        let modified_end = Self::modify(
            end, trans_x, trans_y, trans_z, sin_ra, cos_ra, sin_ha, cos_ha,
        );
        if modified_start.z.abs() >= MINIMUM_RESOLUTION {
            return Err(iae(format!(
                "Start point was not on plane: {}",
                d2s(modified_start.z)
            )));
        }
        if modified_end.z.abs() >= MINIMUM_RESOLUTION {
            return Err(iae(format!(
                "End point was not on plane: {}",
                d2s(modified_end.z)
            )));
        }
        let start_angle = java_math::atan2(modified_start.y, modified_start.x);
        let end_angle = java_math::atan2(modified_end.y, modified_end.x);
        let start_magnitude = java_math::sqrt(
            modified_start.x * modified_start.x + modified_start.y * modified_start.y,
        );
        let delta;
        let mut new_end_angle = end_angle;
        while new_end_angle < start_angle {
            new_end_angle += PI * 2.0;
        }
        if new_end_angle - start_angle <= PI {
            delta = new_end_angle - start_angle;
        } else {
            let mut new_start_angle = start_angle;
            while new_start_angle < end_angle {
                new_start_angle += PI * 2.0;
            }
            delta = new_start_angle - end_angle;
        }
        let mut return_values = Vec::with_capacity(proportions.len());
        for proportion in proportions {
            let new_angle = start_angle + proportion * delta;
            let sin_new_angle = java_math::sin(new_angle);
            let cos_new_angle = java_math::cos(new_angle);
            let new_vector = Vector::new(
                cos_new_angle * start_magnitude,
                sin_new_angle * start_magnitude,
                0.0,
            );
            return_values.push(Self::reverse_modify(
                planet_model,
                &new_vector,
                trans_x,
                trans_y,
                trans_z,
                sin_ra,
                cos_ra,
                sin_ha,
                cos_ha,
            ));
        }
        Ok(return_values)
    }

    /// `modify(GeoPoint, ...)`: translates and rotates a point.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn modify(
        start: &GeoPoint,
        trans_x: f64,
        trans_y: f64,
        trans_z: f64,
        sin_ra: f64,
        cos_ra: f64,
        sin_ha: f64,
        cos_ha: f64,
    ) -> Vector {
        start
            .translate(trans_x, trans_y, trans_z)
            .rotate_xy_sin_cos(sin_ra, cos_ra)
            .rotate_xz_sin_cos(sin_ha, cos_ha)
    }

    /// `reverseModify(PlanetModel, Vector, ...)`: undoes [`Plane::modify`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reverse_modify(
        planet_model: &PlanetModel,
        point: &Vector,
        trans_x: f64,
        trans_y: f64,
        trans_z: f64,
        sin_ra: f64,
        cos_ra: f64,
        sin_ha: f64,
        cos_ha: f64,
    ) -> GeoPoint {
        let result = point
            .rotate_xz_sin_cos(-sin_ha, cos_ha)
            .rotate_xy_sin_cos(-sin_ra, cos_ra)
            .translate(-trans_x, -trans_y, -trans_z);
        planet_model.create_surface_point_xyz(result.x, result.y, result.z)
    }

    /// `findIntersections(PlanetModel, Plane, Membership...)`: `None` when the
    /// planes are numerically identical.
    pub fn find_intersections(
        &self,
        planet_model: &PlanetModel,
        q: &Plane,
        bounds: &[&dyn Membership],
    ) -> Option<Vec<GeoPoint>> {
        if self.is_numerically_identical(q) {
            return None;
        }
        Some(self.find_intersections_with(planet_model, q, bounds, NO_BOUNDS))
    }

    /// `findCrossings(PlanetModel, Plane, Membership...)`: `None` when the
    /// planes are numerically identical.
    pub fn find_crossings(
        &self,
        planet_model: &PlanetModel,
        q: &Plane,
        bounds: &[&dyn Membership],
    ) -> Option<Vec<GeoPoint>> {
        if self.is_numerically_identical(q) {
            return None;
        }
        Some(self.find_crossings_with(planet_model, q, bounds, NO_BOUNDS))
    }

    /// `arePointsCoplanar(GeoPoint, GeoPoint, GeoPoint)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when no normal vector converges.
    pub fn are_points_coplanar(a: &GeoPoint, b: &GeoPoint, c: &GeoPoint) -> Result<bool> {
        Ok(Vector::cross_product_evaluate_is_zero(a, b, c)?
            || Vector::cross_product_evaluate_is_zero(a, c, b)?
            || Vector::cross_product_evaluate_is_zero(b, c, a)?)
    }

    /// The setup common to the intersection computations: `None` where Java
    /// returns early.
    // Java's late-assigned local.
    #[allow(clippy::needless_late_init)]
    fn line_with(&self, q: &Plane) -> Option<Line> {
        let line_vector_x = self.y * q.z - self.z * q.y;
        let line_vector_y = self.z * q.x - self.x * q.z;
        let line_vector_z = self.x * q.y - self.y * q.x;
        if line_vector_x.abs() < MINIMUM_RESOLUTION
            && line_vector_y.abs() < MINIMUM_RESOLUTION
            && line_vector_z.abs() < MINIMUM_RESOLUTION
        {
            return None;
        }
        let x0;
        let y0;
        let z0;
        let denom_yz = self.y * q.z - self.z * q.y;
        let denom_xz = self.x * q.z - self.z * q.x;
        let denom_xy = self.x * q.y - self.y * q.x;
        if denom_yz.abs() >= denom_xz.abs() && denom_yz.abs() >= denom_xy.abs() {
            if denom_yz.abs() < MINIMUM_RESOLUTION_SQUARED {
                return None;
            }
            let denom = 1.0 / denom_yz;
            x0 = 0.0;
            y0 = (-self.d * q.z - self.z * -q.d) * denom;
            z0 = (self.y * -q.d + self.d * q.y) * denom;
        } else if denom_xz.abs() >= denom_xy.abs() && denom_xz.abs() >= denom_yz.abs() {
            if denom_xz.abs() < MINIMUM_RESOLUTION_SQUARED {
                return None;
            }
            let denom = 1.0 / denom_xz;
            x0 = (-self.d * q.z - self.z * -q.d) * denom;
            y0 = 0.0;
            z0 = (self.x * -q.d + self.d * q.x) * denom;
        } else {
            if denom_xy.abs() < MINIMUM_RESOLUTION_SQUARED {
                return None;
            }
            let denom = 1.0 / denom_xy;
            x0 = (-self.d * q.y - self.y * -q.d) * denom;
            y0 = (self.x * -q.d + self.d * q.x) * denom;
            z0 = 0.0;
        }
        Some(Line {
            vx: line_vector_x,
            vy: line_vector_y,
            vz: line_vector_z,
            x0,
            y0,
            z0,
        })
    }

    /// The `A`, `B`, `C` of the quadratic `At^2 + Bt + C = 0` of a line and
    /// the ellipsoid.
    fn quadratic(
        planet_model: &PlanetModel,
        line_vector_x: f64,
        line_vector_y: f64,
        line_vector_z: f64,
        x0: f64,
        y0: f64,
        z0: f64,
    ) -> (f64, f64, f64) {
        let a = line_vector_x * line_vector_x * planet_model.inverse_xy_scaling_squared
            + line_vector_y * line_vector_y * planet_model.inverse_xy_scaling_squared
            + line_vector_z * line_vector_z * planet_model.inverse_z_scaling_squared;
        let b = 2.0
            * (line_vector_x * x0 * planet_model.inverse_xy_scaling_squared
                + line_vector_y * y0 * planet_model.inverse_xy_scaling_squared
                + line_vector_z * z0 * planet_model.inverse_z_scaling_squared);
        let c = x0 * x0 * planet_model.inverse_xy_scaling_squared
            + y0 * y0 * planet_model.inverse_xy_scaling_squared
            + z0 * z0 * planet_model.inverse_z_scaling_squared
            - 1.0;
        (a, b, c)
    }

    fn within_all(bounds: &[&dyn Membership], x: f64, y: f64, z: f64) -> bool {
        for bound in bounds {
            if !bound.is_within_xyz(x, y, z) {
                return false;
            }
        }
        true
    }

    /// `findIntersections(PlanetModel, Plane, Membership[], Membership[])`.
    pub(crate) fn find_intersections_with(
        &self,
        planet_model: &PlanetModel,
        q: &Plane,
        bounds: &[&dyn Membership],
        more_bounds: &[&dyn Membership],
    ) -> Vec<GeoPoint> {
        let Some(l) = self.line_with(q) else {
            return Vec::new();
        };
        let (a, b, c) = Self::quadratic(planet_model, l.vx, l.vy, l.vz, l.x0, l.y0, l.z0);
        let bsquared_minus = b * b - 4.0 * a * c;
        if bsquared_minus.abs() < MINIMUM_RESOLUTION_SQUARED {
            let inverse2_a = 1.0 / (2.0 * a);
            let t = -b * inverse2_a;
            let point_x = l.vx * t + l.x0;
            let point_y = l.vy * t + l.y0;
            let point_z = l.vz * t + l.z0;
            for bound in bounds {
                if !bound.is_within_xyz(point_x, point_y, point_z) {
                    return Vec::new();
                }
            }
            for bound in more_bounds {
                if !bound.is_within_xyz(point_x, point_y, point_z) {
                    return Vec::new();
                }
            }
            vec![GeoPoint::new(point_x, point_y, point_z)]
        } else if bsquared_minus > 0.0 {
            let inverse2_a = 1.0 / (2.0 * a);
            let sqrt_term = java_math::sqrt(bsquared_minus);
            let t1 = (-b + sqrt_term) * inverse2_a;
            let t2 = (-b - sqrt_term) * inverse2_a;
            let point1_x = l.vx * t1 + l.x0;
            let point1_y = l.vy * t1 + l.y0;
            let point1_z = l.vz * t1 + l.z0;
            let point2_x = l.vx * t2 + l.x0;
            let point2_y = l.vy * t2 + l.y0;
            let point2_z = l.vz * t2 + l.z0;
            let point1_valid = Self::within_all(bounds, point1_x, point1_y, point1_z)
                && Self::within_all(more_bounds, point1_x, point1_y, point1_z);
            let point2_valid = Self::within_all(bounds, point2_x, point2_y, point2_z)
                && Self::within_all(more_bounds, point2_x, point2_y, point2_z);
            if point1_valid && point2_valid {
                return vec![
                    GeoPoint::new(point1_x, point1_y, point1_z),
                    GeoPoint::new(point2_x, point2_y, point2_z),
                ];
            }
            if point1_valid {
                return vec![GeoPoint::new(point1_x, point1_y, point1_z)];
            }
            if point2_valid {
                return vec![GeoPoint::new(point2_x, point2_y, point2_z)];
            }
            Vec::new()
        } else {
            Vec::new()
        }
    }

    /// `findCrossings(PlanetModel, Plane, Membership[], Membership[])`.
    pub(crate) fn find_crossings_with(
        &self,
        planet_model: &PlanetModel,
        q: &Plane,
        bounds: &[&dyn Membership],
        more_bounds: &[&dyn Membership],
    ) -> Vec<GeoPoint> {
        let Some(l) = self.line_with(q) else {
            return Vec::new();
        };
        let (a, b, c) = Self::quadratic(planet_model, l.vx, l.vy, l.vz, l.x0, l.y0, l.z0);
        let bsquared_minus = b * b - 4.0 * a * c;
        if bsquared_minus.abs() < MINIMUM_RESOLUTION_SQUARED {
            Vec::new()
        } else if bsquared_minus > 0.0 {
            let inverse2_a = 1.0 / (2.0 * a);
            let sqrt_term = java_math::sqrt(bsquared_minus);
            let t1 = (-b + sqrt_term) * inverse2_a;
            let t2 = (-b - sqrt_term) * inverse2_a;
            let point1_x = l.vx * t1 + l.x0;
            let point1_y = l.vy * t1 + l.y0;
            let point1_z = l.vz * t1 + l.z0;
            let point2_x = l.vx * t2 + l.x0;
            let point2_y = l.vy * t2 + l.y0;
            let point2_z = l.vz * t2 + l.z0;
            let point1_valid = Self::within_all(bounds, point1_x, point1_y, point1_z)
                && Self::within_all(more_bounds, point1_x, point1_y, point1_z);
            let point2_valid = Self::within_all(bounds, point2_x, point2_y, point2_z)
                && Self::within_all(more_bounds, point2_x, point2_y, point2_z);
            if point1_valid && point2_valid {
                return vec![
                    GeoPoint::new(point1_x, point1_y, point1_z),
                    GeoPoint::new(point2_x, point2_y, point2_z),
                ];
            }
            if point1_valid {
                return vec![GeoPoint::new(point1_x, point1_y, point1_z)];
            }
            if point2_valid {
                return vec![GeoPoint::new(point2_x, point2_y, point2_z)];
            }
            Vec::new()
        } else {
            Vec::new()
        }
    }

    /// `findIntersectionBounds(PlanetModel, Bounds, Plane, Membership...)`.
    pub(crate) fn find_intersection_bounds(
        &self,
        planet_model: &PlanetModel,
        bounds_info: &mut dyn Bounds,
        q: &Plane,
        bounds: &[&dyn Membership],
    ) {
        let line_vector_x = self.y * q.z - self.z * q.y;
        let line_vector_y = self.z * q.x - self.x * q.z;
        let line_vector_z = self.x * q.y - self.y * q.x;
        if line_vector_x.abs() < MINIMUM_RESOLUTION
            && line_vector_y.abs() < MINIMUM_RESOLUTION
            && line_vector_z.abs() < MINIMUM_RESOLUTION
        {
            return;
        }
        let denom_yz = self.y * q.z - self.z * q.y;
        let denom_xz = self.x * q.z - self.z * q.x;
        let denom_xy = self.x * q.y - self.y * q.x;
        let r = MINIMUM_RESOLUTION;
        let td = self.d;
        let qd = q.d;
        let mut rec = |x0: f64, y0: f64, z0: f64| {
            Self::record_line_bounds(
                planet_model,
                bounds_info,
                line_vector_x,
                line_vector_y,
                line_vector_z,
                x0,
                y0,
                z0,
                bounds,
            );
        };
        if denom_yz.abs() >= denom_xz.abs() && denom_yz.abs() >= denom_xy.abs() {
            if denom_yz.abs() < MINIMUM_RESOLUTION_SQUARED {
                return;
            }
            let denom = 1.0 / denom_yz;
            rec(
                0.0,
                (-(td + r) * q.z - self.z * -(qd + r)) * denom,
                (self.y * -(qd + r) + (td + r) * q.y) * denom,
            );
            rec(
                0.0,
                (-(td - r) * q.z - self.z * -(qd + r)) * denom,
                (self.y * -(qd + r) + (td - r) * q.y) * denom,
            );
            rec(
                0.0,
                (-(td + r) * q.z - self.z * -(qd - r)) * denom,
                (self.y * -(qd - r) + (td + r) * q.y) * denom,
            );
            rec(
                0.0,
                (-(td - r) * q.z - self.z * -(qd - r)) * denom,
                (self.y * -(qd - r) + (td - r) * q.y) * denom,
            );
        } else if denom_xz.abs() >= denom_xy.abs() && denom_xz.abs() >= denom_yz.abs() {
            if denom_xz.abs() < MINIMUM_RESOLUTION_SQUARED {
                return;
            }
            let denom = 1.0 / denom_xz;
            rec(
                (-(td + r) * q.z - self.z * -(qd + r)) * denom,
                0.0,
                (self.x * -(qd + r) + (td + r) * q.x) * denom,
            );
            rec(
                (-(td - r) * q.z - self.z * -(qd + r)) * denom,
                0.0,
                (self.x * -(qd + r) + (td - r) * q.x) * denom,
            );
            rec(
                (-(td + r) * q.z - self.z * -(qd - r)) * denom,
                0.0,
                (self.x * -(qd - r) + (td + r) * q.x) * denom,
            );
            rec(
                (-(td - r) * q.z - self.z * -(qd - r)) * denom,
                0.0,
                (self.x * -(qd - r) + (td - r) * q.x) * denom,
            );
        } else {
            if denom_xy.abs() < MINIMUM_RESOLUTION_SQUARED {
                return;
            }
            let denom = 1.0 / denom_xy;
            rec(
                (-(td + r) * q.y - self.y * -(qd + r)) * denom,
                (self.x * -(qd + r) + (td + r) * q.x) * denom,
                0.0,
            );
            rec(
                (-(td - r) * q.y - self.y * -(qd + r)) * denom,
                (self.x * -(qd + r) + (td - r) * q.x) * denom,
                0.0,
            );
            rec(
                (-(td + r) * q.y - self.y * -(qd - r)) * denom,
                (self.x * -(qd - r) + (td + r) * q.x) * denom,
                0.0,
            );
            rec(
                (-(td - r) * q.y - self.y * -(qd - r)) * denom,
                (self.x * -(qd - r) + (td - r) * q.x) * denom,
                0.0,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_line_bounds(
        planet_model: &PlanetModel,
        bounds_info: &mut dyn Bounds,
        line_vector_x: f64,
        line_vector_y: f64,
        line_vector_z: f64,
        x0: f64,
        y0: f64,
        z0: f64,
        bounds: &[&dyn Membership],
    ) {
        let (a, b, c) = Self::quadratic(
            planet_model,
            line_vector_x,
            line_vector_y,
            line_vector_z,
            x0,
            y0,
            z0,
        );
        let bsquared_minus = b * b - 4.0 * a * c;
        if bsquared_minus.abs() < MINIMUM_RESOLUTION_SQUARED {
            let inverse2_a = 1.0 / (2.0 * a);
            let t = -b * inverse2_a;
            let point_x = line_vector_x * t + x0;
            let point_y = line_vector_y * t + y0;
            let point_z = line_vector_z * t + z0;
            for bound in bounds {
                if !bound.is_within_xyz(point_x, point_y, point_z) {
                    return;
                }
            }
            bounds_info.add_point(&GeoPoint::new(point_x, point_y, point_z));
        } else if bsquared_minus > 0.0 {
            let inverse2_a = 1.0 / (2.0 * a);
            let sqrt_term = java_math::sqrt(bsquared_minus);
            let t1 = (-b + sqrt_term) * inverse2_a;
            let t2 = (-b - sqrt_term) * inverse2_a;
            let point1_x = line_vector_x * t1 + x0;
            let point1_y = line_vector_y * t1 + y0;
            let point1_z = line_vector_z * t1 + z0;
            let point2_x = line_vector_x * t2 + x0;
            let point2_y = line_vector_y * t2 + y0;
            let point2_z = line_vector_z * t2 + z0;
            let point1_valid = Self::within_all(bounds, point1_x, point1_y, point1_z);
            let point2_valid = Self::within_all(bounds, point2_x, point2_y, point2_z);
            if point1_valid {
                bounds_info.add_point(&GeoPoint::new(point1_x, point1_y, point1_z));
            }
            if point2_valid {
                bounds_info.add_point(&GeoPoint::new(point2_x, point2_y, point2_z));
            }
        } else {
            bounds_info.no_bound(planet_model);
        }
    }

    /// `recordBounds(PlanetModel, XYZBounds, Plane, Membership...)`.
    pub fn record_bounds_intersection_xyz(
        &self,
        planet_model: &PlanetModel,
        bounds_info: &mut XYZBounds,
        p: &Plane,
        bounds: &[&dyn Membership],
    ) {
        self.find_intersection_bounds(planet_model, bounds_info, p, bounds);
    }

    /// `recordBounds(PlanetModel, XYZBounds, Membership...)`.
    pub fn record_bounds_xyz(
        &self,
        planet_model: &PlanetModel,
        bounds_info: &mut XYZBounds,
        bounds: &[&dyn Membership],
    ) {
        let a_ = self.x;
        let b_ = self.y;
        let c_ = self.z;
        let d_ = self.d;
        if !bounds_info.is_smallest_min_z(planet_model)
            || !bounds_info.is_largest_max_z(planet_model)
        {
            if a_.abs() >= MINIMUM_RESOLUTION || b_.abs() >= MINIMUM_RESOLUTION {
                let normalized_z_plane =
                    Self::construct_normalized_z_plane(a_, b_).unwrap_or_else(|| npe());
                let points = self.find_intersections_with(
                    planet_model,
                    &normalized_z_plane,
                    bounds,
                    NO_BOUNDS,
                );
                for point in &points {
                    debug_assert!(planet_model.point_on_surface(point));
                    Self::add_point(bounds_info, bounds, point);
                }
            } else {
                let mut points = self.find_intersections_with(
                    planet_model,
                    &NORMAL_Y_PLANE,
                    NO_BOUNDS,
                    NO_BOUNDS,
                );
                if points.is_empty() {
                    points = self.find_intersections_with(
                        planet_model,
                        &NORMAL_X_PLANE,
                        NO_BOUNDS,
                        NO_BOUNDS,
                    );
                }
                if points.is_empty() {
                    bounds_info.add_z_value(&GeoPoint::new(0.0, 0.0, -self.z));
                } else {
                    bounds_info.add_z_value(&points[0]);
                }
            }
        }
        let x = self.x;
        let y = self.y;
        let z = self.z;
        let k = 1.0
            / ((x * x + y * y) * planet_model.xy_scaling * planet_model.xy_scaling
                + z * z * planet_model.z_scaling * planet_model.z_scaling);
        let ab_squared = planet_model.xy_scaling * planet_model.xy_scaling;
        let c_squared = planet_model.z_scaling * planet_model.z_scaling;
        let a_squared = a_ * a_;
        let b_squared = b_ * b_;
        let cc_squared = c_ * c_;
        let r = 2.0 * d_ * k;
        let r_squared = r * r;
        if !bounds_info.is_smallest_min_x(planet_model)
            || !bounds_info.is_largest_max_x(planet_model)
        {
            let q = a_ * ab_squared * k;
            let q_squared = q * q;
            let a = a_squared * ab_squared * r_squared
                + b_squared * ab_squared * r_squared
                + cc_squared * c_squared * r_squared
                - 4.0;
            let b = -2.0 * a_ * ab_squared * r
                + 2.0 * a_squared * ab_squared * r * q
                + 2.0 * b_squared * ab_squared * r * q
                + 2.0 * cc_squared * c_squared * r * q;
            let c = ab_squared - 2.0 * a_ * ab_squared * q
                + a_squared * ab_squared * q_squared
                + b_squared * ab_squared * q_squared
                + cc_squared * c_squared * q_squared;
            let point_x = |l: f64, denom: f64| {
                GeoPoint::new(
                    (1.0 - l * a_) * ab_squared * denom,
                    -l * b_ * ab_squared * denom,
                    -l * c_ * c_squared * denom,
                )
            };
            if a.abs() >= MINIMUM_RESOLUTION_SQUARED {
                let sqrt_term = b * b - 4.0 * a * c;
                if sqrt_term.abs() < MINIMUM_RESOLUTION_SQUARED {
                    let m = -b / (2.0 * a);
                    if m.abs() >= MINIMUM_RESOLUTION {
                        let l = r * m + q;
                        let denom0 = 0.5 / m;
                        let the_point = point_x(l, denom0);
                        Self::add_point(bounds_info, bounds, &the_point);
                    } else {
                        bounds_info.add_x_value_f64(-d_ / a_);
                    }
                } else if sqrt_term > 0.0 {
                    let sqrt_result = java_math::sqrt(sqrt_term);
                    let common_denom = 0.5 / a;
                    let m1 = (-b + sqrt_result) * common_denom;
                    debug_assert!((a * m1 * m1 + b * m1 + c).abs() < MINIMUM_RESOLUTION);
                    let m2 = (-b - sqrt_result) * common_denom;
                    debug_assert!((a * m2 * m2 + b * m2 + c).abs() < MINIMUM_RESOLUTION);
                    if m1.abs() >= MINIMUM_RESOLUTION || m2.abs() >= MINIMUM_RESOLUTION {
                        let l1 = r * m1 + q;
                        let l2 = r * m2 + q;
                        let denom1 = 0.5 / m1;
                        let denom2 = 0.5 / m2;
                        let the_point1 = point_x(l1, denom1);
                        let the_point2 = point_x(l2, denom2);
                        Self::add_point(bounds_info, bounds, &the_point1);
                        Self::add_point(bounds_info, bounds, &the_point2);
                    } else {
                        bounds_info.add_x_value_f64(-d_ / a_);
                    }
                }
            } else if b.abs() > MINIMUM_RESOLUTION_SQUARED {
                let m = -c / b;
                let l = r * m + q;
                let denom0 = 0.5 / m;
                let the_point = point_x(l, denom0);
                Self::add_point(bounds_info, bounds, &the_point);
            }
        }
        if !bounds_info.is_smallest_min_y(planet_model)
            || !bounds_info.is_largest_max_y(planet_model)
        {
            let q = b_ * ab_squared * k;
            let q_squared = q * q;
            let a = a_squared * ab_squared * r_squared
                + b_squared * ab_squared * r_squared
                + cc_squared * c_squared * r_squared
                - 4.0;
            let b = 2.0 * a_squared * ab_squared * r * q - 2.0 * b_ * ab_squared * r
                + 2.0 * b_squared * ab_squared * r * q
                + 2.0 * cc_squared * c_squared * r * q;
            let c = a_squared * ab_squared * q_squared + ab_squared - 2.0 * b_ * ab_squared * q
                + b_squared * ab_squared * q_squared
                + cc_squared * c_squared * q_squared;
            let point_y = |l: f64, denom: f64| {
                GeoPoint::new(
                    -l * a_ * ab_squared * denom,
                    (1.0 - l * b_) * ab_squared * denom,
                    -l * c_ * c_squared * denom,
                )
            };
            if a.abs() >= MINIMUM_RESOLUTION_SQUARED {
                let sqrt_term = b * b - 4.0 * a * c;
                if sqrt_term.abs() < MINIMUM_RESOLUTION_SQUARED {
                    let m = -b / (2.0 * a);
                    if m.abs() >= MINIMUM_RESOLUTION {
                        let l = r * m + q;
                        let denom0 = 0.5 / m;
                        let the_point = point_y(l, denom0);
                        Self::add_point(bounds_info, bounds, &the_point);
                    } else {
                        bounds_info.add_y_value_f64(-d_ / b_);
                    }
                } else if sqrt_term > 0.0 {
                    let sqrt_result = java_math::sqrt(sqrt_term);
                    let common_denom = 0.5 / a;
                    let m1 = (-b + sqrt_result) * common_denom;
                    debug_assert!((a * m1 * m1 + b * m1 + c).abs() < MINIMUM_RESOLUTION);
                    let m2 = (-b - sqrt_result) * common_denom;
                    debug_assert!((a * m2 * m2 + b * m2 + c).abs() < MINIMUM_RESOLUTION);
                    if m1.abs() >= MINIMUM_RESOLUTION || m2.abs() >= MINIMUM_RESOLUTION {
                        let l1 = r * m1 + q;
                        let l2 = r * m2 + q;
                        let denom1 = 0.5 / m1;
                        let denom2 = 0.5 / m2;
                        let the_point1 = point_y(l1, denom1);
                        let the_point2 = point_y(l2, denom2);
                        Self::add_point(bounds_info, bounds, &the_point1);
                        Self::add_point(bounds_info, bounds, &the_point2);
                    } else {
                        bounds_info.add_y_value_f64(-d_ / b_);
                    }
                }
            } else if b.abs() > MINIMUM_RESOLUTION_SQUARED {
                let m = -c / b;
                let l = r * m + q;
                let denom0 = 0.5 / m;
                let the_point = point_y(l, denom0);
                Self::add_point(bounds_info, bounds, &the_point);
            }
        }
    }

    /// `recordBounds(PlanetModel, LatLonBounds, Plane, Membership...)`.
    pub fn record_bounds_intersection_lat_lon(
        &self,
        planet_model: &PlanetModel,
        bounds_info: &mut LatLonBounds,
        p: &Plane,
        bounds: &[&dyn Membership],
    ) {
        self.find_intersection_bounds(planet_model, bounds_info, p, bounds);
    }

    /// `recordBounds(PlanetModel, LatLonBounds, Membership...)`.
    pub fn record_bounds_lat_lon(
        &self,
        planet_model: &PlanetModel,
        bounds_info: &mut LatLonBounds,
        bounds: &[&dyn Membership],
    ) {
        let a_ = self.x;
        let b_ = self.y;
        let c_ = self.z;
        let d_ = self.d;
        if !bounds_info.check_no_top_latitude_bound()
            || !bounds_info.check_no_bottom_latitude_bound()
        {
            if a_.abs() >= MINIMUM_RESOLUTION || b_.abs() >= MINIMUM_RESOLUTION {
                let vertical_plane =
                    Self::construct_normalized_z_plane(a_, b_).unwrap_or_else(|| npe());
                let points =
                    self.find_intersections_with(planet_model, &vertical_plane, bounds, NO_BOUNDS);
                for point in &points {
                    Self::add_point(bounds_info, bounds, point);
                }
            } else {
                let mut points = self.find_intersections_with(
                    planet_model,
                    &NORMAL_X_PLANE,
                    NO_BOUNDS,
                    NO_BOUNDS,
                );
                if points.is_empty() {
                    points = self.find_intersections_with(
                        planet_model,
                        &NORMAL_Y_PLANE,
                        NO_BOUNDS,
                        NO_BOUNDS,
                    );
                }
                if points.is_empty() {
                    bounds_info.add_z_value(&GeoPoint::new(0.0, 0.0, -self.z));
                } else {
                    bounds_info.add_z_value(&points[0]);
                }
            }
        }
        if !bounds_info.check_no_longitude_bound() {
            let a;
            let b;
            let c;
            if c_.abs() < MINIMUM_RESOLUTION {
                if d_.abs() >= MINIMUM_RESOLUTION {
                    if a_.abs() > b_.abs() {
                        a = b_ * b_ * planet_model.inverse_xy_scaling_squared
                            + a_ * a_ * planet_model.inverse_xy_scaling_squared;
                        b = 2.0 * b_ * d_ * planet_model.inverse_xy_scaling_squared;
                        c = d_ * d_ * planet_model.inverse_xy_scaling_squared - a_ * a_;
                        let sqrt_clause = b * b - 4.0 * a * c;
                        if sqrt_clause.abs() < MINIMUM_RESOLUTION_SQUARED {
                            let y0 = -b / (2.0 * a);
                            let x0 = (-d_ - b_ * y0) / a_;
                            let z0 = 0.0;
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0, y0, z0));
                        } else if sqrt_clause > 0.0 {
                            let sqrt_result = java_math::sqrt(sqrt_clause);
                            let denom = 1.0 / (2.0 * a);
                            let hdenom = 1.0 / a_;
                            let y0a = (-b + sqrt_result) * denom;
                            let y0b = (-b - sqrt_result) * denom;
                            let x0a = (-d_ - b_ * y0a) * hdenom;
                            let x0b = (-d_ - b_ * y0b) * hdenom;
                            let z0a = 0.0;
                            let z0b = 0.0;
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0a, y0a, z0a));
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0b, y0b, z0b));
                        }
                    } else {
                        a = b_ * b_ * planet_model.inverse_xy_scaling_squared
                            + a_ * a_ * planet_model.inverse_xy_scaling_squared;
                        b = 2.0 * a_ * d_ * planet_model.inverse_xy_scaling_squared;
                        c = d_ * d_ * planet_model.inverse_xy_scaling_squared - b_ * b_;
                        let sqrt_clause = b * b - 4.0 * a * c;
                        if sqrt_clause.abs() < MINIMUM_RESOLUTION_SQUARED {
                            let x0 = -b / (2.0 * a);
                            let y0 = (-d_ - a_ * x0) / b_;
                            let z0 = 0.0;
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0, y0, z0));
                        } else if sqrt_clause > 0.0 {
                            let sqrt_result = java_math::sqrt(sqrt_clause);
                            let denom = 1.0 / (2.0 * a);
                            let idenom = 1.0 / b_;
                            let x0a = (-b + sqrt_result) * denom;
                            let x0b = (-b - sqrt_result) * denom;
                            let y0a = (-d_ - a_ * x0a) * idenom;
                            let y0b = (-d_ - a_ * x0b) * idenom;
                            let z0a = 0.0;
                            let z0b = 0.0;
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0a, y0a, z0a));
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0b, y0b, z0b));
                        }
                    }
                }
            } else {
                let e = a_ * a_ * planet_model.inverse_z_scaling_squared
                    + c_ * c_ * planet_model.inverse_xy_scaling_squared;
                let f = b_ * b_ * planet_model.inverse_z_scaling_squared
                    + c_ * c_ * planet_model.inverse_xy_scaling_squared;
                let g = 2.0 * a_ * b_ * planet_model.inverse_z_scaling_squared;
                let h = 2.0 * a_ * d_ * planet_model.inverse_z_scaling_squared;
                let i = 2.0 * b_ * d_ * planet_model.inverse_z_scaling_squared;
                let j = d_ * d_ * planet_model.inverse_z_scaling_squared - c_ * c_;
                if j.abs() >= MINIMUM_RESOLUTION && j > 0.0 {
                    if h.abs() > i.abs() {
                        a = e * i * i - g * h * i + f * h * h;
                        b = 4.0 * e * i * j - 2.0 * g * h * j;
                        c = 4.0 * e * j * j - j * h * h;
                        let sqrt_clause = b * b - 4.0 * a * c;
                        if sqrt_clause.abs() < MINIMUM_RESOLUTION_CUBED {
                            let y0 = -b / (2.0 * a);
                            let x0 = (-2.0 * j - i * y0) / h;
                            let z0 = (-a_ * x0 - b_ * y0 - d_) / c_;
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0, y0, z0));
                        } else if sqrt_clause > 0.0 {
                            let sqrt_result = java_math::sqrt(sqrt_clause);
                            let denom = 1.0 / (2.0 * a);
                            let hdenom = 1.0 / h;
                            let cdenom = 1.0 / c_;
                            let y0a = (-b + sqrt_result) * denom;
                            let y0b = (-b - sqrt_result) * denom;
                            let x0a = (-2.0 * j - i * y0a) * hdenom;
                            let x0b = (-2.0 * j - i * y0b) * hdenom;
                            let z0a = (-a_ * x0a - b_ * y0a - d_) * cdenom;
                            let z0b = (-a_ * x0b - b_ * y0b - d_) * cdenom;
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0a, y0a, z0a));
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0b, y0b, z0b));
                        }
                    } else {
                        a = e * i * i - g * h * i + f * h * h;
                        b = 4.0 * f * h * j - 2.0 * g * i * j;
                        c = 4.0 * f * j * j - j * i * i;
                        let sqrt_clause = b * b - 4.0 * a * c;
                        if sqrt_clause.abs() < MINIMUM_RESOLUTION_CUBED {
                            let x0 = -b / (2.0 * a);
                            let y0 = (-2.0 * j - h * x0) / i;
                            let z0 = (-a_ * x0 - b_ * y0 - d_) / c_;
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0, y0, z0));
                        } else if sqrt_clause > 0.0 {
                            let sqrt_result = java_math::sqrt(sqrt_clause);
                            let denom = 1.0 / (2.0 * a);
                            let idenom = 1.0 / i;
                            let cdenom = 1.0 / c_;
                            let x0a = (-b + sqrt_result) * denom;
                            let x0b = (-b - sqrt_result) * denom;
                            let y0a = (-2.0 * j - h * x0a) * idenom;
                            let y0b = (-2.0 * j - h * x0b) * idenom;
                            let z0a = (-a_ * x0a - b_ * y0a - d_) * cdenom;
                            let z0b = (-a_ * x0b - b_ * y0b - d_) * cdenom;
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0a, y0a, z0a));
                            Self::add_point(bounds_info, bounds, &GeoPoint::new(x0b, y0b, z0b));
                        }
                    }
                }
            }
        }
    }

    fn add_point(bounds_info: &mut dyn Bounds, bounds: &[&dyn Membership], point: &GeoPoint) {
        for bound in bounds {
            if !bound.is_within(point) {
                return;
            }
        }
        bounds_info.add_point(point);
    }

    /// `intersects(PlanetModel, Plane, GeoPoint[], GeoPoint[], Membership[],
    /// Membership...)`.
    pub fn intersects(
        &self,
        planet_model: &PlanetModel,
        q: &Plane,
        notable_points: &[GeoPoint],
        more_notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
        more_bounds: &[&dyn Membership],
    ) -> bool {
        if self.is_numerically_identical(q) {
            for p in notable_points {
                if Self::meets_all_bounds2(p, bounds, more_bounds) {
                    return true;
                }
            }
            for p in more_notable_points {
                if Self::meets_all_bounds2(p, bounds, more_bounds) {
                    return true;
                }
            }
            return false;
        }
        let Some(l) = self.line_with(q) else {
            return false;
        };
        let (a, b, c) = Self::quadratic(planet_model, l.vx, l.vy, l.vz, l.x0, l.y0, l.z0);
        let bsquared_minus = b * b - 4.0 * a * c;
        if bsquared_minus.abs() < MINIMUM_RESOLUTION_SQUARED {
            let inverse2_a = 1.0 / (2.0 * a);
            let t = -b * inverse2_a;
            let point_x = l.vx * t + l.x0;
            let point_y = l.vy * t + l.y0;
            let point_z = l.vz * t + l.z0;
            Self::within_all(bounds, point_x, point_y, point_z)
                && Self::within_all(more_bounds, point_x, point_y, point_z)
        } else if bsquared_minus > 0.0 {
            let inverse2_a = 1.0 / (2.0 * a);
            let sqrt_term = java_math::sqrt(bsquared_minus);
            let t1 = (-b + sqrt_term) * inverse2_a;
            let t2 = (-b - sqrt_term) * inverse2_a;
            let point1_x = l.vx * t1 + l.x0;
            let point1_y = l.vy * t1 + l.y0;
            let point1_z = l.vz * t1 + l.z0;
            if Self::within_all(bounds, point1_x, point1_y, point1_z)
                && Self::within_all(more_bounds, point1_x, point1_y, point1_z)
            {
                return true;
            }
            let point2_x = l.vx * t2 + l.x0;
            let point2_y = l.vy * t2 + l.y0;
            let point2_z = l.vz * t2 + l.z0;
            Self::within_all(bounds, point2_x, point2_y, point2_z)
                && Self::within_all(more_bounds, point2_x, point2_y, point2_z)
        } else {
            false
        }
    }

    /// `crosses(PlanetModel, Plane, GeoPoint[], GeoPoint[], Membership[],
    /// Membership...)`.
    pub fn crosses(
        &self,
        planet_model: &PlanetModel,
        q: &Plane,
        notable_points: &[GeoPoint],
        more_notable_points: &[GeoPoint],
        bounds: &[&dyn Membership],
        more_bounds: &[&dyn Membership],
    ) -> bool {
        if self.is_numerically_identical(q) {
            for p in notable_points {
                if Self::meets_all_bounds2(p, bounds, more_bounds) {
                    return true;
                }
            }
            for p in more_notable_points {
                if Self::meets_all_bounds2(p, bounds, more_bounds) {
                    return true;
                }
            }
            return false;
        }
        let Some(l) = self.line_with(q) else {
            return false;
        };
        let (a, b, c) = Self::quadratic(planet_model, l.vx, l.vy, l.vz, l.x0, l.y0, l.z0);
        let bsquared_minus = b * b - 4.0 * a * c;
        if bsquared_minus.abs() < MINIMUM_RESOLUTION_SQUARED {
            false
        } else if bsquared_minus > 0.0 {
            let inverse2_a = 1.0 / (2.0 * a);
            let sqrt_term = java_math::sqrt(bsquared_minus);
            let t1 = (-b + sqrt_term) * inverse2_a;
            let t2 = (-b - sqrt_term) * inverse2_a;
            let point1_x = l.vx * t1 + l.x0;
            let point1_y = l.vy * t1 + l.y0;
            let point1_z = l.vz * t1 + l.z0;
            if Self::within_all(bounds, point1_x, point1_y, point1_z)
                && Self::within_all(more_bounds, point1_x, point1_y, point1_z)
            {
                return true;
            }
            let point2_x = l.vx * t2 + l.x0;
            let point2_y = l.vy * t2 + l.y0;
            let point2_z = l.vz * t2 + l.z0;
            Self::within_all(bounds, point2_x, point2_y, point2_z)
                && Self::within_all(more_bounds, point2_x, point2_y, point2_z)
        } else {
            false
        }
    }

    /// `isFunctionallyIdentical(Plane)`.
    pub fn is_functionally_identical(&self, p: &Plane) -> bool {
        let cross1 = self.y * p.z - self.z * p.y;
        let cross2 = self.z * p.x - self.x * p.z;
        let cross3 = self.x * p.y - self.y * p.x;
        if cross1 * cross1 + cross2 * cross2 + cross3 * cross3 >= 5.0 * MINIMUM_RESOLUTION {
            return false;
        }
        let denom = 1.0 / (p.x * p.x + p.y * p.y + p.z * p.z);
        self.evaluate_is_zero_xyz(-p.x * p.d * denom, -p.y * p.d * denom, -p.z * p.d * denom)
    }

    /// `isNumericallyIdentical(Plane)`.
    pub fn is_numerically_identical(&self, p: &Plane) -> bool {
        let cross1 = self.y * p.z - self.z * p.y;
        let cross2 = self.z * p.x - self.x * p.z;
        let cross3 = self.x * p.y - self.y * p.x;
        if cross1 * cross1 + cross2 * cross2 + cross3 * cross3 >= MINIMUM_RESOLUTION_SQUARED {
            return false;
        }
        let denom = 1.0 / (p.x * p.x + p.y * p.y + p.z * p.z);
        self.evaluate_is_zero_xyz(-p.x * p.d * denom, -p.y * p.d * denom, -p.z * p.d * denom)
    }

    /// `findArcDistancePoints(PlanetModel, double, GeoPoint, Membership...)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when the plane does not go through the origin;
    /// `IllegalArgumentException` when the start point is not on the plane.
    pub fn find_arc_distance_points(
        &self,
        planet_model: &PlanetModel,
        arc_distance_value: f64,
        start_point: &GeoPoint,
        bounds: &[&dyn Membership],
    ) -> Result<Vec<GeoPoint>> {
        if self.d.abs() >= MINIMUM_RESOLUTION {
            return Err(ise(
                "Can't find arc distance using plane that doesn't go through origin",
            ));
        }
        if !self.evaluate_is_zero(start_point) {
            return Err(iae("Start point is not on plane"));
        }
        let azimuth_magnitude = java_math::sqrt(self.x * self.x + self.y * self.y);
        let cos_plane_altitude = self.z;
        let sin_plane_altitude = azimuth_magnitude;
        let cos_plane_azimuth = self.x / azimuth_magnitude;
        let sin_plane_azimuth = self.y / azimuth_magnitude;
        debug_assert!(
            (sin_plane_altitude * sin_plane_altitude + cos_plane_altitude * cos_plane_altitude
                - 1.0)
                .abs()
                < MINIMUM_RESOLUTION,
            "Improper sin/cos of altitude: {}",
            d2s(sin_plane_altitude * sin_plane_altitude + cos_plane_altitude * cos_plane_altitude)
        );
        debug_assert!(
            (sin_plane_azimuth * sin_plane_azimuth + cos_plane_azimuth * cos_plane_azimuth - 1.0)
                .abs()
                < MINIMUM_RESOLUTION,
            "Improper sin/cos of azimuth: {}",
            d2s(sin_plane_azimuth * sin_plane_azimuth + cos_plane_azimuth * cos_plane_azimuth)
        );
        let x0 = start_point.x;
        let y0 = start_point.y;
        let z0 = start_point.z;
        let x1 = x0 * cos_plane_azimuth + y0 * sin_plane_azimuth;
        let y1 = -x0 * sin_plane_azimuth + y0 * cos_plane_azimuth;
        let z1 = z0;
        let x2 = x1 * cos_plane_altitude - z1 * sin_plane_altitude;
        let y2 = y1;
        let z2 = x1 * sin_plane_altitude + z1 * cos_plane_altitude;
        debug_assert!(
            z2.abs() < MINIMUM_RESOLUTION,
            "Rotation should have put startpoint on x-y plane, instead has value {}",
            d2s(z2)
        );
        let start_angle = java_math::atan2(y2, x2);
        let point1_angle = start_angle + arc_distance_value;
        let point2_angle = start_angle - arc_distance_value;
        let point1x2 = java_math::cos(point1_angle);
        let point1y2 = java_math::sin(point1_angle);
        let point1z2 = 0.0;
        let point2x2 = java_math::cos(point2_angle);
        let point2y2 = java_math::sin(point2_angle);
        let point2z2 = 0.0;
        let point1x1 = point1x2 * cos_plane_altitude + point1z2 * sin_plane_altitude;
        let point1y1 = point1y2;
        let point1z1 = -point1x2 * sin_plane_altitude + point1z2 * cos_plane_altitude;
        let point2x1 = point2x2 * cos_plane_altitude + point2z2 * sin_plane_altitude;
        let point2y1 = point2y2;
        let point2z1 = -point2x2 * sin_plane_altitude + point2z2 * cos_plane_altitude;
        let point1x0 = point1x1 * cos_plane_azimuth - point1y1 * sin_plane_azimuth;
        let point1y0 = point1x1 * sin_plane_azimuth + point1y1 * cos_plane_azimuth;
        let point1z0 = point1z1;
        let point2x0 = point2x1 * cos_plane_azimuth - point2y1 * sin_plane_azimuth;
        let point2y0 = point2x1 * sin_plane_azimuth + point2y1 * cos_plane_azimuth;
        let point2z0 = point2z1;
        let point1 = planet_model.create_surface_point_xyz(point1x0, point1y0, point1z0);
        let point2 = planet_model.create_surface_point_xyz(point2x0, point2y0, point2z0);
        let is_point1_inside = Self::meets_all_bounds(&point1, bounds);
        let is_point2_inside = Self::meets_all_bounds(&point2, bounds);
        Ok(if is_point1_inside {
            if is_point2_inside {
                vec![point1, point2]
            } else {
                vec![point1]
            }
        } else if is_point2_inside {
            vec![point2]
        } else {
            Vec::new()
        })
    }

    fn meets_all_bounds(p: &Vector, bounds: &[&dyn Membership]) -> bool {
        Self::meets_all_bounds_xyz(p.x, p.y, p.z, bounds)
    }

    fn meets_all_bounds_xyz(x: f64, y: f64, z: f64, bounds: &[&dyn Membership]) -> bool {
        Self::within_all(bounds, x, y, z)
    }

    fn meets_all_bounds2(
        p: &Vector,
        bounds: &[&dyn Membership],
        more_bounds: &[&dyn Membership],
    ) -> bool {
        Self::meets_all_bounds_xyz(p.x, p.y, p.z, bounds)
            && Self::meets_all_bounds_xyz(p.x, p.y, p.z, more_bounds)
    }

    /// `getSampleIntersectionPoint(PlanetModel, Plane)`.
    pub fn get_sample_intersection_point(
        &self,
        planet_model: &PlanetModel,
        q: &Plane,
    ) -> Option<GeoPoint> {
        let intersections = self.find_intersections_with(planet_model, q, NO_BOUNDS, NO_BOUNDS);
        intersections.into_iter().next()
    }

    /// `Plane.equals(Object)` against another plane.
    pub fn java_equals(&self, other: &Plane) -> bool {
        self.v.java_equals(&other.v) && other.d == self.d
    }

    /// `Plane.hashCode()`.
    pub fn java_hash_code(&self) -> i32 {
        self.v
            .java_hash_code()
            .wrapping_mul(31)
            .wrapping_add(dhash(self.d))
    }
}

impl PartialEq for Plane {
    fn eq(&self, other: &Self) -> bool {
        self.java_equals(other)
    }
}

impl Eq for Plane {}

impl Hash for Plane {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.java_hash_code());
    }
}

impl fmt::Display for Plane {
    /// `Plane.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[A={}, B={}; C={}; D={}]",
            d2s(self.x),
            d2s(self.y),
            d2s(self.z),
            d2s(self.d)
        )
    }
}
