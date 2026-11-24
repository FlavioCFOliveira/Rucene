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

//! Port of `TestPlane.java`.

use crate::spatial3d_support::prelude::*;

/// `TestPlane`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestPlane",
        vec![
            test("testIdenticalPlanes", test_identical_planes).annotate(Test::new()),
            test("testIdenticalVector", test_identical_vector).annotate(Test::new()),
            test("testInterpolation", test_interpolation).annotate(Test::new()),
            test("testFindArcPoints", test_find_arc_points).annotate(Test::new()),
        ],
    )
}

fn test_identical_planes() -> Result<()> {
    let p = GeoPoint::new_lat_lon(&SPHERE, 0.123, -0.456)?;
    let plane1 = Plane::new_vector_d(&p, 0.0);
    let plane2 = Plane::new_vector_d(&p, 0.0);
    Assert::assert_true(plane1.is_numerically_identical(&plane2));
    let plane3 = Plane::new_vector_d(&p, 0.1);
    Assert::assert_false(plane1.is_numerically_identical(&plane3));
    let v1 = Vector::new(0.1, -0.732, 0.9);
    let constant = 0.432;
    let v2 = Vector::new(v1.x * constant, v1.y * constant, v1.z * constant);
    let p1 = Plane::new_vector_d(&v1, 0.2);
    let p2 = Plane::new_vector_d(&v2, 0.2 * constant);
    Assert::assert_true(p1.is_numerically_identical(&p2));
    Ok(())
}

fn test_identical_vector() -> Result<()> {
    let v1 = Vector::new(1.0, 0.0, 0.0);
    let v2 = Vector::new(1.0, 0.0, 0.0);
    let v3 = Vector::new(-1.0, 0.0, 0.0);
    Assert::assert_true(v1.is_numerically_identical(&v2));
    Assert::assert_false(v1.is_numerically_identical(&v3));
    Ok(())
}

fn test_interpolation() -> Result<()> {
    // [X=0.35168818443386646, Y=-0.19637966197066342, Z=0.9152870857244183],
    // [X=0.5003343189532654, Y=0.522128543226148, Z=0.6906861469771293],
    let start = GeoPoint::new(
        0.35168818443386646,
        -0.19637966197066342,
        0.9152870857244183,
    );
    let end = GeoPoint::new(0.5003343189532654, 0.522128543226148, 0.6906861469771293);
    // [A=-0.6135342247741855, B=0.21504338363863665, C=0.28188192383666794, D=0.0, side=-1.0]
    // internal? false;
    let p = Plane::new(
        -0.6135342247741855,
        0.21504338363863665,
        0.28188192383666794,
        0.0,
    );
    let points = p.interpolate(&SPHERE, &start, &end, &vec![0.25, 0.50, 0.75])?;
    for point in points.iter() {
        Assert::assert_true(p.evaluate_is_zero(point));
    }
    Ok(())
}

fn test_find_arc_points() -> Result<()> {
    // Create two points
    let p1 = GeoPoint::new_lat_lon(&WGS84, 0.123, -0.456)?;
    let p2 = GeoPoint::new_lat_lon(&WGS84, -0.368, 0.888)?;
    // Create a plane that links them.
    let plane = Plane::new_cross(&p1, &p2)?;
    // Now, use that plane to find points that are a certain distance from the original
    let new_points = plane.find_arc_distance_points(&WGS84, 0.20, &p1, &[])?;
    Assert::assert_true((new_points.len() as i32) == 2);
    Assert::assert_true(plane.evaluate_is_zero(&new_points[0]));
    Assert::assert_true(plane.evaluate_is_zero(&new_points[1]));
    Assert::assert_true(WGS84.point_on_surface(&new_points[0]));
    Assert::assert_true(WGS84.point_on_surface(&new_points[1]));
    Assert::assert_equals_double_msg(None, 0.20, p1.arc_distance(&new_points[0]), 1e-6);
    Assert::assert_equals_double_msg(None, 0.20, p1.arc_distance(&new_points[1]), 1e-6);
    Ok(())
}
