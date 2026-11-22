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

//! The types declared in `Vector.java`, of the Java package
//! `org.apache.lucene.spatial3d.geom`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use rucene::error::Result;

use super::membership::Membership;
use super::planet_model::PlanetModel;
use super::{d2s, dhash, iae};

/// A 3d vector in space, not necessarily going through the origin.
///
/// Equivalent to `org.apache.lucene.spatial3d.geom.Vector`.
///
/// **Divergence.** Java's `GeoPoint` extends `Vector` and overrides
/// `magnitude()` with a cached value that its constructors may set to the
/// ellipsoid magnitude rather than the computed one (`GeoPoint.java:30-35`,
/// `GeoPoint.java:302-309`), so a `GeoPoint` seen through a `Vector`
/// reference answers `magnitude()` from that cache. Rust has no virtual
/// override of an inherited method; the port keeps the cache in the `Vector`
/// itself (`point_magnitude`, present only for the vector part of a
/// [`GeoPoint`](super::geo_point::GeoPoint)), so every `magnitude()` call
/// answers what Java's virtual dispatch answers.
pub struct Vector {
    /// The x value.
    pub x: f64,
    /// The y value.
    pub y: f64,
    /// The z value.
    pub z: f64,
    /// `GeoPoint.magnitude`, bits of a `double` (`NEGATIVE_INFINITY` when not
    /// yet computed); `None` for a plain `Vector`.
    pub(crate) point_magnitude: Option<AtomicU64>,
}

impl Clone for Vector {
    fn clone(&self) -> Self {
        Self {
            x: self.x,
            y: self.y,
            z: self.z,
            point_magnitude: self
                .point_magnitude
                .as_ref()
                .map(|m| AtomicU64::new(m.load(Ordering::Relaxed))),
        }
    }
}

impl fmt::Debug for Vector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Vector {
    /// Values that are all considered to be essentially zero have a magnitude
    /// less than this.
    pub const MINIMUM_RESOLUTION: f64 = 1.0e-12;
    /// Angular version of minimum resolution.
    pub const MINIMUM_ANGULAR_RESOLUTION: f64 = std::f64::consts::PI * Self::MINIMUM_RESOLUTION;
    /// For squared quantities, the bound is squared too.
    pub const MINIMUM_RESOLUTION_SQUARED: f64 =
        Self::MINIMUM_RESOLUTION * Self::MINIMUM_RESOLUTION;
    /// For cubed quantities, cube the bound.
    pub const MINIMUM_RESOLUTION_CUBED: f64 =
        Self::MINIMUM_RESOLUTION_SQUARED * Self::MINIMUM_RESOLUTION;
    /// The maximum Gram-Schmidt dot-product envelope.
    const MINIMUM_GRAM_SCHMIDT_ENVELOPE: f64 = Self::MINIMUM_RESOLUTION * 0.5;

    /// `new Vector(double x, double y, double z)`.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self {
            x,
            y,
            z,
            point_magnitude: None,
        }
    }

    /// `new Vector(Vector A, double BX, double BY, double BZ)`: the normalized
    /// cross product of two vectors.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel.
    pub fn new_cross_xyz(a: &Vector, bx: f64, by: f64, bz: f64) -> Result<Self> {
        Self::new_cross_components(a.x, a.y, a.z, bx, by, bz)
    }

    /// `new Vector(double AX, double AY, double AZ, double BX, double BY,
    /// double BZ)`: the normalized cross product of two vectors.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel or no normal
    /// vector converges.
    pub fn new_cross_components(
        ax: f64,
        ay: f64,
        az: f64,
        bx: f64,
        by: f64,
        bz: f64,
    ) -> Result<Self> {
        let this_x = ay * bz - az * by;
        let this_y = az * bx - ax * bz;
        let this_z = ax * by - ay * bx;
        let magnitude = Self::magnitude_of(this_x, this_y, this_z);
        if magnitude == 0.0 {
            return Err(iae("Degenerate/parallel vector constructed"));
        }
        let inverse_magnitude = 1.0 / magnitude;
        let mut normalize_x = this_x * inverse_magnitude;
        let mut normalize_y = this_y * inverse_magnitude;
        let mut normalize_z = this_z * inverse_magnitude;
        let mut i = 0;
        loop {
            let current_dot_prod_a = ax * normalize_x + ay * normalize_y + az * normalize_z;
            let current_dot_prod_b = bx * normalize_x + by * normalize_y + bz * normalize_z;
            if current_dot_prod_a.abs() < Self::MINIMUM_GRAM_SCHMIDT_ENVELOPE
                && current_dot_prod_b.abs() < Self::MINIMUM_GRAM_SCHMIDT_ENVELOPE
            {
                break;
            }
            let (cvx, cvy, cvz, current_dot_prod) =
                if current_dot_prod_a.abs() > current_dot_prod_b.abs() {
                    (ax, ay, az, current_dot_prod_a)
                } else {
                    (bx, by, bz, current_dot_prod_b)
                };
            normalize_x = normalize_x - current_dot_prod * cvx;
            normalize_y = normalize_y - current_dot_prod * cvy;
            normalize_z = normalize_z - current_dot_prod * cvz;
            let corrected_magnitude = Self::magnitude_of(normalize_x, normalize_y, normalize_z);
            let inverse_corrected_magnitude = 1.0 / corrected_magnitude;
            normalize_x = normalize_x * inverse_corrected_magnitude;
            normalize_y = normalize_y * inverse_corrected_magnitude;
            normalize_z = normalize_z * inverse_corrected_magnitude;
            let old = i;
            i += 1;
            if old > 10 {
                return Err(iae(
                    "Plane could not be constructed! Could not find a normal vector.",
                ));
            }
        }
        Ok(Self::new(normalize_x, normalize_y, normalize_z))
    }

    /// `new Vector(Vector A, Vector B)`: the normalized cross product.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the vectors are parallel.
    pub fn new_cross(a: &Vector, b: &Vector) -> Result<Self> {
        Self::new_cross_xyz(a, b.x, b.y, b.z)
    }

    /// `Vector.magnitude(double, double, double)`.
    #[inline]
    pub fn magnitude_of(x: f64, y: f64, z: f64) -> f64 {
        (x * x + y * y + z * z).sqrt()
    }

    /// `normalize()`: `None` when the magnitude is too small.
    pub fn normalize(&self) -> Option<Vector> {
        let denom = self.magnitude();
        if denom < Self::MINIMUM_RESOLUTION {
            return None;
        }
        let norm_factor = 1.0 / denom;
        Some(Vector::new(
            self.x * norm_factor,
            self.y * norm_factor,
            self.z * norm_factor,
        ))
    }

    /// `crossProductEvaluateIsZero(Vector A, Vector B, Vector point)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when no normal vector converges.
    pub fn cross_product_evaluate_is_zero(a: &Vector, b: &Vector, point: &Vector) -> Result<bool> {
        let this_x = a.y * b.z - a.z * b.y;
        let this_y = a.z * b.x - a.x * b.z;
        let this_z = a.x * b.y - a.y * b.x;
        let magnitude = Self::magnitude_of(this_x, this_y, this_z);
        if magnitude == 0.0 {
            return Ok(true);
        }
        let inverse_magnitude = 1.0 / magnitude;
        let mut normalize_x = this_x * inverse_magnitude;
        let mut normalize_y = this_y * inverse_magnitude;
        let mut normalize_z = this_z * inverse_magnitude;
        let mut i = 0;
        loop {
            let current_dot_prod_a = a.x * normalize_x + a.y * normalize_y + a.z * normalize_z;
            let current_dot_prod_b = b.x * normalize_x + b.y * normalize_y + b.z * normalize_z;
            if current_dot_prod_a.abs() < Self::MINIMUM_GRAM_SCHMIDT_ENVELOPE
                && current_dot_prod_b.abs() < Self::MINIMUM_GRAM_SCHMIDT_ENVELOPE
            {
                break;
            }
            let (cvx, cvy, cvz, current_dot_prod) =
                if current_dot_prod_a.abs() > current_dot_prod_b.abs() {
                    (a.x, a.y, a.z, current_dot_prod_a)
                } else {
                    (b.x, b.y, b.z, current_dot_prod_b)
                };
            normalize_x = normalize_x - current_dot_prod * cvx;
            normalize_y = normalize_y - current_dot_prod * cvy;
            normalize_z = normalize_z - current_dot_prod * cvz;
            let corrected_magnitude = Self::magnitude_of(normalize_x, normalize_y, normalize_z);
            let inverse_corrected_magnitude = 1.0 / corrected_magnitude;
            normalize_x = normalize_x * inverse_corrected_magnitude;
            normalize_y = normalize_y * inverse_corrected_magnitude;
            normalize_z = normalize_z * inverse_corrected_magnitude;
            let old = i;
            i += 1;
            if old > 10 {
                return Err(iae(
                    "Plane could not be constructed! Could not find a normal vector.",
                ));
            }
        }
        Ok(
            (normalize_x * point.x + normalize_y * point.y + normalize_z * point.z).abs()
                < Self::MINIMUM_RESOLUTION,
        )
    }

    /// `dotProduct(Vector)`.
    #[inline]
    pub fn dot_product(&self, v: &Vector) -> f64 {
        self.x * v.x + self.y * v.y + self.z * v.z
    }

    /// `dotProduct(double, double, double)`.
    #[inline]
    pub fn dot_product_xyz(&self, x: f64, y: f64, z: f64) -> f64 {
        self.x * x + self.y * y + self.z * z
    }

    /// `isWithin(Membership[] bounds, Membership... moreBounds)`; a `None`
    /// bound is skipped, as Java skips `null`.
    pub fn is_within_bounds(
        &self,
        bounds: &[Option<&dyn Membership>],
        more_bounds: &[Option<&dyn Membership>],
    ) -> bool {
        for bound in bounds.iter().flatten() {
            if !bound.is_within(self) {
                return false;
            }
        }
        for bound in more_bounds.iter().flatten() {
            if !bound.is_within(self) {
                return false;
            }
        }
        true
    }

    /// `translate(double, double, double)`.
    pub fn translate(&self, x_offset: f64, y_offset: f64, z_offset: f64) -> Vector {
        Vector::new(self.x - x_offset, self.y - y_offset, self.z - z_offset)
    }

    /// `rotateXY(double angle)`.
    pub fn rotate_xy(&self, angle: f64) -> Vector {
        self.rotate_xy_sin_cos(angle.sin(), angle.cos())
    }

    /// `rotateXY(double sinAngle, double cosAngle)`.
    pub fn rotate_xy_sin_cos(&self, sin_angle: f64, cos_angle: f64) -> Vector {
        Vector::new(
            self.x * cos_angle - self.y * sin_angle,
            self.x * sin_angle + self.y * cos_angle,
            self.z,
        )
    }

    /// `rotateXZ(double angle)`.
    pub fn rotate_xz(&self, angle: f64) -> Vector {
        self.rotate_xz_sin_cos(angle.sin(), angle.cos())
    }

    /// `rotateXZ(double sinAngle, double cosAngle)`.
    pub fn rotate_xz_sin_cos(&self, sin_angle: f64, cos_angle: f64) -> Vector {
        Vector::new(
            self.x * cos_angle - self.z * sin_angle,
            self.y,
            self.x * sin_angle + self.z * cos_angle,
        )
    }

    /// `rotateZY(double angle)`.
    pub fn rotate_zy(&self, angle: f64) -> Vector {
        self.rotate_zy_sin_cos(angle.sin(), angle.cos())
    }

    /// `rotateZY(double sinAngle, double cosAngle)`.
    pub fn rotate_zy_sin_cos(&self, sin_angle: f64, cos_angle: f64) -> Vector {
        Vector::new(
            self.x,
            self.z * sin_angle + self.y * cos_angle,
            self.z * cos_angle - self.y * sin_angle,
        )
    }

    /// `linearDistanceSquared(Vector)`.
    pub fn linear_distance_squared(&self, v: &Vector) -> f64 {
        let delta_x = self.x - v.x;
        let delta_y = self.y - v.y;
        let delta_z = self.z - v.z;
        delta_x * delta_x + delta_y * delta_y + delta_z * delta_z
    }

    /// `linearDistanceSquared(double, double, double)`.
    pub fn linear_distance_squared_xyz(&self, x: f64, y: f64, z: f64) -> f64 {
        let delta_x = self.x - x;
        let delta_y = self.y - y;
        let delta_z = self.z - z;
        delta_x * delta_x + delta_y * delta_y + delta_z * delta_z
    }

    /// `linearDistance(Vector)`.
    pub fn linear_distance(&self, v: &Vector) -> f64 {
        self.linear_distance_squared(v).sqrt()
    }

    /// `linearDistance(double, double, double)`.
    pub fn linear_distance_xyz(&self, x: f64, y: f64, z: f64) -> f64 {
        self.linear_distance_squared_xyz(x, y, z).sqrt()
    }

    /// `normalDistanceSquared(Vector)`.
    pub fn normal_distance_squared(&self, v: &Vector) -> f64 {
        let t = self.dot_product(v);
        let delta_x = self.x * t - v.x;
        let delta_y = self.y * t - v.y;
        let delta_z = self.z * t - v.z;
        delta_x * delta_x + delta_y * delta_y + delta_z * delta_z
    }

    /// `normalDistanceSquared(double, double, double)`.
    pub fn normal_distance_squared_xyz(&self, x: f64, y: f64, z: f64) -> f64 {
        let t = self.dot_product_xyz(x, y, z);
        let delta_x = self.x * t - x;
        let delta_y = self.y * t - y;
        let delta_z = self.z * t - z;
        delta_x * delta_x + delta_y * delta_y + delta_z * delta_z
    }

    /// `normalDistance(Vector)`.
    pub fn normal_distance(&self, v: &Vector) -> f64 {
        self.normal_distance_squared(v).sqrt()
    }

    /// `normalDistance(double, double, double)`.
    pub fn normal_distance_xyz(&self, x: f64, y: f64, z: f64) -> f64 {
        self.normal_distance_squared_xyz(x, y, z).sqrt()
    }

    /// `magnitude()`, with `GeoPoint`'s cached override for the vector part
    /// of a point.
    pub fn magnitude(&self) -> f64 {
        match &self.point_magnitude {
            None => Self::magnitude_of(self.x, self.y, self.z),
            Some(cache) => {
                let mut mag = f64::from_bits(cache.load(Ordering::Relaxed));
                if mag == f64::NEG_INFINITY {
                    mag = Self::magnitude_of(self.x, self.y, self.z);
                    cache.store(mag.to_bits(), Ordering::Relaxed);
                }
                mag
            }
        }
    }

    /// `isNumericallyIdentical(double, double, double)`.
    pub fn is_numerically_identical_xyz(&self, other_x: f64, other_y: f64, other_z: f64) -> bool {
        let delta_x = self.x - other_x;
        let delta_y = self.y - other_y;
        let delta_z = self.z - other_z;
        delta_x * delta_x + delta_y * delta_y + delta_z * delta_z
            < Self::MINIMUM_RESOLUTION_SQUARED
    }

    /// `isNumericallyIdentical(Vector)`.
    pub fn is_numerically_identical(&self, other: &Vector) -> bool {
        let delta_x = self.x - other.x;
        let delta_y = self.y - other.y;
        let delta_z = self.z - other.z;
        delta_x * delta_x + delta_y * delta_y + delta_z * delta_z
            < Self::MINIMUM_RESOLUTION_SQUARED
    }

    /// `isParallel(double, double, double)`.
    pub fn is_parallel_xyz(&self, other_x: f64, other_y: f64, other_z: f64) -> bool {
        let this_x = self.y * other_z - self.z * other_y;
        let this_y = self.z * other_x - self.x * other_z;
        let this_z = self.x * other_y - self.y * other_x;
        this_x * this_x + this_y * this_y + this_z * this_z < Self::MINIMUM_RESOLUTION_SQUARED
    }

    /// `isParallel(Vector)`.
    pub fn is_parallel(&self, other: &Vector) -> bool {
        let this_x = self.y * other.z - self.z * other.y;
        let this_y = self.z * other.x - self.x * other.z;
        let this_z = self.x * other.y - self.y * other.x;
        this_x * this_x + this_y * this_y + this_z * this_z < Self::MINIMUM_RESOLUTION_SQUARED
    }

    /// `computeDesiredEllipsoidMagnitude(PlanetModel, double, double, double)`.
    pub(crate) fn compute_desired_ellipsoid_magnitude(
        planet_model: &PlanetModel,
        x: f64,
        y: f64,
        z: f64,
    ) -> f64 {
        1.0 / (x * x * planet_model.inverse_xy_scaling_squared
            + y * y * planet_model.inverse_xy_scaling_squared
            + z * z * planet_model.inverse_z_scaling_squared)
            .sqrt()
    }

    /// `computeDesiredEllipsoidMagnitude(PlanetModel, double)`.
    pub(crate) fn compute_desired_ellipsoid_magnitude_z(planet_model: &PlanetModel, z: f64) -> f64 {
        1.0 / ((1.0 - z * z) * planet_model.inverse_xy_scaling_squared
            + z * z * planet_model.inverse_z_scaling_squared)
            .sqrt()
    }

    /// `Vector.equals(Object)` against another vector.
    pub fn java_equals(&self, other: &Vector) -> bool {
        other.x == self.x && other.y == self.y && other.z == self.z
    }

    /// `Vector.hashCode()`.
    pub fn java_hash_code(&self) -> i32 {
        let mut result = dhash(self.x);
        result = result.wrapping_mul(31).wrapping_add(dhash(self.y));
        result = result.wrapping_mul(31).wrapping_add(dhash(self.z));
        result
    }
}

impl PartialEq for Vector {
    fn eq(&self, other: &Self) -> bool {
        self.java_equals(other)
    }
}

impl Eq for Vector {}

impl Hash for Vector {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.java_hash_code());
    }
}

impl fmt::Display for Vector {
    /// `Vector.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[X={}, Y={}, Z={}]",
            d2s(self.x),
            d2s(self.y),
            d2s(self.z)
        )
    }
}
