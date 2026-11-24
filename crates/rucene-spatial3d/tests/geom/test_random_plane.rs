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

//! Port of `TestRandomPlane.java`.

use crate::spatial3d_support::prelude::*;

type G = RandomGeo3dShapeGenerator;

/// `TestRandomPlane`.
pub fn class_def() -> Arc<Class> {
    let repeat = || Repeat {
        iterations: 10,
        use_constant_seed: false,
    };
    class(
        "org.apache.lucene.spatial3d.geom.TestRandomPlane",
        vec![
            test("testPlaneAccuracy", test_plane_accuracy)
                .annotate(Test::new())
                .annotate(repeat()),
            test(
                "testPlaneThreePointsAccuracy",
                test_plane_three_points_accuracy,
            )
            .annotate(Test::new())
            .annotate(repeat()),
            test("testPolygonAccuracy", test_polygon_accuracy)
                .annotate(Test::new())
                .annotate(repeat()),
        ],
    )
}

fn test_plane_accuracy() -> Result<()> {
    let planet_model = G::random_planet_model();
    let point1 = G::random_geo_point(&planet_model)?;
    for _i in 0..1000 {
        let dist = random().next_double() * Vector::MINIMUM_ANGULAR_RESOLUTION
            + Vector::MINIMUM_ANGULAR_RESOLUTION;
        let bearing = random().next_double() * 2.0 * PI;
        let point2 = planet_model.surface_point_on_bearing(&point1, dist, bearing)?;
        let check = G::random_geo_point(&planet_model)?;
        if !point1.is_numerically_identical(&point2) {
            let plane = SidedPlane::new_cross(&check, &point1, &point2)?;
            let msg = format!(
                "{} point 1: {}, point 2: {} , check: {}",
                d2s(dist),
                point1,
                point2,
                check
            );
            Assert::assert_true_msg(&msg, plane.is_within(&check));
            Assert::assert_true_msg(&msg, plane.is_within(&point2));
            Assert::assert_true_msg(&msg, plane.is_within(&point1));
        } else {
            Assert::assert_false_msg("numerically identical", true);
        }
    }
    Ok(())
}

fn test_plane_three_points_accuracy() -> Result<()> {
    let planet_model = G::random_planet_model();
    for _i in 0..1000 {
        let point1 = G::random_geo_point(&planet_model)?;
        let mut dist = random().next_double() * PI - Vector::MINIMUM_ANGULAR_RESOLUTION;
        let mut bearing = random().next_double() * 2.0 * PI;
        let point2 = planet_model.surface_point_on_bearing(&point1, dist, bearing)?;
        dist = random().next_double() * Vector::MINIMUM_ANGULAR_RESOLUTION
            + Vector::MINIMUM_ANGULAR_RESOLUTION;
        bearing = random().next_double() * 2.0 * PI;
        let point3 = planet_model.surface_point_on_bearing(&point1, dist, bearing)?;
        let check = G::random_geo_point(&planet_model)?;
        let plane = SidedPlane::construct_normalized_three_point_sided_plane(
            &check, &point1, &point2, &point3,
        );
        let msg = format!(
            "{} point 1: {}, point 2: {}, point 3: {} , check: {}",
            planet_model, point1, point2, point3, check
        );
        let Some(plane) = plane else {
            Assert::fail(&msg);
        };
        // This is not expected
        // assertTrue(plane.evaluate(check) + " " + msg, plane.isWithin(check));
        Assert::assert_true_msg(
            &format!("{} {}", d2s(plane.evaluate(&point1)), msg),
            plane.is_within(&point1),
        );
        Assert::assert_true_msg(
            &format!("{} {}", d2s(plane.evaluate(&point2)), msg),
            plane.is_within(&point2),
        );
        Assert::assert_true_msg(
            &format!("{} {}", d2s(plane.evaluate(&point3)), msg),
            plane.is_within(&point3),
        );
    }
    Ok(())
}

fn test_polygon_accuracy() -> Result<()> {
    let planet_model = G::random_planet_model();
    let point1 = G::random_geo_point(&planet_model)?;
    for _i in 0..1000 {
        let dist = random().next_double() * 1e-6 + Vector::MINIMUM_ANGULAR_RESOLUTION;
        let point2 = planet_model.surface_point_on_bearing(&point1, dist, 0.0)?;
        let point3 = planet_model.surface_point_on_bearing(&point1, dist, 0.5 * PI)?;

        let points = vec![point1.clone(), point2, point3];
        GeoPolygonFactory::make_geo_polygon(&planet_model, points)?;
    }
    Ok(())
}
