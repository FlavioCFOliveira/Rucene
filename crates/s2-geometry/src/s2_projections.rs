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

//! The types declared in `S2Projections.java`, of the Java package
//! `com.google.common.geometry`.

// The metric constants are s2-geometry-library-java's literals, kept verbatim.
#![allow(clippy::excessive_precision)]

use crate::r2_vector::R2Vector;
use crate::s2::{Metric, S2};
use crate::s2_point::S2Point;
use rucene::util::java_math;

/// Equivalent to `com.google.common.geometry.S2Projections.Projections`
/// (`public enum`): the cube-face projections the library knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)]
pub enum Projections {
    /// `S2_LINEAR_PROJECTION`.
    S2LinearProjection,
    /// `S2_TAN_PROJECTION`.
    S2TanProjection,
    /// `S2_QUADRATIC_PROJECTION`.
    S2QuadraticProjection,
}

/// Equivalent to `com.google.common.geometry.S2Projections` (`public final
/// strictfp class`): the projection between cube-face `(s,t)`, `(u,v)` and
/// `(x,y,z)` coordinates, and the cell metrics it implies.
pub struct S2Projections;

const S2_PROJECTION: Projections = Projections::S2QuadraticProjection;

const fn select(linear: f64, tan: f64, quadratic: f64) -> f64 {
    match S2_PROJECTION {
        Projections::S2LinearProjection => linear,
        Projections::S2TanProjection => tan,
        Projections::S2QuadraticProjection => quadratic,
    }
}

// `Math.sqrt(3)` and `Math.sqrt(6)`, correctly rounded as `Math.sqrt` is.
const SQRT_3: f64 = 1.7320508075688772;
const SQRT_6: f64 = 2.449489742783178;

impl S2Projections {
    /// `MIN_AREA`.
    pub const MIN_AREA: Metric = Metric::new(
        2,
        select(
            1.0 / (3.0 * SQRT_3),
            (S2::M_PI * S2::M_PI) / (16.0 * S2::M_SQRT2),
            2.0 * S2::M_SQRT2 / 9.0,
        ),
    );
    /// `MAX_AREA`.
    pub const MAX_AREA: Metric = Metric::new(
        2,
        select(1.0, S2::M_PI * S2::M_PI / 16.0, 0.65894981424079037),
    );
    /// `AVG_AREA`.
    pub const AVG_AREA: Metric = Metric::new(2, S2::M_PI / 6.0);
    /// `MIN_ANGLE_SPAN`.
    pub const MIN_ANGLE_SPAN: Metric = Metric::new(1, select(0.5, S2::M_PI / 4.0, 2. / 3.0));
    /// `MAX_ANGLE_SPAN`.
    pub const MAX_ANGLE_SPAN: Metric =
        Metric::new(1, select(1.0, S2::M_PI / 4.0, 0.85244858959960922));
    /// `AVG_ANGLE_SPAN`.
    pub const AVG_ANGLE_SPAN: Metric = Metric::new(1, S2::M_PI / 4.0);
    /// `MIN_WIDTH`.
    pub const MIN_WIDTH: Metric = Metric::new(
        1,
        select(
            1.0 / SQRT_6,
            S2::M_PI / (4.0 * S2::M_SQRT2),
            S2::M_SQRT2 / 3.0,
        ),
    );
    /// `MAX_WIDTH`: `MAX_ANGLE_SPAN.deriv()`.
    pub const MAX_WIDTH: Metric = Metric::new(1, select(1.0, S2::M_PI / 4.0, 0.85244858959960922));
    /// `AVG_WIDTH`.
    pub const AVG_WIDTH: Metric = Metric::new(
        1,
        select(
            0.70572967292222848,
            0.71865931946258044,
            0.71726183644304969,
        ),
    );
    /// `MIN_EDGE`.
    pub const MIN_EDGE: Metric = Metric::new(
        1,
        select(
            S2::M_SQRT2 / 3.0,
            S2::M_PI / (4.0 * S2::M_SQRT2),
            S2::M_SQRT2 / 3.0,
        ),
    );
    /// `MAX_EDGE`: `MAX_ANGLE_SPAN.deriv()`.
    pub const MAX_EDGE: Metric = Metric::new(1, select(1.0, S2::M_PI / 4.0, 0.85244858959960922));
    /// `AVG_EDGE`.
    pub const AVG_EDGE: Metric = Metric::new(
        1,
        select(
            0.72001709647780182,
            0.73083351627336963,
            0.72960687319305303,
        ),
    );
    /// `MIN_DIAG`.
    pub const MIN_DIAG: Metric = Metric::new(
        1,
        select(
            S2::M_SQRT2 / 3.0,
            S2::M_PI / (3.0 * S2::M_SQRT2),
            4.0 * S2::M_SQRT2 / 9.0,
        ),
    );
    /// `MAX_DIAG`.
    pub const MAX_DIAG: Metric = Metric::new(
        1,
        select(S2::M_SQRT2, S2::M_PI / SQRT_6, 1.2193272972170106),
    );
    /// `AVG_DIAG`.
    pub const AVG_DIAG: Metric = Metric::new(
        1,
        select(1.0159089332094063, 1.0318115985978178, 1.03021136949923584),
    );
    /// `MAX_EDGE_ASPECT`.
    pub const MAX_EDGE_ASPECT: f64 = select(S2::M_SQRT2, S2::M_SQRT2, 1.44261527445268292);
    /// `MAX_DIAG_ASPECT`: `Math.sqrt(3)`.
    pub const MAX_DIAG_ASPECT: f64 = SQRT_3;

    /// `stToUV(double)`.
    pub fn st_to_uv(s: f64) -> f64 {
        match S2_PROJECTION {
            Projections::S2LinearProjection => s,
            Projections::S2TanProjection => {
                let s = java_math::tan(S2::M_PI_4 * s);
                s + (1.0 / (1u64 << 53) as f64) * s
            }
            Projections::S2QuadraticProjection => {
                if s >= 0.0 {
                    (1.0 / 3.0) * ((1.0 + s) * (1.0 + s) - 1.0)
                } else {
                    (1.0 / 3.0) * (1.0 - (1.0 - s) * (1.0 - s))
                }
            }
        }
    }

    /// `uvToST(double)`.
    pub fn uv_to_st(u: f64) -> f64 {
        match S2_PROJECTION {
            Projections::S2LinearProjection => u,
            Projections::S2TanProjection => (4.0 * S2::M_1_PI) * java_math::atan(u),
            Projections::S2QuadraticProjection => {
                if u >= 0.0 {
                    java_math::sqrt(1.0 + 3.0 * u) - 1.0
                } else {
                    1.0 - java_math::sqrt(1.0 - 3.0 * u)
                }
            }
        }
    }

    /// `faceUvToXyz(int, double, double)`.
    pub fn face_uv_to_xyz(face: i32, u: f64, v: f64) -> S2Point {
        match face {
            0 => S2Point::new(1.0, u, v),
            1 => S2Point::new(-u, 1.0, v),
            2 => S2Point::new(-u, -v, 1.0),
            3 => S2Point::new(-1.0, -v, -u),
            4 => S2Point::new(v, -1.0, -u),
            _ => S2Point::new(v, u, -1.0),
        }
    }

    /// `validFaceXyzToUv(int, S2Point)`.
    pub fn valid_face_xyz_to_uv(face: i32, p: &S2Point) -> R2Vector {
        let (pu, pv) = match face {
            0 => (p.y / p.x, p.z / p.x),
            1 => (-p.x / p.y, p.z / p.y),
            2 => (-p.x / p.z, -p.y / p.z),
            3 => (p.z / p.x, p.y / p.x),
            4 => (p.z / p.y, -p.x / p.y),
            _ => (-p.y / p.z, -p.x / p.z),
        };
        R2Vector::new(pu, pv)
    }

    /// `xyzToFace(S2Point)`.
    pub fn xyz_to_face(p: &S2Point) -> i32 {
        let mut face = p.largest_abs_component();
        if p.get(face) < 0.0 {
            face += 3;
        }
        face
    }

    /// `faceXyzToUv(int, S2Point)`: `None` (Java `null`) when `p` is not on
    /// the positive side of `face`.
    pub fn face_xyz_to_uv(face: i32, p: &S2Point) -> Option<R2Vector> {
        if face < 3 {
            if p.get(face) <= 0.0 {
                return None;
            }
        } else if p.get(face - 3) >= 0.0 {
            return None;
        }
        Some(Self::valid_face_xyz_to_uv(face, p))
    }

    /// `getUNorm(int, double)`.
    pub fn get_u_norm(face: i32, u: f64) -> S2Point {
        match face {
            0 => S2Point::new(u, -1.0, 0.0),
            1 => S2Point::new(1.0, u, 0.0),
            2 => S2Point::new(1.0, 0.0, u),
            3 => S2Point::new(-u, 0.0, 1.0),
            4 => S2Point::new(0.0, -u, 1.0),
            _ => S2Point::new(0.0, -1.0, -u),
        }
    }

    /// `getVNorm(int, double)`.
    pub fn get_v_norm(face: i32, v: f64) -> S2Point {
        match face {
            0 => S2Point::new(-v, 0.0, 1.0),
            1 => S2Point::new(0.0, -v, 1.0),
            2 => S2Point::new(0.0, -1.0, -v),
            3 => S2Point::new(v, -1.0, 0.0),
            4 => S2Point::new(1.0, v, 0.0),
            _ => S2Point::new(1.0, 0.0, v),
        }
    }

    /// `getNorm(int)`.
    pub fn get_norm(face: i32) -> S2Point {
        Self::face_uv_to_xyz(face, 0.0, 0.0)
    }

    /// `getUAxis(int)`.
    pub fn get_u_axis(face: i32) -> S2Point {
        match face {
            0 => S2Point::new(0.0, 1.0, 0.0),
            1 | 2 => S2Point::new(-1.0, 0.0, 0.0),
            3 | 4 => S2Point::new(0.0, 0.0, -1.0),
            _ => S2Point::new(0.0, 1.0, 0.0),
        }
    }

    /// `getVAxis(int)`.
    pub fn get_v_axis(face: i32) -> S2Point {
        match face {
            0 | 1 => S2Point::new(0.0, 0.0, 1.0),
            2 | 3 => S2Point::new(0.0, -1.0, 0.0),
            _ => S2Point::new(1.0, 0.0, 0.0),
        }
    }
}
