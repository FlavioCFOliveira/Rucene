/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *    http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The types declared in `DistanceUtils.java`, of the Java package
//! `org.locationtech.spatial4j.distance`.

use rucene::util::java_math;
use std::sync::Arc;

use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;

/// Equivalent to `org.locationtech.spatial4j.distance.DistanceUtils`
/// (`public class`): distance and unit-conversion utilities, mostly for
/// geospatial (lat/lon degrees) contexts.
pub struct DistanceUtils;

impl DistanceUtils {
    /// `DEG_45_AS_RADS` (deprecated).
    pub const DEG_45_AS_RADS: f64 = std::f64::consts::PI / 4.0;
    /// `SIN_45_AS_RADS` (deprecated): `Math.sin(DEG_45_AS_RADS)`, correctly
    /// rounded.
    pub const SIN_45_AS_RADS: f64 = 0.7071067811865475;
    /// `DEG_90_AS_RADS`.
    pub const DEG_90_AS_RADS: f64 = std::f64::consts::PI / 2.0;
    /// `DEG_180_AS_RADS`.
    pub const DEG_180_AS_RADS: f64 = std::f64::consts::PI;
    /// `DEG_225_AS_RADS` (deprecated).
    pub const DEG_225_AS_RADS: f64 = 5.0 * Self::DEG_45_AS_RADS;
    /// `DEG_270_AS_RADS` (deprecated).
    pub const DEG_270_AS_RADS: f64 = 3.0 * Self::DEG_90_AS_RADS;
    /// `DEGREES_TO_RADIANS`.
    pub const DEGREES_TO_RADIANS: f64 = std::f64::consts::PI / 180.0;
    /// `RADIANS_TO_DEGREES`.
    pub const RADIANS_TO_DEGREES: f64 = 1.0 / Self::DEGREES_TO_RADIANS;
    /// `KM_TO_MILES`.
    pub const KM_TO_MILES: f64 = 0.621371192;
    /// `MILES_TO_KM`.
    pub const MILES_TO_KM: f64 = 1.0 / Self::KM_TO_MILES; //1.609
    /// `EARTH_MEAN_RADIUS_KM`: the mean radius of the WGS84 ellipsoid.
    pub const EARTH_MEAN_RADIUS_KM: f64 = 6371.0087714;
    /// `EARTH_EQUATORIAL_RADIUS_KM`.
    pub const EARTH_EQUATORIAL_RADIUS_KM: f64 = 6378.1370;
    /// `DEG_TO_KM`: the kilometres in a degree, at the mean radius.
    pub const DEG_TO_KM: f64 = Self::DEGREES_TO_RADIANS * Self::EARTH_MEAN_RADIUS_KM;
    /// `KM_TO_DEG`.
    pub const KM_TO_DEG: f64 = 1.0 / Self::DEG_TO_KM;
    /// `EARTH_MEAN_RADIUS_MI`.
    pub const EARTH_MEAN_RADIUS_MI: f64 = Self::EARTH_MEAN_RADIUS_KM * Self::KM_TO_MILES;
    /// `EARTH_EQUATORIAL_RADIUS_MI`.
    pub const EARTH_EQUATORIAL_RADIUS_MI: f64 =
        Self::EARTH_EQUATORIAL_RADIUS_KM * Self::KM_TO_MILES;

    /// `vectorDistance(double[] vec1, double[] vec2, double power)`
    /// (deprecated).
    pub fn vector_distance(vec1: &[f64], vec2: &[f64], power: f64) -> f64 {
        let one_over_power = if power == 0.0 || power == 1.0 || power == 2.0 {
            f64::NAN
        } else {
            1.0 / power
        };
        Self::vector_distance_with(vec1, vec2, power, one_over_power)
    }

    /// `vectorDistance(double[] vec1, double[] vec2, double power, double
    /// oneOverPower)` (deprecated).
    pub fn vector_distance_with(
        vec1: &[f64],
        vec2: &[f64],
        power: f64,
        one_over_power: f64,
    ) -> f64 {
        let mut result = 0.0;
        if power == 0.0 {
            for i in 0..vec1.len() {
                result += if vec1[i] - vec2[i] == 0.0 { 0.0 } else { 1.0 };
            }
        } else if power == 1.0 {
            // Manhattan
            for i in 0..vec1.len() {
                result += (vec1[i] - vec2[i]).abs();
            }
        } else if power == 2.0 {
            // Cartesian
            result = java_math::sqrt(Self::dist_squared_cartesian(vec1, vec2));
        } else if power == f64::from(i32::MAX) || power.is_infinite() {
            //infinite norm?
            for i in 0..vec1.len() {
                result = f64::max(result, f64::max(vec1[i], vec2[i]));
            }
        } else {
            for i in 0..vec1.len() {
                result += java_math::pow(vec1[i] - vec2[i], power);
            }
            result = java_math::pow(result, one_over_power);
        }
        result
    }

    /// `vectorBoxCorner(double[] center, double[] result, double distance,
    /// boolean upperRight)` (deprecated).
    pub fn vector_box_corner(
        center: &[f64],
        result: Option<Vec<f64>>,
        mut distance: f64,
        upper_right: bool,
    ) -> Vec<f64> {
        let mut result = match result {
            Some(r) if r.len() == center.len() => r,
            _ => vec![0.0; center.len()],
        };
        if !upper_right {
            distance = -distance;
        }
        //We don't care about the power here,
        // b/c we are always in a rectangular coordinate system, so any norm can be used by
        //using the definition of sine
        distance *= Self::SIN_45_AS_RADS; // sin(Pi/4) == (2^0.5)/2 == opp/hyp == opp/distance, solve for opp, similarly for cosine
        for i in 0..center.len() {
            result[i] = center[i] + distance;
        }
        result
    }

    /// `pointOnBearingRAD(double startLat, double startLon, double
    /// distanceRAD, double bearingRAD, SpatialContext ctx, Point reuse)`: the
    /// result in radians, `x` the longitude.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makePoint`.
    pub fn point_on_bearing_rad(
        start_lat: f64,
        start_lon: f64,
        distance_rad: f64,
        bearing_rad: f64,
        ctx: &SpatialContext,
        reuse: Option<Arc<dyn Point>>,
    ) -> Result<Arc<dyn Point>> {
        /*
          lat2 = asin(sin(lat1)*cos(d/R) + cos(lat1)*sin(d/R)*cos(θ))
          lon2 = lon1 + atan2(sin(θ)*sin(d/R)*cos(lat1), cos(d/R)−sin(lat1)*sin(lat2))
        */
        let cos_ang_dist = java_math::cos(distance_rad);
        let cos_start_lat = java_math::cos(start_lat);
        let sin_ang_dist = java_math::sin(distance_rad);
        let sin_start_lat = java_math::sin(start_lat);
        let sin_lat2 = sin_start_lat * cos_ang_dist
            + cos_start_lat * sin_ang_dist * java_math::cos(bearing_rad);
        let mut lat2 = java_math::asin(sin_lat2);
        let mut lon2 = start_lon
            + java_math::atan2(
                java_math::sin(bearing_rad) * sin_ang_dist * cos_start_lat,
                cos_ang_dist - sin_start_lat * sin_lat2,
            );

        // normalize lon first
        if lon2 > Self::DEG_180_AS_RADS {
            lon2 = -(Self::DEG_180_AS_RADS - (lon2 - Self::DEG_180_AS_RADS)); // Java: -1 * (...)
        } else if lon2 < -Self::DEG_180_AS_RADS {
            lon2 = (lon2 + Self::DEG_180_AS_RADS) + Self::DEG_180_AS_RADS;
        }

        // normalize lat - could flip poles
        if lat2 > Self::DEG_90_AS_RADS {
            lat2 = Self::DEG_90_AS_RADS - (lat2 - Self::DEG_90_AS_RADS);
            if lon2 < 0.0 {
                lon2 += Self::DEG_180_AS_RADS;
            } else {
                lon2 -= Self::DEG_180_AS_RADS;
            }
        } else if lat2 < -Self::DEG_90_AS_RADS {
            lat2 = -Self::DEG_90_AS_RADS - (lat2 + Self::DEG_90_AS_RADS);
            if lon2 < 0.0 {
                lon2 += Self::DEG_180_AS_RADS;
            } else {
                lon2 -= Self::DEG_180_AS_RADS;
            }
        }

        match reuse {
            None => ctx.make_point(lon2, lat2),
            Some(reuse) => {
                reuse.reset(lon2, lat2); //x y
                Ok(reuse)
            }
        }
    }

    /// `normLonDEG(double)`: puts the longitude in the range of -180 to
    /// 180.
    pub fn norm_lon_deg(lon_deg: f64) -> f64 {
        if (-180.0..=180.0).contains(&lon_deg) {
            return lon_deg; //common case, and avoids slight double precision shifting
        }
        let off = (lon_deg + 180.0) % 360.0;
        if off < 0.0 {
            180.0 + off
        } else if off == 0.0 && lon_deg > 0.0 {
            180.0
        } else {
            -180.0 + off
        }
    }

    /// `normLatDEG(double)`: puts the latitude in the range of -90 to 90.
    pub fn norm_lat_deg(lat_deg: f64) -> f64 {
        if (-90.0..=90.0).contains(&lat_deg) {
            return lat_deg; //common case, and avoids slight double precision shifting
        }
        let off = ((lat_deg + 90.0) % 360.0).abs();
        (if off <= 180.0 { off } else { 360.0 - off }) - 90.0
    }

    /// `calcBoxByDistFromPtDEG(double lat, double lon, double distDEG,
    /// SpatialContext ctx, Rectangle reuse)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makeRectangle`.
    pub fn calc_box_by_dist_from_pt_deg(
        lat: f64,
        lon: f64,
        dist_deg: f64,
        ctx: &SpatialContext,
        reuse: Option<Arc<dyn Rectangle>>,
    ) -> Result<Arc<dyn Rectangle>> {
        //See http://janmatuschek.de/LatitudeLongitudeBoundingCoordinates Section 3.1, 3.2 and 3.3
        let (min_x, max_x, mut min_y, mut max_y);
        if dist_deg == 0.0 {
            min_x = lon;
            max_x = lon;
            min_y = lat;
            max_y = lat;
        } else if dist_deg >= 180.0 {
            //distance is >= opposite side of the globe
            min_x = -180.0;
            max_x = 180.0;
            min_y = -90.0;
            max_y = 90.0;
        } else {
            //--calc latitude bounds
            max_y = lat + dist_deg;
            min_y = lat - dist_deg;

            if max_y >= 90.0 || min_y <= -90.0 {
                //touches either pole
                //we have special logic for longitude
                let (mut lo_x, mut hi_x) = (-180.0, 180.0); //world wrap: 360 deg
                if max_y <= 90.0 && min_y >= -90.0 {
                    //doesn't pass either pole: 180 deg
                    lo_x = Self::norm_lon_deg(lon - 90.0);
                    hi_x = Self::norm_lon_deg(lon + 90.0);
                }
                if max_y > 90.0 {
                    max_y = 90.0;
                }
                if min_y < -90.0 {
                    min_y = -90.0;
                }
                min_x = lo_x;
                max_x = hi_x;
            } else {
                //--calc longitude bounds
                let lon_delta_deg =
                    Self::calc_box_by_dist_from_pt_delta_lon_deg(lat, lon, dist_deg);
                min_x = Self::norm_lon_deg(lon - lon_delta_deg);
                max_x = Self::norm_lon_deg(lon + lon_delta_deg);
            }
        }
        match reuse {
            None => ctx.make_rectangle(min_x, max_x, min_y, max_y),
            Some(reuse) => {
                reuse.reset(min_x, max_x, min_y, max_y);
                Ok(reuse)
            }
        }
    }

    /// `calcBoxByDistFromPt_deltaLonDEG(double lat, double lon, double
    /// distDEG)`.
    pub fn calc_box_by_dist_from_pt_delta_lon_deg(lat: f64, _lon: f64, dist_deg: f64) -> f64 {
        //http://gis.stackexchange.com/questions/19221/find-tangent-point-on-circle-furthest-east-or-west
        if dist_deg == 0.0 {
            return 0.0;
        }
        let lat_rad = Self::to_radians(lat);
        let dist_rad = Self::to_radians(dist_deg);
        let result_rad = java_math::asin(java_math::sin(dist_rad) / java_math::cos(lat_rad));
        if !result_rad.is_nan() {
            return Self::to_degrees(result_rad);
        }
        90.0
    }

    /// `calcBoxByDistFromPt_latHorizAxisDEG(double lat, double lon, double
    /// distDEG)`.
    pub fn calc_box_by_dist_from_pt_lat_horiz_axis_deg(lat: f64, _lon: f64, dist_deg: f64) -> f64 {
        //http://gis.stackexchange.com/questions/19221/find-tangent-point-on-circle-furthest-east-or-west
        if dist_deg == 0.0 {
            return lat;
        } else if lat + dist_deg >= 90.0 {
            return 90.0;
        } else if lat - dist_deg <= -90.0 {
            return -90.0;
        }
        let lat_rad = Self::to_radians(lat);
        let dist_rad = Self::to_radians(dist_deg);
        let result_rad = java_math::asin(java_math::sin(lat_rad) / java_math::cos(dist_rad));
        if !result_rad.is_nan() {
            return Self::to_degrees(result_rad);
        }
        //handle NaN (shouldn't happen due to checks earlier)
        if lat > 0.0 {
            return 90.0;
        }
        if lat < 0.0 {
            return -90.0;
        }
        lat //0
    }

    /// `calcLonDegreesAtLat(double lat, double dist)`.
    pub fn calc_lon_degrees_at_lat(lat: f64, dist: f64) -> f64 {
        //This code was pulled out of DistanceUtils.pointOnBearingRAD() and
        // optimized
        // for bearing = 90 degrees, and so we can get just the longitude delta
        // TODO reuse pointOnBearing via extract method.

        //(convert inputs to radians)
        let distance_rad = Self::to_radians(dist);
        let start_lat = Self::to_radians(lat);

        let cos_ang_dist = java_math::cos(distance_rad);
        let cos_start_lat = java_math::cos(start_lat);
        let sin_ang_dist = java_math::sin(distance_rad);
        let sin_start_lat = java_math::sin(start_lat);

        let lon_delta = java_math::atan2(
            sin_ang_dist * cos_start_lat,
            cos_ang_dist * (1.0 - sin_start_lat * sin_start_lat),
        );

        Self::to_degrees(lon_delta)
    }

    /// `distSquaredCartesian(double[] vec1, double[] vec2)` (deprecated).
    pub fn dist_squared_cartesian(vec1: &[f64], vec2: &[f64]) -> f64 {
        let mut result = 0.0;
        for i in 0..vec1.len() {
            let v = vec1[i] - vec2[i];
            result += v * v;
        }
        result
    }

    /// `distHaversineRAD(double lat1, double lon1, double lat2, double lon2)`.
    pub fn dist_haversine_rad(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        //TODO investigate slightly different formula using asin() and min() http://www.movable-type.co.uk/scripts/gis-faq-5.1.html

        // Check for same position
        if lat1 == lat2 && lon1 == lon2 {
            return 0.0;
        }
        let hsin_x = java_math::sin((lon1 - lon2) * 0.5);
        let hsin_y = java_math::sin((lat1 - lat2) * 0.5);
        let mut h =
            hsin_y * hsin_y + (java_math::cos(lat1) * java_math::cos(lat2) * hsin_x * hsin_x);
        if h > 1.0 {
            //numeric robustness issue. If we didn't check, the answer would be NaN!
            h = 1.0;
        }
        2.0 * java_math::atan2(java_math::sqrt(h), java_math::sqrt(1.0 - h))
    }

    /// `distLawOfCosinesRAD(double lat1, double lon1, double lat2, double
    /// lon2)`.
    pub fn dist_law_of_cosines_rad(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        //(MIGRATED FROM org.apache.lucene.spatial.geometry.LatLng.arcDistance()
        // (Lucene 3x)

        // Check for same position
        if lat1 == lat2 && lon1 == lon2 {
            return 0.0;
        }

        // Get the longitude difference. Don't need to worry about
        // crossing dateline since cos(x) = cos(-x)
        let d_lon = lon2 - lon1;

        let cos_b = (java_math::sin(lat1) * java_math::sin(lat2))
            + (java_math::cos(lat1) * java_math::cos(lat2) * java_math::cos(d_lon));

        // Find angle subtended (with some bounds checking) in radians
        if cos_b < -1.0 {
            std::f64::consts::PI
        } else if cos_b >= 1.0 {
            0.0
        } else {
            java_math::acos(cos_b)
        }
    }

    /// `distVincentyRAD(double lat1, double lon1, double lat2, double lon2)`.
    pub fn dist_vincenty_rad(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        // Check for same position
        if lat1 == lat2 && lon1 == lon2 {
            return 0.0;
        }

        let cos_lat1 = java_math::cos(lat1);
        let cos_lat2 = java_math::cos(lat2);
        let sin_lat1 = java_math::sin(lat1);
        let sin_lat2 = java_math::sin(lat2);
        let d_lon = lon2 - lon1;
        let cos_d_lon = java_math::cos(d_lon);
        let sin_d_lon = java_math::sin(d_lon);

        let a = cos_lat2 * sin_d_lon;
        let b = cos_lat1 * sin_lat2 - sin_lat1 * cos_lat2 * cos_d_lon;
        let c = sin_lat1 * sin_lat2 + cos_lat1 * cos_lat2 * cos_d_lon;

        java_math::atan2(java_math::sqrt(a * a + b * b), c)
    }

    /// `dist2Degrees(double dist, double radius)`.
    pub fn dist2_degrees(dist: f64, radius: f64) -> f64 {
        Self::to_degrees(Self::dist2_radians(dist, radius))
    }

    /// `degrees2Dist(double degrees, double radius)`.
    pub fn degrees2_dist(degrees: f64, radius: f64) -> f64 {
        Self::radians2_dist(Self::to_radians(degrees), radius)
    }

    /// `dist2Radians(double dist, double radius)`.
    pub fn dist2_radians(dist: f64, radius: f64) -> f64 {
        dist / radius
    }

    /// `radians2Dist(double radians, double radius)`.
    pub fn radians2_dist(radians: f64, radius: f64) -> f64 {
        radians * radius
    }

    /// `toRadians(double)`: `degrees * DEGREES_TO_RADIANS`.
    pub fn to_radians(degrees: f64) -> f64 {
        degrees * Self::DEGREES_TO_RADIANS
    }

    /// `toDegrees(double)`: `radians * RADIANS_TO_DEGREES`.
    pub fn to_degrees(radians: f64) -> f64 {
        radians * Self::RADIANS_TO_DEGREES
    }
}
