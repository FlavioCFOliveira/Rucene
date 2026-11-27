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

//! Port of `TestGeo3d.java` (package `org.apache.lucene.spatial.spatial4j`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene_spatial_extras::spatial4j::geo3d_shape::geo3d_shape_of;
use rucene_spatial_extras::spatial4j::geo3d_spatial_context_factory::Geo3dSpatialContextFactory;
use rucene_test_framework::util::java_exceptions::expect_throws;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;
use spatial4j::exception::invalid_shape_exception::InvalidShapeException;
use spatial4j::io::shape_reader::ShapeReader;
use spatial4j::shape::shape::Shape;

use crate::core_support::{class, test};

/// `TestGeo3d`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial.spatial4j.TestGeo3d",
        vec![
            test("testWKT", test_wkt).annotate(Test::new()),
            test(
                "testPolygonWithCoplanarPoints",
                test_polygon_with_coplanar_points,
            )
            .annotate(Test::new()),
        ],
    )
}

/// `ctx.getFormats().getWktReader()`.
fn wkt_reader(ctx: &SpatialContext) -> Arc<dyn ShapeReader> {
    ctx.get_formats()
        .get_wkt_reader()
        .expect("java.lang.NullPointerException: no WKT reader")
}

/// `ctx.getFormats().getWktReader().read(wkt)` then `assertTrue(s instanceof Geo3dShape<?>)`.
fn assert_reads_geo3d(ctx: &SpatialContext, wkt: &str) -> Result<()> {
    let s: Option<Arc<dyn Shape>> = wkt_reader(ctx).read(wkt)?;
    Assert::assert_true(s.as_deref().is_some_and(|s| geo3d_shape_of(s).is_some()));
    Ok(())
}

/// `testWKT()`.
fn test_wkt() -> Result<()> {
    let mut factory = Geo3dSpatialContextFactory::new();
    let ctx = factory.new_spatial_context()?;
    assert_reads_geo3d(
        &ctx,
        "POLYGON ((20.0 -60.4, 20.1 -60.4, 20.1 -60.3, 20.0  -60.3,20.0 -60.4))",
    )?;
    assert_reads_geo3d(&ctx, "POINT (30 10)")?;
    assert_reads_geo3d(&ctx, "LINESTRING (30 10, 10 30, 40 40)")?;
    assert_reads_geo3d(
        &ctx,
        "POLYGON ((35 10, 45 45, 15 40, 10 20, 35 10), (20 30, 35 35, 30 20, 20 30))",
    )?;
    assert_reads_geo3d(&ctx, "MULTIPOINT ((10 40), (40 30), (20 20), (30 10))")?;
    assert_reads_geo3d(
        &ctx,
        "MULTILINESTRING ((10 10, 20 20, 10 40),(40 40, 30 30, 40 20, 30 10))",
    )?;
    assert_reads_geo3d(
        &ctx,
        "MULTIPOLYGON (((40 40, 20 45, 45 30, 40 40)), ((20 35, 10 30, 10 10, 30 5, 45 20, 20 35),(30 20, 20 15, 20 25, 30 20)))",
    )?;
    assert_reads_geo3d(&ctx, "GEOMETRYCOLLECTION(POINT(4 6),LINESTRING(4 6,7 10))")?;
    assert_reads_geo3d(&ctx, "ENVELOPE(1, 2, 4, 3)")?;
    assert_reads_geo3d(&ctx, "BUFFER(POINT(-10 30), 5.2)")?;
    // wkt = "BUFFER(LINESTRING(1 2, 3 4), 0.5)";
    // s = ctx.getFormats().getWktReader().read(wkt);
    // assertTrue(s instanceof  Geo3dShape<?>);
    Ok(())
}

/// `testPolygonWithCoplanarPoints()`.
fn test_polygon_with_coplanar_points() -> Result<()> {
    let mut factory = Geo3dSpatialContextFactory::new();
    let ctx = factory.new_spatial_context()?;

    let polygon = "POLYGON ((-180 90, -180 -90, 180 -90, 180 90,-180 -90))";
    expect_throws(InvalidShapeException::CLASS, || {
        wkt_reader(&ctx).read(polygon)
    });

    let polygon_with_hole = "POLYGON ((35 10, 45 45, 15 40, 10 20, 35 10), (20 30, 20 30, 20 30))";
    expect_throws(InvalidShapeException::CLASS, || {
        wkt_reader(&ctx).read(polygon_with_hole)
    });

    let geometry_collection = "GEOMETRYCOLLECTION(POINT(4 6), LINESTRING(4 6,7 10), POLYGON ((-180 90, -180 -90, 180 -90, 180 90,-180 -90)))";
    expect_throws(InvalidShapeException::CLASS, || {
        wkt_reader(&ctx).read(geometry_collection)
    });

    let multi_polygon = "MULTIPOLYGON (((40 40, 20 45, 45 30, 40 40)), ((20 35, 10 30, 10 10, 30 5, 45 20, 20 35),(30 20, 20 15, 20 25, 30 20)), ((180 90, 90 90, 180 90)))";
    expect_throws(InvalidShapeException::CLASS, || {
        wkt_reader(&ctx).read(multi_polygon)
    });
    Ok(())
}
