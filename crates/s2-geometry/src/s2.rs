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

//! The types declared in `S2.java`, of the Java package
//! `com.google.common.geometry`.

use rucene::error::{LuceneError, Result};
use rucene::util::java_math;

use crate::r2_vector::R2Vector;
use crate::s2_cell_id::S2CellId;
use crate::s2_point::S2Point;

/// Equivalent to `com.google.common.geometry.S2` (`public final strictfp
/// class`): constants and static geometric predicates over [`S2Point`]s.
pub struct S2;

impl S2 {
    /// `M_PI`.
    pub const M_PI: f64 = std::f64::consts::PI;
    /// `M_1_PI`, `1.0 / Math.PI`.
    pub const M_1_PI: f64 = 1.0 / std::f64::consts::PI;
    /// `M_PI_2`, `Math.PI / 2.0`.
    pub const M_PI_2: f64 = std::f64::consts::PI / 2.0;
    /// `M_PI_4`, `Math.PI / 4.0`.
    pub const M_PI_4: f64 = std::f64::consts::PI / 4.0;
    /// `M_SQRT2`, `Math.sqrt(2)`.
    pub const M_SQRT2: f64 = std::f64::consts::SQRT_2;
    /// `M_E`, `Math.E`.
    pub const M_E: f64 = std::f64::consts::E;
    /// `SWAP_MASK`.
    pub const SWAP_MASK: i32 = 0x01;
    /// `INVERT_MASK`.
    pub const INVERT_MASK: i32 = 0x02;

    const EXPONENT_SHIFT: i32 = 52;
    const EXPONENT_MASK: i64 = 0x7ff0000000000000;

    const POS_TO_ORIENTATION: [i32; 4] =
        [Self::SWAP_MASK, 0, 0, Self::INVERT_MASK + Self::SWAP_MASK];

    const POS_TO_IJ: [[i32; 4]; 4] = [
        [0, 1, 3, 2], // canonical order: (0,0), (0,1), (1,1), (1,0)
        [0, 2, 3, 1], // axes swapped: (0,0), (1,0), (1,1), (0,1)
        [3, 2, 0, 1], // bits inverted: (1,1), (1,0), (0,0), (0,1)
        [3, 1, 0, 2], // swapped & inverted: (1,1), (0,1), (0,0), (1,0)
    ];

    const IJ_TO_POS: [[i32; 4]; 4] = [
        [0, 1, 3, 2], // canonical order
        [0, 3, 1, 2], // axes swapped
        [2, 3, 1, 0], // bits inverted
        [2, 1, 3, 0], // swapped & inverted
    ];

    /// `exp(double)`: the exponent of `v` in the `frexp` convention (0 for 0).
    pub(crate) fn exp(v: f64) -> i32 {
        if v == 0.0 {
            return 0;
        }
        let bits = v.to_bits() as i64;
        (((Self::EXPONENT_MASK & bits) >> Self::EXPONENT_SHIFT) as i32) - 1022
    }

    /// `posToOrientation(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `position` is outside `[0,3]`.
    pub fn pos_to_orientation(position: i32) -> Result<i32> {
        if !(0..4).contains(&position) {
            return Err(LuceneError::illegal_argument(format!(
                "Position should be in range [0,3] but got {position}"
            )));
        }
        Ok(Self::POS_TO_ORIENTATION[position as usize])
    }

    /// `posToIJ(int, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when an argument is outside `[0,3]`.
    pub fn pos_to_ij(orientation: i32, position: i32) -> Result<i32> {
        if !(0..4).contains(&orientation) {
            return Err(LuceneError::illegal_argument(format!(
                "Orientation should be in range [0,3] but got {orientation}"
            )));
        }
        if !(0..4).contains(&position) {
            return Err(LuceneError::illegal_argument(format!(
                "Position should be in range [0,3] but got {position}"
            )));
        }
        Ok(Self::POS_TO_IJ[orientation as usize][position as usize])
    }

    /// `ijToPos(int, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when an argument is outside `[0,3]`.
    pub fn ij_to_pos(orientation: i32, ij_index: i32) -> Result<i32> {
        if !(0..4).contains(&orientation) {
            return Err(LuceneError::illegal_argument(format!(
                "Orientation should be in range [0,3] but got {orientation}"
            )));
        }
        if !(0..4).contains(&ij_index) {
            return Err(LuceneError::illegal_argument(format!(
                "Index of ij should be in range [0,3] but got {ij_index}"
            )));
        }
        Ok(Self::IJ_TO_POS[orientation as usize][ij_index as usize])
    }

    /// `origin()`.
    pub fn origin() -> S2Point {
        S2Point::new(0.0, 1.0, 0.0)
    }

    /// `isUnitLength(S2Point)`.
    pub fn is_unit_length(p: &S2Point) -> bool {
        (p.norm2() - 1.0).abs() <= 1e-15
    }

    /// `simpleCrossing(S2Point, S2Point, S2Point, S2Point)`.
    pub fn simple_crossing(a: &S2Point, b: &S2Point, c: &S2Point, d: &S2Point) -> bool {
        let ab = S2Point::cross_prod(a, b);
        let cd = S2Point::cross_prod(c, d);
        let acb = -ab.dot_prod(c);
        let cbd = -cd.dot_prod(b);
        let bda = ab.dot_prod(d);
        let dac = cd.dot_prod(a);
        (acb * cbd > 0.0) && (cbd * bda > 0.0) && (bda * dac > 0.0)
    }

    /// `robustCrossProd(S2Point, S2Point)`.
    pub fn robust_cross_prod(a: &S2Point, b: &S2Point) -> S2Point {
        let x = S2Point::cross_prod(&S2Point::add(b, a), &S2Point::sub(b, a));
        if x != S2Point::new(0.0, 0.0, 0.0) {
            return x;
        }
        Self::ortho(a)
    }

    /// `ortho(S2Point)`.
    pub fn ortho(a: &S2Point) -> S2Point {
        a.ortho()
    }

    /// `area(S2Point, S2Point, S2Point)` (package-private).
    pub(crate) fn area(a: &S2Point, b: &S2Point, c: &S2Point) -> f64 {
        let sa = b.angle(c);
        let sb = c.angle(a);
        let sc = a.angle(b);
        let s = 0.5 * (sa + sb + sc);
        if s >= 3e-4 {
            let s2 = s * s;
            let dmin = s - sa.max(sb.max(sc));
            if dmin < 1e-2 * s * s2 * s2 {
                let area = Self::girard_area(a, b, c);
                if dmin < s * (0.1 * area) {
                    return area;
                }
            }
        }
        4.0 * java_math::atan(java_math::sqrt(0.0f64.max(
            java_math::tan(0.5 * s)
                * java_math::tan(0.5 * (s - sa))
                * java_math::tan(0.5 * (s - sb))
                * java_math::tan(0.5 * (s - sc)),
        )))
    }

    /// `girardArea(S2Point, S2Point, S2Point)`.
    pub fn girard_area(a: &S2Point, b: &S2Point, c: &S2Point) -> f64 {
        let ab = S2Point::cross_prod(a, b);
        let bc = S2Point::cross_prod(b, c);
        let ac = S2Point::cross_prod(a, c);
        0.0f64.max(ab.angle(&ac) - ab.angle(&bc) + bc.angle(&ac))
    }

    /// `signedArea(S2Point, S2Point, S2Point)`.
    pub fn signed_area(a: &S2Point, b: &S2Point, c: &S2Point) -> f64 {
        Self::area(a, b, c) * f64::from(Self::robust_ccw(a, b, c))
    }

    /// `planarCentroid(S2Point, S2Point, S2Point)`.
    pub fn planar_centroid(a: &S2Point, b: &S2Point, c: &S2Point) -> S2Point {
        S2Point::new(
            (a.x + b.x + c.x) / 3.0,
            (a.y + b.y + c.y) / 3.0,
            (a.z + b.z + c.z) / 3.0,
        )
    }

    /// `trueCentroid(S2Point, S2Point, S2Point)`.
    pub fn true_centroid(a: &S2Point, b: &S2Point, c: &S2Point) -> S2Point {
        let sina = S2Point::cross_prod(b, c).norm();
        let sinb = S2Point::cross_prod(c, a).norm();
        let sinc = S2Point::cross_prod(a, b).norm();
        let ra = if sina == 0.0 {
            1.0
        } else {
            java_math::asin(sina) / sina
        };
        let rb = if sinb == 0.0 {
            1.0
        } else {
            java_math::asin(sinb) / sinb
        };
        let rc = if sinc == 0.0 {
            1.0
        } else {
            java_math::asin(sinc) / sinc
        };
        let x = S2Point::new(a.x, b.x, c.x);
        let y = S2Point::new(a.y, b.y, c.y);
        let z = S2Point::new(a.z, b.z, c.z);
        let r = S2Point::new(ra, rb, rc);
        S2Point::new(
            0.5 * S2Point::cross_prod(&y, &z).dot_prod(&r),
            0.5 * S2Point::cross_prod(&z, &x).dot_prod(&r),
            0.5 * S2Point::cross_prod(&x, &y).dot_prod(&r),
        )
    }

    /// `simpleCCW(S2Point, S2Point, S2Point)`.
    pub fn simple_ccw(a: &S2Point, b: &S2Point, c: &S2Point) -> bool {
        S2Point::cross_prod(c, a).dot_prod(b) > 0.0
    }

    /// `robustCCW(S2Point, S2Point, S2Point)`.
    pub fn robust_ccw(a: &S2Point, b: &S2Point, c: &S2Point) -> i32 {
        Self::robust_ccw_with_cross(a, b, c, &S2Point::cross_prod(a, b))
    }

    /// `robustCCW(S2Point, S2Point, S2Point, S2Point aCrossB)`.
    pub fn robust_ccw_with_cross(
        a: &S2Point,
        b: &S2Point,
        c: &S2Point,
        a_cross_b: &S2Point,
    ) -> i32 {
        let k_min_abs_value = 1.6e-15; // 2 * 14 * 2**-54
        let det = a_cross_b.dot_prod(c);
        if det > k_min_abs_value {
            return 1;
        }
        if det < -k_min_abs_value {
            return -1;
        }
        Self::expensive_ccw(a, b, c)
    }

    fn expensive_ccw(a: &S2Point, b: &S2Point, c: &S2Point) -> i32 {
        if a == b || b == c || c == a {
            return 0;
        }
        let sab = if a.dot_prod(b) > 0.0 { -1.0 } else { 1.0 };
        let sbc = if b.dot_prod(c) > 0.0 { -1.0 } else { 1.0 };
        let sca = if c.dot_prod(a) > 0.0 { -1.0 } else { 1.0 };
        let vab = S2Point::add(a, &S2Point::mul(b, sab));
        let vbc = S2Point::add(b, &S2Point::mul(c, sbc));
        let vca = S2Point::add(c, &S2Point::mul(a, sca));
        let dab = vab.norm2();
        let dbc = vbc.norm2();
        let dca = vca.norm2();
        let sign = if dca < dbc || (dca == dbc && a.less_than(b)) {
            if dab < dbc || (dab == dbc && a.less_than(c)) {
                S2Point::cross_prod(&vab, &vca).dot_prod(a) * sab // BC is longest
            } else {
                S2Point::cross_prod(&vca, &vbc).dot_prod(c) * sca // AB is longest
            }
        } else if dab < dca || (dab == dca && b.less_than(c)) {
            S2Point::cross_prod(&vbc, &vab).dot_prod(b) * sbc // CA is longest
        } else {
            S2Point::cross_prod(&vca, &vbc).dot_prod(c) * sca // AB is longest
        };
        if sign > 0.0 {
            return 1;
        }
        if sign < 0.0 {
            return -1;
        }
        let mut ccw = Self::planar_ordered_ccw(
            &R2Vector::new(a.y, a.z),
            &R2Vector::new(b.y, b.z),
            &R2Vector::new(c.y, c.z),
        );
        if ccw == 0 {
            ccw = Self::planar_ordered_ccw(
                &R2Vector::new(a.z, a.x),
                &R2Vector::new(b.z, b.x),
                &R2Vector::new(c.z, c.x),
            );
            if ccw == 0 {
                ccw = Self::planar_ordered_ccw(
                    &R2Vector::new(a.x, a.y),
                    &R2Vector::new(b.x, b.y),
                    &R2Vector::new(c.x, c.y),
                );
            }
        }
        ccw
    }

    /// `planarCCW(R2Vector, R2Vector)`.
    pub fn planar_ccw(a: &R2Vector, b: &R2Vector) -> i32 {
        let sab = if a.dot_prod(b) > 0.0 { -1.0 } else { 1.0 };
        let vab = R2Vector::add(a, &R2Vector::mul(b, sab));
        let da = a.norm2();
        let db = b.norm2();
        let sign = if da < db || (da == db && a.less_than(b)) {
            a.cross_prod(&vab) * sab
        } else {
            vab.cross_prod(b)
        };
        if sign > 0.0 {
            return 1;
        }
        if sign < 0.0 {
            return -1;
        }
        0
    }

    /// `planarOrderedCCW(R2Vector, R2Vector, R2Vector)`.
    pub fn planar_ordered_ccw(a: &R2Vector, b: &R2Vector, c: &R2Vector) -> i32 {
        let mut sum = 0;
        sum += Self::planar_ccw(a, b);
        sum += Self::planar_ccw(b, c);
        sum += Self::planar_ccw(c, a);
        if sum > 0 {
            return 1;
        }
        if sum < 0 {
            return -1;
        }
        0
    }

    /// `orderedCCW(S2Point, S2Point, S2Point, S2Point)`.
    pub fn ordered_ccw(a: &S2Point, b: &S2Point, c: &S2Point, o: &S2Point) -> bool {
        let mut sum = 0;
        if Self::robust_ccw(b, o, a) >= 0 {
            sum += 1;
        }
        if Self::robust_ccw(c, o, b) >= 0 {
            sum += 1;
        }
        if Self::robust_ccw(a, o, c) > 0 {
            sum += 1;
        }
        sum >= 2
    }

    /// `angle(S2Point, S2Point, S2Point)`.
    pub fn angle(a: &S2Point, b: &S2Point, c: &S2Point) -> f64 {
        S2Point::cross_prod(a, b).angle(&S2Point::cross_prod(c, b))
    }

    /// `turnAngle(S2Point, S2Point, S2Point)`.
    pub fn turn_angle(a: &S2Point, b: &S2Point, c: &S2Point) -> f64 {
        let out_angle = S2Point::cross_prod(b, a).angle(&S2Point::cross_prod(c, b));
        if Self::robust_ccw(a, b, c) > 0 {
            out_angle
        } else {
            -out_angle
        }
    }

    /// `approxEquals(S2Point, S2Point, double)`.
    pub fn approx_equals_points_with(a: &S2Point, b: &S2Point, max_error: f64) -> bool {
        a.angle(b) <= max_error
    }

    /// `approxEquals(S2Point, S2Point)`.
    pub fn approx_equals_points(a: &S2Point, b: &S2Point) -> bool {
        Self::approx_equals_points_with(a, b, 1e-15)
    }

    /// `approxEquals(double, double, double)`.
    pub fn approx_equals_with(a: f64, b: f64, max_error: f64) -> bool {
        (a - b).abs() <= max_error
    }

    /// `approxEquals(double, double)`.
    pub fn approx_equals(a: f64, b: f64) -> bool {
        Self::approx_equals_with(a, b, 1e-15)
    }
}

/// Equivalent to `com.google.common.geometry.S2.Metric` (`public static
/// class`): a measure of a cell size that scales by a factor per level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metric {
    deriv: f64,
    dim: i32,
}

impl Metric {
    /// `Metric(int dim, double deriv)`.
    pub const fn new(dim: i32, deriv: f64) -> Self {
        Self { deriv, dim }
    }

    /// `deriv()`.
    pub fn deriv(&self) -> f64 {
        self.deriv
    }

    /// `getValue(int level)`: `StrictMath.scalb(deriv, dim * (1 - level))`.
    pub fn get_value(&self, level: i32) -> f64 {
        scalb(self.deriv, self.dim * (1 - level))
    }

    /// `getClosestLevel(double)`.
    pub fn get_closest_level(&self, value: f64) -> i32 {
        self.get_min_level(S2::M_SQRT2 * value)
    }

    /// `getMinLevel(double)`.
    pub fn get_min_level(&self, value: f64) -> i32 {
        if value <= 0.0 {
            return S2CellId::MAX_LEVEL;
        }
        let exponent = S2::exp(value / (f64::from(1i32 << self.dim) * self.deriv));
        0.max(S2CellId::MAX_LEVEL.min(-((exponent - 1) >> (self.dim - 1))))
    }

    /// `getMaxLevel(double)`.
    pub fn get_max_level(&self, value: f64) -> i32 {
        if value <= 0.0 {
            return S2CellId::MAX_LEVEL;
        }
        let exponent = S2::exp(f64::from(1i32 << self.dim) * self.deriv / value);
        0.max(S2CellId::MAX_LEVEL.min((exponent - 1) >> (self.dim - 1)))
    }
}

/// `java.lang.StrictMath.scalb(double, int)`: `d * 2^scale_factor`, rounded
/// once, as the JDK specifies (fdlibm `scalbn`).
pub(crate) fn scalb(d: f64, scale_factor: i32) -> f64 {
    // The JDK clamps the scale factor to a range that already saturates every
    // finite double, then multiplies by powers of two in steps that cannot
    // underflow twice (`Math.scalb`, JDK 25).
    const MAX_SCALE: i32 = 1023 + 1022 + 53 + 1;
    let mut scale = scale_factor;
    let scale_increment: i32;
    let exp_delta: f64;
    if scale < 0 {
        scale = scale.max(-MAX_SCALE);
        scale_increment = -512;
        exp_delta = f64::from_bits(((-512i64 + 1023) as u64) << 52);
    } else {
        scale = scale.min(MAX_SCALE);
        scale_increment = 512;
        exp_delta = f64::from_bits(((512i64 + 1023) as u64) << 52);
    }
    let t = ((scale >> (9 - 1)) as u32 >> (32 - 9)) as i32;
    let exp_adjust = ((scale + t) & (512 - 1)) - t;
    let mut d = d * f64::from_bits(((i64::from(exp_adjust) + 1023) as u64) << 52);
    scale -= exp_adjust;
    while scale != 0 {
        d *= exp_delta;
        scale -= scale_increment;
    }
    d
}

/// `java.lang.Math.round(double)`: the closest `long`, ties toward positive
/// infinity, `NaN` to 0, saturating at the `long` range.
pub(crate) fn java_round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    let f = a.floor();
    // `a - floor(a)` is exact for every double.
    let r = if a - f >= 0.5 { f + 1.0 } else { f };
    r as i64
}

/// `java.lang.Double.doubleToLongBits(double)`: the IEEE 754 bits, with every
/// NaN collapsed to the canonical `0x7ff8000000000000L`.
pub(crate) fn double_to_long_bits(v: f64) -> i64 {
    if v.is_nan() {
        0x7ff8000000000000
    } else {
        v.to_bits() as i64
    }
}

/// `new <class>()`: an unchecked Java exception whose `getMessage()` is
/// `null`.
pub(crate) fn null_message_error(class: &'static str) -> rucene::error::LuceneError {
    rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::Unchecked {
        class,
        message: None,
        cause: None,
    })
}
