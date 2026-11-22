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

//! The types declared in `S2LatLng.java`, of the Java package
//! `com.google.common.geometry`.

use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::util::java_fmt::double_to_string;

use crate::s1_angle::S1Angle;
use crate::s2::{double_to_long_bits, ieee_remainder, S2};
use crate::s2_point::S2Point;

/// Equivalent to `com.google.common.geometry.S2LatLng` (`public strictfp
/// class`): a point on the unit sphere as a latitude-longitude pair.
#[derive(Clone, Copy, Debug, Default)]
pub struct S2LatLng {
    lat_radians: f64,
    lng_radians: f64,
}

impl S2LatLng {
    /// `EARTH_RADIUS_METERS`.
    pub const EARTH_RADIUS_METERS: f64 = 6367000.0;
    /// `CENTER`.
    pub const CENTER: S2LatLng = S2LatLng { lat_radians: 0.0, lng_radians: 0.0 };

    /// `fromRadians(double, double)`.
    pub fn from_radians(lat_radians: f64, lng_radians: f64) -> S2LatLng {
        S2LatLng { lat_radians, lng_radians }
    }

    /// `fromDegrees(double, double)`.
    pub fn from_degrees(lat_degrees: f64, lng_degrees: f64) -> S2LatLng {
        S2LatLng::new(&S1Angle::from_degrees(lat_degrees), &S1Angle::from_degrees(lng_degrees))
    }

    /// `fromE5(long, long)`.
    pub fn from_e5(lat_e5: i64, lng_e5: i64) -> S2LatLng {
        S2LatLng::new(&S1Angle::from_e5(lat_e5), &S1Angle::from_e5(lng_e5))
    }

    /// `fromE6(long, long)`.
    pub fn from_e6(lat_e6: i64, lng_e6: i64) -> S2LatLng {
        S2LatLng::new(&S1Angle::from_e6(lat_e6), &S1Angle::from_e6(lng_e6))
    }

    /// `fromE7(long, long)`.
    pub fn from_e7(lat_e7: i64, lng_e7: i64) -> S2LatLng {
        S2LatLng::new(&S1Angle::from_e7(lat_e7), &S1Angle::from_e7(lng_e7))
    }

    /// `latitude(S2Point)`.
    pub fn latitude(p: &S2Point) -> S1Angle {
        S1Angle::from_radians(
            p.get(2).atan2((p.get(0) * p.get(0) + p.get(1) * p.get(1)).sqrt()),
        )
    }

    /// `longitude(S2Point)`.
    pub fn longitude(p: &S2Point) -> S1Angle {
        S1Angle::from_radians(p.get(1).atan2(p.get(0)))
    }

    /// `S2LatLng(S1Angle lat, S1Angle lng)`.
    pub fn new(lat: &S1Angle, lng: &S1Angle) -> S2LatLng {
        Self::from_radians(lat.radians(), lng.radians())
    }

    /// `S2LatLng(S2Point)`.
    pub fn from_point(p: &S2Point) -> S2LatLng {
        Self::from_radians(p.z.atan2((p.x * p.x + p.y * p.y).sqrt()), p.y.atan2(p.x))
    }

    /// `lat()`.
    pub fn lat(&self) -> S1Angle {
        S1Angle::from_radians(self.lat_radians)
    }

    /// `latRadians()`.
    pub fn lat_radians(&self) -> f64 {
        self.lat_radians
    }

    /// `latDegrees()`.
    pub fn lat_degrees(&self) -> f64 {
        180.0 / std::f64::consts::PI * self.lat_radians
    }

    /// `lng()`.
    pub fn lng(&self) -> S1Angle {
        S1Angle::from_radians(self.lng_radians)
    }

    /// `lngRadians()`.
    pub fn lng_radians(&self) -> f64 {
        self.lng_radians
    }

    /// `lngDegrees()`.
    pub fn lng_degrees(&self) -> f64 {
        180.0 / std::f64::consts::PI * self.lng_radians
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        self.lat().radians().abs() <= S2::M_PI_2 && self.lng().radians().abs() <= S2::M_PI
    }

    /// `normalized()`.
    pub fn normalized(&self) -> S2LatLng {
        S2LatLng::from_radians(
            (-S2::M_PI_2).max(S2::M_PI_2.min(self.lat().radians())),
            ieee_remainder(self.lng().radians(), 2.0 * S2::M_PI),
        )
    }

    /// `toPoint()`.
    pub fn to_point(&self) -> S2Point {
        let phi = self.lat().radians();
        let theta = self.lng().radians();
        let cosphi = phi.cos();
        S2Point::new(theta.cos() * cosphi, theta.sin() * cosphi, phi.sin())
    }

    /// `getDistance(S2LatLng)`.
    pub fn get_distance(&self, o: &S2LatLng) -> S1Angle {
        let lat1 = self.lat().radians();
        let lat2 = o.lat().radians();
        let lng1 = self.lng().radians();
        let lng2 = o.lng().radians();
        let dlat = (0.5 * (lat2 - lat1)).sin();
        let dlng = (0.5 * (lng2 - lng1)).sin();
        let x = dlat * dlat + dlng * dlng * lat1.cos() * lat2.cos();
        S1Angle::from_radians(2.0 * x.sqrt().atan2(0.0f64.max(1.0 - x).sqrt()))
    }

    /// `getDistance(S2LatLng, double radius)`.
    pub fn get_distance_with_radius(&self, o: &S2LatLng, radius: f64) -> f64 {
        self.get_distance(o).radians() * radius
    }

    /// `getEarthDistance(S2LatLng)`.
    pub fn get_earth_distance(&self, o: &S2LatLng) -> f64 {
        self.get_distance_with_radius(o, Self::EARTH_RADIUS_METERS)
    }

    /// `add(S2LatLng)`.
    pub fn add(&self, o: &S2LatLng) -> S2LatLng {
        S2LatLng::from_radians(self.lat_radians + o.lat_radians, self.lng_radians + o.lng_radians)
    }

    /// `sub(S2LatLng)`.
    pub fn sub(&self, o: &S2LatLng) -> S2LatLng {
        S2LatLng::from_radians(self.lat_radians - o.lat_radians, self.lng_radians - o.lng_radians)
    }

    /// `mul(double)`.
    pub fn mul(&self, m: f64) -> S2LatLng {
        S2LatLng::from_radians(self.lat_radians * m, self.lng_radians * m)
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut value: i64 = 17;
        value = value.wrapping_add(
            37i64.wrapping_mul(value).wrapping_add(double_to_long_bits(self.lat_radians)),
        );
        value = value.wrapping_add(
            37i64.wrapping_mul(value).wrapping_add(double_to_long_bits(self.lng_radians)),
        );
        (value ^ ((value as u64) >> 32) as i64) as i32
    }

    /// `approxEquals(S2LatLng, double)`.
    pub fn approx_equals_with(&self, o: &S2LatLng, max_error: f64) -> bool {
        (self.lat_radians - o.lat_radians).abs() < max_error
            && (self.lng_radians - o.lng_radians).abs() < max_error
    }

    /// `approxEquals(S2LatLng)`.
    pub fn approx_equals(&self, o: &S2LatLng) -> bool {
        self.approx_equals_with(o, 1e-9)
    }

    /// `toStringDegrees()`.
    pub fn to_string_degrees(&self) -> String {
        format!(
            "({}, {})",
            double_to_string(self.lat_degrees()),
            double_to_string(self.lng_degrees())
        )
    }
}

impl PartialEq for S2LatLng {
    /// `equals(Object)`.
    fn eq(&self, o: &Self) -> bool {
        self.lat_radians == o.lat_radians && self.lng_radians == o.lng_radians
    }
}

impl Hash for S2LatLng {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

impl fmt::Display for S2LatLng {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "({}, {})",
            double_to_string(self.lat_radians),
            double_to_string(self.lng_radians)
        )
    }
}
