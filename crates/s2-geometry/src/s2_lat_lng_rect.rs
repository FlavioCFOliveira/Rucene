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

//! The types declared in `S2LatLngRect.java`, of the Java package
//! `com.google.common.geometry`.

use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::error::{LuceneError, Result};

use crate::r1_interval::R1Interval;
use crate::s1_angle::S1Angle;
use crate::s1_interval::S1Interval;
use crate::s2::{ieee_remainder, null_message_error, S2};
use crate::s2_cap::S2Cap;
use crate::s2_cell::S2Cell;
use crate::s2_edge_util::S2EdgeUtil;
use crate::s2_lat_lng::S2LatLng;
use crate::s2_point::S2Point;
use crate::s2_region::S2Region;

/// Equivalent to `com.google.common.geometry.S2LatLngRect` (`public strictfp
/// class S2LatLngRect implements S2Region`): a closed latitude-longitude
/// rectangle.
#[derive(Clone, Copy, Debug)]
pub struct S2LatLngRect {
    lat: R1Interval,
    lng: S1Interval,
}

impl S2LatLngRect {
    /// `S2LatLngRect(S2LatLng lo, S2LatLng hi)`.
    pub fn from_lo_hi(lo: &S2LatLng, hi: &S2LatLng) -> Self {
        Self {
            lat: R1Interval::new(lo.lat().radians(), hi.lat().radians()),
            lng: S1Interval::new(lo.lng().radians(), hi.lng().radians()),
        }
    }

    /// `S2LatLngRect(R1Interval lat, S1Interval lng)`.
    pub fn new(lat: R1Interval, lng: S1Interval) -> Self {
        Self { lat, lng }
    }

    /// `empty()`.
    pub fn empty() -> Self {
        Self::new(R1Interval::empty(), S1Interval::empty())
    }

    /// `full()`.
    pub fn full() -> Self {
        Self::new(Self::full_lat(), Self::full_lng())
    }

    /// `fullLat()`.
    pub fn full_lat() -> R1Interval {
        R1Interval::new(-S2::M_PI_2, S2::M_PI_2)
    }

    /// `fullLng()`.
    pub fn full_lng() -> S1Interval {
        S1Interval::full()
    }

    /// `fromCenterSize(S2LatLng, S2LatLng)`.
    pub fn from_center_size(center: &S2LatLng, size: &S2LatLng) -> Self {
        Self::from_point(center).expanded(&size.mul(0.5))
    }

    /// `fromPoint(S2LatLng)`.
    pub fn from_point(p: &S2LatLng) -> Self {
        Self::from_lo_hi(p, p)
    }

    /// `fromPointPair(S2LatLng, S2LatLng)`.
    pub fn from_point_pair(p1: &S2LatLng, p2: &S2LatLng) -> Self {
        Self::new(
            R1Interval::from_point_pair(p1.lat().radians(), p2.lat().radians()),
            S1Interval::from_point_pair(p1.lng().radians(), p2.lng().radians()),
        )
    }

    /// `fromEdge(S2Point, S2Point)`.
    pub fn from_edge(a: &S2Point, b: &S2Point) -> Self {
        let r = Self::from_point_pair(&S2LatLng::from_point(a), &S2LatLng::from_point(b));
        let ab = S2::robust_cross_prod(a, b);
        let dir = S2Point::cross_prod(&ab, &S2Point::new(0.0, 0.0, 1.0));
        let da = dir.dot_prod(a);
        let db = dir.dot_prod(b);
        if da * db >= 0.0 {
            return r;
        }
        let abs_lat = (ab.z / ab.norm()).abs().acos();
        if da < 0.0 {
            Self::new(R1Interval::new(r.lat().lo(), abs_lat), r.lng())
        } else {
            Self::new(R1Interval::new(-abs_lat, r.lat().hi()), r.lng())
        }
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        self.lat.lo().abs() <= S2::M_PI_2
            && self.lat.hi().abs() <= S2::M_PI_2
            && self.lng.is_valid()
            && self.lat.is_empty() == self.lng.is_empty()
    }

    /// `latLo()`.
    pub fn lat_lo(&self) -> S1Angle {
        S1Angle::from_radians(self.lat.lo())
    }

    /// `latHi()`.
    pub fn lat_hi(&self) -> S1Angle {
        S1Angle::from_radians(self.lat.hi())
    }

    /// `lngLo()`.
    pub fn lng_lo(&self) -> S1Angle {
        S1Angle::from_radians(self.lng.lo())
    }

    /// `lngHi()`.
    pub fn lng_hi(&self) -> S1Angle {
        S1Angle::from_radians(self.lng.hi())
    }

    /// `lat()`.
    pub fn lat(&self) -> R1Interval {
        self.lat
    }

    /// `lng()`.
    pub fn lng(&self) -> S1Interval {
        self.lng
    }

    /// `lo()`.
    pub fn lo(&self) -> S2LatLng {
        S2LatLng::new(&self.lat_lo(), &self.lng_lo())
    }

    /// `hi()`.
    pub fn hi(&self) -> S2LatLng {
        S2LatLng::new(&self.lat_hi(), &self.lng_hi())
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.lat.is_empty()
    }

    /// `isFull()`.
    pub fn is_full(&self) -> bool {
        self.lat == Self::full_lat() && self.lng.is_full()
    }

    /// `isInverted()`.
    pub fn is_inverted(&self) -> bool {
        self.lng.is_inverted()
    }

    /// `getVertex(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `k` is outside `[0,3]`.
    pub fn get_vertex(&self, k: i32) -> Result<S2LatLng> {
        match k {
            0 => Ok(S2LatLng::from_radians(self.lat.lo(), self.lng.lo())),
            1 => Ok(S2LatLng::from_radians(self.lat.lo(), self.lng.hi())),
            2 => Ok(S2LatLng::from_radians(self.lat.hi(), self.lng.hi())),
            3 => Ok(S2LatLng::from_radians(self.lat.hi(), self.lng.lo())),
            _ => Err(LuceneError::illegal_argument("Invalid vertex index.")),
        }
    }

    fn vertex(&self, k: i32) -> S2LatLng {
        self.get_vertex(k).expect("vertex index in [0,3]")
    }

    /// `getCenter()`.
    pub fn get_center(&self) -> S2LatLng {
        S2LatLng::from_radians(self.lat.get_center(), self.lng.get_center())
    }

    /// `getDistance(S2LatLng)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when this rectangle is empty;
    /// `IllegalArgumentException` when `p` is not valid; the errors of
    /// [`S2EdgeUtil::get_distance_with_cross`].
    pub fn get_distance_to_point(&self, p: &S2LatLng) -> Result<S1Angle> {
        let a = self;
        if a.is_empty() {
            return Err(null_message_error("java.lang.IllegalStateException"));
        }
        if !p.is_valid() {
            return Err(null_message_error("java.lang.IllegalArgumentException"));
        }
        if a.lng().contains_point(p.lng().radians()) {
            return Ok(S1Angle::from_radians(
                0.0f64.max((p.lat().radians() - a.lat().hi()).max(a.lat().lo() - p.lat().radians())),
            ));
        }
        let interval = S1Interval::new(a.lng().hi(), a.lng().complement().get_center());
        let mut a_lng = a.lng().lo();
        if interval.contains_point(p.lng().radians()) {
            a_lng = a.lng().hi();
        }
        let lo = S2LatLng::from_radians(a.lat().lo(), a_lng).to_point();
        let hi = S2LatLng::from_radians(a.lat().hi(), a_lng).to_point();
        let lo_cross_hi = S2LatLng::from_radians(0.0, a_lng - S2::M_PI_2).normalized().to_point();
        S2EdgeUtil::get_distance_with_cross(&p.to_point(), &lo, &hi, &lo_cross_hi)
    }

    /// `getDistance(S2LatLngRect)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when this rectangle is empty;
    /// `IllegalArgumentException` when `other` is empty; the errors of
    /// [`S2EdgeUtil::get_distance_with_cross`].
    pub fn get_distance(&self, other: &S2LatLngRect) -> Result<S1Angle> {
        let a = self;
        let b = other;
        if a.is_empty() {
            return Err(null_message_error("java.lang.IllegalStateException"));
        }
        if b.is_empty() {
            return Err(null_message_error("java.lang.IllegalArgumentException"));
        }
        if a.lng().intersects(&b.lng()) {
            if a.lat().intersects(&b.lat()) {
                return Ok(S1Angle::from_radians(0.0)); // Intersection between a and b.
            }
            let (lo, hi) = if a.lat().lo() > b.lat().hi() {
                (b.lat_hi(), a.lat_lo())
            } else {
                (a.lat_hi(), b.lat_lo())
            };
            return Ok(S1Angle::from_radians(hi.radians() - lo.radians()));
        }
        let lo_hi = S1Interval::from_point_pair(a.lng().lo(), b.lng().hi());
        let hi_lo = S1Interval::from_point_pair(a.lng().hi(), b.lng().lo());
        let (a_lng, b_lng) = if lo_hi.get_length() < hi_lo.get_length() {
            (a.lng_lo(), b.lng_hi())
        } else {
            (a.lng_hi(), b.lng_lo())
        };
        let a_lo = S2LatLng::new(&a.lat_lo(), &a_lng).to_point();
        let a_hi = S2LatLng::new(&a.lat_hi(), &a_lng).to_point();
        let a_lo_cross_hi =
            S2LatLng::from_radians(0.0, a_lng.radians() - S2::M_PI_2).normalized().to_point();
        let b_lo = S2LatLng::new(&b.lat_lo(), &b_lng).to_point();
        let b_hi = S2LatLng::new(&b.lat_hi(), &b_lng).to_point();
        let b_lo_cross_hi =
            S2LatLng::from_radians(0.0, b_lng.radians() - S2::M_PI_2).normalized().to_point();
        // Java evaluates the four distances left to right, as written.
        let d1 = S2EdgeUtil::get_distance_with_cross(&a_lo, &b_lo, &b_hi, &b_lo_cross_hi)?;
        let d2 = S2EdgeUtil::get_distance_with_cross(&a_hi, &b_lo, &b_hi, &b_lo_cross_hi)?;
        let d3 = S2EdgeUtil::get_distance_with_cross(&b_lo, &a_lo, &a_hi, &a_lo_cross_hi)?;
        let d4 = S2EdgeUtil::get_distance_with_cross(&b_hi, &a_lo, &a_hi, &a_lo_cross_hi)?;
        Ok(S1Angle::min(d1, S1Angle::min(d2, S1Angle::min(d3, d4))))
    }

    /// `getSize()`.
    pub fn get_size(&self) -> S2LatLng {
        S2LatLng::from_radians(self.lat.get_length(), self.lng.get_length())
    }

    /// `contains(S2LatLng)`.
    pub fn contains_lat_lng(&self, ll: &S2LatLng) -> bool {
        self.lat.contains_point(ll.lat().radians()) && self.lng.contains_point(ll.lng().radians())
    }

    /// `interiorContains(S2Point)`.
    pub fn interior_contains_point(&self, p: &S2Point) -> bool {
        self.interior_contains_lat_lng(&S2LatLng::from_point(p))
    }

    /// `interiorContains(S2LatLng)`.
    pub fn interior_contains_lat_lng(&self, ll: &S2LatLng) -> bool {
        self.lat.interior_contains_point(ll.lat().radians())
            && self.lng.interior_contains_point(ll.lng().radians())
    }

    /// `contains(S2LatLngRect)`.
    pub fn contains(&self, other: &S2LatLngRect) -> bool {
        self.lat.contains(&other.lat) && self.lng.contains(&other.lng)
    }

    /// `interiorContains(S2LatLngRect)`.
    pub fn interior_contains(&self, other: &S2LatLngRect) -> bool {
        self.lat.interior_contains(&other.lat) && self.lng.interior_contains(&other.lng)
    }

    /// `intersects(S2LatLngRect)`.
    pub fn intersects(&self, other: &S2LatLngRect) -> bool {
        self.lat.intersects(&other.lat) && self.lng.intersects(&other.lng)
    }

    /// `intersects(S2Cell)`.
    pub fn intersects_cell(&self, cell: &S2Cell) -> bool {
        if self.is_empty() {
            return false;
        }
        if self.contains_point(&cell.get_center()) {
            return true;
        }
        if cell.contains_point(&self.get_center().to_point()) {
            return true;
        }
        if !self.intersects(&cell.get_rect_bound()) {
            return false;
        }
        let mut cell_v = [S2Point::default(); 4];
        let mut cell_ll = [S2LatLng::default(); 4];
        for i in 0..4 {
            cell_v[i] = cell.get_vertex(i as i32); // Must be normalized.
            cell_ll[i] = S2LatLng::from_point(&cell_v[i]);
            if self.contains_lat_lng(&cell_ll[i]) {
                return true; // Quick acceptance test.
            }
        }
        for i in 0..4 {
            let edge_lng = S1Interval::from_point_pair(
                cell_ll[i].lng().radians(),
                cell_ll[(i + 1) & 3].lng().radians(),
            );
            if !self.lng.intersects(&edge_lng) {
                continue;
            }
            let a = &cell_v[i];
            let b = &cell_v[(i + 1) & 3];
            if edge_lng.contains_point(self.lng.lo())
                && Self::intersects_lng_edge(a, b, &self.lat, self.lng.lo())
            {
                return true;
            }
            if edge_lng.contains_point(self.lng.hi())
                && Self::intersects_lng_edge(a, b, &self.lat, self.lng.hi())
            {
                return true;
            }
            if Self::intersects_lat_edge(a, b, self.lat.lo(), &self.lng) {
                return true;
            }
            if Self::intersects_lat_edge(a, b, self.lat.hi(), &self.lng) {
                return true;
            }
        }
        false
    }

    /// `interiorIntersects(S2LatLngRect)`.
    pub fn interior_intersects(&self, other: &S2LatLngRect) -> bool {
        self.lat.interior_intersects(&other.lat) && self.lng.interior_intersects(&other.lng)
    }

    /// `addPoint(S2Point)`.
    pub fn add_point(&self, p: &S2Point) -> S2LatLngRect {
        self.add_lat_lng(&S2LatLng::from_point(p))
    }

    /// `addPoint(S2LatLng)`.
    pub fn add_lat_lng(&self, ll: &S2LatLng) -> S2LatLngRect {
        let new_lat = self.lat.add_point(ll.lat().radians());
        let new_lng = self.lng.add_point(ll.lng().radians());
        S2LatLngRect::new(new_lat, new_lng)
    }

    /// `expanded(S2LatLng)`.
    pub fn expanded(&self, margin: &S2LatLng) -> S2LatLngRect {
        if self.is_empty() {
            return *self;
        }
        S2LatLngRect::new(
            self.lat.expanded(margin.lat().radians()).intersection(&Self::full_lat()),
            self.lng.expanded(margin.lng().radians()),
        )
    }

    /// `union(S2LatLngRect)`.
    pub fn union(&self, other: &S2LatLngRect) -> S2LatLngRect {
        S2LatLngRect::new(self.lat.union(&other.lat), self.lng.union(&other.lng))
    }

    /// `intersection(S2LatLngRect)`.
    pub fn intersection(&self, other: &S2LatLngRect) -> S2LatLngRect {
        let intersect_lat = self.lat.intersection(&other.lat);
        let intersect_lng = self.lng.intersection(&other.lng);
        if intersect_lat.is_empty() || intersect_lng.is_empty() {
            return Self::empty();
        }
        S2LatLngRect::new(intersect_lat, intersect_lng)
    }

    /// `convolveWithCap(S1Angle)`.
    pub fn convolve_with_cap(&self, angle: &S1Angle) -> S2LatLngRect {
        let cap = S2Cap::from_axis_angle(S2Point::new(1.0, 0.0, 0.0), angle);
        let mut r = *self;
        for k in 0..4 {
            let vertex_cap = S2Cap::from_axis_height(self.vertex(k).to_point(), cap.height());
            r = r.union(&vertex_cap.get_rect_bound());
        }
        r
    }

    /// `area()`.
    pub fn area(&self) -> f64 {
        if self.is_empty() {
            return 0.0;
        }
        self.lng().get_length() * (self.lat_hi().radians().sin() - self.lat_lo().radians().sin()).abs()
    }

    /// `approxEquals(S2LatLngRect, double)`.
    pub fn approx_equals_with(&self, other: &S2LatLngRect, max_error: f64) -> bool {
        self.lat.approx_equals_with(&other.lat, max_error)
            && self.lng.approx_equals_with(&other.lng, max_error)
    }

    /// `approxEquals(S2LatLngRect)`.
    pub fn approx_equals(&self, other: &S2LatLngRect) -> bool {
        self.approx_equals_with(other, 1e-15)
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut value: i32 = 17;
        value = 37i32.wrapping_mul(value).wrapping_add(self.lat.hash_code());
        37i32.wrapping_mul(value).wrapping_add(self.lng.hash_code())
    }

    /// `clone()`: `new S2LatLngRect(lo(), hi())`.
    pub fn clone_region(&self) -> S2LatLngRect {
        S2LatLngRect::from_lo_hi(&self.lo(), &self.hi())
    }

    /// `contains(S2Point)`.
    pub fn contains_point(&self, p: &S2Point) -> bool {
        self.contains_lat_lng(&S2LatLng::from_point(p))
    }

    fn intersects_lng_edge(a: &S2Point, b: &S2Point, lat: &R1Interval, lng: f64) -> bool {
        S2::simple_crossing(
            a,
            b,
            &S2LatLng::from_radians(lat.lo(), lng).to_point(),
            &S2LatLng::from_radians(lat.hi(), lng).to_point(),
        )
    }

    fn intersects_lat_edge(a: &S2Point, b: &S2Point, lat: f64, lng: &S1Interval) -> bool {
        let mut z = S2Point::normalize(&S2::robust_cross_prod(a, b));
        if z.z < 0.0 {
            z = S2Point::neg(&z);
        }
        let y = S2Point::normalize(&S2::robust_cross_prod(&z, &S2Point::new(0.0, 0.0, 1.0)));
        let x = S2Point::cross_prod(&y, &z);
        let sin_lat = lat.sin();
        if sin_lat.abs() >= x.z {
            return false; // The great circle does not reach the given latitude.
        }
        let cos_theta = sin_lat / x.z;
        let sin_theta = (1.0 - cos_theta * cos_theta).sqrt();
        let theta = sin_theta.atan2(cos_theta);
        let ab_theta = S1Interval::from_point_pair(
            a.dot_prod(&y).atan2(a.dot_prod(&x)),
            b.dot_prod(&y).atan2(b.dot_prod(&x)),
        );
        if ab_theta.contains_point(theta) {
            let isect = S2Point::add(&S2Point::mul(&x, cos_theta), &S2Point::mul(&y, sin_theta));
            if lng.contains_point(isect.y.atan2(isect.x)) {
                return true;
            }
        }
        if ab_theta.contains_point(-theta) {
            let intersection =
                S2Point::sub(&S2Point::mul(&x, cos_theta), &S2Point::mul(&y, sin_theta));
            if lng.contains_point(intersection.y.atan2(intersection.x)) {
                return true;
            }
        }
        false
    }
}

impl S2Region for S2LatLngRect {
    /// `getCapBound()`.
    fn get_cap_bound(&self) -> S2Cap {
        if self.is_empty() {
            return S2Cap::empty();
        }
        let (pole_z, pole_angle) = if self.lat.lo() + self.lat.hi() < 0.0 {
            (-1.0, S2::M_PI_2 + self.lat.hi())
        } else {
            (1.0, S2::M_PI_2 - self.lat.lo())
        };
        let pole_cap = S2Cap::from_axis_angle(
            S2Point::new(0.0, 0.0, pole_z),
            &S1Angle::from_radians(pole_angle),
        );
        let lng_span = self.lng.hi() - self.lng.lo();
        if ieee_remainder(lng_span, 2.0 * S2::M_PI) >= 0.0 && lng_span < 2.0 * S2::M_PI {
            let mut mid_cap =
                S2Cap::from_axis_angle(self.get_center().to_point(), &S1Angle::from_radians(0.0));
            for k in 0..4 {
                mid_cap = mid_cap.add_point(&self.vertex(k).to_point());
            }
            if mid_cap.height() < pole_cap.height() {
                return mid_cap;
            }
        }
        pole_cap
    }

    /// `getRectBound()`.
    fn get_rect_bound(&self) -> S2LatLngRect {
        *self
    }

    /// `contains(S2Cell)`.
    fn contains_cell(&self, cell: &S2Cell) -> bool {
        self.contains(&cell.get_rect_bound())
    }

    /// `mayIntersect(S2Cell)`.
    fn may_intersect(&self, cell: &S2Cell) -> bool {
        self.intersects(&cell.get_rect_bound())
    }
}

impl PartialEq for S2LatLngRect {
    /// `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.lat() == other.lat() && self.lng() == other.lng()
    }
}

impl Hash for S2LatLngRect {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for S2LatLngRect {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[Lo={}, Hi={}]", self.lo(), self.hi())
    }
}
