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

//! The types declared in `R1Interval.java`, of the Java package
//! `com.google.common.geometry`.

use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::util::java_fmt::double_to_string;

use crate::s2::double_to_long_bits;

/// Equivalent to `com.google.common.geometry.R1Interval` (`public final
/// strictfp class`): a closed interval on the real line.
#[derive(Clone, Copy, Debug)]
pub struct R1Interval {
    lo: f64,
    hi: f64,
}

impl R1Interval {
    /// `R1Interval(double lo, double hi)`.
    pub fn new(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }

    /// `empty()`.
    pub fn empty() -> Self {
        Self::new(1.0, 0.0)
    }

    /// `fromPoint(double)`.
    pub fn from_point(p: f64) -> Self {
        Self::new(p, p)
    }

    /// `fromPointPair(double, double)`.
    pub fn from_point_pair(p1: f64, p2: f64) -> Self {
        if p1 <= p2 {
            Self::new(p1, p2)
        } else {
            Self::new(p2, p1)
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

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.lo() > self.hi()
    }

    /// `getCenter()`.
    pub fn get_center(&self) -> f64 {
        0.5 * (self.lo() + self.hi())
    }

    /// `getLength()`.
    pub fn get_length(&self) -> f64 {
        self.hi() - self.lo()
    }

    /// `contains(double)`.
    pub fn contains_point(&self, p: f64) -> bool {
        p >= self.lo() && p <= self.hi()
    }

    /// `interiorContains(double)`.
    pub fn interior_contains_point(&self, p: f64) -> bool {
        p > self.lo() && p < self.hi()
    }

    /// `contains(R1Interval)`.
    pub fn contains(&self, y: &R1Interval) -> bool {
        if y.is_empty() {
            return true;
        }
        y.lo() >= self.lo() && y.hi() <= self.hi()
    }

    /// `interiorContains(R1Interval)`.
    pub fn interior_contains(&self, y: &R1Interval) -> bool {
        if y.is_empty() {
            return true;
        }
        y.lo() > self.lo() && y.hi() < self.hi()
    }

    /// `intersects(R1Interval)`.
    pub fn intersects(&self, y: &R1Interval) -> bool {
        if self.lo() <= y.lo() {
            y.lo() <= self.hi() && y.lo() <= y.hi()
        } else {
            self.lo() <= y.hi() && self.lo() <= self.hi()
        }
    }

    /// `interiorIntersects(R1Interval)`.
    pub fn interior_intersects(&self, y: &R1Interval) -> bool {
        y.lo() < self.hi() && self.lo() < y.hi() && self.lo() < self.hi() && y.lo() <= y.hi()
    }

    /// `addPoint(double)`.
    pub fn add_point(&self, p: f64) -> R1Interval {
        if self.is_empty() {
            R1Interval::from_point(p)
        } else if p < self.lo() {
            R1Interval::new(p, self.hi())
        } else if p > self.hi() {
            R1Interval::new(self.lo(), p)
        } else {
            R1Interval::new(self.lo(), self.hi())
        }
    }

    /// `expanded(double)`.
    pub fn expanded(&self, radius: f64) -> R1Interval {
        if self.is_empty() {
            return *self;
        }
        R1Interval::new(self.lo() - radius, self.hi() + radius)
    }

    /// `union(R1Interval)`.
    pub fn union(&self, y: &R1Interval) -> R1Interval {
        if self.is_empty() {
            return *y;
        }
        if y.is_empty() {
            return *self;
        }
        R1Interval::new(self.lo().min(y.lo()), self.hi().max(y.hi()))
    }

    /// `intersection(R1Interval)`.
    pub fn intersection(&self, y: &R1Interval) -> R1Interval {
        R1Interval::new(self.lo().max(y.lo()), self.hi().min(y.hi()))
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        if self.is_empty() {
            return 17;
        }
        let mut value: i64 = 17;
        value = 37i64
            .wrapping_mul(value)
            .wrapping_add(double_to_long_bits(self.lo));
        value = 37i64
            .wrapping_mul(value)
            .wrapping_add(double_to_long_bits(self.hi));
        (value ^ ((value as u64) >> 32) as i64) as i32
    }

    /// `approxEquals(R1Interval)`.
    pub fn approx_equals(&self, y: &R1Interval) -> bool {
        self.approx_equals_with(y, 1e-15)
    }

    /// `approxEquals(R1Interval, double)`.
    pub fn approx_equals_with(&self, y: &R1Interval, max_error: f64) -> bool {
        if self.is_empty() {
            return y.get_length() <= max_error;
        }
        if y.is_empty() {
            return self.get_length() <= max_error;
        }
        (y.lo() - self.lo()).abs() + (y.hi() - self.hi()).abs() <= max_error
    }
}

impl PartialEq for R1Interval {
    /// `equals(Object)`.
    fn eq(&self, y: &Self) -> bool {
        (self.lo() == y.lo() && self.hi() == y.hi()) || (self.is_empty() && y.is_empty())
    }
}

impl Hash for R1Interval {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for R1Interval {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}, {}]",
            double_to_string(self.lo()),
            double_to_string(self.hi())
        )
    }
}
