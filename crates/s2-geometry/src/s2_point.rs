/*
 * Copyright 2006 Google Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The types declared in `S2Point.java`, of the Java package
//! `com.google.common.geometry`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::util::java_fmt::double_to_string;

use crate::s2::double_to_long_bits;
use crate::s2_lat_lng::S2LatLng;

/// Equivalent to `com.google.common.geometry.S2Point` (`public strictfp class
/// S2Point implements Comparable<S2Point>`): a point on the unit sphere as a
/// 3D vector.
#[derive(Clone, Copy, Debug, Default)]
pub struct S2Point {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
}

impl S2Point {
    /// `S2Point(double x, double y, double z)`.
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// The `x` field (package-private in Java).
    pub fn x(&self) -> f64 {
        self.x
    }

    /// The `y` field (package-private in Java).
    pub fn y(&self) -> f64 {
        self.y
    }

    /// The `z` field (package-private in Java).
    pub fn z(&self) -> f64 {
        self.z
    }

    /// `minus(S2Point, S2Point)`.
    pub fn minus(p1: &S2Point, p2: &S2Point) -> S2Point {
        Self::sub(p1, p2)
    }

    /// `neg(S2Point)`.
    pub fn neg(p: &S2Point) -> S2Point {
        S2Point::new(-p.x, -p.y, -p.z)
    }

    /// `norm2()`.
    pub fn norm2(&self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    /// `norm()`.
    pub fn norm(&self) -> f64 {
        self.norm2().sqrt()
    }

    /// `crossProd(S2Point, S2Point)`.
    pub fn cross_prod(p1: &S2Point, p2: &S2Point) -> S2Point {
        S2Point::new(
            p1.y * p2.z - p1.z * p2.y,
            p1.z * p2.x - p1.x * p2.z,
            p1.x * p2.y - p1.y * p2.x,
        )
    }

    /// `add(S2Point, S2Point)`.
    pub fn add(p1: &S2Point, p2: &S2Point) -> S2Point {
        S2Point::new(p1.x + p2.x, p1.y + p2.y, p1.z + p2.z)
    }

    /// `sub(S2Point, S2Point)`.
    pub fn sub(p1: &S2Point, p2: &S2Point) -> S2Point {
        S2Point::new(p1.x - p2.x, p1.y - p2.y, p1.z - p2.z)
    }

    /// `dotProd(S2Point)`.
    pub fn dot_prod(&self, that: &S2Point) -> f64 {
        self.x * that.x + self.y * that.y + self.z * that.z
    }

    /// `mul(S2Point, double)`.
    pub fn mul(p: &S2Point, m: f64) -> S2Point {
        S2Point::new(m * p.x, m * p.y, m * p.z)
    }

    /// `div(S2Point, double)`.
    pub fn div(p: &S2Point, m: f64) -> S2Point {
        S2Point::new(p.x / m, p.y / m, p.z / m)
    }

    /// `ortho()`: a vector orthogonal to this one.
    pub fn ortho(&self) -> S2Point {
        let k = self.largest_abs_component();
        let temp = if k == 1 {
            S2Point::new(1.0, 0.0, 0.0)
        } else if k == 2 {
            S2Point::new(0.0, 1.0, 0.0)
        } else {
            S2Point::new(0.0, 0.0, 1.0)
        };
        S2Point::normalize(&Self::cross_prod(self, &temp))
    }

    /// `largestAbsComponent()`.
    pub fn largest_abs_component(&self) -> i32 {
        let temp = Self::fabs(self);
        if temp.x > temp.y {
            if temp.x > temp.z {
                0
            } else {
                2
            }
        } else if temp.y > temp.z {
            1
        } else {
            2
        }
    }

    /// `fabs(S2Point)`.
    pub fn fabs(p: &S2Point) -> S2Point {
        S2Point::new(p.x.abs(), p.y.abs(), p.z.abs())
    }

    /// `normalize(S2Point)`.
    pub fn normalize(p: &S2Point) -> S2Point {
        let mut norm = p.norm();
        if norm != 0.0 {
            norm = 1.0 / norm;
        }
        S2Point::mul(p, norm)
    }

    /// `get(int axis)`.
    pub fn get(&self, axis: i32) -> f64 {
        if axis == 0 {
            self.x
        } else if axis == 1 {
            self.y
        } else {
            self.z
        }
    }

    /// `angle(S2Point)`: the angle between two vectors in radians.
    pub fn angle(&self, va: &S2Point) -> f64 {
        Self::cross_prod(self, va).norm().atan2(self.dot_prod(va))
    }

    /// `aequal(S2Point, double)` (package-private).
    #[allow(dead_code)] // Package-private in Java; no caller in this subset.
    pub(crate) fn aequal(&self, that: &S2Point, margin: f64) -> bool {
        (self.x - that.x).abs() < margin
            && (self.y - that.y).abs() < margin
            && (self.z - that.z).abs() < margin
    }

    /// `lessThan(S2Point)`.
    pub fn less_than(&self, vb: &S2Point) -> bool {
        if self.x < vb.x {
            return true;
        }
        if vb.x < self.x {
            return false;
        }
        if self.y < vb.y {
            return true;
        }
        if vb.y < self.y {
            return false;
        }
        self.z < vb.z
    }

    /// `compareTo(S2Point)`.
    pub fn compare_to(&self, other: &S2Point) -> i32 {
        if self.less_than(other) {
            -1
        } else if self == other {
            0
        } else {
            1
        }
    }

    /// `toDegreesString()`.
    pub fn to_degrees_string(&self) -> String {
        let s2_lat_lng = S2LatLng::from_point(self);
        format!(
            "({}, {})",
            double_to_string(s2_lat_lng.lat_degrees()),
            double_to_string(s2_lat_lng.lng_degrees())
        )
    }

    /// `hashCode()`: ignores the sign of the coordinates.
    pub fn hash_code(&self) -> i32 {
        let mut value: i64 = 17;
        for c in [self.x, self.y, self.z] {
            value = value
                .wrapping_add(37i64.wrapping_mul(value).wrapping_add(double_to_long_bits(c.abs())));
        }
        (value ^ ((value as u64) >> 32) as i64) as i32
    }
}

impl PartialEq for S2Point {
    /// `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y && self.z == other.z
    }
}

impl PartialOrd for S2Point {
    /// `compareTo(S2Point)`.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.compare_to(other).cmp(&0))
    }
}

impl Hash for S2Point {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for S2Point {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "({}, {}, {})",
            double_to_string(self.x),
            double_to_string(self.y),
            double_to_string(self.z)
        )
    }
}
