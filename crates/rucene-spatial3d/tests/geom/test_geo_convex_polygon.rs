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

//! Port of `TestGeoConvexPolygon.java`.

use crate::spatial3d_support::prelude::*;

/// `TestGeoConvexPolygon`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestGeoConvexPolygon",
        vec![
            test("testPolygonPointWithin", test_polygon_point_within).annotate(Test::new()),
            test("testPolygonBounds", test_polygon_bounds).annotate(Test::new()),
        ],
    )
}

fn test_polygon_point_within() -> Result<()> {
    let mut c: GeoConvexPolygon;
    let mut gp: GeoPoint;
    c = GeoConvexPolygon::new_start(&SPHERE, -0.1, -0.5)?;
    c.add_point(0.0, -0.6, false)?;
    c.add_point(0.1, -0.5, false)?;
    c.add_point(0.0, -0.4, false)?;
    c.done(false)?;
    // Sample some points within
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.5)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.55)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.45)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.05, -0.5)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.05, -0.5)?;
    Assert::assert_true(c.is_within(&gp));
    // Sample some nearby points outside, and compute distance-to-shape for them as well
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.65)?;
    Assert::assert_false(c.is_within(&gp));
    Assert::assert_equals_double_msg(None, 0.05, c.compute_outside_distance(&ARC, &gp)?, 1e-12);
    Assert::assert_equals_double_msg(None, 0.05, c.compute_outside_distance(&NORMAL, &gp)?, 1e-3);
    Assert::assert_equals_double_msg(None, 0.05, c.compute_outside_distance(&LINEAR, &gp)?, 1e-3);
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.35)?;
    Assert::assert_false(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.15, -0.5)?;
    Assert::assert_false(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.15, -0.5)?;
    Assert::assert_false(c.is_within(&gp));
    // Random points outside
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.0)?;
    Assert::assert_false(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, PI * 0.5, 0.0)?;
    Assert::assert_false(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, PI)?;
    Assert::assert_false(c.is_within(&gp));
    Ok(())
}

fn test_polygon_bounds() -> Result<()> {
    let mut c: GeoConvexPolygon;
    let mut b: LatLonBounds;
    c = GeoConvexPolygon::new_start(&SPHERE, -0.1, -0.5)?;
    c.add_point(0.0, -0.6, false)?;
    c.add_point(0.1, -0.5, false)?;
    c.add_point(0.0, -0.4, false)?;
    c.done(false)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, -0.6, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.4, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.1, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.1, unbox(b.get_max_latitude()), 0.000001);
    Ok(())
}
