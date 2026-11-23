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

//! Port of `TestSimpleGeoPolygonRelationships.java`.

use crate::spatial3d_support::prelude::*;

/// `TestSimpleGeoPolygonRelationships`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestSimpleGeoPolygonRelationships",
        vec![
            test("testGeoSimplePolygon1", test_geo_simple_polygon1).annotate(Test::new()),
            test("testGeoSimplePolygon2", test_geo_simple_polygon2).annotate(Test::new()),
            test("testGeoSimplePolygon3", test_geo_simple_polygon3).annotate(Test::new()),
            test("testGeoSimplePolygon4", test_geo_simple_polygon4).annotate(Test::new()),
            test(
                "testGeoSimplePolygonWithHole1",
                test_geo_simple_polygon_with_hole1,
            )
            .annotate(Test::new()),
            test(
                "testGeoSimplePolygonWithHole2",
                test_geo_simple_polygon_with_hole2,
            )
            .annotate(Test::new()),
            test(
                "testGeoSimplePolygonWithHole3",
                test_geo_simple_polygon_with_hole3,
            )
            .annotate(Test::new()),
            test(
                "testGeoSimplePolygonWithHole4",
                test_geo_simple_polygon_with_hole4,
            )
            .annotate(Test::new()),
            test(
                "testGeoSimplePolygonWithCircle",
                test_geo_simple_polygon_with_circle,
            )
            .annotate(Test::new()),
            test(
                "testGeoSimplePolygonWithBBox",
                test_geo_simple_polygon_with_b_box,
            )
            .annotate(Test::new()),
            test(
                "testGeoSimplePolygonWithComposite",
                test_geo_simple_polygon_with_composite,
            )
            .annotate(Test::new()),
            test(
                "testDegeneratedPointIntersectShape",
                test_degenerated_point_intersect_shape,
            )
            .annotate(Test::new()),
            test("testDegeneratedPointInPole", test_degenerated_point_in_pole)
                .annotate(Test::new()),
            test("testDegeneratePathShape", test_degenerate_path_shape).annotate(Test::new()),
        ],
    )
}

//Test with two shapes with no crossing edges and no points in common in convex case.
fn test_geo_simple_polygon1() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 19.845091 -60.452631)) disjoint
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_concave_pol: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_complex_pol: Arc<dyn GeoPolygon> = build_complex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let pol_convex: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(20.0, -60.4, 20.1, -60.4, 20.1, -60.3, 20.0, -60.3)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.0, -60.4, 20.1, -60.4, 20.1, -60.3, 20.0, -60.3)?;
    // Convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(WITHIN, rel);
    // Check
    // Concave
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // Complex
    rel = original_complex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_complex_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_complex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*original_complex_pol)?;
    Assert::assert_equals(WITHIN, rel);
    Ok(())
}

//Test with two shapes with crossing edges and some points inside in convex case.
fn test_geo_simple_polygon2() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 19.845091 -60.452631)) disjoint
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_concave_pol: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_complex_pol: Arc<dyn GeoPolygon> = build_complex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    // POLYGON ((20.0 -60.4, 23.1 -60.4, 23.1 -60.3, 20.0  -60.3,20.0 -60.4))
    let pol_convex: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(20.0, -60.4, 23.1, -60.4, 23.1, -60.3, 20.0, -60.3)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.0, -60.4, 23.1, -60.4, 23.1, -60.3, 20.0, -60.3)?;
    // Convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // Concave
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // Complex
    rel = original_complex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_complex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_complex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_complex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

//Test with two shapes with no crossing edges and all points inside in convex case.
fn test_geo_simple_polygon3() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 19.845091 -60.452631)) disjoint
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_concave_pol: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_complex_pol: Arc<dyn GeoPolygon> = build_complex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    // POLYGON ((20.0 -61.1, 20.1 -61.1, 20.1 -60.5, 20.0  -60.5,20.0 -61.1))
    let pol_convex: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(20.0, -61.1, 20.1, -61.1, 20.1, -60.5, 20.0, -60.5)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.0, -61.1, 20.1, -61.1, 20.1, -60.5, 20.0, -60.5)?;
    // Convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // Concave
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(WITHIN, rel);
    // check
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    // Complex
    rel = original_complex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_complex_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_complex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_complex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

//Test with two shapes with crossing edges and no points inside in convex case.
fn test_geo_simple_polygon4() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 19.845091 -60.452631)) disjoint
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_concave_pol: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_complex_pol: Arc<dyn GeoPolygon> = build_complex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    // POLYGON ((20.0 -62.4, 20.1 -62.4, 20.1 -60.3, 20.0  -60.3,20.0 -62.4)) intersects no points
    // inside
    let pol_convex: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(20.0, -62.4, 20.1, -62.4, 20.1, -60.3, 20.0, -60.3)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.0, -62.4, 20.1, -62.4, 20.1, -60.3, 20.0, -60.3)?;
    // Convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // concave
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // Complex
    rel = original_complex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_complex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_complex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_complex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

//Test with two shapes with no crossing edges and polygon in hole in convex case.
fn test_geo_simple_polygon_with_hole1() -> Result<()> {
    // POLYGON((-135 -31, -135 -30, -137 -30, -137 -31, -135 -31),(-135.5 -30.7, -135.5 -30.4,
    // -136.5 -30.4, -136.5 -30.7, -135.5 -30.7))
    let hole: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-135.5, -30.7, -135.5, -30.4, -136.5, -30.4, -136.5, -30.7)?;
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon_with_hole(
        -135.0,
        -31.0,
        -135.0,
        -30.0,
        -137.0,
        -30.0,
        -137.0,
        -31.0,
        hole.clone(),
    )?;
    let hole_inv: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(-135.0, -31.0, -135.0, -30.0, -137.0, -30.0, -137.0, -31.0)?;
    let original_convex_pol_inv: Arc<dyn GeoPolygon> = build_concave_geo_polygon_with_hole(
        -135.5,
        -30.7,
        -135.5,
        -30.4,
        -136.5,
        -30.4,
        -136.5,
        -30.7,
        hole_inv.clone(),
    )?;
    // POLYGON((-135.7 -30.6, -135.7 -30.45, -136 -30.45, -136 -30.6, -135.7 -30.6)) in the hole
    let pol_convex: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(-135.7, -30.6, -135.7, -30.45, -136.0, -30.45, -136.0, -30.6)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-135.7, -30.6, -135.7, -30.45, -136.0, -30.45, -136.0, -30.6)?;
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(WITHIN, rel);
    rel = original_convex_pol_inv.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol_inv)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_convex_pol_inv.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol_inv)?;
    Assert::assert_equals(WITHIN, rel);
    Ok(())
}

//Test with two shapes with crossing edges in hole and some points inside in convex case.
fn test_geo_simple_polygon_with_hole2() -> Result<()> {
    // POLYGON((-135 -31, -135 -30, -137 -30, -137 -31, -135 -31),(-135.5 -30.7, -135.5 -30.4,
    // -136.5 -30.4, -136.5 -30.7, -135.5 -30.7))
    let hole: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-135.5, -30.7, -135.5, -30.4, -136.5, -30.4, -136.5, -30.7)?;
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon_with_hole(
        -135.0,
        -31.0,
        -135.0,
        -30.0,
        -137.0,
        -30.0,
        -137.0,
        -31.0,
        hole.clone(),
    )?;
    let hole_inv: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(-135.0, -31.0, -135.0, -30.0, -137.0, -30.0, -137.0, -31.0)?;
    let original_convex_pol_inv: Arc<dyn GeoPolygon> = build_concave_geo_polygon_with_hole(
        -135.5,
        -30.7,
        -135.5,
        -30.4,
        -136.5,
        -30.4,
        -136.5,
        -30.7,
        hole_inv.clone(),
    )?;
    // POLYGON((-135.5 -31.2, -135.5 -30.8, -136 -30.8, -136 -31.2, -135.5 -31.2)) intersects the
    // hole
    let pol_convex: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(-135.5, -30.2, -135.5, -30.8, -136.0, -30.8, -136.0, -30.2)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-135.5, -30.2, -135.5, -30.8, -136.0, -30.8, -136.0, -30.2)?;
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol_inv.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol_inv)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol_inv.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol_inv)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

//Test with two shapes with crossing edges and some points inside in convex case.
fn test_geo_simple_polygon_with_hole3() -> Result<()> {
    // POLYGON((-135 -31, -135 -30, -137 -30, -137 -31, -135 -31),(-135.5 -30.7, -135.5 -30.4,
    // -136.5 -30.4, -136.5 -30.7, -135.5 -30.7))
    let hole: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-135.5, -30.7, -135.5, -30.4, -136.5, -30.4, -136.5, -30.7)?;
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon_with_hole(
        -135.0,
        -31.0,
        -135.0,
        -30.0,
        -137.0,
        -30.0,
        -137.0,
        -31.0,
        hole.clone(),
    )?;
    let hole_inv: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(-135.0, -31.0, -135.0, -30.0, -137.0, -30.0, -137.0, -31.0)?;
    let original_convex_pol_inv: Arc<dyn GeoPolygon> = build_concave_geo_polygon_with_hole(
        -135.5,
        -30.7,
        -135.5,
        -30.4,
        -136.5,
        -30.4,
        -136.5,
        -30.7,
        hole_inv.clone(),
    )?;
    // POLYGON((-135.2 -30.8, -135.2 -30.2, -136.8 -30.2, -136.8 -30.8, -135.2 -30.8)) inside the
    // polygon covering the hole
    let pol_convex: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(-135.2, -30.8, -135.2, -30.3, -136.8, -30.2, -136.8, -30.8)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-135.2, -30.8, -135.2, -30.3, -136.8, -30.2, -136.8, -30.8)?;
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol_inv.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol_inv)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol_inv.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol_inv)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

//Test with two shapes with no crossing edges and all points inside in convex case.
fn test_geo_simple_polygon_with_hole4() -> Result<()> {
    // POLYGON((-135 -31, -135 -30, -137 -30, -137 -31, -135 -31),(-135.5 -30.7, -135.5 -30.4,
    // -136.5 -30.4, -136.5 -30.7, -135.5 -30.7))
    let hole: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-135.5, -30.7, -135.5, -30.4, -136.5, -30.4, -136.5, -30.7)?;
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon_with_hole(
        -135.0,
        -31.0,
        -135.0,
        -30.0,
        -137.0,
        -30.0,
        -137.0,
        -31.0,
        hole.clone(),
    )?;
    let hole_inv: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(-135.0, -31.0, -135.0, -30.0, -137.0, -30.0, -137.0, -31.0)?;
    let original_convex_pol_inv: Arc<dyn GeoPolygon> = build_concave_geo_polygon_with_hole(
        -135.5,
        -30.7,
        -135.5,
        -30.4,
        -136.5,
        -30.4,
        -136.5,
        -30.7,
        hole_inv.clone(),
    )?;
    // POLYGON((-135.7 -30.3, -135.7 -30.2, -136 -30.2, -136 -30.3, -135.7 -30.3))inside the polygon
    let pol_convex: Arc<dyn GeoPolygon> =
        build_convex_geo_polygon(-135.7, -30.3, -135.7, -30.2, -136.0, -30.2, -136.0, -30.3)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-135.7, -30.3, -135.7, -30.2, -136.0, -30.2, -136.0, -30.3)?;
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol_inv.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol_inv)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_convex_pol_inv.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol_inv)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_geo_simple_polygon_with_circle() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 19.845091 -60.452631)) disjoint
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_concave_pol: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_complex_pol: Arc<dyn GeoPolygon> = build_complex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let out_circle: Arc<dyn GeoCircle> = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        Geo3DUtil::from_degrees(-70.0),
        Geo3DUtil::from_degrees(23.0),
        Geo3DUtil::from_degrees(1.0),
    )?;
    let mut rel = original_convex_pol.get_relationship(&*out_circle)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_concave_pol.get_relationship(&*out_circle)?;
    Assert::assert_equals(WITHIN, rel);
    rel = original_complex_pol.get_relationship(&*out_circle)?;
    Assert::assert_equals(DISJOINT, rel);
    let overlap_circle: Arc<dyn GeoCircle> = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.5),
        Geo3DUtil::from_degrees(20.0),
        Geo3DUtil::from_degrees(1.0),
    )?;
    rel = original_convex_pol.get_relationship(&*overlap_circle)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_concave_pol.get_relationship(&*overlap_circle)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_complex_pol.get_relationship(&*overlap_circle)?;
    Assert::assert_equals(OVERLAPS, rel);
    let in_circle: Arc<dyn GeoCircle> = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.0),
        Geo3DUtil::from_degrees(21.0),
        Geo3DUtil::from_degrees(0.1),
    )?;
    rel = original_convex_pol.get_relationship(&*in_circle)?;
    Assert::assert_equals(WITHIN, rel);
    rel = original_concave_pol.get_relationship(&*in_circle)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_complex_pol.get_relationship(&*in_circle)?;
    Assert::assert_equals(WITHIN, rel);
    let on_circle: Arc<dyn GeoCircle> = GeoCircleFactory::make_geo_circle(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.0),
        Geo3DUtil::from_degrees(21.0),
        Geo3DUtil::from_degrees(10.0),
    )?;
    rel = original_convex_pol.get_relationship(&*on_circle)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_concave_pol.get_relationship(&*on_circle)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_complex_pol.get_relationship(&*on_circle)?;
    Assert::assert_equals(CONTAINS, rel);
    Ok(())
}

fn test_geo_simple_polygon_with_b_box() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 19.845091 -60.452631)) disjoint
    let original_convex_pol: Arc<dyn GeoPolygon> = build_convex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_concave_pol: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let original_complex_pol: Arc<dyn GeoPolygon> = build_complex_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
    )?;
    let out_rectangle: Arc<dyn GeoBBox> = GeoBBoxFactory::make_geo_bbox(
        &SPHERE,
        Geo3DUtil::from_degrees(-69.0),
        Geo3DUtil::from_degrees(-70.0),
        Geo3DUtil::from_degrees(22.0),
        Geo3DUtil::from_degrees(23.0),
    )?;
    let mut rel = original_convex_pol.get_relationship(&*out_rectangle)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = out_rectangle.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_concave_pol.get_relationship(&*out_rectangle)?;
    Assert::assert_equals(WITHIN, rel);
    rel = original_complex_pol.get_relationship(&*out_rectangle)?;
    Assert::assert_equals(DISJOINT, rel);
    let overlap_rectangle: Arc<dyn GeoBBox> = GeoBBoxFactory::make_geo_bbox(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.0),
        Geo3DUtil::from_degrees(-62.0),
        Geo3DUtil::from_degrees(22.0),
        Geo3DUtil::from_degrees(23.0),
    )?;
    rel = original_convex_pol.get_relationship(&*overlap_rectangle)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = overlap_rectangle.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_concave_pol.get_relationship(&*overlap_rectangle)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_complex_pol.get_relationship(&*overlap_rectangle)?;
    Assert::assert_equals(OVERLAPS, rel);
    let in_rectangle: Arc<dyn GeoBBox> = GeoBBoxFactory::make_geo_bbox(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.0),
        Geo3DUtil::from_degrees(-61.1),
        Geo3DUtil::from_degrees(22.5),
        Geo3DUtil::from_degrees(23.0),
    )?;
    rel = original_convex_pol.get_relationship(&*in_rectangle)?;
    Assert::assert_equals(WITHIN, rel);
    rel = in_rectangle.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_concave_pol.get_relationship(&*in_rectangle)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_complex_pol.get_relationship(&*in_rectangle)?;
    Assert::assert_equals(WITHIN, rel);
    let on_rectangle: Arc<dyn GeoBBox> = GeoBBoxFactory::make_geo_bbox(
        &SPHERE,
        Geo3DUtil::from_degrees(-59.0),
        Geo3DUtil::from_degrees(-64.1),
        Geo3DUtil::from_degrees(18.5),
        Geo3DUtil::from_degrees(27.0),
    )?;
    rel = original_convex_pol.get_relationship(&*on_rectangle)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = on_rectangle.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(WITHIN, rel);
    rel = original_concave_pol.get_relationship(&*on_rectangle)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_complex_pol.get_relationship(&*on_rectangle)?;
    Assert::assert_equals(CONTAINS, rel);
    Ok(())
}

fn test_geo_simple_polygon_with_composite() -> Result<()> {
    let shape: Arc<dyn GeoShape> = get_composite_shape()?;
    // POLYGON((-145.8555 -5.13, -145.8540 -5.13, -145.8540 -5.12, -145.8555 -5.12, -145.8555
    // -5.13))
    let mut pol_convex: Arc<dyn GeoPolygon> = build_convex_geo_polygon(
        -145.8555, -5.13, -145.8540, -5.13, -145.8540, -5.12, -145.8555, -5.12,
    )?;
    let mut pol_concave: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        -145.8555, -5.13, -145.8540, -5.13, -145.8540, -5.12, -145.8555, -5.12,
    )?;
    let mut rel = pol_convex.get_relationship(&*shape)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_concave.get_relationship(&*shape)?;
    Assert::assert_equals(WITHIN, rel);
    // POLYGON((-145.8555 -5.13, -145.85 -5.13, -145.85 -5.12, -145.8555 -5.12, -145.8555 -5.13))
    pol_convex = build_convex_geo_polygon(
        -145.8555, -5.13, -145.85, -5.13, -145.85, -5.12, -145.8555, -5.12,
    )?;
    pol_concave = build_concave_geo_polygon(
        -145.8555, -5.13, -145.85, -5.13, -145.85, -5.12, -145.8555, -5.12,
    )?;
    rel = pol_convex.get_relationship(&*shape)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*shape)?;
    Assert::assert_equals(OVERLAPS, rel);
    // POLYGON((-146 -5.18, -145.854 -5.18, -145.854 -5.11, -146 -5.11, -146 -5.18))
    // Case overlaping on of the shapes
    pol_convex = build_convex_geo_polygon(
        -146.0, -5.18, -145.854, -5.18, -145.854, -5.11, -146.0, -5.11,
    )?;
    pol_concave = build_concave_geo_polygon(
        -146.0, -5.18, -145.854, -5.18, -145.854, -5.11, -146.0, -5.11,
    )?;
    rel = pol_convex.get_relationship(&*shape)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*shape)?;
    Assert::assert_equals(OVERLAPS, rel);
    // POLYGON((-145.88 -5.13, -145.87 -5.13, -145.87 -5.12, -145.88 -5.12, -145.88 -5.13))
    pol_convex = build_convex_geo_polygon(
        -145.88, -5.13, -145.87, -5.13, -145.87, -5.12, -145.88, -5.12,
    )?;
    pol_concave = build_concave_geo_polygon(
        -145.88, -5.13, -145.87, -5.13, -145.87, -5.12, -145.88, -5.12,
    )?;
    rel = pol_convex.get_relationship(&*shape)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*shape)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_degenerated_point_intersect_shape() -> Result<()> {
    let b_box1: Arc<dyn GeoBBox> = GeoBBoxFactory::make_geo_bbox(&SPHERE, 1.0, 0.0, 0.0, 1.0)?;
    let b_box2: Arc<dyn GeoBBox> = GeoBBoxFactory::make_geo_bbox(&SPHERE, 1.0, 1.0, 1.0, 1.0)?;
    let mut rel = b_box1.get_relationship(&*b_box2)?;
    // OVERLAPS instead of WITHIN. In this case the degenerated point lies on the edge of the shape.
    // intersects() returns true for one plane of the BBox and hence method return OVERLAPS.
    Assert::assert_equals(OVERLAPS, rel);
    rel = b_box2.get_relationship(&*b_box1)?;
    // The degenerated point cannot compute if it is on the edge. Uses WITHIN that is true
    // and therefore CONTAINS
    Assert::assert_equals(CONTAINS, rel);
    Ok(())
}

fn test_degenerated_point_in_pole() -> Result<()> {
    let b_box1: Arc<dyn GeoBBox> =
        GeoBBoxFactory::make_geo_bbox(&SPHERE, PI * 0.5, PI * 0.5, 0.0, 0.0)?;
    let point = GeoPoint::new_lat_lon(&SPHERE, PI * 0.5, PI)?;
    randomizedtesting::jdk::system::out_println(&format!("bbox1 = {}; point = {}", b_box1, point));
    Assert::assert_true(b_box1.is_within(&point));
    Ok(())
}

fn test_degenerate_path_shape() -> Result<()> {
    let point1 = GeoPoint::new_lat_lon(&SPHERE, 0.0, 0.0)?;
    let point2 = GeoPoint::new_lat_lon(&SPHERE, 0.0, 1.0)?;
    let point_path1 = vec![point1.clone(), point2.clone()];
    let path1: Arc<dyn GeoPath> = GeoPathFactory::make_geo_path(&SPHERE, 0.0, &point_path1)?;
    let path2: Arc<dyn GeoPath> = GeoPathFactory::make_geo_path(&SPHERE, 1.0, &point_path1)?;
    randomizedtesting::jdk::system::out_println(&format!("path1 = {} path2 = {}", path1, path2));
    let mut rel = path1.get_relationship(&*path2)?;
    // if an end point is inside the shape it will always return intersects
    Assert::assert_equals(CONTAINS, rel);
    // should be contains?
    rel = path2.get_relationship(&*path1)?;
    Assert::assert_equals(WITHIN, rel);
    Ok(())
}

fn build_convex_geo_polygon(
    lon1: f64,
    lat1: f64,
    lon2: f64,
    lat2: f64,
    lon3: f64,
    lat3: f64,
    lon4: f64,
    lat4: f64,
) -> Result<Arc<dyn GeoPolygon>> {
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat1),
        Geo3DUtil::from_degrees(lon1),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat2),
        Geo3DUtil::from_degrees(lon2),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat3),
        Geo3DUtil::from_degrees(lon3),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat4),
        Geo3DUtil::from_degrees(lon4),
    )?;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(point1.clone());
    points.push(point2.clone());
    points.push(point3.clone());
    points.push(point4.clone());
    Ok(npe(GeoPolygonFactory::make_geo_polygon(
        &SPHERE,
        points.clone(),
    )?)?)
}

fn build_concave_geo_polygon(
    lon1: f64,
    lat1: f64,
    lon2: f64,
    lat2: f64,
    lon3: f64,
    lat3: f64,
    lon4: f64,
    lat4: f64,
) -> Result<Arc<dyn GeoPolygon>> {
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat1),
        Geo3DUtil::from_degrees(lon1),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat2),
        Geo3DUtil::from_degrees(lon2),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat3),
        Geo3DUtil::from_degrees(lon3),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat4),
        Geo3DUtil::from_degrees(lon4),
    )?;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(point1.clone());
    points.push(point2.clone());
    points.push(point3.clone());
    points.push(point4.clone());
    Ok(GeoPolygonFactory::make_geo_concave_polygon(
        &SPHERE,
        points.clone(),
    )?)
}

fn build_complex_geo_polygon(
    lon1: f64,
    lat1: f64,
    lon2: f64,
    lat2: f64,
    lon3: f64,
    lat3: f64,
    lon4: f64,
    lat4: f64,
) -> Result<Arc<dyn GeoPolygon>> {
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat1),
        Geo3DUtil::from_degrees(lon1),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat2),
        Geo3DUtil::from_degrees(lon2),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat3),
        Geo3DUtil::from_degrees(lon3),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat4),
        Geo3DUtil::from_degrees(lon4),
    )?;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(point1.clone());
    points.push(point2.clone());
    points.push(point3.clone());
    points.push(point4.clone());
    let pd = PolygonDescription::new(points.clone());
    Ok(GeoPolygonFactory::make_large_geo_polygon(
        &SPHERE,
        &vec![pd.clone()],
    )?)
}

fn build_convex_geo_polygon_with_hole(
    lon1: f64,
    lat1: f64,
    lon2: f64,
    lat2: f64,
    lon3: f64,
    lat3: f64,
    lon4: f64,
    lat4: f64,
    hole: Arc<dyn GeoPolygon>,
) -> Result<Arc<dyn GeoPolygon>> {
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat1),
        Geo3DUtil::from_degrees(lon1),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat2),
        Geo3DUtil::from_degrees(lon2),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat3),
        Geo3DUtil::from_degrees(lon3),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat4),
        Geo3DUtil::from_degrees(lon4),
    )?;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(point1.clone());
    points.push(point2.clone());
    points.push(point3.clone());
    points.push(point4.clone());
    // return new GeoConvexPolygon(PlanetModel.SPHERE,points, Collections.singletonList(hole));
    Ok(npe(GeoPolygonFactory::make_geo_polygon_with_holes(
        &SPHERE,
        points.clone(),
        Some(vec![hole.clone()]),
    )?)?)
}

fn build_concave_geo_polygon_with_hole(
    lon1: f64,
    lat1: f64,
    lon2: f64,
    lat2: f64,
    lon3: f64,
    lat3: f64,
    lon4: f64,
    lat4: f64,
    hole: Arc<dyn GeoPolygon>,
) -> Result<Arc<dyn GeoPolygon>> {
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat1),
        Geo3DUtil::from_degrees(lon1),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat2),
        Geo3DUtil::from_degrees(lon2),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat3),
        Geo3DUtil::from_degrees(lon3),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat4),
        Geo3DUtil::from_degrees(lon4),
    )?;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(point1.clone());
    points.push(point2.clone());
    points.push(point3.clone());
    points.push(point4.clone());
    Ok(GeoPolygonFactory::make_geo_concave_polygon_with_holes(
        &SPHERE,
        points.clone(),
        Some(vec![hole.clone()]),
    )?)
}

fn get_composite_shape() -> Result<Arc<dyn GeoShape>> {
    // MULTIPOLYGON(((-145.790967486 -5.17543698881, -145.790854979 -5.11348060995, -145.853073512
    // -5.11339421216, -145.853192037 -5.17535061936, -145.790967486 -5.17543698881)),
    // ((-145.8563923 -5.17527125408, -145.856222168 -5.11332154814, -145.918433943 -5.11317773171,
    // -145.918610092 -5.17512738429, -145.8563923 -5.17527125408)))
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-5.17543698881),
        Geo3DUtil::from_degrees(-145.790967486),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-5.11348060995),
        Geo3DUtil::from_degrees(-145.790854979),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-5.11339421216),
        Geo3DUtil::from_degrees(-145.853073512),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-5.17535061936),
        Geo3DUtil::from_degrees(-145.853192037),
    )?;
    let mut points1: Vec<GeoPoint> = Vec::new();
    points1.push(point1.clone());
    points1.push(point2.clone());
    points1.push(point3.clone());
    points1.push(point4.clone());
    let pol1: Arc<dyn GeoPolygon> = npe(GeoPolygonFactory::make_geo_polygon(
        &SPHERE,
        points1.clone(),
    )?)?;
    let point5 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-5.17527125408),
        Geo3DUtil::from_degrees(-145.8563923),
    )?;
    let point6 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-5.11332154814),
        Geo3DUtil::from_degrees(-145.856222168),
    )?;
    let point7 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-5.11317773171),
        Geo3DUtil::from_degrees(-145.918433943),
    )?;
    let point8 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-5.17512738429),
        Geo3DUtil::from_degrees(-145.918610092),
    )?;
    let mut points2: Vec<GeoPoint> = Vec::new();
    points2.push(point5.clone());
    points2.push(point6.clone());
    points2.push(point7.clone());
    points2.push(point8.clone());
    let pol2: Arc<dyn GeoPolygon> = npe(GeoPolygonFactory::make_geo_polygon(
        &SPHERE,
        points2.clone(),
    )?)?;
    let mut composite = GeoCompositeMembershipShape::new(&SPHERE);
    composite.add_shape(pol1.clone())?;
    composite.add_shape(pol2.clone())?;
    Ok(Arc::new(composite))
}
