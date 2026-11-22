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

//! The types declared in `S1Interval.java`, of the Java package
//! `com.google.common.geometry`.

use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::util::java_fmt::double_to_string;

use crate::s2::{double_to_long_bits, ieee_remainder, S2};

/// Equivalent to `com.google.common.geometry.S1Interval` (`public final
/// strictfp class S1Interval implements Cloneable`): a closed interval on the
/// unit circle.
#[derive(Clone, Copy, Debug)]
pub struct S1Interval {
    lo: f64,
    hi: f64,
}

impl S1Interval {
    /// `S1Interval(double lo, double hi)`: `-PI` is normalised to `PI`.
    pub fn new(lo: f64, hi: f64) -> Self {
        Self::new_checked(lo, hi, false)
    }

    /// `S1Interval(S1Interval)`: a copy.
    pub fn copy_of(interval: &S1Interval) -> Self {
        Self { lo: interval.lo, hi: interval.hi }
    }

    fn new_checked(lo: f64, hi: f64, checked: bool) -> Self {
        let mut new_lo = lo;
        let mut new_hi = hi;
        if !checked {
            if lo == -S2::M_PI && hi != S2::M_PI {
                new_lo = S2::M_PI;
            }
            if hi == -S2::M_PI && lo != S2::M_PI {
                new_hi = S2::M_PI;
            }
        }
        Self { lo: new_lo, hi: new_hi }
    }

    /// `empty()`.
    pub fn empty() -> Self {
        Self::new_checked(S2::M_PI, -S2::M_PI, true)
    }

    /// `full()`.
    pub fn full() -> Self {
        Self::new_checked(-S2::M_PI, S2::M_PI, true)
    }

    /// `fromPoint(double)`.
    pub fn from_point(mut p: f64) -> Self {
        if p == -S2::M_PI {
            p = S2::M_PI;
        }
        Self::new_checked(p, p, true)
    }

    /// `fromPointPair(double, double)`.
    pub fn from_point_pair(mut p1: f64, mut p2: f64) -> Self {
        if p1 == -S2::M_PI {
            p1 = S2::M_PI;
        }
        if p2 == -S2::M_PI {
            p2 = S2::M_PI;
        }
        if Self::positive_distance(p1, p2) <= S2::M_PI {
            Self::new_checked(p1, p2, true)
        } else {
            Self::new_checked(p2, p1, true)
        }
    }

    /// `lo()`.
    pub fn lo(&self) -> f64 {
        self.lo
    }

    /// `hi()`.
    pub fn hi(&self) -> f64 {
        self.hi
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        self.lo().abs() <= S2::M_PI
            && self.hi().abs() <= S2::M_PI
            && !(self.lo() == -S2::M_PI && self.hi() != S2::M_PI)
            && !(self.hi() == -S2::M_PI && self.lo() != S2::M_PI)
    }

    /// `isFull()`.
    pub fn is_full(&self) -> bool {
        self.hi() - self.lo() == 2.0 * S2::M_PI
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.lo() - self.hi() == 2.0 * S2::M_PI
    }

    /// `isInverted()`.
    pub fn is_inverted(&self) -> bool {
        self.lo() > self.hi()
    }

    /// `getCenter()`.
    pub fn get_center(&self) -> f64 {
        let center = 0.5 * (self.lo() + self.hi());
        if !self.is_inverted() {
            return center;
        }
        if center <= 0.0 {
            center + S2::M_PI
        } else {
            center - S2::M_PI
        }
    }

    /// `getLength()`.
    pub fn get_length(&self) -> f64 {
        let mut length = self.hi() - self.lo();
        if length >= 0.0 {
            return length;
        }
        length += 2.0 * S2::M_PI;
        if length > 0.0 {
            length
        } else {
            -1.0
        }
    }

    /// `complement()`.
    pub fn complement(&self) -> S1Interval {
        if self.lo() == self.hi() {
            return Self::full(); // Singleton.
        }
        Self::new_checked(self.hi(), self.lo(), true)
    }

    /// `contains(double)`.
    pub fn contains_point(&self, mut p: f64) -> bool {
        if p == -S2::M_PI {
            p = S2::M_PI;
        }
        self.fast_contains(p)
    }

    /// `fastContains(double)`.
    pub fn fast_contains(&self, p: f64) -> bool {
        if self.is_inverted() {
            (p >= self.lo() || p <= self.hi()) && !self.is_empty()
        } else {
            p >= self.lo() && p <= self.hi()
        }
    }

    /// `interiorContains(double)`.
    pub fn interior_contains_point(&self, mut p: f64) -> bool {
        if p == -S2::M_PI {
            p = S2::M_PI;
        }
        if self.is_inverted() {
            p > self.lo() || p < self.hi()
        } else {
            (p > self.lo() && p < self.hi()) || self.is_full()
        }
    }

    /// `contains(S1Interval)`.
    pub fn contains(&self, y: &S1Interval) -> bool {
        if self.is_inverted() {
            if y.is_inverted() {
                return y.lo() >= self.lo() && y.hi() <= self.hi();
            }
            (y.lo() >= self.lo() || y.hi() <= self.hi()) && !self.is_empty()
        } else {
            if y.is_inverted() {
                return self.is_full() || y.is_empty();
            }
            y.lo() >= self.lo() && y.hi() <= self.hi()
        }
    }

    /// `interiorContains(S1Interval)`.
    pub fn interior_contains(&self, y: &S1Interval) -> bool {
        if self.is_inverted() {
            if !y.is_inverted() {
                return y.lo() > self.lo() || y.hi() < self.hi();
            }
            (y.lo() > self.lo() && y.hi() < self.hi()) || y.is_empty()
        } else {
            if y.is_inverted() {
                return self.is_full() || y.is_empty();
            }
            (y.lo() > self.lo() && y.hi() < self.hi()) || self.is_full()
        }
    }

    /// `intersects(S1Interval)`.
    pub fn intersects(&self, y: &S1Interval) -> bool {
        if self.is_empty() || y.is_empty() {
            return false;
        }
        if self.is_inverted() {
            y.is_inverted() || y.lo() <= self.hi() || y.hi() >= self.lo()
        } else {
            if y.is_inverted() {
                return y.lo() <= self.hi() || y.hi() >= self.lo();
            }
            y.lo() <= self.hi() && y.hi() >= self.lo()
        }
    }

    /// `interiorIntersects(S1Interval)`.
    pub fn interior_intersects(&self, y: &S1Interval) -> bool {
        if self.is_empty() || y.is_empty() || self.lo() == self.hi() {
            return false;
        }
        if self.is_inverted() {
            y.is_inverted() || y.lo() < self.hi() || y.hi() > self.lo()
        } else {
            if y.is_inverted() {
                return y.lo() < self.hi() || y.hi() > self.lo();
            }
            (y.lo() < self.hi() && y.hi() > self.lo()) || self.is_full()
        }
    }

    /// `addPoint(double)`.
    pub fn add_point(&self, mut p: f64) -> S1Interval {
        if p == -S2::M_PI {
            p = S2::M_PI;
        }
        if self.fast_contains(p) {
            return Self::copy_of(self);
        }
        if self.is_empty() {
            S1Interval::from_point(p)
        } else {
            let dlo = Self::positive_distance(p, self.lo());
            let dhi = Self::positive_distance(self.hi(), p);
            if dlo < dhi {
                S1Interval::new(p, self.hi())
            } else {
                S1Interval::new(self.lo(), p)
            }
        }
    }

    /// `expanded(double)`.
    pub fn expanded(&self, radius: f64) -> S1Interval {
        if self.is_empty() {
            return *self;
        }
        if self.get_length() + 2.0 * radius >= 2.0 * S2::M_PI - 1e-15 {
            return Self::full();
        }
        let mut low = ieee_remainder(self.lo() - radius, 2.0 * S2::M_PI);
        let high = ieee_remainder(self.hi() + radius, 2.0 * S2::M_PI);
        if low == -S2::M_PI {
            low = S2::M_PI;
        }
        S1Interval::new(low, high)
    }

    /// `union(S1Interval)`.
    pub fn union(&self, y: &S1Interval) -> S1Interval {
        if y.is_empty() {
            return *self;
        }
        if self.fast_contains(y.lo()) {
            if self.fast_contains(y.hi()) {
                if self.contains(y) {
                    return *self; // is_full() code path
                }
                return Self::full();
            }
            return Self::new_checked(self.lo(), y.hi(), true);
        }
        if self.fast_contains(y.hi()) {
            return Self::new_checked(y.lo(), self.hi(), true);
        }
        if self.is_empty() || y.fast_contains(self.lo()) {
            return *y;
        }
        let dlo = Self::positive_distance(y.hi(), self.lo());
        let dhi = Self::positive_distance(self.hi(), y.lo());
        if dlo < dhi {
            Self::new_checked(y.lo(), self.hi(), true)
        } else {
            Self::new_checked(self.lo(), y.hi(), true)
        }
    }

    /// `intersection(S1Interval)`.
    pub fn intersection(&self, y: &S1Interval) -> S1Interval {
        if y.is_empty() {
            return Self::empty();
        }
        if self.fast_contains(y.lo()) {
            if self.fast_contains(y.hi()) {
                if y.get_length() < self.get_length() {
                    return *y; // is_full() code path
                }
                return *self;
            }
            return Self::new_checked(y.lo(), self.hi(), true);
        }
        if self.fast_contains(y.hi()) {
            return Self::new_checked(self.lo(), y.hi(), true);
        }
        if y.fast_contains(self.lo()) {
            return *self; // is_empty() okay here
        }
        Self::empty()
    }

    /// `approxEquals(S1Interval, double)`.
    pub fn approx_equals_with(&self, y: &S1Interval, max_error: f64) -> bool {
        if self.is_empty() {
            return y.get_length() <= max_error;
        }
        if y.is_empty() {
            return self.get_length() <= max_error;
        }
        (ieee_remainder(y.lo() - self.lo(), 2.0 * S2::M_PI).abs()
            + ieee_remainder(y.hi() - self.hi(), 2.0 * S2::M_PI).abs())
            <= max_error
    }

    /// `approxEquals(S1Interval)`.
    pub fn approx_equals(&self, y: &S1Interval) -> bool {
        self.approx_equals_with(y, 1e-9)
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut value: i64 = 17;
        value = 37i64.wrapping_mul(value).wrapping_add(double_to_long_bits(self.lo()));
        value = 37i64.wrapping_mul(value).wrapping_add(double_to_long_bits(self.hi()));
        (((value as u64) >> 32) as i64 ^ value) as i32
    }

    /// `positiveDistance(double, double)`.
    pub fn positive_distance(a: f64, b: f64) -> f64 {
        let d = b - a;
        if d >= 0.0 {
            return d;
        }
        (b + S2::M_PI) - (a - S2::M_PI)
    }
}

impl PartialEq for S1Interval {
    /// `equals(Object)`.
    fn eq(&self, that: &Self) -> bool {
        self.lo() == that.lo() && self.hi() == that.hi()
    }
}

impl Hash for S1Interval {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for S1Interval {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}, {}]", double_to_string(self.lo()), double_to_string(self.hi()))
    }
}
