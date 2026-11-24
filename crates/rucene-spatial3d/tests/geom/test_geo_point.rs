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

//! Port of `TestGeoPoint.java`.

use crate::spatial3d_support::prelude::*;

/// `TestGeoPoint`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestGeoPoint",
        vec![
            test("testConversion", test_conversion).annotate(Test::new()),
            test("testSurfaceDistance", test_surface_distance).annotate(Test::new()),
            test("testBisection", test_bisection).annotate(Test::new()),
            test("testBadLatLon", test_bad_lat_lon)
                .annotate(Test::expected("java.lang.IllegalArgumentException")),
        ],
    )
}

const DEGREES_TO_RADIANS: f64 = PI / 180.0;

fn test_conversion() -> Result<()> {
    test_point_round_trip(&SPHERE, 90.0 * DEGREES_TO_RADIANS, 0.0, 1e-6)?;
    test_point_round_trip(&SPHERE, (-90.0) * DEGREES_TO_RADIANS, 0.0, 1e-6)?;
    test_point_round_trip(&WGS84, 90.0 * DEGREES_TO_RADIANS, 0.0, 1e-6)?;
    test_point_round_trip(&WGS84, (-90.0) * DEGREES_TO_RADIANS, 0.0, 1e-6)?;
    let times = LuceneTestCase::at_least(100);
    for _i in 0..times {
        let p_lat = (f64::from(RandomizedTest::random_float()) * 180.0 - 90.0) * DEGREES_TO_RADIANS;
        let p_lon =
            (f64::from(RandomizedTest::random_float()) * 360.0 - 180.0) * DEGREES_TO_RADIANS;
        test_point_round_trip(&SPHERE, p_lat, p_lon, 1e-6)?;
        // 1e-6 since there's a square root in there (Karl says)
        test_point_round_trip(&WGS84, p_lat, p_lon, 1e-6)?;
    }
    Ok(())
}

fn test_point_round_trip(
    planet_model: &Arc<PlanetModel>,
    p_lat: f64,
    p_lon: f64,
    epsilon: f64,
) -> Result<()> {
    let p1 = GeoPoint::new_lat_lon(planet_model, p_lat, p_lon)?;
    // In order to force the reverse conversion, we have to construct a geopoint from just x,y,z
    let p2 = GeoPoint::new(p1.x, p1.y, p1.z);
    // Now, construct the final point based on getLatitude() and getLongitude()
    let p3 = GeoPoint::new_lat_lon(planet_model, p2.get_latitude(), p2.get_longitude())?;
    let dist = p1.arc_distance(&p3);
    Assert::assert_equals_double_msg(None, 0.0, dist, epsilon);
    Ok(())
}

fn test_surface_distance() -> Result<()> {
    let times = LuceneTestCase::at_least(100);
    for _i in 0..times {
        let p1_lat =
            (f64::from(RandomizedTest::random_float()) * 180.0 - 90.0) * DEGREES_TO_RADIANS;
        let p1_lon =
            (f64::from(RandomizedTest::random_float()) * 360.0 - 180.0) * DEGREES_TO_RADIANS;
        let p2_lat =
            (f64::from(RandomizedTest::random_float()) * 180.0 - 90.0) * DEGREES_TO_RADIANS;
        let p2_lon =
            (f64::from(RandomizedTest::random_float()) * 360.0 - 180.0) * DEGREES_TO_RADIANS;
        let p1 = GeoPoint::new_lat_lon(&SPHERE, p1_lat, p1_lon)?;
        let p2 = GeoPoint::new_lat_lon(&SPHERE, p2_lat, p2_lon)?;
        let arc_distance = p1.arc_distance(&p2);
        // Compute ellipsoid distance; it should agree for a sphere
        let surface_distance = SPHERE.surface_distance(&p1, &p2);
        Assert::assert_equals_double_msg(None, arc_distance, surface_distance, 1e-6);
    }
    // Now try some WGS84 points (taken randomly and compared against a known-good implementation)
    Assert::assert_equals_double_msg(
        None,
        1.1444648695765323,
        WGS84.surface_distance(
            &GeoPoint::new_lat_lon(&WGS84, 0.038203808753702884, -0.6701260455506466)?,
            &GeoPoint::new_lat_lon(&WGS84, -0.8453720422675458, 0.1737353153814496)?,
        ),
        1e-6,
    );
    Assert::assert_equals_double_msg(
        None,
        1.4345148695890722,
        WGS84.surface_distance(
            &GeoPoint::new_lat_lon(&WGS84, 0.5220926323378574, 0.6758041581907408)?,
            &GeoPoint::new_lat_lon(&WGS84, -0.8453720422675458, 0.1737353153814496)?,
        ),
        1e-6,
    );
    Assert::assert_equals_double_msg(
        None,
        2.32418144616446,
        WGS84.surface_distance(
            &GeoPoint::new_lat_lon(&WGS84, 0.09541335760967473, 1.2091829760623236)?,
            &GeoPoint::new_lat_lon(&WGS84, -0.8501591797459979, -2.3044806381627594)?,
        ),
        1e-6,
    );
    Assert::assert_equals_double_msg(
        None,
        2.018421047005435,
        WGS84.surface_distance(
            &GeoPoint::new_lat_lon(&WGS84, 0.3402853531962009, -0.43544195327249957)?,
            &GeoPoint::new_lat_lon(&WGS84, -0.8501591797459979, -2.3044806381627594)?,
        ),
        1e-6,
    );
    Ok(())
}

fn test_bisection() -> Result<()> {
    let times = LuceneTestCase::at_least(100);
    for _i in 0..times {
        let p1_lat =
            (f64::from(RandomizedTest::random_float()) * 180.0 - 90.0) * DEGREES_TO_RADIANS;
        let p1_lon =
            (f64::from(RandomizedTest::random_float()) * 360.0 - 180.0) * DEGREES_TO_RADIANS;
        let p2_lat =
            (f64::from(RandomizedTest::random_float()) * 180.0 - 90.0) * DEGREES_TO_RADIANS;
        let p2_lon =
            (f64::from(RandomizedTest::random_float()) * 360.0 - 180.0) * DEGREES_TO_RADIANS;
        let p1 = GeoPoint::new_lat_lon(&WGS84, p1_lat, p1_lon)?;
        let p2 = GeoPoint::new_lat_lon(&WGS84, p2_lat, p2_lon)?;
        let p_mid = WGS84.bisection(&p1, &p2);
        if let Some(p_mid) = p_mid {
            let arc_distance = p1.arc_distance(&p2);
            let sum = p_mid.arc_distance(&p1) + p_mid.arc_distance(&p2);
            Assert::assert_equals_double_msg(None, arc_distance, sum, 1e-6);
        }
    }
    Ok(())
}

fn test_bad_lat_lon() -> Result<()> {
    GeoPoint::new_lat_lon(&SPHERE, 50.0, 32.2)?;
    Ok(())
}
