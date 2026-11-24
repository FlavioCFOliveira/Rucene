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

//! Port of `TestCompositeGeoPolygonRelationships.java`.

use crate::spatial3d_support::prelude::*;

/// `TestCompositeGeoPolygonRelationships`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestCompositeGeoPolygonRelationships",
        vec![
            test("testGeoCompositePolygon1", test_geo_composite_polygon1).annotate(Test::new()),
            test("testGeoCompositePolygon2", test_geo_composite_polygon2).annotate(Test::new()),
            test("testGeoCompositePolygon3", test_geo_composite_polygon3).annotate(Test::new()),
            test("testGeoCompositePolygon4", test_geo_composite_polygon4).annotate(Test::new()),
            test("testGeoCompositePolygon5", test_geo_composite_polygon5).annotate(Test::new()),
            test("testGeoCompositePolygon6", test_geo_composite_polygon6).annotate(Test::new()),
            test("testGeoCompositePolygon7", test_geo_composite_polygon7).annotate(Test::new()),
            test("testGeoCompositePolygon8", test_geo_composite_polygon8).annotate(Test::new()),
            test("testGeoPolygonPole1", test_geo_polygon_pole1).annotate(Test::new()),
            test("testGeoPolygonPole2", test_geo_polygon_pole2).annotate(Test::new()),
            test("testGeoPolygonPole3", test_geo_polygon_pole3).annotate(Test::new()),
            test("testMultiPolygon1", test_multi_polygon1).annotate(Test::new()),
            test("testMultiPolygon2", test_multi_polygon2).annotate(Test::new()),
            test("testMultiPolygon3", test_multi_polygon3).annotate(Test::new()),
            test("testMultiPolygon4", test_multi_polygon4).annotate(Test::new()),
            test("testMultiPolygon5", test_multi_polygon5).annotate(Test::new()),
        ],
    )
}

fn test_geo_composite_polygon1() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 21 -61,19.845091 -60.452631))
    let original_convex_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
        21.0, -61.0,
    )?;
    // POLYGON ((19.845091 -60.452631, 21 -61,22.820804 -60.257713,23.207901 -61.453298, 20.119948
    // -61.655652, 19.845091 -60.452631))
    let original_concave_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 21.0, -61.0, 22.820804, -60.257713, 23.207901, -61.453298, 20.119948,
        -61.655652,
    )?;
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        20.0, -60.4, 20.1, -60.4, 20.1, -60.3, 20.0, -60.3, 20.0, -60.3,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.0, -60.4, 20.1, -60.4, 20.1, -60.3, 20.0, -60.3)?;
    // convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(WITHIN, rel);
    // concave
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_geo_composite_polygon2() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 21 -61,19.845091 -60.452631))
    let original_convex_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
        21.0, -61.0,
    )?;
    // POLYGON ((19.845091 -60.452631, 21 -61,22.820804 -60.257713,23.207901 -61.453298, 20.119948
    // -61.655652, 19.845091 -60.452631))
    let original_concave_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 21.0, -61.0, 22.820804, -60.257713, 23.207901, -61.453298, 20.119948,
        -61.655652,
    )?;
    // POLYGON ((20.9 -60.8, 21.1 -60.8, 21.1 -60.6, 20.9  -60.6,20.9 -60.8))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        20.9, -60.8, 21.1, -60.8, 21.1, -60.6, 20.9, -60.6, 20.9, -60.6,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.9, -60.8, 21.1, -60.8, 21.1, -60.6, 20.9, -60.6)?;
    // convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(WITHIN, rel);
    // concave
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_geo_composite_polygon3() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 21 -61,19.845091 -60.452631))
    let original_convex_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
        21.0, -61.0,
    )?;
    // POLYGON ((19.845091 -60.452631, 21 -61,22.820804 -60.257713,23.207901 -61.453298, 20.119948
    // -61.655652, 19.845091 -60.452631))
    let original_concave_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 21.0, -61.0, 22.820804, -60.257713, 23.207901, -61.453298, 20.119948,
        -61.655652,
    )?;
    // POLYGON ((20.9 -61.1, 21.1 -61.1, 21.1 -60.9, 20.9  -60.9,20.9 -61.1))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        20.9, -61.1, 21.1, -61.1, 21.1, -60.9, 20.9, -60.9, 20.9, -60.9,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.9, -61.1, 21.1, -61.1, 21.1, -60.9, 20.9, -60.9)?;
    // convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // concave
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_geo_composite_polygon4() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 21 -61,19.845091 -60.452631))
    let original_convex_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
        21.0, -61.0,
    )?;
    // POLYGON ((19.845091 -60.452631, 21 -61,22.820804 -60.257713,23.207901 -61.453298, 20.119948
    // -61.655652, 19.845091 -60.452631))
    let original_concave_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 21.0, -61.0, 22.820804, -60.257713, 23.207901, -61.453298, 20.119948,
        -61.655652,
    )?;
    // POLYGON ((20.9 -61.4, 21.1 -61.4, 21.1 -61.2, 20.9  -61.2,20.9 -61.4))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        20.9, -61.4, 21.1, -61.4, 21.1, -61.2, 20.9, -61.2, 20.9, -61.2,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.9, -61.4, 21.1, -61.4, 21.1, -61.2, 20.9, -61.2)?;
    // convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // concave
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(WITHIN, rel);
    Ok(())
}

fn test_geo_composite_polygon5() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 21 -61,19.845091 -60.452631))
    let origina_convexl_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
        21.0, -61.0,
    )?;
    // POLYGON ((19.845091 -60.452631, 21 -61,22.820804 -60.257713,23.207901 -61.453298, 20.119948
    // -61.655652, 19.845091 -60.452631))
    let original_concave_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 21.0, -61.0, 22.820804, -60.257713, 23.207901, -61.453298, 20.119948,
        -61.655652,
    )?;
    // POLYGON ((19 -62, 23 -62, 23 -60, 19 -60,19 -62))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.0, -62.0, 23.0, -62.0, 23.0, -60.0, 19.0, -60.0, 19.0, -60.0,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(19.0, -62.0, 23.0, -62.0, 23.0, -60.0, 19.0, -60.0)?;
    // convex
    let mut rel = origina_convexl_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*origina_convexl_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = origina_convexl_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*origina_convexl_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // concave
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_geo_composite_polygon6() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 21 -61,19.845091 -60.452631))
    let original_convex_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
        21.0, -61.0,
    )?;
    // POLYGON ((19.845091 -60.452631, 21 -61,22.820804 -60.257713,23.207901 -61.453298, 20.119948
    // -61.655652, 19.845091 -60.452631))
    let original_concave_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 21.0, -61.0, 22.820804, -60.257713, 23.207901, -61.453298, 20.119948,
        -61.655652,
    )?;
    // POLYGON ((19 -62, 24 -62, 24 -60, 19 -60,19 -62))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.0, -62.0, 24.0, -62.0, 24.0, -60.0, 19.0, -60.0, 19.0, -60.0,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(19.0, -62.0, 24.0, -62.0, 24.0, -60.0, 19.0, -60.0)?;
    // convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(WITHIN, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    // concave
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = original_concave_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_concave.get_relationship(&*original_concave_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    Ok(())
}

fn test_geo_composite_polygon7() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713, 21 -61,19.845091 -60.452631))
    let original_convex_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
        21.0, -61.0,
    )?;
    // POLYGON ((19.845091 -60.452631, 21 -61,22.820804 -60.257713,23.207901 -61.453298, 20.119948
    // -61.655652, 19.845091 -60.452631))
    let original_concave_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 21.0, -61.0, 22.820804, -60.257713, 23.207901, -61.453298, 20.119948,
        -61.655652,
    )?;
    // POLYGON ((20.2 -61.4, 20.5 -61.4, 20.5 -60.8, 20.2 -60.8,20.2  -61.4))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        20.2, -61.4, 20.5, -61.4, 20.5, -60.8, 20.2, -60.8, 20.2, -60.8,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.2, -61.4, 20.5, -61.4, 20.5, -60.8, 20.2, -60.8)?;
    // convex
    let mut rel = original_convex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    // concave
    rel = original_concave_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = original_convex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*original_convex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_geo_composite_polygon8() -> Result<()> {
    // POLYGON ((19.845091 -60.452631, 20.119948 -61.655652, 23.207901 -61.453298, 22.820804
    // -60.257713,21 -61, 19.845091 -60.452631))
    let original_pol: Arc<dyn GeoPolygon> = build_geo_polygon(
        19.84509, -60.452631, 20.119948, -61.655652, 23.207901, -61.453298, 22.820804, -60.257713,
        21.0, -61.0,
    )?;
    let shape: Arc<dyn GeoShape> = get_inside_composite_shape()?;
    let rel = original_pol.get_relationship(&*shape)?;
    Assert::assert_equals(WITHIN, rel);
    Ok(())
}

fn test_geo_polygon_pole1() -> Result<()> {
    // POLYGON((0 80, 45 85 ,90 80,135 85,180 80, -135 85, -90 80, -45 85,0 80))
    let composite_pol: Arc<dyn GeoPolygon> = get_composite_polygon()?;
    let complex_pol: Arc<dyn GeoPolygon> = get_complex_polygon()?;
    // POLYGON ((20.9 -61.4, 21.1 -61.4, 21.1 -61.2, 20.9  -61.2,20.9 -61.4))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        20.9, -61.4, 21.1, -61.4, 21.1, -61.2, 20.9, -61.2, 20.9, -61.2,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(20.9, -61.4, 21.1, -61.4, 21.1, -61.2, 20.9, -61.2)?;
    let mut rel = composite_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*composite_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = composite_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*composite_pol)?;
    Assert::assert_equals(WITHIN, rel);
    rel = complex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*complex_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = complex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*complex_pol)?;
    Assert::assert_equals(WITHIN, rel);
    Ok(())
}

fn test_geo_polygon_pole2() -> Result<()> {
    // POLYGON((0 80, 45 85 ,90 80,135 85,180 80, -135 85, -90 80, -45 85,0 80))
    let composite_pol: Arc<dyn GeoPolygon> = get_composite_polygon()?;
    let complex_pol: Arc<dyn GeoPolygon> = get_complex_polygon()?;
    // POLYGON((-1 81, -1 79,1 79,1 81, -1 81))
    let pol_convex: Arc<dyn GeoPolygon> =
        build_geo_polygon(-1.0, 81.0, -1.0, 79.0, 1.0, 79.0, 1.0, 81.0, 1.0, 81.0)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-1.0, 81.0, -1.0, 79.0, 1.0, 79.0, 1.0, 81.0)?;
    let mut rel = composite_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*composite_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = composite_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*composite_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = complex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*complex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = complex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*complex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_geo_polygon_pole3() -> Result<()> {
    // POLYGON((0 80, 45 85 ,90 80,135 85,180 80, -135 85, -90 80, -45 85,0 80))
    let composite_pol: Arc<dyn GeoPolygon> = get_composite_polygon()?;
    let complex_pol: Arc<dyn GeoPolygon> = get_complex_polygon()?;
    // POLYGON((-1 86, -1 84,1 84,1 86, -1 86))
    let pol_convex: Arc<dyn GeoPolygon> =
        build_geo_polygon(-1.0, 86.0, -1.0, 84.0, 1.0, 84.0, 1.0, 86.0, 1.0, 86.0)?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-1.0, 86.0, -1.0, 84.0, 1.0, 84.0, 1.0, 86.0)?;
    let mut rel = composite_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*composite_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = composite_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*composite_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = complex_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*complex_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = complex_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*complex_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Ok(())
}

fn test_multi_polygon1() -> Result<()> {
    // MULTIPOLYGON(((-145.790967486 -5.17543698881, -145.790854979 -5.11348060995, -145.853073512
    // -5.11339421216, -145.853192037 -5.17535061936, -145.790967486 -5.17543698881)),
    // ((-145.8563923 -5.17527125408, -145.856222168 -5.11332154814, -145.918433943 -5.11317773171,
    // -145.918610092 -5.17512738429, -145.8563923 -5.17527125408)))
    let multi_pol: Arc<dyn GeoPolygon> = get_multi_polygon()?;
    // POLYGON((-145.8555 -5.13, -145.8540 -5.13, -145.8540 -5.12, -145.8555 -5.12, -145.8555
    // -5.13))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        -145.8555, -5.13, -145.8540, -5.13, -145.8540, -5.12, -145.8555, -5.12, -145.8555, -5.12,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        -145.8555, -5.13, -145.8540, -5.13, -145.8540, -5.12, -145.8555, -5.12,
    )?;
    let mut rel = multi_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_convex.get_relationship(&*multi_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    Assert::assert_equals(false, multi_pol.intersects_shape(&*pol_convex));
    Assert::assert_equals(false, pol_convex.intersects_shape(&*multi_pol));
    rel = multi_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_concave.get_relationship(&*multi_pol)?;
    Assert::assert_equals(WITHIN, rel);
    Assert::assert_equals(false, multi_pol.intersects_shape(&*pol_concave));
    Assert::assert_equals(false, pol_concave.intersects_shape(&*multi_pol));
    Ok(())
}

fn test_multi_polygon2() -> Result<()> {
    // MULTIPOLYGON(((-145.790967486 -5.17543698881, -145.790854979 -5.11348060995, -145.853073512
    // -5.11339421216, -145.853192037 -5.17535061936, -145.790967486 -5.17543698881)),
    // ((-145.8563923 -5.17527125408, -145.856222168 -5.11332154814, -145.918433943 -5.11317773171,
    // -145.918610092 -5.17512738429, -145.8563923 -5.17527125408)))
    let multi_pol: Arc<dyn GeoPolygon> = get_multi_polygon()?;
    // POLYGON((-145.8555 -5.13, -145.85 -5.13, -145.85 -5.12, -145.8555 -5.12, -145.8555 -5.13))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        -145.8555, -5.13, -145.85, -5.13, -145.85, -5.12, -145.8555, -5.12, -145.8555, -5.12,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        -145.8555, -5.13, -145.85, -5.13, -145.85, -5.12, -145.8555, -5.12,
    )?;
    let mut rel = multi_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*multi_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Assert::assert_equals(true, multi_pol.intersects_shape(&*pol_convex));
    Assert::assert_equals(true, pol_convex.intersects_shape(&*multi_pol));
    rel = multi_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*multi_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Assert::assert_equals(true, multi_pol.intersects_shape(&*pol_concave));
    Assert::assert_equals(true, pol_concave.intersects_shape(&*multi_pol));
    Ok(())
}

fn test_multi_polygon3() -> Result<()> {
    // MULTIPOLYGON(((-145.790967486 -5.17543698881, -145.790854979 -5.11348060995, -145.853073512
    // -5.11339421216, -145.853192037 -5.17535061936, -145.790967486 -5.17543698881)),
    // ((-145.8563923 -5.17527125408, -145.856222168 -5.11332154814, -145.918433943 -5.11317773171,
    // -145.918610092 -5.17512738429, -145.8563923 -5.17527125408)))
    let multi_pol: Arc<dyn GeoPolygon> = get_multi_polygon()?;
    // POLYGON((-146 -5.18, -145.854 -5.18, -145.854 -5.11, -146 -5.11, -146 -5.18))
    // Case overlapping one of the polygons so intersection is false!
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        -146.0, -5.18, -145.854, -5.18, -145.854, -5.11, -146.0, -5.11, -146.0, -5.11,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        -146.0, -5.18, -145.854, -5.18, -145.854, -5.11, -146.0, -5.11,
    )?;
    let mut rel = multi_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_convex.get_relationship(&*multi_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Assert::assert_equals(false, multi_pol.intersects_shape(&*pol_convex));
    Assert::assert_equals(false, pol_convex.intersects_shape(&*multi_pol));
    rel = multi_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*multi_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Assert::assert_equals(false, multi_pol.intersects_shape(&*pol_concave));
    Assert::assert_equals(false, pol_concave.intersects_shape(&*multi_pol));
    Ok(())
}

fn test_multi_polygon4() -> Result<()> {
    // MULTIPOLYGON(((-145.790967486 -5.17543698881, -145.790854979 -5.11348060995, -145.853073512
    // -5.11339421216, -145.853192037 -5.17535061936, -145.790967486 -5.17543698881)),
    // ((-145.8563923 -5.17527125408, -145.856222168 -5.11332154814, -145.918433943 -5.11317773171,
    // -145.918610092 -5.17512738429, -145.8563923 -5.17527125408)))
    let multi_pol: Arc<dyn GeoPolygon> = get_multi_polygon()?;
    // POLYGON((-145.88 -5.13, -145.87 -5.13, -145.87 -5.12, -145.88 -5.12, -145.88 -5.13))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        -145.88, -5.13, -145.87, -5.13, -145.87, -5.12, -145.88, -5.12, -145.88, -5.12,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> = build_concave_geo_polygon(
        -145.88, -5.13, -145.87, -5.13, -145.87, -5.12, -145.88, -5.12,
    )?;
    let mut rel = multi_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(WITHIN, rel);
    rel = pol_convex.get_relationship(&*multi_pol)?;
    Assert::assert_equals(CONTAINS, rel);
    Assert::assert_equals(false, multi_pol.intersects_shape(&*pol_convex));
    Assert::assert_equals(false, pol_convex.intersects_shape(&*multi_pol));
    rel = multi_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(OVERLAPS, rel);
    rel = pol_concave.get_relationship(&*multi_pol)?;
    Assert::assert_equals(OVERLAPS, rel);
    Assert::assert_equals(false, multi_pol.intersects_shape(&*pol_concave));
    Assert::assert_equals(false, pol_concave.intersects_shape(&*multi_pol));
    Ok(())
}

fn test_multi_polygon5() -> Result<()> {
    // MULTIPOLYGON(((-145.790967486 -5.17543698881, -145.790854979 -5.11348060995, -145.853073512
    // -5.11339421216, -145.853192037 -5.17535061936, -145.790967486 -5.17543698881)),
    // ((-145.8563923 -5.17527125408, -145.856222168 -5.11332154814, -145.918433943 -5.11317773171,
    // -145.918610092 -5.17512738429, -145.8563923 -5.17527125408)))
    let multi_pol: Arc<dyn GeoPolygon> = get_multi_polygon()?;
    // POLYGON((-146 -5.18, -145 -5.18, -145 -5.11, -146 -5.11, -146 -5.18))
    let pol_convex: Arc<dyn GeoPolygon> = build_geo_polygon(
        -146.0, -5.18, -145.0, -5.18, -145.0, -5.11, -146.0, -5.11, -146.0, -5.11,
    )?;
    let pol_concave: Arc<dyn GeoPolygon> =
        build_concave_geo_polygon(-146.0, -5.18, -145.0, -5.18, -145.0, -5.11, -146.0, -5.11)?;
    let mut rel = multi_pol.get_relationship(&*pol_convex)?;
    Assert::assert_equals(CONTAINS, rel);
    rel = pol_convex.get_relationship(&*multi_pol)?;
    Assert::assert_equals(WITHIN, rel);
    Assert::assert_equals(false, multi_pol.intersects_shape(&*pol_convex));
    rel = multi_pol.get_relationship(&*pol_concave)?;
    Assert::assert_equals(DISJOINT, rel);
    rel = pol_concave.get_relationship(&*multi_pol)?;
    Assert::assert_equals(DISJOINT, rel);
    Assert::assert_equals(false, multi_pol.intersects_shape(&*pol_concave));
    Ok(())
}

fn build_geo_polygon(
    lon1: f64,
    lat1: f64,
    lon2: f64,
    lat2: f64,
    lon3: f64,
    lat3: f64,
    lon4: f64,
    lat4: f64,
    lon5: f64,
    lat5: f64,
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
    let point5 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(lat5),
        Geo3DUtil::from_degrees(lon5),
    )?;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(point1.clone());
    points.push(point2.clone());
    points.push(point3.clone());
    points.push(point4.clone());
    points.push(point5.clone());
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

fn get_composite_polygon() -> Result<Arc<dyn GeoPolygon>> {
    // POLYGON((0 80, 45 85 ,90 80,135 85,180 80, -135 85, -90 80, -45 85,0 80))
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(80.0),
        Geo3DUtil::from_degrees(0.0),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(85.0),
        Geo3DUtil::from_degrees(45.0),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(80.0),
        Geo3DUtil::from_degrees(90.0),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(85.0),
        Geo3DUtil::from_degrees(135.0),
    )?;
    let point5 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(80.0),
        Geo3DUtil::from_degrees(180.0),
    )?;
    let point6 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(85.0),
        Geo3DUtil::from_degrees(-135.0),
    )?;
    let point7 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(80.0),
        Geo3DUtil::from_degrees(-90.0),
    )?;
    let point8 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(85.0),
        Geo3DUtil::from_degrees(-45.0),
    )?;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(point1.clone());
    points.push(point2.clone());
    points.push(point3.clone());
    points.push(point4.clone());
    points.push(point5.clone());
    points.push(point6.clone());
    points.push(point7.clone());
    points.push(point8.clone());
    Ok(npe(GeoPolygonFactory::make_geo_polygon(
        &SPHERE,
        points.clone(),
    )?)?)
}

fn get_complex_polygon() -> Result<Arc<dyn GeoPolygon>> {
    // POLYGON((0 80, 45 85 ,90 80,135 85,180 80, -135 85, -90 80, -45 85,0 80))
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(80.0),
        Geo3DUtil::from_degrees(0.0),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(85.0),
        Geo3DUtil::from_degrees(45.0),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(80.0),
        Geo3DUtil::from_degrees(90.0),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(85.0),
        Geo3DUtil::from_degrees(135.0),
    )?;
    let point5 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(80.0),
        Geo3DUtil::from_degrees(180.0),
    )?;
    let point6 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(85.0),
        Geo3DUtil::from_degrees(-135.0),
    )?;
    let point7 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(80.0),
        Geo3DUtil::from_degrees(-90.0),
    )?;
    let point8 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(85.0),
        Geo3DUtil::from_degrees(-45.0),
    )?;
    let mut points: Vec<GeoPoint> = Vec::new();
    points.push(point1.clone());
    points.push(point2.clone());
    points.push(point3.clone());
    points.push(point4.clone());
    points.push(point5.clone());
    points.push(point6.clone());
    points.push(point7.clone());
    points.push(point8.clone());
    let pd = PolygonDescription::new(points.clone());
    Ok(GeoPolygonFactory::make_large_geo_polygon(
        &SPHERE,
        &vec![pd.clone()],
    )?)
}

fn get_multi_polygon() -> Result<Arc<dyn GeoPolygon>> {
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
    let pd1 = PolygonDescription::new(points1.clone());
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
    let pd2 = PolygonDescription::new(points2.clone());
    let mut pds: Vec<PolygonDescription> = Vec::new();
    pds.push(pd1.clone());
    pds.push(pd2.clone());
    Ok(GeoPolygonFactory::make_large_geo_polygon(&SPHERE, &pds)?)
}

fn get_inside_composite_shape() -> Result<Arc<dyn GeoShape>> {
    // MULTIPOLYGON(((19.945091 -60.552631, 20.319948 -61.555652, 20.9 -61.5, 20.9 -61, 19.945091
    // -60.552631)),
    // ((21.1 -61.5,  23.107901 -61.253298, 22.720804 -60.457713,21.1 -61, 21.1 -61.5)))
    let point1 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-60.552631),
        Geo3DUtil::from_degrees(19.945091),
    )?;
    let point2 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.555652),
        Geo3DUtil::from_degrees(20.319948),
    )?;
    let point3 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.5),
        Geo3DUtil::from_degrees(20.9),
    )?;
    let point4 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.0),
        Geo3DUtil::from_degrees(20.9),
    )?;
    let mut points1: Vec<GeoPoint> = Vec::new();
    points1.push(point1.clone());
    points1.push(point2.clone());
    points1.push(point3.clone());
    points1.push(point4.clone());
    let point5 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.5),
        Geo3DUtil::from_degrees(21.1),
    )?;
    let point6 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.253298),
        Geo3DUtil::from_degrees(23.107901),
    )?;
    let point7 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-60.457713),
        Geo3DUtil::from_degrees(22.720804),
    )?;
    let point8 = GeoPoint::new_lat_lon(
        &SPHERE,
        Geo3DUtil::from_degrees(-61.0),
        Geo3DUtil::from_degrees(21.1),
    )?;
    let mut points2: Vec<GeoPoint> = Vec::new();
    points2.push(point5.clone());
    points2.push(point6.clone());
    points2.push(point7.clone());
    points2.push(point8.clone());
    let p1: Arc<dyn GeoPolygon> = npe(GeoPolygonFactory::make_geo_polygon(
        &SPHERE,
        points1.clone(),
    )?)?;
    let p2: Arc<dyn GeoPolygon> = npe(GeoPolygonFactory::make_geo_polygon(
        &SPHERE,
        points2.clone(),
    )?)?;
    let mut composite_membership_shape = GeoCompositeMembershipShape::new(&SPHERE);
    composite_membership_shape.add_shape(p1.clone())?;
    composite_membership_shape.add_shape(p2.clone())?;
    Ok(Arc::new(composite_membership_shape))
}
