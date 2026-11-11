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

//! Port of `org.apache.lucene.tests.geo.ShapeTestUtil`: generates random
//! cartesian geometry; heavy reuse of `GeoTestUtil`.
//!
//! **Divergence** (`Math.cos`/`Math.sin`): see
//! [`geo_test_util`](super::geo_test_util).

use std::sync::Arc;

use randomizedtesting::generators::BiasedNumbers;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::RandomizedContext;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::geo::{GeoUtils, XYCircle, XYEncodingUtils, XYLine, XYPoint, XYPolygon, XYRectangle};

use crate::util::lucene_test_case::TEST_NIGHTLY;
use crate::util::TestUtil;

/// `random()`: `RandomizedContext.current().getRandom()`.
fn random() -> Arc<dyn Random> {
    RandomizedContext::current().get_random()
}

/// Rethrows an error as the Java exception it stands for.
fn rethrow(e: LuceneError) -> ! {
    crate::util::java_exceptions::rethrow_java(e)
}

/// `new XYPolygon(x, y)`.
fn xy_polygon(x: Vec<f32>, y: Vec<f32>) -> XYPolygon {
    XYPolygon::new(x, y, Vec::new()).unwrap_or_else(|e| rethrow(e))
}

/// `nextPolygon()`: returns next pseudorandom polygon.
pub fn next_polygon() -> XYPolygon {
    let random = random();
    if random.next_boolean() {
        return surprise_me_polygon(&*random);
    } else if *TEST_NIGHTLY && random.next_int_bound(10) == 1 {
        // this poly is slow to create ... only do it 10% of the time:
        loop {
            let gons = TestUtil::next_int(&*random, 4, 500);
            // So the poly can cover at most 50% of the earth's surface:
            let radius = random.next_double() * 0.5 * f64::from(f32::MAX) + 1.0;
            let center_x = f64::from(next_float(&*random));
            let center_y = f64::from(next_float(&*random));
            match create_regular_polygon(center_x, center_y, radius, gons) {
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

    let r#box = next_box(&*random);
    if random.next_boolean() {
        // box
        box_polygon(&r#box)
    } else {
        // triangle
        triangle_polygon(&r#box)
    }
}

/// `nextXYPoint()`.
pub fn next_xy_point() -> XYPoint {
    let random = random();
    let x = next_float(&*random);
    let y = next_float(&*random);
    XYPoint::new(x, y).unwrap_or_else(|e| rethrow(e))
}

/// `nextLine()`.
pub fn next_line() -> XYLine {
    let poly = next_polygon();
    let n = poly.num_points() - 1;
    let x: Vec<f32> = (0..n).map(|i| poly.get_polyx_at(i)).collect();
    let y: Vec<f32> = (0..n).map(|i| poly.get_polyy_at(i)).collect();
    XYLine::new(x, y).unwrap_or_else(|e| rethrow(e))
}

/// `nextCircle()`.
pub fn next_circle() -> XYCircle {
    let random = random();
    let x = next_float(&*random);
    let y = next_float(&*random);
    let mut radius = 0f32;
    while radius == 0.0 {
        radius = self::random().next_float() * f32::MAX / 2.0;
    }
    randomizedtesting::jdk::exception::java_assert_no_message(radius != 0.0);
    XYCircle::new(x, y, radius).unwrap_or_else(|e| rethrow(e))
}

/// `trianglePolygon(XYRectangle box)` (`private`).
fn triangle_polygon(r#box: &XYRectangle) -> XYPolygon {
    xy_polygon(
        vec![r#box.min_x(), r#box.max_x(), r#box.max_x(), r#box.min_x()],
        vec![r#box.min_y(), r#box.min_y(), r#box.max_y(), r#box.min_y()],
    )
}

/// `nextBox(Random random)`.
pub fn next_box(random: &dyn Random) -> XYRectangle {
    // prevent lines instead of boxes
    let mut x0 = next_float(random);
    let mut x1 = next_float(random);
    while x0 == x1 {
        x1 = next_float(random);
    }
    // prevent lines instead of boxes
    let mut y0 = next_float(random);
    let mut y1 = next_float(random);
    while y0 == y1 {
        y1 = next_float(random);
    }

    if x1 < x0 {
        std::mem::swap(&mut x0, &mut x1);
    }

    if y1 < y0 {
        std::mem::swap(&mut y0, &mut y1);
    }

    XYRectangle::new(x0, x1, y0, y1).unwrap_or_else(|e| rethrow(e))
}

/// `boxPolygon(XYRectangle box)` (`private`).
fn box_polygon(r#box: &XYRectangle) -> XYPolygon {
    xy_polygon(
        vec![
            r#box.min_x(),
            r#box.max_x(),
            r#box.max_x(),
            r#box.min_x(),
            r#box.min_x(),
        ],
        vec![
            r#box.min_y(),
            r#box.min_y(),
            r#box.max_y(),
            r#box.max_y(),
            r#box.min_y(),
        ],
    )
}

/// `surpriseMePolygon(Random random)` (`private`).
fn surprise_me_polygon(random: &dyn Random) -> XYPolygon {
    let center_x = next_float(random);
    let center_y = next_float(random);
    let radius = 0.1 + 20.0 * random.next_double();
    let radius_delta = random.next_double();

    let mut x_list: Vec<f32> = Vec::new();
    let mut y_list: Vec<f32> = Vec::new();
    let mut angle = 0.0f64;
    loop {
        angle += random.next_double() * 40.0;
        if angle > 360.0 {
            break;
        }
        let mut len = radius * (1.0 - radius_delta + radius_delta * random.next_double());
        let max_x = (f32::MAX - center_x)
            .abs()
            .min((-f32::MAX - center_x).abs());
        let max_y = (f32::MAX - center_y)
            .abs()
            .min((-f32::MAX - center_y).abs());

        len = len.min(f64::from(max_x.min(max_y)));

        let x = (f64::from(center_x) + len * angle.to_radians().cos()) as f32;
        let y = (f64::from(center_y) + len * angle.to_radians().sin()) as f32;

        x_list.push(x);
        y_list.push(y);
    }

    // close it
    x_list.push(x_list[0]);
    y_list.push(y_list[0]);

    xy_polygon(x_list, y_list)
}

/// `createRegularPolygon(double centerX, double centerY, double radius, int
/// gons)`: makes an n-gon, centered at the provided x/y, and each vertex
/// approximately distanceMeters away from the center.
///
/// Do not invoke me across the dateline or a pole!!
///
/// # Errors
///
/// What `new XYPolygon` throws for the vertices.
pub fn create_regular_polygon(
    center_x: f64,
    center_y: f64,
    radius: f64,
    gons: i32,
) -> Result<XYPolygon> {
    let max = f64::from(f32::MAX);
    let max_x = (max - center_x).abs().min((-max - center_x).abs());
    let max_y = (max - center_y).abs().min((-max - center_y).abs());

    let radius = radius.min(max_x.min(max_y));

    let gons = gons as usize;
    let mut result0 = vec![0f32; gons + 1];
    let mut result1 = vec![0f32; gons + 1];
    for i in 0..gons {
        // 0 to 360
        let angle = 360.0 - i as f64 * (360.0 / gons as f64);
        let x = angle.to_radians().cos();
        let y = angle.to_radians().sin();
        result0[i] = (center_y + y * radius) as f32;
        result1[i] = (center_x + x * radius) as f32;
    }

    // close poly
    result0[gons] = result0[0];
    result1[gons] = result1[0];

    XYPolygon::new(result0, result1, Vec::new())
}

/// `nextFloat(Random random)`.
pub fn next_float(random: &dyn Random) -> f32 {
    BiasedNumbers::random_float_between(random, -f32::MAX, f32::MAX)
}

/// `containsSlowly(XYPolygon polygon, double x, double y)`: simple slow
/// point in polygon check (for testing).
///
/// # Errors
///
/// `UnsupportedOperationException` for a polygon with holes.
pub fn contains_slowly(polygon: &XYPolygon, x: f64, y: f64) -> Result<bool> {
    if !polygon.get_holes().is_empty() {
        return Err(LuceneError::unsupported_operation(
            "this testing method does not support holes".to_string(),
        ));
    }
    let poly_xs = XYEncodingUtils::float_array_to_double_array(polygon.get_polyx());
    let poly_ys = XYEncodingUtils::float_array_to_double_array(polygon.get_polyy());
    // bounding box check required due to rounding errors (we don't solve that problem)
    if x < f64::from(polygon.min_x())
        || x > f64::from(polygon.max_x())
        || y < f64::from(polygon.min_y())
        || y > f64::from(polygon.max_y())
    {
        return Ok(false);
    }

    let mut c = false;
    let nvert = poly_ys.len();
    let verty = &poly_ys;
    let vertx = &poly_xs;
    let testy = y;
    let testx = x;
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
