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

//! Port of `TestGeoCircle.java`.

use crate::spatial3d_support::prelude::*;

/// `TestGeoCircle`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestGeoCircle",
        vec![
            test("testCircleDistance", test_circle_distance).annotate(Test::new()),
            test("testCircleFullWorld", test_circle_full_world).annotate(Test::new()),
            test("testCirclePointWithin", test_circle_point_within).annotate(Test::new()),
            test("testCircleBounds", test_circle_bounds).annotate(Test::new()),
            test("testBoundsFailureCase1", test_bounds_failure_case1).annotate(Test::new()),
            test("testBoundsFailureCase2", test_bounds_failure_case2).annotate(Test::new()),
            test("testWholeWorld", test_whole_world).annotate(Test::new()),
        ],
    )
}

fn test_circle_distance() -> Result<()> {
    let c: Arc<dyn GeoCircle>;
    let mut gp: GeoPoint;
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.0, -0.5, 0.1)?;
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.0)?;
    Assert::assert_equals_double_msg(None, f64::INFINITY, c.compute_distance(&ARC, &gp)?, 0.0);
    Assert::assert_equals_double_msg(None, f64::INFINITY, c.compute_distance(&NORMAL, &gp)?, 0.0);
    Assert::assert_equals_double_msg(None, f64::INFINITY, c.compute_distance(&NORMAL, &gp)?, 0.0);
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.5)?;
    Assert::assert_equals_double_msg(None, 0.0, c.compute_distance(&ARC, &gp)?, 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, c.compute_distance(&NORMAL, &gp)?, 0.000001);
    Assert::assert_equals_double_msg(None, 0.0, c.compute_distance(&NORMAL, &gp)?, 0.000001);
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.05, -0.5)?;
    Assert::assert_equals_double_msg(None, 0.05, c.compute_distance(&ARC, &gp)?, 0.000001);
    Assert::assert_equals_double_msg(None, 0.049995, c.compute_distance(&LINEAR, &gp)?, 0.000001);
    Assert::assert_equals_double_msg(None, 0.049979, c.compute_distance(&NORMAL, &gp)?, 0.000001);
    Ok(())
}

fn test_circle_full_world() -> Result<()> {
    let c: Arc<dyn GeoCircle>;
    let mut gp: GeoPoint;
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.0, -0.5, PI)?;
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.0)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.5)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.55)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.45)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, PI * 0.5, 0.0)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, PI)?;
    Assert::assert_true(c.is_within(&gp));
    let mut b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    Ok(())
}

fn test_circle_point_within() -> Result<()> {
    let c: Arc<dyn GeoCircle>;
    let mut gp: GeoPoint;
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.0, -0.5, 0.1)?;
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.0)?;
    Assert::assert_false(c.is_within(&gp));
    Assert::assert_equals_double_msg(None, 0.4, c.compute_outside_distance(&ARC, &gp)?, 1e-12);
    Assert::assert_equals_double_msg(None, 0.12, c.compute_outside_distance(&NORMAL, &gp)?, 0.01);
    Assert::assert_equals_double_msg(None, 0.4, c.compute_outside_distance(&LINEAR, &gp)?, 0.01);
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.5)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.55)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, -0.45)?;
    Assert::assert_true(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, PI * 0.5, 0.0)?;
    Assert::assert_false(c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&SPHERE, 0.0, PI)?;
    Assert::assert_false(c.is_within(&gp));
    Ok(())
}

// The Java test assigns values it never reads.
#[allow(unused_assignments)]
fn test_circle_bounds() -> Result<()> {
    let mut c: Arc<dyn GeoCircle>;
    let mut b: LatLonBounds;
    let mut xyzb: XYZBounds;
    let mut area: Arc<dyn GeoArea>;
    let mut p1: GeoPoint;
    let mut p2: GeoPoint;
    let mut relationship: i32;
    // ...
    c = GeoCircleFactory::make_geo_circle(
        &WGS84,
        -0.005931145568901605,
        -0.001942031539653079,
        1.2991918568260272E-4,
    )?;
    area = GeoAreaFactory::make_geo_area_xyz(
        &WGS84,
        1.001098377143621,
        1.001100011578687,
        -0.00207467080358696,
        -0.0018136665346280983,
        -0.006067808248760161,
        -0.005807683665759485,
    )?;
    p1 = GeoPoint::new_lat_lon(&WGS84, -0.00591253844632244, -0.0020069187259065093)?;
    p2 = GeoPoint::new(
        1.001099185736782,
        -0.0020091272069679327,
        -0.005919118245803968,
    );
    Assert::assert_true(c.is_within(&p1));
    Assert::assert_true(area.is_within(&p1));
    relationship = area.get_relationship(&*c)?;
    Assert::assert_true(relationship != DISJOINT);
    // Twelfth BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &WGS84,
        -0.00824379317765984,
        -0.0011677469001838581,
        0.0011530035396910402,
    )?;
    p1 = GeoPoint::new_lat_lon(&WGS84, -0.006505092992723671, 0.007654282718327381)?;
    p2 = GeoPoint::new(
        1.0010681673665647,
        0.007662608264336381,
        -0.006512324005914593,
    );
    Assert::assert_true(!c.is_within(&p1));
    Assert::assert_true(!c.is_within(&p2));
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &WGS84,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    relationship = area.get_relationship(&*c)?;
    Assert::assert_true(relationship == OVERLAPS || relationship == WITHIN);
    // Eleventh BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        -0.004431288600558495,
        -0.003687846671278374,
        1.704543429364245E-8,
    )?;
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    // System.err.println(area);
    relationship = area.get_relationship(&*c)?;
    Assert::assert_true(WITHIN == relationship || OVERLAPS == relationship);
    // Tenth BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &WGS84,
        -0.0018829770647349636,
        -0.001969499061382591,
        1.3045439293158305E-5,
    )?;
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &WGS84,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    // System.err.println(area);
    relationship = area.get_relationship(&*c)?;
    Assert::assert_true(WITHIN == relationship || OVERLAPS == relationship);
    // Ninth BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        -4.211990380885122E-5,
        -0.0022958453508173044,
        1.4318475623498535E-5,
    )?;
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    // System.err.println(area);
    relationship = area.get_relationship(&*c)?;
    Assert::assert_true(WITHIN == relationship || OVERLAPS == relationship);
    // Eighth BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        0.005321278689117842,
        -0.00216937368755372,
        1.5306034422500785E-4,
    )?;
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    // System.err.println(area);
    relationship = area.get_relationship(&*c)?;
    Assert::assert_true(WITHIN == relationship || OVERLAPS == relationship);
    // Seventh BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        -0.0021627146783861745,
        -0.0017298167021592304,
        2.0818312293195752E-4,
    )?;
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    // System.err.println(area);
    relationship = area.get_relationship(&*c)?;
    Assert::assert_true(WITHIN == relationship || OVERLAPS == relationship);
    // Sixth BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &WGS84,
        -0.006450320645814321,
        0.004660694205115142,
        0.00489710732634323,
    )?;
    // xyzb = new XYZBounds();
    // zScaling.getBounds(xyzb);
    // System.err.println("xmin="+xyzb.getMinimumX()+",
    // xmax="+xyzb.getMaximumX()+",ymin="+xyzb.getMinimumY()+",
    // ymax="+xyzb.getMaximumY()+",zmin="+xyzb.getMinimumZ()+", zmax="+xyzb.getMaximumZ());
    // xmin=1.0010356621420726, xmax=1.0011141249179447,ymin=-2.5326643901354566E-4,
    // ymax=0.009584741915757169,zmin=-0.011359874956269283, zmax=-0.0015549504447452225
    area = GeoAreaFactory::make_geo_area_xyz(
        &WGS84,
        1.0010822580620098,
        1.0010945779732867,
        0.007079167343247293,
        0.007541006774427837,
        -0.0021855011220022575,
        -0.001896122718181518,
    )?;
    Assert::assert_true(CONTAINS != area.get_relationship(&*c)?);
    /*
    p1 = new GeoPoint(1.0010893045436076,0.007380935180644008,-0.002140671370616495);
    // This has a different bounding box, so we can't use it.
    //p2 = new GeoPoint(PlanetModel.WGS84,-0.002164069780096702, 0.007505617500830066);
    p2 = new GeoPoint(PlanetModel.WGS84,p1.getLatitude(),p1.getLongitude());
    assertTrue(PlanetModel.WGS84.pointOnSurface(p2));
    assertTrue(!zScaling.isWithin(p2));
    assertTrue(!area.isWithin(p2));
    assertTrue(!zScaling.isWithin(p1));
    assertTrue(PlanetModel.WGS84.pointOnSurface(p1)); // This fails
    assertTrue(!area.isWithin(p1)); // This fails
     */
    // Fifth BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        -0.004282454525970269,
        -1.6739831367422277E-4,
        1.959639723134033E-6,
    )?;
    Assert::assert_true(c.is_within(&c.get_edge_points()[0]));
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    Assert::assert_true(
        WITHIN == area.get_relationship(&*c)? || OVERLAPS == area.get_relationship(&*c)?,
    );
    // Fourth BKD discovered failure
    c = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        -0.0048795517261255,
        0.004053904306995974,
        5.93699764258874E-6,
    )?;
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    Assert::assert_true(
        WITHIN == area.get_relationship(&*c)? || OVERLAPS == area.get_relationship(&*c)?,
    );
    // Another test case from BKD
    c = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        -0.005955031040627789,
        -0.0029274772647399153,
        1.601488279374338E-5,
    )?;
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    area = GeoAreaFactory::make_geo_area_xyz(
        &SPHERE,
        unbox(xyzb.get_minimum_x()),
        unbox(xyzb.get_maximum_x()),
        unbox(xyzb.get_minimum_y()),
        unbox(xyzb.get_maximum_y()),
        unbox(xyzb.get_minimum_z()),
        unbox(xyzb.get_maximum_z()),
    )?;
    relationship = area.get_relationship(&*c)?;
    Assert::assert_true(relationship == WITHIN || relationship == OVERLAPS);
    // Test case from BKD
    c = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        -0.765816119338,
        0.991848766844,
        0.8153163226330487,
    )?;
    p1 = GeoPoint::new(
        0.7692262265236023,
        -0.055089298115534646,
        -0.6365973465711254,
    );
    Assert::assert_true(c.is_within(&p1));
    xyzb = XYZBounds::new();
    c.get_bounds(&mut xyzb);
    Assert::assert_true(p1.x >= unbox(xyzb.get_minimum_x()) && p1.x <= unbox(xyzb.get_maximum_x()));
    Assert::assert_true(p1.y >= unbox(xyzb.get_minimum_y()) && p1.y <= unbox(xyzb.get_maximum_y()));
    Assert::assert_true(p1.z >= unbox(xyzb.get_minimum_z()) && p1.z <= unbox(xyzb.get_maximum_z()));
    // Vertical circle cases
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.0, -0.5, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, -0.6, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.4, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.1, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.1, unbox(b.get_max_latitude()), 0.000001);
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.0, 0.5, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.4, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.6, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.1, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.1, unbox(b.get_max_latitude()), 0.000001);
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.0, 0.0, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, -0.1, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.1, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.1, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.1, unbox(b.get_max_latitude()), 0.000001);
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.0, PI, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, PI - 0.1, unbox(b.get_left_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, (-PI) + 0.1, unbox(b.get_right_longitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.1, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, 0.1, unbox(b.get_max_latitude()), 0.000001);
    // Horizontal circle cases
    c = GeoCircleFactory::make_geo_circle(&SPHERE, PI * 0.5, 0.0, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_true(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, PI * 0.5 - 0.1, unbox(b.get_min_latitude()), 0.000001);
    c = GeoCircleFactory::make_geo_circle(&SPHERE, (-PI) * 0.5, 0.0, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_true(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_true(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(
        None,
        (-PI) * 0.5 + 0.1,
        unbox(b.get_max_latitude()),
        0.000001,
    );
    // Now do a somewhat tilted plane, facing different directions.
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.01, 0.0, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.11, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.09, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.1, unbox(b.get_left_longitude()), 0.00001);
    Assert::assert_equals_double_msg(None, 0.1, unbox(b.get_right_longitude()), 0.00001);
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.01, PI, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.11, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.09, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, PI - 0.1, unbox(b.get_left_longitude()), 0.00001);
    Assert::assert_equals_double_msg(None, (-PI) + 0.1, unbox(b.get_right_longitude()), 0.00001);
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.01, PI * 0.5, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.11, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.09, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, PI * 0.5 - 0.1, unbox(b.get_left_longitude()), 0.00001);
    Assert::assert_equals_double_msg(
        None,
        PI * 0.5 + 0.1,
        unbox(b.get_right_longitude()),
        0.00001,
    );
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.01, (-PI) * 0.5, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.11, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.09, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(
        None,
        (-PI) * 0.5 - 0.1,
        unbox(b.get_left_longitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        None,
        (-PI) * 0.5 + 0.1,
        unbox(b.get_right_longitude()),
        0.00001,
    );
    // Slightly tilted, PI/4 direction.
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.01, PI * 0.25, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.11, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.09, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(
        None,
        PI * 0.25 - 0.1,
        unbox(b.get_left_longitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        None,
        PI * 0.25 + 0.1,
        unbox(b.get_right_longitude()),
        0.00001,
    );
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.01, (-PI) * 0.25, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.11, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.09, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(
        None,
        (-PI) * 0.25 - 0.1,
        unbox(b.get_left_longitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        None,
        (-PI) * 0.25 + 0.1,
        unbox(b.get_right_longitude()),
        0.00001,
    );
    c = GeoCircleFactory::make_geo_circle(&SPHERE, -0.01, PI * 0.25, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.09, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.11, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(
        None,
        PI * 0.25 - 0.1,
        unbox(b.get_left_longitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        None,
        PI * 0.25 + 0.1,
        unbox(b.get_right_longitude()),
        0.00001,
    );
    c = GeoCircleFactory::make_geo_circle(&SPHERE, -0.01, (-PI) * 0.25, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.09, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.11, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(
        None,
        (-PI) * 0.25 - 0.1,
        unbox(b.get_left_longitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        None,
        (-PI) * 0.25 + 0.1,
        unbox(b.get_right_longitude()),
        0.00001,
    );
    // Now do a somewhat tilted plane.
    c = GeoCircleFactory::make_geo_circle(&SPHERE, 0.01, -0.5, 0.1)?;
    b = LatLonBounds::new();
    c.get_bounds(&mut b);
    Assert::assert_false(b.check_no_longitude_bound());
    Assert::assert_false(b.check_no_top_latitude_bound());
    Assert::assert_false(b.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 0.11, unbox(b.get_max_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.09, unbox(b.get_min_latitude()), 0.000001);
    Assert::assert_equals_double_msg(None, -0.6, unbox(b.get_left_longitude()), 0.00001);
    Assert::assert_equals_double_msg(None, -0.4, unbox(b.get_right_longitude()), 0.00001);
    Ok(())
}

fn test_bounds_failure_case1() -> Result<()> {
    // lat=2.7399499693409367E-13, lon=-3.141592653589793([X=-1.0011188539924791,
    // Y=-1.226017000107956E-16, Z=2.743015573303327E-13])], radius=2.1814042682464985
    let gc: Arc<dyn GeoCircle> = GeoCircleFactory::make_geo_circle(
        &WGS84,
        2.7399499693409367E-13,
        -3.141592653589793,
        2.1814042682464985,
    )?;
    // With a circle like this, zmin should equal zmax, and xmin should be PlanetModel.minimumX.
    let gp = GeoPoint::new(
        0.0054866241253590815,
        -0.004009749293376541,
        0.997739304376186,
    );
    let gp_on_surface = WGS84.create_surface_point(&gp);
    let mut bounds = XYZBounds::new();
    gc.get_bounds(&mut bounds);
    // System.out.println("Bounds: "+bounds);
    let solid: Arc<dyn XYZSolid> = XYZSolidFactory::make_xyz_solid(
        &WGS84,
        unbox(bounds.get_minimum_x()),
        unbox(bounds.get_maximum_x()),
        unbox(bounds.get_minimum_y()),
        unbox(bounds.get_maximum_y()),
        unbox(bounds.get_minimum_z()),
        unbox(bounds.get_maximum_z()),
    )?;
    Assert::assert_true(gc.is_within(&gp_on_surface));
    Assert::assert_true(gc.is_within(&gp));
    Assert::assert_true(solid.is_within(&gp_on_surface));
    // This fails
    Assert::assert_true(solid.is_within(&gp));
    Ok(())
}

fn test_bounds_failure_case2() -> Result<()> {
    let gc: Arc<dyn GeoCircle> = GeoCircleFactory::make_geo_circle(
        &WGS84,
        -2.7574435614238194E-13,
        0.0,
        1.5887859182593391,
    )?;
    let gp = GeoPoint::new_lat_lon(&WGS84, 0.7980359504429014, 1.5964981068121482)?;
    let mut bounds = XYZBounds::new();
    gc.get_bounds(&mut bounds);
    randomizedtesting::jdk::system::out_println(&format!("Bounds = {}", bounds));
    randomizedtesting::jdk::system::out_println(&format!("Point = {}", gp));
    let solid: Arc<dyn XYZSolid> = XYZSolidFactory::make_xyz_solid(
        &WGS84,
        unbox(bounds.get_minimum_x()),
        unbox(bounds.get_maximum_x()),
        unbox(bounds.get_minimum_y()),
        unbox(bounds.get_maximum_y()),
        unbox(bounds.get_minimum_z()),
        unbox(bounds.get_maximum_z()),
    )?;
    java_assert!(if gc.is_within(&gp) {
        solid.is_within(&gp)
    } else {
        true
    });
    Ok(())
}

fn test_whole_world() -> Result<()> {
    let circle1: Arc<dyn GeoCircle> =
        GeoCircleFactory::make_geo_circle(&SPHERE, 0.0, 0.0, 3.1415926535897913)?;
    let circle2: Arc<dyn GeoCircle> =
        GeoCircleFactory::make_geo_circle(&SPHERE, PI * 0.5, PI, 3.141592653589792)?;
    Assert::assert_true(circle1.get_relationship(&*circle2)? == OVERLAPS);
    Ok(())
}
