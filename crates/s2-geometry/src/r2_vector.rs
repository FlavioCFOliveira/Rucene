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

//! The types declared in `R2Vector.java`, of the Java package
//! `com.google.common.geometry`.

use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::error::{LuceneError, Result};
use rucene::util::java_fmt::double_to_string;

use crate::s2::double_to_long_bits;

/// Equivalent to `com.google.common.geometry.R2Vector` (`public final
/// strictfp class`): a vector in two-dimensional space.
#[derive(Clone, Copy, Debug, Default)]
pub struct R2Vector {
    x: f64,
    y: f64,
}

impl R2Vector {
    /// `R2Vector(double x, double y)`.
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// `R2Vector(double[] coord)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when `coord` does not hold exactly 2 values.
    pub fn from_coords(coord: &[f64]) -> Result<Self> {
        if coord.len() != 2 {
            return Err(LuceneError::illegal_state("Points must have exactly 2 coordinates"));
        }
        Ok(Self { x: coord[0], y: coord[1] })
    }

    /// `x()`.
    pub fn x(&self) -> f64 {
        self.x
    }

    /// `y()`.
    pub fn y(&self) -> f64 {
        self.y
    }

    /// `get(int index)`.
    ///
    /// # Errors
    ///
    /// `ArrayIndexOutOfBoundsException` when `index > 1`.
    pub fn get(&self, index: i32) -> Result<f64> {
        if index > 1 {
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Array index out of range: {index}"),
            ));
        }
        Ok(if index == 0 { self.x } else { self.y })
    }

    /// `add(R2Vector, R2Vector)`.
    pub fn add(p1: &R2Vector, p2: &R2Vector) -> R2Vector {
        R2Vector::new(p1.x + p2.x, p1.y + p2.y)
    }

    /// `mul(R2Vector, double)`.
    pub fn mul(p: &R2Vector, m: f64) -> R2Vector {
        R2Vector::new(m * p.x, m * p.y)
    }

    /// `norm2()`.
    pub fn norm2(&self) -> f64 {
        (self.x * self.x) + (self.y * self.y)
    }

    /// `dotProd(R2Vector, R2Vector)`.
    pub fn dot_prod_of(p1: &R2Vector, p2: &R2Vector) -> f64 {
        (p1.x * p2.x) + (p1.y * p2.y)
    }

    /// `dotProd(R2Vector)`.
    pub fn dot_prod(&self, that: &R2Vector) -> f64 {
        Self::dot_prod_of(self, that)
    }

    /// `crossProd(R2Vector)`.
    pub fn cross_prod(&self, that: &R2Vector) -> f64 {
        self.x * that.y - self.y * that.x
    }

    /// `lessThan(R2Vector)`.
    pub fn less_than(&self, vb: &R2Vector) -> bool {
        if self.x < vb.x {
            return true;
        }
        if vb.x < self.x {
            return false;
        }
        self.y < vb.y
    }

    /// `hashCode()`: ignores the sign of the coordinates.
    pub fn hash_code(&self) -> i32 {
        let mut value: i64 = 17;
        value = value
            .wrapping_add(37i64.wrapping_mul(value).wrapping_add(double_to_long_bits(self.x.abs())));
        value = value
            .wrapping_add(37i64.wrapping_mul(value).wrapping_add(double_to_long_bits(self.y.abs())));
        (value ^ ((value as u64) >> 32) as i64) as i32
    }
}

impl PartialEq for R2Vector {
    /// `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y
    }
}

impl Hash for R2Vector {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for R2Vector {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", double_to_string(self.x), double_to_string(self.y))
    }
}
