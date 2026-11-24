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

//! The types declared in `S1Angle.java`, of the Java package
//! `com.google.common.geometry`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::util::java_fmt::double_to_string;

use crate::s2::{double_to_long_bits, java_round};
use crate::s2_point::S2Point;

/// Equivalent to `com.google.common.geometry.S1Angle` (`public final strictfp
/// class S1Angle implements Comparable<S1Angle>`): an angle in radians.
#[derive(Clone, Copy, Debug, Default)]
pub struct S1Angle {
    radians: f64,
}

impl S1Angle {
    /// `S1Angle()`: a zero angle.
    pub fn new() -> Self {
        Self { radians: 0.0 }
    }

    /// `S1Angle(S2Point x, S2Point y)`: the angle between two points.
    pub fn between(x: &S2Point, y: &S2Point) -> Self {
        Self {
            radians: x.angle(y),
        }
    }

    /// `radians()`.
    pub fn radians(&self) -> f64 {
        self.radians
    }

    /// `degrees()`.
    pub fn degrees(&self) -> f64 {
        self.radians * (180.0 / std::f64::consts::PI)
    }

    /// `e5()`.
    pub fn e5(&self) -> i64 {
        java_round(self.degrees() * 1e5)
    }

    /// `e6()`.
    pub fn e6(&self) -> i64 {
        java_round(self.degrees() * 1e6)
    }

    /// `e7()`.
    pub fn e7(&self) -> i64 {
        java_round(self.degrees() * 1e7)
    }

    /// `lessThan(S1Angle)`.
    pub fn less_than(&self, that: &S1Angle) -> bool {
        self.radians() < that.radians()
    }

    /// `greaterThan(S1Angle)`.
    pub fn greater_than(&self, that: &S1Angle) -> bool {
        self.radians() > that.radians()
    }

    /// `lessOrEquals(S1Angle)`.
    pub fn less_or_equals(&self, that: &S1Angle) -> bool {
        self.radians() <= that.radians()
    }

    /// `greaterOrEquals(S1Angle)`.
    pub fn greater_or_equals(&self, that: &S1Angle) -> bool {
        self.radians() >= that.radians()
    }

    /// `max(S1Angle, S1Angle)`.
    pub fn max(left: S1Angle, right: S1Angle) -> S1Angle {
        if right.greater_than(&left) {
            right
        } else {
            left
        }
    }

    /// `min(S1Angle, S1Angle)`.
    pub fn min(left: S1Angle, right: S1Angle) -> S1Angle {
        if right.greater_than(&left) {
            left
        } else {
            right
        }
    }

    /// `radians(double)`.
    pub const fn from_radians(radians: f64) -> S1Angle {
        S1Angle { radians }
    }

    /// `degrees(double)`.
    pub fn from_degrees(degrees: f64) -> S1Angle {
        S1Angle::from_radians(degrees * (std::f64::consts::PI / 180.0))
    }

    /// `e5(long)`.
    pub fn from_e5(e5: i64) -> S1Angle {
        Self::from_degrees(e5 as f64 * 1e-5)
    }

    /// `e6(long)`.
    pub fn from_e6(e6: i64) -> S1Angle {
        // Multiplying by 1e-6 isn't quite as accurate as dividing by 1e6,
        // but it's about 10 times faster and more than accurate enough.
        Self::from_degrees(e6 as f64 * 1e-6)
    }

    /// `e7(long)`.
    pub fn from_e7(e7: i64) -> S1Angle {
        Self::from_degrees(e7 as f64 * 1e-7)
    }

    /// `compareTo(S1Angle)`.
    pub fn compare_to(&self, that: &S1Angle) -> i32 {
        if self.radians < that.radians {
            -1
        } else if self.radians > that.radians {
            1
        } else {
            0
        }
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let value = double_to_long_bits(self.radians);
        (value ^ ((value as u64) >> 32) as i64) as i32
    }
}

impl PartialEq for S1Angle {
    /// `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.radians() == other.radians()
    }
}

impl PartialOrd for S1Angle {
    /// `compareTo(S1Angle)`.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.compare_to(other).cmp(&0))
    }
}

impl Hash for S1Angle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for S1Angle {
    /// `toString()`: the angle in degrees with a `d` suffix.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}d", double_to_string(self.degrees()))
    }
}
