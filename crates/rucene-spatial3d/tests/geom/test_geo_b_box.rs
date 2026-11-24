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

//! Port of `TestGeoBBox.java`.

use crate::spatial3d_support::prelude::*;

/// `TestGeoBBox`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestGeoBBox",
        vec![
            test("testBBoxDegenerate", test_b_box_degenerate).annotate(Test::new()),
            test("testBBoxPointWithin", test_b_box_point_within).annotate(Test::new()),
            test("testBBoxExpand", test_b_box_expand).annotate(Test::new()),
            test("testBBoxBounds", test_b_box_bounds).annotate(Test::new()),
            test("testFailureCase1", test_failure_case1).annotate(Test::new()),
            test("testFailureCase2", test_failure_case2).annotate(Test::new()),
            test("testLUCENE10508", test_lucene10508).annotate(Test::new()),
            test("testBBoxRandomDegenerate", test_b_box_random_degenerate).annotate(Test::new()),
            test("testBBoxLatDegenerate", test_b_box_lat_degenerate).annotate(Test::new()),
            test(
                "testBBoxRandomLatDegenerate",
                test_b_box_random_lat_degenerate,
            )
            .annotate(Test::new()),
            test(
                "testBBoxRandomLonDegenerate",
                test_b_box_random_lon_degenerate,
            )
            .annotate(Test::new()),
        ],
    )
}

const DEGREES_TO_RADIANS: f64 = PI / 180.0;

fn test_b_box_degenerate() -> Result<()> {
    let mut r#box: Arc<dyn GeoBBox>;
    let mut relationship: i32;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(GeoPoint::new_lat_lon(
        &SPHERE,
        (-49.0) * DEGREES_TO_RADIANS,
        (-176.0) * DEGREES_TO_RADIANS,
    )?);
    points.push(GeoPoint::new_lat_lon(
        &SPHERE,
        (-11.0) * DEGREES_TO_RADIANS,
        101.0 * DEGREES_TO_RADIANS,
    )?);
    points.push(GeoPoint::new_lat_lon(
        &SPHERE,
        24.0 * DEGREES_TO_RADIANS,
        (-30.0) * DEGREES_TO_RADIANS,
    )?);
    let shape: Arc<dyn GeoMembershipShape> = npe(GeoPolygonFactory::make_geo_polygon(
        &SPHERE,
        points.clone(),
    )?)?;
    r#box = GeoBBoxFactory::make_geo_bbox(
        &SPHERE,
        (-64.0) * DEGREES_TO_RADIANS,
        (-64.0) * DEGREES_TO_RADIANS,
        (-180.0) * DEGREES_TO_RADIANS,
        180.0 * DEGREES_TO_RADIANS,
    )?;
    relationship = r#box.get_relationship(&*shape)?;
    Assert::assert_equals(CONTAINS, relationship);
    r#box = GeoBBoxFactory::make_geo_bbox(
        &SPHERE,
        (-61.85) * DEGREES_TO_RADIANS,
        (-67.5) * DEGREES_TO_RADIANS,
        (-180.0) * DEGREES_TO_RADIANS,
        (-168.75) * DEGREES_TO_RADIANS,
    )?;
    // System.out.println("Shape = " + shape + " Rect = " + box);
    relationship = r#box.get_relationship(&*shape)?;
    Assert::assert_equals(CONTAINS, relationship);
    Ok(())
}

fn test_b_box_point_within() -> Result<()> {
    let mut r#box: Arc<dyn GeoBBox>;
    let mut gp: GeoPoint;
    // Standard normal Rect box, not crossing dateline
    r#box = GeoBBoxFactory::make_geo_bbox(&SPHERE, 0.0, (-PI) * 0.25, -1.0, 1.0)?;
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, 0.0)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.1, 0.0)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.5, 0.0)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, 1.1)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, -1.1)?;
    Assert::assert_false(r#box.is_within(&gp));
    Assert::assert_equals_double_msg(None, 0.1, r#box.compute_outside_distance(&ARC, &gp)?, 1e-2);
    Assert::assert_equals_double_msg(
        None,
        0.1,
        r#box.compute_outside_distance(&NORMAL, &gp)?,
        1e-2,
    );
    Assert::assert_equals_double_msg(
        None,
        0.1,
        r#box.compute_outside_distance(&NORMAL, &gp)?,
        1e-2,
    );
    // Standard normal Rect box, crossing dateline
    r#box = GeoBBoxFactory::make_geo_bbox(&SPHERE, 0.0, (-PI) * 0.25, PI - 1.0, (-PI) + 1.0)?;
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, -PI)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.1, -PI)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.5, -PI)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, (-PI) + 1.1)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, ((-PI) - 1.1) + PI * 2.0)?;
    Assert::assert_false(r#box.is_within(&gp));
    // Latitude zone rectangle
    r#box = GeoBBoxFactory::make_geo_bbox(&SPHERE, 0.0, (-PI) * 0.25, -PI, PI)?;
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, -PI)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.1, -PI)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.5, -PI)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, (-PI) + 1.1)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, ((-PI) - 1.1) + PI * 2.0)?;
    Assert::assert_true(r#box.is_within(&gp));
    // World
    r#box = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, -PI, PI)?;
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, -PI)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.1, -PI)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.5, -PI)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, (-PI) + 1.1)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, ((-PI) - 1.1) + PI * 2.0)?;
    Assert::assert_true(r#box.is_within(&gp));
    Ok(())
}

fn test_b_box_expand() -> Result<()> {
    let mut r#box: Arc<dyn GeoBBox>;
    let mut gp: GeoPoint;
    // Standard normal Rect box, not crossing dateline
    r#box = GeoBBoxFactory::make_geo_bbox(&SPHERE, 0.0, (-PI) * 0.25, -1.0, 1.0)?;
    r#box = r#box.clone().expand(0.1)?;
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.05, 0.0)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.15, 0.0)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.25 - 0.05, 0.0)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, (-PI) * 0.25 - 0.15, 0.0)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, -1.05)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, -1.15)?;
    Assert::assert_false(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, 1.05)?;
    Assert::assert_true(r#box.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, -0.1, 1.15)?;
    Assert::assert_false(r#box.is_within(&gp));
    Ok(())
}

fn test_b_box_bounds() -> Result<()> {
    let mut c: Arc<dyn GeoBBox>;
    let mut b: LatLonBounds;
    let mut xyzb: XYZBounds;
    let mut solid: Arc<dyn GeoArea>;
    let mut point: GeoPoint;
    let mut relationship: i32;
    c = GeoBBoxFactory::make_geo_bbox(
        &SPHERE,
        0.7570958596622309,
        -0.7458670829264561,
        -0.9566079379002148,
        1.4802570961901191,
    )?;
    solid = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        0.10922258701604912,
        0.1248184603754517,
        -0.8172414690802067,
        0.9959041483215542,
        -0.6136586624726926,
        0.6821740363641521,
    )?;
    point = GeoPoint::new_lat_lon(&SPHERE, 0.3719987557178081, 1.4529582778845198)?;
    Assert::assert_true(c.is_within(&point));
    Assert::assert_true(solid.is_within(&point));
    relationship = solid.get_relationship(&*c)?;
    Assert::assert_true(
        relationship == OVERLAPS || relationship == CONTAINS || relationship == WITHIN,
    );
    c = GeoBBoxFactory::make_geo_bbox(
        &SPHERE,
        0.006607096847842122,
        -0.002828135860810422,
        -0.0012934461873348349,
        0.006727418645092394,
    )?;
    solid = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        0.9999995988328008,
        1.0000000002328306,
        -0.0012934708508166816,
        0.006727393021214471,
        -0.002828157275369464,
        0.006607074060760007,
    )?;
    point = GeoPoint::new_lat_lon(&SPHERE, -5.236470872437899E-4, 3.992578692654256E-4)?;
    Assert::assert_true(c.is_within(&point));
    Assert::assert_true(solid.is_within(&point));
    relationship = solid.get_relationship(&*c)?;
    Assert::assert_true(
        relationship == OVERLAPS || relationship == CONTAINS || relationship == WITHIN,
    );
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.25, (-PI) * 0.25, -1.0, 1.0)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, -1.0, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, (-PI) * 0.25, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, PI * 0.25, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.382051, unbox(xyzb.get_minimum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(xyzb.get_maximum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.841471, unbox(xyzb.get_minimum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.841471, unbox(xyzb.get_maximum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.707107, unbox(xyzb.get_minimum_z()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.707107, unbox(xyzb.get_maximum_z()), 0.000001);
    let area: Arc<dyn GeoArea> = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        unbox(xyzb.get_minimum_x()) - (2.0 * Vector::MINIMUM_RESOLUTION),
        unbox(xyzb.get_maximum_x()) + 2.0 * Vector::MINIMUM_RESOLUTION,
        unbox(xyzb.get_minimum_y()) - (2.0 * Vector::MINIMUM_RESOLUTION),
        unbox(xyzb.get_maximum_y()) + 2.0 * Vector::MINIMUM_RESOLUTION,
        unbox(xyzb.get_minimum_z()) - (2.0 * Vector::MINIMUM_RESOLUTION),
        unbox(xyzb.get_maximum_z()) + 2.0 * Vector::MINIMUM_RESOLUTION,
    )?;
    Assert::assert_equals(WITHIN, area.get_relationship(&*c)?);
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, 0.0, (-PI) * 0.25, -1.0, 1.0)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, -1.0, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, (-PI) * 0.25, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.382051, unbox(xyzb.get_minimum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(xyzb.get_maximum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.841471, unbox(xyzb.get_minimum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.841471, unbox(xyzb.get_maximum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.707107, unbox(xyzb.get_minimum_z()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(xyzb.get_maximum_z()), 0.000001);
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, 0.0, (-PI) * 0.25, 1.0, -1.0)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    // assertEquals(1.0,b.getLeftLongitude(),0.000001);
    // assertEquals(-1.0,b.getRightLongitude(),0.000001);
    Assert::assert_equals_double_msg(None, (-PI) * 0.25, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -1.0, unbox(xyzb.get_minimum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.540303, unbox(xyzb.get_maximum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, -1.0, unbox(xyzb.get_minimum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(xyzb.get_maximum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.707107, unbox(xyzb.get_minimum_z()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(xyzb.get_maximum_z()), 0.000001);
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, -1.0, 1.0)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    // assertEquals(-1.0, b.getLeftLongitude(), 0.000001);
    // assertEquals(1.0, b.getRightLongitude(), 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(xyzb.get_minimum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(xyzb.get_maximum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.841471, unbox(xyzb.get_minimum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.841471, unbox(xyzb.get_maximum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, -1.0, unbox(xyzb.get_minimum_z()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(xyzb.get_maximum_z()), 0.000001);
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, 1.0, -1.0)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    // assertEquals(1.0,b.getLeftLongitude(),0.000001);
    // assertEquals(-1.0,b.getRightLongitude(),0.000001);
    Assert::assert_equals_double_msg(None, -1.0, unbox(xyzb.get_minimum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.540303, unbox(xyzb.get_maximum_x()), 0.000001);
    Assert::assert_equals_double_msg(None, -1.0, unbox(xyzb.get_minimum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(xyzb.get_maximum_y()), 0.000001);
    Assert::assert_equals_double_msg(None, -1.0, unbox(xyzb.get_minimum_z()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(xyzb.get_maximum_z()), 0.000001);
    // Check wide variants of rectangle and longitude slice
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, 0.0, (-PI) * 0.25, (-PI) + 0.1, PI - 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    // assertEquals(-Math.PI+0.1,b.getLeftLongitude(),0.000001);
    // assertEquals(Math.PI-0.1,b.getRightLongitude(),0.000001);
    Assert::assert_equals_double_msg(None, (-PI) * 0.25, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(b.get_max_latitude()), 0.000001);
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, 0.0, (-PI) * 0.25, PI - 0.1, (-PI) + 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, PI - 0.1, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, (-PI) + 0.1, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, (-PI) * 0.25, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(b.get_max_latitude()), 0.000001);
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, (-PI) + 0.1, PI - 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    // assertEquals(-Math.PI+0.1,b.getLeftLongitude(),0.000001);
    // assertEquals(Math.PI-0.1,b.getRightLongitude(),0.000001);
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, PI - 0.1, (-PI) + 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    // assertEquals(Math.PI - 0.1, b.getLeftLongitude(), 0.000001);
    // assertEquals(-Math.PI + 0.1, b.getRightLongitude(), 0.000001);
    // Check latitude zone
    c = GeoBBoxFactory::make_geo_bbox(&SPHERE, 1.0, -1.0, -PI, PI)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, -1.0, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(b.get_max_latitude()), 0.000001);
    // Now, combine a few things to test the bounds object
    let mut c1: Arc<dyn GeoBBox>;
    let mut c2: Arc<dyn GeoBBox>;
    c1 = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, -PI, 0.0)?;
    c2 = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, 0.0, PI)?;
    b = LatLonBounds::new();
    c1.get_bounds(&mut b);
    c2.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    c1 = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, -PI, 0.0)?;
    c2 = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, 0.0, PI * 0.5)?;
    b = LatLonBounds::new();
    c1.get_bounds(&mut b);
    c2.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    // assertEquals(-Math.PI,b.getLeftLongitude(),0.000001);
    // assertEquals(Math.PI*0.5,b.getRightLongitude(),0.000001);
    c1 = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, (-PI) * 0.5, 0.0)?;
    c2 = GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, (-PI) * 0.5, 0.0, PI)?;
    b = LatLonBounds::new();
    c1.get_bounds(&mut b);
    c2.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    // assertEquals(-Math.PI * 0.5,b.getLeftLongitude(),0.000001);
    // assertEquals(Math.PI,b.getRightLongitude(),0.000001);
    Ok(())
}

fn test_failure_case1() -> Result<()> {
    let point = GeoPoint::new(
        -0.017413370801260174,
        -2.132522881412925E-18,
        0.9976113450663769,
    );
    let r#box: Arc<dyn GeoBBox> = Arc::new(GeoNorthRectangle::new(
        &WGS84,
        0.35451471030934045,
        9.908337057950734E-15,
        2.891004593509811E-11,
    )?);
    let mut bounds = XYZBounds::new();
    r#box.get_bounds(&mut bounds);
    let solid: Arc<dyn XYZSolid> = XYZSolidFactory::make_xyz_solid(
        &WGS84,
        unbox(bounds.get_minimum_x()),
        unbox(bounds.get_maximum_x()),
        unbox(bounds.get_minimum_y()),
        unbox(bounds.get_maximum_y()),
        unbox(bounds.get_minimum_z()),
        unbox(bounds.get_maximum_z()),
    )?;
    Assert::assert_true(if r#box.is_within(&point) {
        solid.is_within(&point)
    } else {
        true
    });
    Ok(())
}

fn test_failure_case2() -> Result<()> {
    // final GeoPoint point = new GeoPoint(-0.7375647084975573, -2.3309121299774915E-10,
    // 0.6746626163258577);
    let point = GeoPoint::new(
        -0.737564708579924,
        -9.032562595264542E-17,
        0.6746626165197899,
    );
    let r#box: Arc<dyn GeoBBox> = Arc::new(GeoRectangle::new(
        &WGS84,
        0.7988584710911523,
        0.25383311815493353,
        -1.2236144735575564E-12,
        7.356011300929654E-49,
    )?);
    let mut bounds = XYZBounds::new();
    r#box.get_bounds(&mut bounds);
    let solid: Arc<dyn XYZSolid> = XYZSolidFactory::make_xyz_solid(
        &WGS84,
        unbox(bounds.get_minimum_x()),
        unbox(bounds.get_maximum_x()),
        unbox(bounds.get_minimum_y()),
        unbox(bounds.get_maximum_y()),
        unbox(bounds.get_minimum_z()),
        unbox(bounds.get_maximum_z()),
    )?;
    // System.out.println("Is within Y value? "+(point.y >= bounds.getMinimumY() && point.y <=
    // bounds.getMaximumY()));
    // System.out.println("Shape = "+box+" is within? "+box.isWithin(point));
    // System.out.println("XYZBounds = "+bounds+" is within? "+solid.isWithin(point)+"
    // solid="+solid);
    Assert::assert_true(r#box.is_within(&point) == solid.is_within(&point));
    Ok(())
}

fn test_lucene10508() -> Result<()> {
    let min_x = Geo3DUtil::from_degrees(GeoTestUtil::next_longitude());
    let max_x = Geo3DUtil::from_degrees(GeoTestUtil::next_longitude());
    let min_y = (-PI) * 0.5;
    let max_y = (-PI) * 0.5 + 1e-8;
    assert_not_null(&GeoAreaFactory::make_geo_area(
        &SPHERE, max_y, min_y, min_x, max_x,
    )?);
    Ok(())
}

fn test_b_box_random_degenerate() -> Result<()> {
    for i in 0..100 {
        let min_x = Geo3DUtil::from_degrees(GeoTestUtil::next_longitude());
        let max_x = java_next_up(min_x + (i) as f64 * Vector::MINIMUM_ANGULAR_RESOLUTION);
        let min_y = Geo3DUtil::from_degrees(GeoTestUtil::next_latitude());
        let max_y = java_next_up(min_y + (i) as f64 * Vector::MINIMUM_ANGULAR_RESOLUTION);
        assert_not_null(&GeoAreaFactory::make_geo_area(
            &SPHERE, max_y, min_y, min_x, max_x,
        )?);
    }
    Ok(())
}

fn test_b_box_lat_degenerate() -> Result<()> {
    let min_x = Geo3DUtil::from_degrees(-180.0);
    let max_x = Geo3DUtil::from_degrees(-174.37500008381903);
    let min_y = Geo3DUtil::from_degrees(89.99999765306711);
    let max_y = Geo3DUtil::from_degrees(89.99999794643372);
    assert_not_null(&GeoAreaFactory::make_geo_area(
        &SPHERE, max_y, min_y, min_x, max_x,
    )?);
    Ok(())
}

fn test_b_box_random_lat_degenerate() -> Result<()> {
    for i in 0..100 {
        let min_x = Geo3DUtil::from_degrees(GeoTestUtil::next_longitude());
        let max_x = Geo3DUtil::from_degrees(GeoTestUtil::next_longitude());
        let min_y = Geo3DUtil::from_degrees(GeoTestUtil::next_latitude());
        let max_y = java_next_up(min_y + (i) as f64 * Vector::MINIMUM_ANGULAR_RESOLUTION);
        assert_not_null(&GeoAreaFactory::make_geo_area(
            &SPHERE, max_y, min_y, min_x, max_x,
        )?);
    }
    Ok(())
}

fn test_b_box_random_lon_degenerate() -> Result<()> {
    for i in 0..100 {
        let min_x = Geo3DUtil::from_degrees(GeoTestUtil::next_longitude());
        let max_x = java_next_up(min_x + (i) as f64 * Vector::MINIMUM_ANGULAR_RESOLUTION);
        let mut min_y = Geo3DUtil::from_degrees(GeoTestUtil::next_latitude());
        let mut max_y = Geo3DUtil::from_degrees(GeoTestUtil::next_latitude());
        if min_y > max_y {
            std::mem::swap(&mut min_y, &mut max_y);
        }
        assert_not_null(&GeoAreaFactory::make_geo_area(
            &SPHERE, max_y, min_y, min_x, max_x,
        )?);
    }
    Ok(())
}
