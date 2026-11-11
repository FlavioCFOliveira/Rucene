/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.geo.GeoTestUtil`: random geo shapes for
//! tests.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `Math.cos`/`Math.sin` (`createRegularPolygon`, `surpriseMePolygon`):
//!   the JVM's intrinsics and Rust's `f64::cos`/`f64::sin` are both within
//!   one ulp of the exact result, and may differ in the last bit; the draws
//!   (and the order of them) do not depend on those bits except at the
//!   boundary checks of `surpriseMePolygon`.
//! * `toSVG(Object...)`: Java dispatches on the runtime class of each
//!   argument; the port takes [`SvgObject`]s, one variant per class Java
//!   handles (`Polygon`, `Polygon[]`, `Rectangle`, `double[]` point).
//! * `readShape(String)`: Java reads a classpath resource of the package;
//!   the port embeds the same resources (`src/geo/resources/`, copied from
//!   `lucene/test-framework/src/resources/org/apache/lucene/tests/geo/`).

use std::io::Read;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::RandomizedContext;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::geo::{Circle, GeoUtils, Line, Point, Polygon, Rectangle};
use rucene::util::java_fmt::double_to_string;
use rucene::util::sloppy_math::SloppyMath;
use rucene::util::NumericUtils;

use crate::jdk::gzip::GzipInputStream;
use crate::util::TestUtil;

/// `random()`: `RandomizedContext.current().getRandom()`.
fn random() -> Arc<dyn Random> {
    RandomizedContext::current().get_random()
}

/// Throws the `AssertionError` of a Java `assert` with a message.
fn java_assert(condition: bool, message: impl FnOnce() -> String) {
    randomizedtesting::jdk::exception::java_assert(condition, message);
}

/// Rethrows an error as the Java exception it stands for.
fn rethrow(e: LuceneError) -> ! {
    crate::util::java_exceptions::rethrow_java(e)
}

/// `Math.nextAfter(start, direction)`.
pub fn next_after(start: f64, direction: f64) -> f64 {
    if start.is_nan() || direction.is_nan() {
        f64::NAN
    } else if start == direction {
        direction
    } else if direction > start {
        start.next_up()
    } else {
        start.next_down()
    }
}

/// `Math.nextAfter(float start, double direction)`: the overload Java
/// selects for an `int` `start`, since `int` widens to `float` and `float`
/// is more specific than `double`; the `float` result widens to `double` at
/// the call site.
pub fn next_after_float(start: f32, direction: f64) -> f32 {
    let start_d = f64::from(start);
    if start_d > direction {
        start.next_down()
    } else if start_d < direction {
        start.next_up()
    } else if start_d == direction {
        direction as f32
    } else {
        // At least one is NaN.
        start + direction as f32
    }
}

/// `nextLatitude()`: returns next pseudorandom latitude (anywhere).
pub fn next_latitude() -> f64 {
    next_double_internal(GeoUtils::MIN_LAT_INCL, GeoUtils::MAX_LAT_INCL)
}

/// `nextLongitude()`: returns next pseudorandom longitude (anywhere).
pub fn next_longitude() -> f64 {
    next_double_internal(GeoUtils::MIN_LON_INCL, GeoUtils::MAX_LON_INCL)
}

/// `nextDoubleInternal(double low, double high)` (`private`): returns a
/// double within the closed range `[low, high]`, biased towards the edges.
fn next_double_internal(low: f64, high: f64) -> f64 {
    java_assert(low >= f64::from(i32::MIN), String::new);
    java_assert(high <= f64::from(i32::MAX), String::new);
    java_assert(low.is_finite(), String::new);
    java_assert(high.is_finite(), String::new);
    java_assert(high >= low, || {
        format!(
            "low={} high={}",
            double_to_string(low),
            double_to_string(high)
        )
    });

    // if they are equal, not much we can do
    if low == high {
        return low;
    }

    // first pick a base value.
    let base_value;
    let surprise_me = random().next_int_bound(17);
    if surprise_me == 0 {
        // random bits
        let low_bits = NumericUtils::double_to_sortable_long(low);
        let high_bits = NumericUtils::double_to_sortable_long(high);
        base_value = NumericUtils::sortable_long_to_double(TestUtil::next_long(
            &*random(),
            low_bits,
            high_bits,
        ));
    } else if surprise_me == 1 {
        // edge case
        base_value = low;
    } else if surprise_me == 2 {
        // edge case
        base_value = high;
    } else if surprise_me == 3 && low <= 0.0 && high >= 0.0 {
        // may trigger divide by 0
        base_value = 0.0;
    } else if surprise_me == 4 {
        // divide up space into block of 360
        let delta = (high - low) / 360.0;
        let block = random().next_int_bound(360);
        base_value = low + delta * f64::from(block);
    } else {
        // distributed ~ evenly
        base_value = low + (high - low) * random().next_double();
    }

    java_assert(base_value >= low, String::new);
    java_assert(base_value <= high, String::new);

    // either return the base value or adjust it by 1 ulp in a random direction (if possible)
    let adjust_me = random().next_int_bound(17);
    // `Math.nextAfter(adjustMe, high)` with an `int` `adjustMe` is the
    // `nextAfter(float, double)` overload (`GeoTestUtil.java:111-114`).
    if adjust_me == 0 {
        f64::from(next_after_float(adjust_me as f32, high))
    } else if adjust_me == 1 {
        f64::from(next_after_float(adjust_me as f32, low))
    } else {
        base_value
    }
}

/// `nextLatitudeNear(double otherLatitude, double delta)` (`private`):
/// returns a latitude that is near the given latitude.
fn next_latitude_near(other_latitude: f64, delta: f64) -> f64 {
    let delta = delta.abs();
    GeoUtils::check_latitude(other_latitude).unwrap_or_else(|e| rethrow(e));
    let surprise_me = random().next_int_bound(97);
    if surprise_me == 0 {
        // purely random
        next_latitude()
    } else if surprise_me < 49 {
        // upper half of region (the exact point or 1 ulp difference is still likely)
        next_double_internal(other_latitude, 90f64.min(other_latitude + delta))
    } else {
        // lower half of region (the exact point or 1 ulp difference is still likely)
        next_double_internal((-90f64).max(other_latitude - delta), other_latitude)
    }
}

/// `nextLongitudeNear(double otherLongitude, double delta)` (`private`):
/// returns a longitude that is near the given longitude.
fn next_longitude_near(other_longitude: f64, delta: f64) -> f64 {
    let delta = delta.abs();
    GeoUtils::check_longitude(other_longitude).unwrap_or_else(|e| rethrow(e));
    let surprise_me = random().next_int_bound(97);
    if surprise_me == 0 {
        // purely random
        next_longitude()
    } else if surprise_me < 49 {
        // upper half of region (the exact point or 1 ulp difference is still likely)
        next_double_internal(other_longitude, 180f64.min(other_longitude + delta))
    } else {
        // lower half of region (the exact point or 1 ulp difference is still likely)
        next_double_internal((-180f64).max(other_longitude - delta), other_longitude)
    }
}

/// `nextLatitudeBetween(double minLatitude, double maxLatitude)`
/// (`private`): returns a latitude within the closed range, with some
/// overshoot.
fn next_latitude_between(min_latitude: f64, max_latitude: f64) -> f64 {
    java_assert(max_latitude >= min_latitude, String::new);
    GeoUtils::check_latitude(min_latitude).unwrap_or_else(|e| rethrow(e));
    GeoUtils::check_latitude(max_latitude).unwrap_or_else(|e| rethrow(e));
    if random().next_int_bound(47) == 0 {
        // purely random
        next_latitude()
    } else {
        // extend the range by 1%
        let difference = (max_latitude - min_latitude) / 100.0;
        let lower = (-90f64).max(min_latitude - difference);
        let upper = 90f64.min(max_latitude + difference);
        next_double_internal(lower, upper)
    }
}

/// `nextLongitudeBetween(double minLongitude, double maxLongitude)`
/// (`private`): returns a longitude within the closed range, with some
/// overshoot.
fn next_longitude_between(min_longitude: f64, max_longitude: f64) -> f64 {
    java_assert(max_longitude >= min_longitude, String::new);
    GeoUtils::check_longitude(min_longitude).unwrap_or_else(|e| rethrow(e));
    GeoUtils::check_longitude(max_longitude).unwrap_or_else(|e| rethrow(e));
    if random().next_int_bound(47) == 0 {
        // purely random
        next_longitude()
    } else {
        // extend the range by 1%
        let difference = (max_longitude - min_longitude) / 100.0;
        let lower = (-180f64).max(min_longitude - difference);
        let upper = 180f64.min(max_longitude + difference);
        next_double_internal(lower, upper)
    }
}

/// `nextPointAroundLine(double lat1, double lon1, double lat2, double
/// lon2)` (`private`): returns a point (lat/lon) near a line segment.
fn next_point_around_line(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> [f64; 2] {
    let x1 = lon1;
    let x2 = lon2;
    let y1 = lat1;
    let y2 = lat2;
    let min_x = x1.min(x2);
    let max_x = x1.max(x2);
    let min_y = y1.min(y2);
    let max_y = y1.max(y2);
    if min_x == max_x {
        // Java evaluates the array elements left to right.
        let lat = next_latitude_between(min_y, max_y);
        let lon = next_longitude_near(min_x, 0.01 * (max_y - min_y));
        [lat, lon]
    } else if min_y == max_y {
        let lat = next_latitude_near(min_y, 0.01 * (max_x - min_x));
        let lon = next_longitude_between(min_x, max_x);
        [lat, lon]
    } else {
        // choose a point on the line
        let x = next_longitude_between(min_x, max_x);
        let mut y = (y1 - y2) / (x1 - x2) * (x - x1) + y1;
        if !y.is_finite() {
            // this can happen due to underflow when delta between x values is wonderfully tiny!
            y = 90f64.copysign(x1);
        }
        let delta = (max_y - min_y) * 0.01;
        // our formula may put the point outside of the box, so restrict to the box
        y = 90f64.min(y);
        y = (-90f64).max(y);
        [next_latitude_near(y, delta), x]
    }
}

/// `nextPointNear(Rectangle rectangle)`: returns next pseudorandom point
/// (lat/lon) near the box.
pub fn next_point_near_rectangle(rectangle: &Rectangle) -> [f64; 2] {
    if rectangle.crosses_dateline() {
        // pick a "side" of the two boxes we really are
        if random().next_boolean() {
            next_point_near_rectangle(&rect(
                rectangle.min_lat(),
                rectangle.max_lat(),
                -180.0,
                rectangle.max_lon(),
            ))
        } else {
            next_point_near_rectangle(&rect(
                rectangle.min_lat(),
                rectangle.max_lat(),
                rectangle.min_lon(),
                180.0,
            ))
        }
    } else {
        next_point_near(&box_polygon(rectangle))
    }
}

/// `new Rectangle(minLat, maxLat, minLon, maxLon)`.
fn rect(min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> Rectangle {
    Rectangle::new(min_lat, max_lat, min_lon, max_lon).unwrap_or_else(|e| rethrow(e))
}

/// `new Polygon(lats, lons)`.
fn polygon(lats: Vec<f64>, lons: Vec<f64>) -> Polygon {
    Polygon::new(lats, lons, Vec::new()).unwrap_or_else(|e| rethrow(e))
}

/// `nextPointNear(Polygon polygon)`: returns next pseudorandom point
/// (lat/lon) near the polygon.
pub fn next_point_near(polygon: &Polygon) -> [f64; 2] {
    let poly_lats = polygon.get_polylats();
    let poly_lons = polygon.get_polylons();
    let holes = polygon.get_holes();

    // if there are any holes, target them aggressively
    if !holes.is_empty() && random().next_int_bound(3) == 0 {
        return next_point_near(&holes[random().next_int_bound(holes.len() as i32) as usize]);
    }

    let surprise_me = random().next_int_bound(97);
    if surprise_me == 0 {
        // purely random
        let lat = next_latitude();
        let lon = next_longitude();
        [lat, lon]
    } else if surprise_me < 5 {
        // purely random within bounding box
        let lat = next_latitude_between(polygon.min_lat(), polygon.max_lat());
        let lon = next_longitude_between(polygon.min_lon(), polygon.max_lon());
        [lat, lon]
    } else if surprise_me < 20 {
        // target a vertex
        let vertex = random().next_int_bound(poly_lats.len() as i32 - 1) as usize;
        let lat = next_latitude_near(poly_lats[vertex], poly_lats[vertex + 1] - poly_lats[vertex]);
        let lon = next_longitude_near(poly_lons[vertex], poly_lons[vertex + 1] - poly_lons[vertex]);
        [lat, lon]
    } else if surprise_me < 30 {
        // target points around the bounding box edges
        let container = box_polygon(&rect(
            polygon.min_lat(),
            polygon.max_lat(),
            polygon.min_lon(),
            polygon.max_lon(),
        ));
        let container_lats = container.get_polylats();
        let container_lons = container.get_polylons();
        let start_vertex = random().next_int_bound(container_lats.len() as i32 - 1) as usize;
        next_point_around_line(
            container_lats[start_vertex],
            container_lons[start_vertex],
            container_lats[start_vertex + 1],
            container_lons[start_vertex + 1],
        )
    } else {
        // target points around diagonals between vertices
        let start_vertex = random().next_int_bound(poly_lats.len() as i32 - 1) as usize;
        // but favor edges heavily
        let end_vertex = if random().next_boolean() {
            start_vertex + 1
        } else {
            random().next_int_bound(poly_lats.len() as i32 - 1) as usize
        };
        next_point_around_line(
            poly_lats[start_vertex],
            poly_lons[start_vertex],
            poly_lats[end_vertex],
            poly_lons[end_vertex],
        )
    }
}

/// `nextBoxNear(Polygon polygon)`: returns next pseudorandom box: does not
/// cross the 180th meridian.
pub fn next_box_near(polygon: &Polygon) -> Rectangle {
    let holes = polygon.get_holes();

    // if there are any holes, target them aggressively
    if !holes.is_empty() && random().next_int_bound(3) == 0 {
        return next_box_near(&holes[random().next_int_bound(holes.len() as i32) as usize]);
    }

    let point1;
    let mut point2 = [0f64; 2];
    let surprise_me = random().next_int_bound(97);
    if surprise_me == 0 {
        // formed from two interesting points
        point1 = next_point_near(polygon);
        point2 = next_point_near(polygon);
    } else {
        // formed from one interesting point: then random within delta.
        point1 = next_point_near(polygon);
        let poly_lats = polygon.get_polylats();
        let poly_lons = polygon.get_polylons();
        let vertex = random().next_int_bound(poly_lats.len() as i32 - 1) as usize;
        let delta_x = poly_lons[vertex + 1] - poly_lons[vertex];
        let delta_y = poly_lats[vertex + 1] - poly_lats[vertex];
        let edge_length = (delta_x * delta_x + delta_y * delta_y).sqrt();
        point2[0] = next_latitude_near(point1[0], edge_length);
        point2[1] = next_longitude_near(point1[1], edge_length);
    }

    // form a box from the two points
    let min_lat = point1[0].min(point2[0]);
    let max_lat = point1[0].max(point2[0]);
    let min_lon = point1[1].min(point2[1]);
    let max_lon = point1[1].max(point2[1]);
    rect(min_lat, max_lat, min_lon, max_lon)
}

/// `nextBox()`: returns next pseudorandom box: can cross the 180th
/// meridian.
pub fn next_box() -> Rectangle {
    next_box_internal(true)
}

/// `nextBoxNotCrossingDateline()`: returns next pseudorandom box: does not
/// cross the 180th meridian.
pub fn next_box_not_crossing_dateline() -> Rectangle {
    next_box_internal(false)
}

/// `createRegularPolygon(double centerLat, double centerLon, double
/// radiusMeters, int gons)`: makes an n-gon, centered at the provided
/// lat/lon, and each vertex approximately distanceMeters away from the
/// center.
///
/// Do not invoke me across the dateline or a pole!!
///
/// # Errors
///
/// `IllegalArgumentException` (from `GeoUtils.checkLatitude`/
/// `checkLongitude`) when a vertex leaves the valid range.
pub fn create_regular_polygon(
    center_lat: f64,
    center_lon: f64,
    radius_meters: f64,
    gons: i32,
) -> Result<Polygon> {
    let gons = gons as usize;
    let mut lats = vec![0f64; gons + 1];
    let mut lons = vec![0f64; gons + 1];
    for i in 0..gons {
        // 0 to 360
        let angle = 360.0 - i as f64 * (360.0 / gons as f64);
        let x = angle.to_radians().cos();
        let y = angle.to_radians().sin();
        let mut factor = 2.0;
        let mut step = 1.0;
        let mut last = 0;

        // Iterate out along one spoke until we hone in on the point that's nearly exactly
        // radiusMeters from the center:
        loop {
            // TODO: we could in fact cross a pole?  Just do what surpriseMePolygon does?
            let lat = center_lat + y * factor;
            GeoUtils::check_latitude(lat)?;
            let lon = center_lon + x * factor;
            GeoUtils::check_longitude(lon)?;
            let distance_meters = SloppyMath::haversin_meters(center_lat, center_lon, lat, lon);

            if (distance_meters - radius_meters).abs() < 0.1 {
                // Within 10 cm: close enough!
                lats[i] = lat;
                lons[i] = lon;
                break;
            }

            if distance_meters > radius_meters {
                // too big
                factor -= step;
                if last == 1 {
                    step /= 2.0;
                }
                last = -1;
            } else if distance_meters < radius_meters {
                // too small
                factor += step;
                if last == -1 {
                    step /= 2.0;
                }
                last = 1;
            }
        }
    }

    // close poly
    lats[gons] = lats[0];
    lons[gons] = lons[0];

    Polygon::new(lats, lons, Vec::new())
}

/// `nextPoint()`: returns next pseudorandom point.
pub fn next_point() -> Point {
    let lat = next_latitude();
    let lon = next_longitude();
    Point::new(lat, lon).unwrap_or_else(|e| rethrow(e))
}

/// `nextLine()`: returns next pseudorandom line.
pub fn next_line() -> Line {
    let p = next_polygon();
    let n = p.num_points() - 1;
    let lats: Vec<f64> = (0..n).map(|i| p.get_polylat(i)).collect();
    let lons: Vec<f64> = (0..n).map(|i| p.get_polylon(i)).collect();
    Line::new(lats, lons).unwrap_or_else(|e| rethrow(e))
}

/// `nextCircle()`: returns next pseudorandom circle.
pub fn next_circle() -> Circle {
    let lat = next_latitude();
    let lon = next_longitude();
    let radius_meters =
        random().next_double() * GeoUtils::EARTH_MEAN_RADIUS_METERS * std::f64::consts::PI / 2.0
            + 1.0;
    Circle::new(lat, lon, radius_meters).unwrap_or_else(|e| rethrow(e))
}

/// `nextPolygon()`: returns next pseudorandom polygon.
pub fn next_polygon() -> Polygon {
    if random().next_boolean() {
        return surprise_me_polygon();
    } else if random().next_int_bound(10) == 1 {
        // this poly is slow to create ... only do it 10% of the time:
        loop {
            let gons = TestUtil::next_int(&*random(), 4, 500);
            let radius_meters =
                random().next_double() * GeoUtils::EARTH_MEAN_RADIUS_METERS * std::f64::consts::PI
                    / 2.0
                    + 1.0;
            let lat = next_latitude();
            let lon = next_longitude();
            match create_regular_polygon(lat, lon, radius_meters, gons) {
                Ok(p) => return p,
                Err(LuceneError {
                    kind: LuceneErrorKind::IllegalArgument { .. },
                    ..
                }) => {
                    // we tried to cross dateline or pole ... try again
                }
                Err(e) => rethrow(e),
            }
        }
    }

    let r#box = next_box_internal(false);
    if random().next_boolean() {
        // box
        box_polygon(&r#box)
    } else {
        // triangle
        triangle_polygon(&r#box)
    }
}

/// `nextBoxInternal(boolean canCrossDateLine)` (`private`).
fn next_box_internal(can_cross_date_line: bool) -> Rectangle {
    // prevent lines instead of boxes
    let mut lat0 = next_latitude();
    let mut lat1 = next_latitude();
    while lat0 == lat1 {
        lat1 = next_latitude();
    }
    // prevent lines instead of boxes
    let mut lon0 = next_longitude();
    let mut lon1 = next_longitude();
    while lon0 == lon1 {
        lon1 = next_longitude();
    }

    if lat1 < lat0 {
        std::mem::swap(&mut lat0, &mut lat1);
    }

    if !can_cross_date_line && lon1 < lon0 {
        std::mem::swap(&mut lon0, &mut lon1);
    }

    rect(lat0, lat1, lon0, lon1)
}

/// `boxPolygon(Rectangle box)` (`private`).
fn box_polygon(r#box: &Rectangle) -> Polygon {
    java_assert(!r#box.crosses_dateline(), String::new);
    polygon(
        vec![
            r#box.min_lat(),
            r#box.max_lat(),
            r#box.max_lat(),
            r#box.min_lat(),
            r#box.min_lat(),
        ],
        vec![
            r#box.min_lon(),
            r#box.min_lon(),
            r#box.max_lon(),
            r#box.max_lon(),
            r#box.min_lon(),
        ],
    )
}

/// `trianglePolygon(Rectangle box)` (`private`).
fn triangle_polygon(r#box: &Rectangle) -> Polygon {
    java_assert(!r#box.crosses_dateline(), String::new);
    polygon(
        vec![
            r#box.min_lat(),
            r#box.max_lat(),
            r#box.max_lat(),
            r#box.min_lat(),
        ],
        vec![
            r#box.min_lon(),
            r#box.min_lon(),
            r#box.max_lon(),
            r#box.min_lon(),
        ],
    )
}

/// `surpriseMePolygon()` (`private`).
fn surprise_me_polygon() -> Polygon {
    // repeat until we get a poly that doesn't cross dateline:
    'new_poly: loop {
        let center_lat = next_latitude();
        let center_lon = next_longitude();
        let radius = 0.1 + 20.0 * random().next_double();
        let radius_delta = random().next_double();

        let mut lats: Vec<f64> = Vec::new();
        let mut lons: Vec<f64> = Vec::new();
        let mut angle = 0.0f64;
        loop {
            angle += random().next_double() * 40.0;
            if angle > 360.0 {
                break;
            }
            let len = radius * (1.0 - radius_delta + radius_delta * random().next_double());
            let lat = center_lat + len * angle.to_radians().cos();
            let lon = center_lon + len * angle.to_radians().sin();
            if lon <= GeoUtils::MIN_LON_INCL
                || lon >= GeoUtils::MAX_LON_INCL
                || lat > 90.0
                || lat < -90.0
            {
                // cannot cross dateline or pole: try again!
                continue 'new_poly;
            }
            lats.push(lat);
            lons.push(lon);
        }

        // close it
        lats.push(lats[0]);
        lons.push(lons[0]);

        return polygon(lats, lons);
    }
}

/// An argument of `toSVG(Object...)`, one variant per class Java handles.
pub enum SvgObject<'a> {
    /// A `Polygon`.
    Polygon(&'a Polygon),
    /// A `Polygon[]`, flattened.
    Polygons(&'a [Polygon]),
    /// A `Rectangle`.
    Rectangle(&'a Rectangle),
    /// A `double[]` point (lat, lon).
    Point([f64; 2]),
}

/// `toSVG(Object... objects)`: returns svg of polygon for debugging.
///
/// # Errors
///
/// `IllegalArgumentException("you must pass at least one polygon")`.
pub fn to_svg(objects: &[SvgObject<'_>]) -> Result<String> {
    let mut flattened: Vec<&SvgObject<'_>> = Vec::new();
    let expanded: Vec<SvgObject<'_>> = objects
        .iter()
        .flat_map(|o| match o {
            SvgObject::Polygons(ps) => ps.iter().map(SvgObject::Polygon).collect::<Vec<_>>(),
            SvgObject::Polygon(p) => vec![SvgObject::Polygon(p)],
            SvgObject::Rectangle(r) => vec![SvgObject::Rectangle(r)],
            SvgObject::Point(p) => vec![SvgObject::Point(*p)],
        })
        .collect();
    flattened.extend(expanded.iter());

    // first compute bounding area of all the objects
    let mut min_lat = f64::INFINITY;
    let mut max_lat = f64::NEG_INFINITY;
    let mut min_lon = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    for o in &flattened {
        if let SvgObject::Polygon(p) = o {
            let r = Rectangle::from_polygon(std::slice::from_ref(*p))?;
            min_lat = min_lat.min(r.min_lat());
            max_lat = max_lat.max(r.max_lat());
            min_lon = min_lon.min(r.min_lon());
            max_lon = max_lon.max(r.max_lon());
        }
    }
    if !min_lat.is_finite() || !max_lat.is_finite() || !min_lon.is_finite() || !max_lon.is_finite()
    {
        return Err(LuceneError::illegal_argument(
            "you must pass at least one polygon".to_string(),
        ));
    }

    // add some additional padding so we can really see what happens on the edges too
    let xpadding = (max_lon - min_lon) / 64.0;
    let ypadding = (max_lat - min_lat) / 64.0;
    // expand points to be this large
    let point_x = xpadding * 0.1;
    let point_y = ypadding * 0.1;
    let d = double_to_string;
    let mut sb = String::new();
    sb.push_str(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" height=\"640\" width=\"480\" viewBox=\"",
    );
    sb.push_str(&format!(
        "{} {} {} {}",
        d(min_lon - xpadding),
        d(90.0 - max_lat - ypadding),
        d(max_lon - min_lon + (2.0 * xpadding)),
        d(max_lat - min_lat + (2.0 * ypadding))
    ));
    sb.push_str("\">\n");

    // encode each object
    for o in &flattened {
        // tostring
        match o {
            SvgObject::Point(point) => {
                sb.push_str("<!-- point: ");
                sb.push_str(&format!("{},{}", d(point[0]), d(point[1])));
                sb.push_str(" -->\n");
            }
            SvgObject::Polygon(p) => {
                sb.push_str(&format!("<!-- Polygon: \n{p}\n-->\n"));
            }
            SvgObject::Rectangle(r) => {
                sb.push_str(&format!("<!-- Rectangle: \n{r}\n-->\n"));
            }
            SvgObject::Polygons(_) => {}
        }
        let (gon, style, opacity) = match o {
            SvgObject::Rectangle(r) => (
                box_polygon(r),
                "fill:lightskyblue;stroke:black;stroke-width:0.2%;stroke-dasharray:0.5%,1%;",
                "0.3",
            ),
            SvgObject::Point(point) => (
                box_polygon(&rect(
                    (-90f64).max(point[0] - point_y),
                    90f64.min(point[0] + point_y),
                    (-180f64).max(point[1] - point_x),
                    180f64.min(point[1] + point_x),
                )),
                "fill:red;stroke:red;stroke-width:0.1%;",
                "0.7",
            ),
            SvgObject::Polygon(p) => (
                (*p).clone(),
                "fill:lawngreen;stroke:black;stroke-width:0.3%;",
                "0.5",
            ),
            SvgObject::Polygons(_) => continue,
        };
        // polygon
        let poly_lats = gon.get_polylats();
        let poly_lons = gon.get_polylons();
        sb.push_str(&format!("<polygon fill-opacity=\"{opacity}\" points=\""));
        for i in 0..poly_lats.len() {
            if i > 0 {
                sb.push(' ');
            }
            sb.push_str(&format!("{},{}", d(poly_lons[i]), d(90.0 - poly_lats[i])));
        }
        sb.push_str(&format!("\" style=\"{style}\"/>\n"));
        for hole in gon.get_holes() {
            let hole_lats = hole.get_polylats();
            let hole_lons = hole.get_polylons();
            sb.push_str("<polygon points=\"");
            for i in 0..hole_lats.len() {
                if i > 0 {
                    sb.push(' ');
                }
                sb.push_str(&format!("{},{}", d(hole_lons[i]), d(90.0 - hole_lats[i])));
            }
            sb.push_str("\" style=\"fill:lightgray\"/>\n");
        }
    }
    sb.push_str("</svg>\n");
    Ok(sb)
}

/// `containsSlowly(Polygon polygon, double latitude, double longitude)`:
/// simple slow point in polygon check (for testing).
///
/// # Errors
///
/// `UnsupportedOperationException` for a polygon with holes.
pub fn contains_slowly(polygon: &Polygon, latitude: f64, longitude: f64) -> Result<bool> {
    if !polygon.get_holes().is_empty() {
        return Err(LuceneError::unsupported_operation(
            "this testing method does not support holes".to_string(),
        ));
    }
    let poly_lats = polygon.get_polylats();
    let poly_lons = polygon.get_polylons();
    // bounding box check required due to rounding errors (we don't solve that problem)
    if latitude < polygon.min_lat()
        || latitude > polygon.max_lat()
        || longitude < polygon.min_lon()
        || longitude > polygon.max_lon()
    {
        return Ok(false);
    }

    let mut c = false;
    let nvert = poly_lats.len();
    let verty = poly_lats;
    let vertx = poly_lons;
    let testy = latitude;
    let testx = longitude;
    let mut i = 0;
    let mut j = 1;
    while j < nvert {
        if testy == verty[j] && testy == verty[i]
            || ((testy <= verty[j] && testy >= verty[i])
                != (testy >= verty[j] && testy <= verty[i]))
        {
            if (testx == vertx[j] && testx == vertx[i])
                || ((testx <= vertx[j] && testx >= vertx[i])
                    != (testx >= vertx[j] && testx <= vertx[i])
                    && GeoUtils::orient(vertx[i], verty[i], vertx[j], verty[j], testx, testy) == 0)
            {
                // return true if point is on boundary
                return Ok(true);
            } else if ((verty[i] > testy) != (verty[j] > testy))
                && (testx
                    < (vertx[j] - vertx[i]) * (testy - verty[i]) / (verty[j] - verty[i]) + vertx[i])
            {
                c = !c;
            }
        }
        i += 1;
        j += 1;
    }
    Ok(c)
}

/// The resources of `org/apache/lucene/tests/geo/` the port embeds.
static RESOURCES: &[(&str, &[u8])] = &[
    (
        "github-11986-1.geojson.gz",
        include_bytes!("resources/github-11986-1.geojson.gz"),
    ),
    (
        "github-11986-2.geojson.gz",
        include_bytes!("resources/github-11986-2.geojson.gz"),
    ),
    (
        "github-12352-1.geojson.gz",
        include_bytes!("resources/github-12352-1.geojson.gz"),
    ),
    (
        "github-12352-2.geojson.gz",
        include_bytes!("resources/github-12352-2.geojson.gz"),
    ),
    (
        "github-13841-1.geojson.gz",
        include_bytes!("resources/github-13841-1.geojson.gz"),
    ),
    (
        "github-13841-2.wkt.gz",
        include_bytes!("resources/github-13841-2.wkt.gz"),
    ),
    (
        "github-13841-3.wkt.gz",
        include_bytes!("resources/github-13841-3.wkt.gz"),
    ),
    (
        "github-15205.geojson.gz",
        include_bytes!("resources/github-15205.geojson.gz"),
    ),
    (
        "github-15657.geojson.gz",
        include_bytes!("resources/github-15657.geojson.gz"),
    ),
    (
        "github-15695.geojson.gz",
        include_bytes!("resources/github-15695.geojson.gz"),
    ),
    (
        "github-15731.geojson.gz",
        include_bytes!("resources/github-15731.geojson.gz"),
    ),
    (
        "lucene-10470-2.geojson.gz",
        include_bytes!("resources/lucene-10470-2.geojson.gz"),
    ),
    (
        "lucene-10470-3.geojson.gz",
        include_bytes!("resources/lucene-10470-3.geojson.gz"),
    ),
    (
        "lucene-10470.geojson.gz",
        include_bytes!("resources/lucene-10470.geojson.gz"),
    ),
    (
        "lucene-10470.wkt.gz",
        include_bytes!("resources/lucene-10470.wkt.gz"),
    ),
    (
        "lucene-10563-1.geojson.gz",
        include_bytes!("resources/lucene-10563-1.geojson.gz"),
    ),
    (
        "lucene-10563-2.geojson.gz",
        include_bytes!("resources/lucene-10563-2.geojson.gz"),
    ),
    (
        "lucene-10563-3.geojson.gz",
        include_bytes!("resources/lucene-10563-3.geojson.gz"),
    ),
    (
        "lucene-9251.wkt.gz",
        include_bytes!("resources/lucene-9251.wkt.gz"),
    ),
    (
        "lucene-9417.geojson.gz",
        include_bytes!("resources/lucene-9417.geojson.gz"),
    ),
    (
        "lucene-9538-invalid.geojson.gz",
        include_bytes!("resources/lucene-9538-invalid.geojson.gz"),
    ),
];

/// `readShape(String name)`: reads a shape from a shape resource (`Loader.
/// readShape`): the resource's lines, gunzipped when its name ends with
/// `.gz`, concatenated without line terminators.
///
/// # Errors
///
/// `FileNotFoundException("classpath resource not found: " + name)`, or the
/// I/O error of the decompression.
pub fn read_shape(name: &str) -> Result<String> {
    let bytes = RESOURCES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, b)| *b)
        .ok_or_else(|| {
            LuceneError::from(LuceneErrorKind::Io(
                rucene::store::java_nio_file::JavaIoException::new(
                    "java.io.FileNotFoundException",
                    Some(format!("classpath resource not found: {name}")),
                )
                .into_io_error(),
            ))
        })?;
    let mut data = Vec::new();
    if name.ends_with(".gz") {
        GzipInputStream::new(bytes)?.read_to_end(&mut data)?;
    } else {
        data.extend_from_slice(bytes);
    }
    let text = String::from_utf8_lossy(&data);
    // BufferedReader.lines(): \n, \r and \r\n end a line
    let mut builder = String::new();
    for line in text.split('\n') {
        builder.push_str(line.strip_suffix('\r').unwrap_or(line));
    }
    Ok(builder)
}

#[cfg(test)]
mod next_after_tests {
    use super::*;

    /// rmp #358 (`TestGeoPointSuiteDraws`, `testSamePointManyTimes`):
    /// `nextDoubleInternal` calls `Math.nextAfter(adjustMe, ...)` with an
    /// `int`, which Java resolves to `nextAfter(float, double)`
    /// (`GeoTestUtil.java:111-114`); the port used the `double` overload and
    /// produced `0.9999999999999999` where Java produces `0.9999999403953552`.
    /// Expected values from JDK 25: `Math.nextAfter(1, -90.0)`,
    /// `Math.nextAfter(0, 90.0)`, `Math.nextAfter(1, 1.0)`.
    #[test]
    fn next_after_an_int_is_the_float_overload() {
        assert_eq!(f64::from(next_after_float(1.0, -90.0)), 0.9999999403953552);
        assert_eq!(
            f64::from(next_after_float(0.0, 90.0)),
            1.401298464324817e-45
        );
        assert_eq!(f64::from(next_after_float(1.0, 1.0)), 1.0);
    }
}
