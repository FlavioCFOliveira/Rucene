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

//! The types declared in `S2EdgeUtil.java`, of the Java package
//! `com.google.common.geometry`.

use rucene::error::{LuceneError, Result};
use rucene::util::java_math;

use crate::r1_interval::R1Interval;
use crate::s1_angle::S1Angle;
use crate::s1_interval::S1Interval;
use crate::s2::{null_message_error, S2};
use crate::s2_lat_lng::S2LatLng;
use crate::s2_lat_lng_rect::S2LatLngRect;
use crate::s2_point::S2Point;

/// Equivalent to `com.google.common.geometry.S2EdgeUtil` (`public strictfp
/// class`): edge crossing, intersection and distance predicates.
pub struct S2EdgeUtil;

impl S2EdgeUtil {
    /// `DEFAULT_INTERSECTION_TOLERANCE`.
    pub const DEFAULT_INTERSECTION_TOLERANCE: S1Angle = S1Angle::from_radians(1.5e-15);

    /// `simpleCrossing(S2Point, S2Point, S2Point, S2Point)`.
    pub fn simple_crossing(a: &S2Point, b: &S2Point, c: &S2Point, d: &S2Point) -> bool {
        let ab = S2Point::cross_prod(a, b);
        let acb = -(ab.dot_prod(c));
        let bda = ab.dot_prod(d);
        if acb * bda <= 0.0 {
            return false;
        }
        let cd = S2Point::cross_prod(c, d);
        let cbd = -(cd.dot_prod(b));
        let dac = cd.dot_prod(a);
        (acb * cbd > 0.0) && (acb * dac > 0.0)
    }

    /// `robustCrossing(S2Point, S2Point, S2Point, S2Point)`.
    pub fn robust_crossing(a: &S2Point, b: &S2Point, c: &S2Point, d: &S2Point) -> i32 {
        let a_cross_b = S2Point::cross_prod(a, b);
        let acb = -S2::robust_ccw_with_cross(a, b, c, &a_cross_b);
        let bda = S2::robust_ccw_with_cross(a, b, d, &a_cross_b);
        if (bda & acb) == 0 {
            return 0;
        }
        if bda != acb {
            return -1;
        }
        let c_cross_d = S2Point::cross_prod(c, d);
        let cbd = -S2::robust_ccw_with_cross(c, d, b, &c_cross_d);
        if cbd != acb {
            return -1;
        }
        let dac = S2::robust_ccw_with_cross(c, d, a, &c_cross_d);
        if dac == acb {
            1
        } else {
            -1
        }
    }

    /// `vertexCrossing(S2Point, S2Point, S2Point, S2Point)`.
    pub fn vertex_crossing(a: &S2Point, b: &S2Point, c: &S2Point, d: &S2Point) -> bool {
        if a == b || c == d {
            return false;
        }
        if a == d {
            return S2::ordered_ccw(&S2::ortho(a), c, b, a);
        }
        if b == c {
            return S2::ordered_ccw(&S2::ortho(b), d, a, b);
        }
        if a == c {
            return S2::ordered_ccw(&S2::ortho(a), d, b, a);
        }
        if b == d {
            return S2::ordered_ccw(&S2::ortho(b), c, a, b);
        }
        false
    }

    /// `edgeOrVertexCrossing(S2Point, S2Point, S2Point, S2Point)`.
    pub fn edge_or_vertex_crossing(a: &S2Point, b: &S2Point, c: &S2Point, d: &S2Point) -> bool {
        let crossing = Self::robust_crossing(a, b, c, d);
        if crossing < 0 {
            return false;
        }
        if crossing > 0 {
            return true;
        }
        Self::vertex_crossing(a, b, c, d)
    }

    /// `getIntersection(S2Point, S2Point, S2Point, S2Point)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the edges do not truly cross.
    pub fn get_intersection(
        a0: &S2Point,
        a1: &S2Point,
        b0: &S2Point,
        b1: &S2Point,
    ) -> Result<S2Point> {
        if Self::robust_crossing(a0, a1, b0, b1) <= 0 {
            return Err(LuceneError::illegal_argument(
                "Input edges a0a1 and b0b1 muct have a true robustCrossing.",
            ));
        }
        let a_norm = S2Point::normalize(&S2::robust_cross_prod(a0, a1));
        let b_norm = S2Point::normalize(&S2::robust_cross_prod(b0, b1));
        let mut x = S2Point::normalize(&S2::robust_cross_prod(&a_norm, &b_norm));
        if x.dot_prod(&S2Point::add(&S2Point::add(a0, a1), &S2Point::add(b0, b1))) < 0.0 {
            x = S2Point::neg(&x);
        }
        if S2::ordered_ccw(a0, &x, a1, &a_norm) && S2::ordered_ccw(b0, &x, b1, &b_norm) {
            return Ok(x);
        }
        let mut r = CloserResult::new(10.0, x);
        if S2::ordered_ccw(b0, a0, b1, &b_norm) {
            r.replace_if_closer(&x, a0);
        }
        if S2::ordered_ccw(b0, a1, b1, &b_norm) {
            r.replace_if_closer(&x, a1);
        }
        if S2::ordered_ccw(a0, b0, a1, &a_norm) {
            r.replace_if_closer(&x, b0);
        }
        if S2::ordered_ccw(a0, b1, a1, &a_norm) {
            r.replace_if_closer(&x, b1);
        }
        Ok(r.get_vmin())
    }

    /// `getDistanceFraction(S2Point, S2Point, S2Point)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (no message) when `a0` equals `a1`.
    pub fn get_distance_fraction(x: &S2Point, a0: &S2Point, a1: &S2Point) -> Result<f64> {
        if a0 == a1 {
            return Err(null_message_error("java.lang.IllegalArgumentException"));
        }
        let d0 = x.angle(a0);
        let d1 = x.angle(a1);
        Ok(d0 / (d0 + d1))
    }

    /// `getDistance(S2Point x, S2Point a, S2Point b)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (no message) when a point is not unit
    /// length.
    pub fn get_distance(x: &S2Point, a: &S2Point, b: &S2Point) -> Result<S1Angle> {
        Self::get_distance_with_cross(x, a, b, &S2::robust_cross_prod(a, b))
    }

    /// `getDistance(S2Point x, S2Point a, S2Point b, S2Point aCrossB)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (no message) when a point is not unit
    /// length.
    pub fn get_distance_with_cross(
        x: &S2Point,
        a: &S2Point,
        b: &S2Point,
        a_cross_b: &S2Point,
    ) -> Result<S1Angle> {
        if !S2::is_unit_length(x) || !S2::is_unit_length(a) || !S2::is_unit_length(b) {
            return Err(null_message_error("java.lang.IllegalArgumentException"));
        }
        if S2::simple_ccw(a_cross_b, a, x) && S2::simple_ccw(x, b, a_cross_b) {
            let sin_dist = x.dot_prod(a_cross_b).abs() / a_cross_b.norm();
            return Ok(S1Angle::from_radians(java_math::asin(1.0f64.min(sin_dist))));
        }
        let linear_dist2 = S2Point::minus(x, a)
            .norm2()
            .min(S2Point::minus(x, b).norm2());
        Ok(S1Angle::from_radians(
            2.0 * java_math::asin(1.0f64.min(0.5 * java_math::sqrt(linear_dist2))),
        ))
    }

    /// `getClosestPoint(S2Point x, S2Point a, S2Point b)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (no message) when a point is not unit
    /// length.
    pub fn get_closest_point(x: &S2Point, a: &S2Point, b: &S2Point) -> Result<S2Point> {
        if !S2::is_unit_length(x) || !S2::is_unit_length(a) || !S2::is_unit_length(b) {
            return Err(null_message_error("java.lang.IllegalArgumentException"));
        }
        let cross_prod = S2::robust_cross_prod(a, b);
        let p = S2Point::minus(
            x,
            &S2Point::mul(&cross_prod, x.dot_prod(&cross_prod) / cross_prod.norm2()),
        );
        if S2::simple_ccw(&cross_prod, a, &p) && S2::simple_ccw(&p, b, &cross_prod) {
            return Ok(S2Point::normalize(&p));
        }
        Ok(
            if S2Point::minus(x, a).norm2() <= S2Point::minus(x, b).norm2() {
                *a
            } else {
                *b
            },
        )
    }
}

/// Equivalent to `com.google.common.geometry.S2EdgeUtil.EdgeCrosser`
/// (`public static class`): tests a fixed edge `AB` against a chain of edges
/// `CD`, `DE`, ...
#[derive(Clone, Debug)]
pub struct EdgeCrosser {
    a: S2Point,
    b: S2Point,
    a_cross_b: S2Point,
    c: S2Point,
    acb: i32,
}

impl EdgeCrosser {
    /// `EdgeCrosser(S2Point a, S2Point b, S2Point c)`.
    pub fn new(a: S2Point, b: S2Point, c: S2Point) -> Self {
        let a_cross_b = S2Point::cross_prod(&a, &b);
        let mut crosser = Self {
            a,
            b,
            a_cross_b,
            c,
            acb: 0,
        };
        crosser.restart_at(c);
        crosser
    }

    /// `restartAt(S2Point)`.
    pub fn restart_at(&mut self, c: S2Point) {
        self.c = c;
        self.acb = -S2::robust_ccw_with_cross(&self.a, &self.b, &c, &self.a_cross_b);
    }

    /// `robustCrossing(S2Point)`.
    pub fn robust_crossing(&mut self, d: S2Point) -> i32 {
        let bda = S2::robust_ccw_with_cross(&self.a, &self.b, &d, &self.a_cross_b);
        let result = if bda == -self.acb && bda != 0 {
            -1
        } else if (bda & self.acb) == 0 {
            0
        } else {
            self.robust_crossing_internal(&d) // Slow path.
        };
        self.c = d;
        self.acb = -bda;
        result
    }

    /// `edgeOrVertexCrossing(S2Point)`.
    pub fn edge_or_vertex_crossing(&mut self, d: S2Point) -> bool {
        let c2 = S2Point::new(self.c.get(0), self.c.get(1), self.c.get(2));
        let crossing = self.robust_crossing(d);
        if crossing < 0 {
            return false;
        }
        if crossing > 0 {
            return true;
        }
        S2EdgeUtil::vertex_crossing(&self.a, &self.b, &c2, &d)
    }

    fn robust_crossing_internal(&self, d: &S2Point) -> i32 {
        let c_cross_d = S2Point::cross_prod(&self.c, d);
        let cbd = -S2::robust_ccw_with_cross(&self.c, d, &self.b, &c_cross_d);
        if cbd != self.acb {
            return -1;
        }
        let dac = S2::robust_ccw_with_cross(&self.c, d, &self.a, &c_cross_d);
        if dac == self.acb {
            1
        } else {
            -1
        }
    }
}

/// Equivalent to `com.google.common.geometry.S2EdgeUtil.RectBounder`
/// (`public static class`): the bounding rectangle of a chain of edges.
#[derive(Clone, Debug)]
pub struct RectBounder {
    a: Option<S2Point>,
    a_lat_lng: Option<S2LatLng>,
    bound: S2LatLngRect,
}

impl Default for RectBounder {
    fn default() -> Self {
        Self::new()
    }
}

impl RectBounder {
    /// `RectBounder()`.
    pub fn new() -> Self {
        Self {
            a: None,
            a_lat_lng: None,
            bound: S2LatLngRect::empty(),
        }
    }

    /// `addPoint(S2Point)`.
    pub fn add_point(&mut self, b: S2Point) {
        let b_lat_lng = S2LatLng::from_point(&b);
        if self.bound.is_empty() {
            self.bound = self.bound.add_lat_lng(&b_lat_lng);
        } else {
            let a = self.a.expect("java.lang.NullPointerException: a is null");
            let a_lat_lng = self
                .a_lat_lng
                .expect("java.lang.NullPointerException: aLatLng is null");
            self.bound = self
                .bound
                .union(&S2LatLngRect::from_point_pair(&a_lat_lng, &b_lat_lng));
            let a_cross_b = S2::robust_cross_prod(&a, &b);
            let dir = S2Point::cross_prod(&a_cross_b, &S2Point::new(0.0, 0.0, 1.0));
            let da = dir.dot_prod(&a);
            let db = dir.dot_prod(&b);
            if da * db < 0.0 {
                let abs_lat = java_math::acos((a_cross_b.get(2) / a_cross_b.norm()).abs());
                let mut lat = self.bound.lat();
                if da < 0.0 {
                    lat = R1Interval::new(lat.lo(), abs_lat.max(self.bound.lat().hi()));
                } else {
                    lat = R1Interval::new((-abs_lat).min(self.bound.lat().lo()), lat.hi());
                }
                self.bound = S2LatLngRect::new(lat, self.bound.lng());
            }
        }
        self.a = Some(b);
        self.a_lat_lng = Some(b_lat_lng);
    }

    /// `getBound()`.
    pub fn get_bound(&self) -> S2LatLngRect {
        self.bound
    }
}

/// Equivalent to `com.google.common.geometry.S2EdgeUtil.XYZPruner` (`public
/// static class`): a quick rejection test of edges against the bounding box
/// of other edges.
#[derive(Clone, Debug, Default)]
pub struct XYZPruner {
    last_vertex: Option<S2Point>,
    bound_set: bool,
    xmin: f64,
    ymin: f64,
    zmin: f64,
    xmax: f64,
    ymax: f64,
    zmax: f64,
    max_deformation: f64,
}

impl XYZPruner {
    /// `XYZPruner()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `addEdgeToBounds(S2Point from, S2Point to)`.
    pub fn add_edge_to_bounds(&mut self, from: &S2Point, to: &S2Point) {
        if !self.bound_set {
            self.bound_set = true;
            self.xmin = from.x;
            self.xmax = from.x;
            self.ymin = from.y;
            self.ymax = from.y;
            self.zmin = from.z;
            self.zmax = from.z;
        }
        self.xmin = self.xmin.min(to.x.min(from.x));
        self.ymin = self.ymin.min(to.y.min(from.y));
        self.zmin = self.zmin.min(to.z.min(from.z));
        self.xmax = self.xmax.max(to.x.max(from.x));
        self.ymax = self.ymax.max(to.y.max(from.y));
        self.zmax = self.zmax.max(to.z.max(from.z));
        let approx_arc_len = (from.x - to.x).abs() + (from.y - to.y).abs() + (from.z - to.z).abs();
        if approx_arc_len < 0.025 {
            // less than 2 degrees
            self.max_deformation = self.max_deformation.max(approx_arc_len * 0.0025);
        } else if approx_arc_len < 1.0 {
            // less than 90 degrees
            self.max_deformation = self.max_deformation.max(approx_arc_len * 0.11);
        } else {
            self.max_deformation = approx_arc_len * 0.5;
        }
    }

    /// `setFirstIntersectPoint(S2Point)`.
    pub fn set_first_intersect_point(&mut self, v0: S2Point) {
        self.xmin -= self.max_deformation;
        self.ymin -= self.max_deformation;
        self.zmin -= self.max_deformation;
        self.xmax += self.max_deformation;
        self.ymax += self.max_deformation;
        self.zmax += self.max_deformation;
        self.last_vertex = Some(v0);
    }

    /// `intersects(S2Point)`.
    pub fn intersects(&mut self, v1: S2Point) -> bool {
        let last = self
            .last_vertex
            .expect("java.lang.NullPointerException: lastVertex is null");
        let mut result = true;
        if (v1.x < self.xmin && last.x < self.xmin) || (v1.x > self.xmax && last.x > self.xmax) {
            result = false;
        } else if (v1.y < self.ymin && last.y < self.ymin)
            || (v1.y > self.ymax && last.y > self.ymax)
        {
            result = false;
        } else if (v1.z < self.zmin && last.z < self.zmin)
            || (v1.z > self.zmax && last.z > self.zmax)
        {
            result = false;
        }
        self.last_vertex = Some(v1);
        result
    }
}

/// Equivalent to `com.google.common.geometry.S2EdgeUtil.LongitudePruner`
/// (`public static class`): a quick rejection test of edges against a
/// longitude interval.
#[derive(Clone, Debug)]
pub struct LongitudePruner {
    interval: S1Interval,
    lng0: f64,
}

impl LongitudePruner {
    /// `LongitudePruner(S1Interval interval, S2Point v0)`.
    pub fn new(interval: S1Interval, v0: &S2Point) -> Self {
        Self {
            interval,
            lng0: S2LatLng::longitude(v0).radians(),
        }
    }

    /// `intersects(S2Point)`.
    pub fn intersects(&mut self, v1: &S2Point) -> bool {
        let lng1 = S2LatLng::longitude(v1).radians();
        let result = self
            .interval
            .intersects(&S1Interval::from_point_pair(self.lng0, lng1));
        self.lng0 = lng1;
        result
    }
}

/// Equivalent to `com.google.common.geometry.S2EdgeUtil.WedgeRelation`
/// (`public interface`): a relation between two wedges `A = (a0, ab1, a2)`
/// and `B = (b0, ab1, b2)` sharing their middle vertex.
pub trait WedgeRelation {
    /// `test(S2Point a0, S2Point ab1, S2Point a2, S2Point b0, S2Point b2)`.
    fn test(&self, a0: &S2Point, ab1: &S2Point, a2: &S2Point, b0: &S2Point, b2: &S2Point) -> i32;
}

/// Equivalent to `com.google.common.geometry.S2EdgeUtil.WedgeContains`
/// (`public static class`).
#[derive(Clone, Copy, Debug, Default)]
pub struct WedgeContains;

impl WedgeRelation for WedgeContains {
    fn test(&self, a0: &S2Point, ab1: &S2Point, a2: &S2Point, b0: &S2Point, b2: &S2Point) -> i32 {
        if S2::ordered_ccw(a2, b2, b0, ab1) && S2::ordered_ccw(b0, a0, a2, ab1) {
            1
        } else {
            0
        }
    }
}

/// Equivalent to `com.google.common.geometry.S2EdgeUtil.WedgeIntersects`
/// (`public static class`).
#[derive(Clone, Copy, Debug, Default)]
pub struct WedgeIntersects;

impl WedgeRelation for WedgeIntersects {
    fn test(&self, a0: &S2Point, ab1: &S2Point, a2: &S2Point, b0: &S2Point, b2: &S2Point) -> i32 {
        if S2::ordered_ccw(a0, b2, b0, ab1) && S2::ordered_ccw(b0, a2, a0, ab1) {
            0
        } else {
            -1
        }
    }
}

/// Equivalent to
/// `com.google.common.geometry.S2EdgeUtil.WedgeContainsOrIntersects`
/// (`public static class`).
#[derive(Clone, Copy, Debug, Default)]
pub struct WedgeContainsOrIntersects;

impl WedgeRelation for WedgeContainsOrIntersects {
    fn test(&self, a0: &S2Point, ab1: &S2Point, a2: &S2Point, b0: &S2Point, b2: &S2Point) -> i32 {
        if S2::ordered_ccw(a0, a2, b2, ab1) {
            return if S2::ordered_ccw(b2, b0, a0, ab1) {
                1
            } else {
                -1
            }; // Case 1 vs. 2,5,6.
        }
        if !S2::ordered_ccw(a2, b0, b2, ab1) {
            return 0; // Case 3.
        }
        if a2 == b0 {
            0
        } else {
            -1
        } // Case 3 vs. 2,4.
    }
}

/// Equivalent to
/// `com.google.common.geometry.S2EdgeUtil.WedgeContainsOrCrosses`
/// (`public static class`).
#[derive(Clone, Copy, Debug, Default)]
pub struct WedgeContainsOrCrosses;

impl WedgeRelation for WedgeContainsOrCrosses {
    fn test(&self, a0: &S2Point, ab1: &S2Point, a2: &S2Point, b0: &S2Point, b2: &S2Point) -> i32 {
        if S2::ordered_ccw(a0, a2, b2, ab1) {
            if S2::ordered_ccw(b2, b0, a0, ab1) {
                return 1; // Case 1 (A contains B)
            }
            return if a2 == b2 { 0 } else { -1 }; // Case 2 vs. 5,6.
        }
        if S2::ordered_ccw(a0, b0, a2, ab1) {
            0
        } else {
            -1
        } // Case 2,3 vs. 4.
    }
}

/// Equivalent to `com.google.common.geometry.S2EdgeUtil.CloserResult`
/// (package-private `static class`): the closest candidate seen so far.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CloserResult {
    dmin2: f64,
    vmin: S2Point,
}

impl CloserResult {
    /// `CloserResult(double dmin2, S2Point vmin)`.
    pub(crate) fn new(dmin2: f64, vmin: S2Point) -> Self {
        Self { dmin2, vmin }
    }

    /// `getDmin2()`.
    #[allow(dead_code)] // Package-private in Java; no caller in this subset.
    pub(crate) fn get_dmin2(&self) -> f64 {
        self.dmin2
    }

    /// `getVmin()`.
    pub(crate) fn get_vmin(&self) -> S2Point {
        self.vmin
    }

    /// `replaceIfCloser(S2Point, S2Point)`.
    pub(crate) fn replace_if_closer(&mut self, x: &S2Point, y: &S2Point) {
        let d2 = S2Point::minus(x, y).norm2();
        if d2 < self.dmin2 || (d2 == self.dmin2 && y.less_than(&self.vmin)) {
            self.dmin2 = d2;
            self.vmin = *y;
        }
    }
}
