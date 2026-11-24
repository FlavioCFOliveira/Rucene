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

//! Port of `TestGeoExactCircle.java`.

use crate::spatial3d_support::prelude::*;

/// `TestGeoExactCircle`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestGeoExactCircle",
        vec![
            test("testExactCircle", test_exact_circle).annotate(Test::new()),
            test(
                "testSurfacePointOnBearingScale",
                test_surface_point_on_bearing_scale,
            )
            .annotate(Test::new()),
            test(
                "RandomPointBearingWGS84Test",
                random_point_bearing_wgs84_test,
            )
            .annotate(Test::new())
            .annotate(Repeat {
                iterations: 100,
                use_constant_seed: false,
            }),
            test(
                "RandomPointBearingCardinalTest",
                random_point_bearing_cardinal_test,
            )
            .annotate(Test::new())
            .annotate(Repeat {
                iterations: 100,
                use_constant_seed: false,
            }),
            test("testExactCircleBounds", test_exact_circle_bounds).annotate(Test::new()),
            test("exactCircleLargeTest", exact_circle_large_test).annotate(Test::new()),
            test("testExactCircleDoesNotFit", test_exact_circle_does_not_fit).annotate(Test::new()),
            test("testBigCircleInSphere", test_big_circle_in_sphere),
            test("testRandomLUCENE8054", test_random_lucene8054)
                .annotate(Test::new())
                .annotate(Repeat {
                    iterations: 100,
                    use_constant_seed: false,
                }),
            test("testLUCENE8054", test_lucene8054).annotate(Test::new()),
            test("testLUCENE8056", test_lucene8056).annotate(Test::new()),
            test("testExactCircleLUCENE8054", test_exact_circle_lucene8054).annotate(Test::new()),
            test("testLUCENE8065", test_lucene8065).annotate(Test::new()),
            test("testLUCENE8080", test_lucene8080),
        ],
    )
}

// The Java test assigns values it never reads.
#[allow(unused_assignments)]
fn test_exact_circle() -> Result<()> {
    let mut c: Arc<dyn GeoCircle>;
    let mut gp: GeoPoint;
    // Construct a variety of circles to see how many actual planes are involved
    c = Arc::new(GeoExactCircle::new(&WGS84, 0.0, 0.0, 0.1, 1e-6)?);
    gp = GeoPoint::new_lat_lon(&WGS84, 0.0, 0.2)?;
    Assert::assert_true(!c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&WGS84, 0.0, 0.0)?;
    Assert::assert_true(c.is_within(&gp));
    c = Arc::new(GeoExactCircle::new(&WGS84, 0.1, 0.0, 0.1, 1e-6)?);
    c = Arc::new(GeoExactCircle::new(&WGS84, 0.2, 0.0, 0.1, 1e-6)?);
    c = Arc::new(GeoExactCircle::new(&WGS84, 0.3, 0.0, 0.1, 1e-6)?);
    c = Arc::new(GeoExactCircle::new(&WGS84, 0.4, 0.0, 0.1, 1e-6)?);
    c = Arc::new(GeoExactCircle::new(&WGS84, PI * 0.5, 0.0, 0.1, 1e-6)?);
    gp = GeoPoint::new_lat_lon(&WGS84, PI * 0.5 - 0.2, 0.0)?;
    Assert::assert_true(!c.is_within(&gp));
    gp = GeoPoint::new_lat_lon(&WGS84, PI * 0.5, 0.0)?;
    Assert::assert_true(c.is_within(&gp));
    Ok(())
}

fn test_surface_point_on_bearing_scale() -> Result<()> {
    let p1: Arc<PlanetModel> = WGS84.clone();
    let p2: Arc<PlanetModel> = Arc::new(PlanetModel::new(
        0.5 * WGS84.xy_scaling,
        0.5 * WGS84.z_scaling,
    ));
    let point1_p1 = GeoPoint::new_lat_lon(&p1, 0.0, 0.0)?;
    let point2_p1 = GeoPoint::new_lat_lon(&p1, 1.0, 1.0)?;
    let point1_p2 =
        GeoPoint::new_lat_lon(&p2, point1_p1.get_latitude(), point1_p1.get_longitude())?;
    let point2_p2 =
        GeoPoint::new_lat_lon(&p2, point2_p1.get_latitude(), point2_p1.get_longitude())?;
    let dist = 0.2 * PI;
    let bearing = 0.2 * PI;
    let new1 = p1.surface_point_on_bearing(&point2_p1, dist, bearing)?;
    let new2 = p2.surface_point_on_bearing(&point2_p2, dist, bearing)?;
    Assert::assert_equals_double_msg(None, new1.get_latitude(), new2.get_latitude(), 1e-12);
    Assert::assert_equals_double_msg(None, new1.get_longitude(), new2.get_longitude(), 1e-12);
    // This is true if surfaceDistance return results always in radians
    let d1 = p1.surface_distance(&point1_p1, &point2_p1);
    let d2 = p2.surface_distance(&point1_p2, &point2_p2);
    Assert::assert_equals_double_msg(None, d1, d2, 1e-12);
    Ok(())
}

fn random_point_bearing_wgs84_test() -> Result<()> {
    let planet_model: Arc<PlanetModel> = WGS84.clone();
    let center = RandomGeo3dShapeGenerator::random_geo_point(&planet_model)?;
    let radius = random().next_double() * PI;
    check_bearing_point(&planet_model, &center, radius, 0.0)?;
    check_bearing_point(&planet_model, &center, radius, 0.5 * PI)?;
    check_bearing_point(&planet_model, &center, radius, PI)?;
    check_bearing_point(&planet_model, &center, radius, 1.5 * PI)?;
    Ok(())
}

fn random_point_bearing_cardinal_test() -> Result<()> {
    // surface distance calculations methods start not converging when
    // planet scaledFlattening > 0.4
    let mut planet_model: Arc<PlanetModel>;
    loop {
        let ab = random().next_double() * 2.0;
        let c = random().next_double() * 2.0;
        if random().next_boolean() {
            planet_model = Arc::new(PlanetModel::new(ab, c));
        } else {
            planet_model = Arc::new(PlanetModel::new(c, ab));
        }
        // `do { ... } while (Math.abs(planetModel.scaledFlattening) > 0.4)`: a NaN exits.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(planet_model.scaled_flattening.abs() > 0.4) {
            break;
        }
    }
    let center = RandomGeo3dShapeGenerator::random_geo_point(&planet_model)?;
    let radius = random().next_double() * 0.9 * planet_model.minimum_pole_distance;
    check_bearing_point(&planet_model, &center, radius, 0.0)?;
    check_bearing_point(&planet_model, &center, radius, 0.5 * PI)?;
    check_bearing_point(&planet_model, &center, radius, PI)?;
    check_bearing_point(&planet_model, &center, radius, 1.5 * PI)?;
    Ok(())
}

fn check_bearing_point(
    planet_model: &Arc<PlanetModel>,
    center: &GeoPoint,
    radius: f64,
    bearing_angle: f64,
) -> Result<()> {
    let point = planet_model.surface_point_on_bearing(center, radius, bearing_angle)?;
    let surface_distance = planet_model.surface_distance(center, &point);
    Assert::assert_true_msg(
        &format!(
            "{} {} {}",
            planet_model,
            d2s(surface_distance - radius),
            d2s(radius)
        ),
        surface_distance - radius < Vector::MINIMUM_ANGULAR_RESOLUTION,
    );
    Ok(())
}

fn test_exact_circle_bounds() -> Result<()> {
    let center = GeoPoint::new_lat_lon(&WGS84, 0.0, 0.0)?;
    // Construct four cardinal points, and then we'll build the first two planes
    let north_point = WGS84.surface_point_on_bearing(&center, 1.0, 0.0)?;
    let south_point = WGS84.surface_point_on_bearing(&center, 1.0, PI)?;
    let east_point = WGS84.surface_point_on_bearing(&center, 1.0, PI * 0.5)?;
    let west_point = WGS84.surface_point_on_bearing(&center, 1.0, PI * 1.5)?;
    let circle: Arc<dyn GeoCircle> =
        GeoCircleFactory::make_exact_geo_circle(&WGS84, 0.0, 0.0, 1.0, 1e-6)?;
    let mut bounds = LatLonBounds::new();
    circle.get_bounds(&mut bounds);
    Assert::assert_equals_double_msg(
        None,
        north_point.get_latitude(),
        unbox(bounds.get_max_latitude()),
        1e-2,
    );
    Assert::assert_equals_double_msg(
        None,
        south_point.get_latitude(),
        unbox(bounds.get_min_latitude()),
        1e-2,
    );
    Assert::assert_equals_double_msg(
        None,
        west_point.get_longitude(),
        unbox(bounds.get_left_longitude()),
        1e-2,
    );
    Assert::assert_equals_double_msg(
        None,
        east_point.get_longitude(),
        unbox(bounds.get_right_longitude()),
        1e-2,
    );
    Ok(())
}

fn exact_circle_large_test() -> Result<()> {
    // should not throw exception
    GeoCircleFactory::make_exact_geo_circle(
        &Arc::new(PlanetModel::new(0.99, 1.05)),
        0.25 * PI,
        0.0,
        0.35 * PI,
        1e-12,
    )?;
    // should throw exception
    expect_throws("java.lang.IllegalArgumentException", || -> Result<()> {
        GeoCircleFactory::make_exact_geo_circle(&WGS84, 0.25 * PI, 0.0, 0.9996 * PI, 1e-12)?;
        Ok(())
    });
    Ok(())
}

fn test_exact_circle_does_not_fit() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || -> Result<()> {
        GeoCircleFactory::make_exact_geo_circle(
            &WGS84,
            1.5633796542562415,
            -1.0387149580695152,
            3.1409865861032844,
            1e-12,
        )?;
        Ok(())
    });
    Ok(())
}

fn test_big_circle_in_sphere() -> Result<()> {
    // In Planet model Sphere if circle is close to Math.PI we can get the situation where
    // circle slice planes are bigger than half of a hemisphere. We need to make
    // sure we divide the circle in at least 4 slices.
    let circle1: Arc<dyn GeoCircle> = GeoCircleFactory::make_exact_geo_circle(
        &SPHERE,
        1.1306735252307394,
        -0.7374283438171261,
        3.1415760537549234,
        4.816939220262406E-12,
    )?;
    let point = GeoPoint::new_lat_lon(&SPHERE, -1.5707963267948966, 0.0)?;
    Assert::assert_true(circle1.is_within(&point));
    Ok(())
}

//* in LUCENE-8054 we have problems with exact circles that have edges that are close together.
//   * This test creates those circles with the same center and slightly different radius.
fn test_random_lucene8054() -> Result<()> {
    let planet_model: Arc<PlanetModel> = RandomGeo3dShapeGenerator::random_planet_model();
    let circle1: Arc<dyn GeoCircle> = npe(RandomGeo3dShapeGenerator::random_geo_area_shape(
        RandomGeo3dShapeGenerator::EXACT_CIRCLE,
        &planet_model,
    )?
    .into_geo_circle())?;
    // new radius, a bit smaller than the generated one!
    let radius = circle1.get_radius() * (1.0 - (0.01 * random().next_double()));
    // circle with same center and new radius
    let circle2: Arc<dyn GeoCircle> = GeoCircleFactory::make_exact_geo_circle(
        &planet_model,
        circle1.get_center().get_latitude(),
        circle1.get_center().get_longitude(),
        radius,
        1e-5,
    )?;
    let mut b = String::new();
    b.push_str(&format!("circle1: {}\n", circle1));
    b.push_str(&format!("circle2: {}", circle2));
    // It cannot be disjoint, same center!
    Assert::assert_true_msg(&b, circle1.get_relationship(&*circle2)? != DISJOINT);
    Ok(())
}

fn test_lucene8054() -> Result<()> {
    let circle1: Arc<dyn GeoCircle> = GeoCircleFactory::make_exact_geo_circle(
        &WGS84,
        -1.0394053553992673,
        -1.9037325881389144,
        1.1546166170607672,
        4.231100485201301E-4,
    )?;
    let circle2: Arc<dyn GeoCircle> = GeoCircleFactory::make_exact_geo_circle(
        &WGS84,
        -1.3165961602008989,
        -1.887137823746273,
        1.432516663588956,
        3.172052880854355E-4,
    )?;
    // Relationship between circles must be different than DISJOINT as centers are closer than the
    // radius.
    let rel = circle1.get_relationship(&*circle2)?;
    Assert::assert_true(rel != DISJOINT);
    Ok(())
}

fn test_lucene8056() -> Result<()> {
    let circle: Arc<dyn GeoCircle> = GeoCircleFactory::make_exact_geo_circle(
        &WGS84,
        0.647941905154693,
        0.8542472362428436,
        0.8917883700569315,
        1.2173787103955335E-8,
    )?;
    let b_box: Arc<dyn GeoBBox> = GeoBBoxFactory::make_geo_bbox(
        &WGS84,
        0.5890486225480862,
        0.4908738521234052,
        1.9634954084936207,
        2.159844949342983,
    )?;
    // Center iis out of the shape
    Assert::assert_false(circle.is_within(&b_box.get_center()));
    // Edge point is in the shape
    Assert::assert_true(circle.is_within(&b_box.get_edge_points()[0]));
    // Shape should intersect!!!
    Assert::assert_true(b_box.get_relationship(&*circle)? == OVERLAPS);
    Ok(())
}

fn test_exact_circle_lucene8054() -> Result<()> {
    // [junit4]    > Throwable #1: java.lang.AssertionError: circle1: GeoExactCircle:
    // {planetmodel=PlanetModel.WGS84, center=[lat=-1.2097332228999564,
    // lon=0.749061883738567([X=0.25823775418663625, Y=0.2401212674846636, Z=-0.9338185278804293])],
    //  radius=0.20785254459485322(11.909073566339822), accuracy=6.710701666727661E-9}
    // [junit4]    > circle2: GeoExactCircle: {planetmodel=PlanetModel.WGS84,
    // center=[lat=-1.2097332228999564, lon=0.749061883738567([X=0.25823775418663625,
    // Y=0.2401212674846636, Z=-0.9338185278804293])],
    // radius=0.20701584142315682(11.861134005896407), accuracy=1.0E-5}
    let c1: Arc<dyn GeoCircle> = Arc::new(GeoExactCircle::new(
        &WGS84,
        -1.2097332228999564,
        0.749061883738567,
        0.20785254459485322,
        6.710701666727661E-9,
    )?);
    let c2: Arc<dyn GeoCircle> = Arc::new(GeoExactCircle::new(
        &WGS84,
        -1.2097332228999564,
        0.749061883738567,
        0.20701584142315682,
        1.0E-5,
    )?);
    Assert::assert_true_msg(
        &String::from("cannot be disjoint"),
        c1.get_relationship(&*c2)? != DISJOINT,
    );
    Ok(())
}

fn test_lucene8065() -> Result<()> {
    // Circle planes are convex
    let circle1: Arc<dyn GeoCircle> = GeoCircleFactory::make_exact_geo_circle(
        &WGS84,
        0.03186456479560385,
        -2.2254294002683617,
        1.5702573535090856,
        8.184299676008562E-6,
    )?;
    let circle2: Arc<dyn GeoCircle> = GeoCircleFactory::make_exact_geo_circle(
        &WGS84,
        0.03186456479560385,
        -2.2254294002683617,
        1.5698163157923914,
        1.0E-5,
    )?;
    Assert::assert_true(circle1.get_relationship(&*circle2)? != DISJOINT);
    Ok(())
}

fn test_lucene8080() -> Result<()> {
    let planet_model: Arc<PlanetModel> =
        Arc::new(PlanetModel::new(1.6304230055804751, 1.0199671157571204));
    expect_throws("java.lang.IllegalArgumentException", || -> Result<()> {
        GeoCircleFactory::make_exact_geo_circle(
            &planet_model,
            0.8853814403571284,
            0.9784990176851283,
            0.9071033527030907,
            1e-11,
        )?;
        Ok(())
    });
    Ok(())
}
