/*
 * Copyright 2005 Google Inc.
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

//! The types declared in `S2Cap.java`, of the Java package
//! `com.google.common.geometry`.

use rucene::util::java_math;
use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::util::java_fmt::double_to_string;

use crate::r1_interval::R1Interval;
use crate::s1_angle::S1Angle;
use crate::s1_interval::S1Interval;
use crate::s2::{double_to_long_bits, S2};
use crate::s2_cell::S2Cell;
use crate::s2_lat_lng::S2LatLng;
use crate::s2_lat_lng_rect::S2LatLngRect;
use crate::s2_point::S2Point;
use crate::s2_region::S2Region;

/// Equivalent to `com.google.common.geometry.S2Cap` (`public final strictfp
/// class S2Cap implements S2Region`): a disc-shaped region bounded by a plane,
/// an axis plus a height.
#[derive(Clone, Copy, Debug)]
pub struct S2Cap {
    axis: S2Point,
    height: f64,
}

impl S2Cap {
    const ROUND_UP: f64 = 1.0 + 1.0 / (1u64 << 52) as f64;

    fn new(axis: S2Point, height: f64) -> Self {
        Self { axis, height }
    }

    /// `fromAxisHeight(S2Point, double)`.
    pub fn from_axis_height(axis: S2Point, height: f64) -> S2Cap {
        S2Cap::new(axis, height)
    }

    /// `fromAxisAngle(S2Point, S1Angle)`.
    pub fn from_axis_angle(axis: S2Point, angle: &S1Angle) -> S2Cap {
        let d = java_math::sin(0.5 * angle.radians());
        S2Cap::new(axis, 2.0 * d * d)
    }

    /// `fromAxisArea(S2Point, double)`.
    pub fn from_axis_area(axis: S2Point, area: f64) -> S2Cap {
        S2Cap::new(axis, area / (2.0 * S2::M_PI))
    }

    /// `empty()`.
    pub fn empty() -> S2Cap {
        S2Cap::new(S2Point::new(1.0, 0.0, 0.0), -1.0)
    }

    /// `full()`.
    pub fn full() -> S2Cap {
        S2Cap::new(S2Point::new(1.0, 0.0, 0.0), 2.0)
    }

    /// `axis()`.
    pub fn axis(&self) -> S2Point {
        self.axis
    }

    /// `height()`.
    pub fn height(&self) -> f64 {
        self.height
    }

    /// `area()`.
    pub fn area(&self) -> f64 {
        2.0 * S2::M_PI * 0.0f64.max(self.height)
    }

    /// `angle()`.
    pub fn angle(&self) -> S1Angle {
        if self.is_empty() {
            return S1Angle::from_radians(-1.0);
        }
        S1Angle::from_radians(2.0 * java_math::asin(java_math::sqrt(0.5 * self.height)))
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        S2::is_unit_length(&self.axis) && self.height <= 2.0
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.height < 0.0
    }

    /// `isFull()`.
    pub fn is_full(&self) -> bool {
        self.height >= 2.0
    }

    /// `complement()`.
    pub fn complement(&self) -> S2Cap {
        let c_height = if self.is_full() {
            -1.0
        } else {
            2.0 - self.height.max(0.0)
        };
        S2Cap::from_axis_height(S2Point::neg(&self.axis), c_height)
    }

    /// `contains(S2Cap)`.
    pub fn contains_cap(&self, other: &S2Cap) -> bool {
        if self.is_full() || other.is_empty() {
            return true;
        }
        self.angle().radians() >= self.axis.angle(&other.axis) + other.angle().radians()
    }

    /// `interiorIntersects(S2Cap)`.
    pub fn interior_intersects(&self, other: &S2Cap) -> bool {
        !self.complement().contains_cap(other)
    }

    /// `interiorContains(S2Point)`.
    pub fn interior_contains(&self, p: &S2Point) -> bool {
        self.is_full() || S2Point::sub(&self.axis, p).norm2() < 2.0 * self.height
    }

    /// `addPoint(S2Point)`.
    pub fn add_point(&self, p: &S2Point) -> S2Cap {
        if self.is_empty() {
            S2Cap::new(*p, 0.0)
        } else {
            let dist2 = S2Point::sub(&self.axis, p).norm2();
            let new_height = self.height.max(Self::ROUND_UP * 0.5 * dist2);
            S2Cap::new(self.axis, new_height)
        }
    }

    /// `addCap(S2Cap)`.
    pub fn add_cap(&self, other: &S2Cap) -> S2Cap {
        if self.is_empty() {
            S2Cap::new(other.axis, other.height)
        } else {
            let angle = self.axis.angle(&other.axis) + other.angle().radians();
            if angle >= S2::M_PI {
                S2Cap::new(self.axis, 2.0) // Full cap
            } else {
                let d = java_math::sin(0.5 * angle);
                let new_height = self.height.max(Self::ROUND_UP * 2.0 * d * d);
                S2Cap::new(self.axis, new_height)
            }
        }
    }

    /// `intersects(S2Cell, S2Point[] vertices)`.
    pub fn intersects(&self, cell: &S2Cell, vertices: &[S2Point; 4]) -> bool {
        if self.height >= 1.0 {
            return false;
        }
        if self.is_empty() {
            return false;
        }
        if cell.contains_point(&self.axis) {
            return true;
        }
        let sin2_angle = self.height * (2.0 - self.height); // sin^2(capAngle)
        for k in 0..4 {
            let edge = cell.get_edge_raw(k);
            let dot = self.axis.dot_prod(&edge);
            if dot > 0.0 {
                continue;
            }
            if dot * dot > sin2_angle * edge.norm2() {
                return false; // Entire cap is on the exterior side of this edge.
            }
            let dir = S2Point::cross_prod(&edge, &self.axis);
            if dir.dot_prod(&vertices[k as usize]) < 0.0
                && dir.dot_prod(&vertices[((k + 1) & 3) as usize]) > 0.0
            {
                return true;
            }
        }
        false
    }

    /// `contains(S2Point)`.
    pub fn contains_point(&self, p: &S2Point) -> bool {
        S2Point::sub(&self.axis, p).norm2() <= 2.0 * self.height
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        if self.is_full() {
            return 17;
        } else if self.is_empty() {
            return 37;
        }
        let mut result: i32 = 17;
        result = 37i32
            .wrapping_mul(result)
            .wrapping_add(self.axis.hash_code());
        let height_bits = double_to_long_bits(self.height);
        result = 37i32
            .wrapping_mul(result)
            .wrapping_add((((height_bits as u64) >> 32) as i64 ^ height_bits) as i32);
        result
    }

    /// `approxEquals(S2Cap, double)` (package-private).
    #[allow(dead_code)] // Package-private in Java; no caller in this subset.
    pub(crate) fn approx_equals_with(&self, other: &S2Cap, max_error: f64) -> bool {
        (self.axis.aequal(&other.axis, max_error)
            && (self.height - other.height).abs() <= max_error)
            || (self.is_empty() && other.height <= max_error)
            || (other.is_empty() && self.height <= max_error)
            || (self.is_full() && other.height >= 2.0 - max_error)
            || (other.is_full() && self.height >= 2.0 - max_error)
    }

    /// `approxEquals(S2Cap)` (package-private).
    #[allow(dead_code)] // Package-private in Java; no caller in this subset.
    pub(crate) fn approx_equals(&self, other: &S2Cap) -> bool {
        self.approx_equals_with(other, 1e-14)
    }
}

impl S2Region for S2Cap {
    /// `getCapBound()`.
    fn get_cap_bound(&self) -> S2Cap {
        *self
    }

    /// `getRectBound()`.
    fn get_rect_bound(&self) -> S2LatLngRect {
        if self.is_empty() {
            return S2LatLngRect::empty();
        }
        let axis_lat_lng = S2LatLng::from_point(&self.axis);
        let cap_angle = self.angle().radians();
        let mut all_longitudes = false;
        let mut lat = [0.0f64; 2];
        let mut lng = [-S2::M_PI, S2::M_PI];
        lat[0] = axis_lat_lng.lat().radians() - cap_angle;
        if lat[0] <= -S2::M_PI_2 {
            lat[0] = -S2::M_PI_2;
            all_longitudes = true;
        }
        lat[1] = axis_lat_lng.lat().radians() + cap_angle;
        if lat[1] >= S2::M_PI_2 {
            lat[1] = S2::M_PI_2;
            all_longitudes = true;
        }
        if !all_longitudes {
            let sin_a = java_math::sqrt(self.height * (2.0 - self.height));
            let sin_c = java_math::cos(axis_lat_lng.lat().radians());
            if sin_a <= sin_c {
                let angle_a = java_math::asin(sin_a / sin_c);
                lng[0] = java_math::ieee_remainder(
                    axis_lat_lng.lng().radians() - angle_a,
                    2.0 * S2::M_PI,
                );
                lng[1] = java_math::ieee_remainder(
                    axis_lat_lng.lng().radians() + angle_a,
                    2.0 * S2::M_PI,
                );
            }
        }
        S2LatLngRect::new(
            R1Interval::new(lat[0], lat[1]),
            S1Interval::new(lng[0], lng[1]),
        )
    }

    /// `contains(S2Cell)`.
    fn contains_cell(&self, cell: &S2Cell) -> bool {
        let mut vertices = [S2Point::default(); 4];
        for (k, vertex) in vertices.iter_mut().enumerate() {
            *vertex = cell.get_vertex(k as i32);
            if !self.contains_point(vertex) {
                return false;
            }
        }
        !self.complement().intersects(cell, &vertices)
    }

    /// `mayIntersect(S2Cell)`.
    fn may_intersect(&self, cell: &S2Cell) -> bool {
        let mut vertices = [S2Point::default(); 4];
        for (k, vertex) in vertices.iter_mut().enumerate() {
            *vertex = cell.get_vertex(k as i32);
            if self.contains_point(vertex) {
                return true;
            }
        }
        self.intersects(cell, &vertices)
    }
}

impl PartialEq for S2Cap {
    /// `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        (self.axis == other.axis && self.height == other.height)
            || (self.is_empty() && other.is_empty())
            || (self.is_full() && other.is_full())
    }
}

impl Hash for S2Cap {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for S2Cap {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[Point = {} Height = {}]",
            self.axis,
            double_to_string(self.height)
        )
    }
}
