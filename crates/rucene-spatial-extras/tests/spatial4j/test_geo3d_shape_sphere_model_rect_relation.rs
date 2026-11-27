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

//! Port of `TestGeo3dShapeSphereModelRectRelation.java` (package
//! `org.apache.lucene.spatial.spatial4j`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene_spatial3d::geom::geo_area::WITHIN;
use rucene_spatial3d::geom::geo_b_box_factory::GeoBBoxFactory;
use rucene_spatial3d::geom::geo_circle_factory::GeoCircleFactory;
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::geo_polygon_factory::GeoPolygonFactory;
use rucene_spatial3d::geom::planet_model::{PlanetModel, SPHERE};
use rucene_spatial_extras::spatial4j::geo3d_shape::Geo3dShape;
use rucene_spatial_extras::spatial4j::geo3d_spatial_context_factory::Geo3dSpatialContextFactory;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::spatial4j::shape_rect_relation_test_case::{
    shape_rect_relation_class, tm, ShapeRectRelationCase, ShapeRectRelationFixture,
    RADIANS_PER_DEGREE,
};

const NAME: &str = "org.apache.lucene.spatial.spatial4j.TestGeo3dShapeSphereModelRectRelation";

/// `TestGeo3dShapeSphereModelRectRelation`.
pub fn class_def() -> Arc<Class> {
    type T = TestGeo3dShapeSphereModelRectRelation;
    shape_rect_relation_class::<T>(
        NAME,
        Constructor::no_args(T::new),
        vec![
            tm::<T>("testFailure1", T::test_failure1).annotate(Test::new()),
            tm::<T>("testFailure2_LUCENE6475", T::test_failure2_lucene6475).annotate(Test::new()),
            tm::<T>("pointBearingTest", T::point_bearing_test).annotate(Test::new()),
        ],
        |_, m| m,
    )
}

/// The `TestGeo3dShapeSphereModelRectRelation` instance.
pub struct TestGeo3dShapeSphereModelRectRelation {
    srr: ShapeRectRelationFixture,
    /// `planetModel`.
    planet_model: Arc<PlanetModel>,
}

impl ShapeRectRelationCase for TestGeo3dShapeSphereModelRectRelation {
    fn srr(&self) -> &ShapeRectRelationFixture {
        &self.srr
    }
}

impl TestGeo3dShapeSphereModelRectRelation {
    /// `TestGeo3dShapeSphereModelRectRelation()`.
    fn new() -> Self {
        let planet_model = Arc::clone(&SPHERE);
        let mut srr = ShapeRectRelationFixture::new();
        let mut factory = Geo3dSpatialContextFactory::new();
        factory.planet_model = Some(Arc::clone(&planet_model));
        srr.ctx = factory
            .new_spatial_context()
            .unwrap_or_else(|e| rethrow_java(e));
        Self { srr, planet_model }
    }

    /// `testFailure1()`.
    fn test_failure1(&mut self) -> Result<()> {
        let pm = &self.planet_model;
        let rect = GeoBBoxFactory::make_geo_bbox(
            pm,
            88.0 * RADIANS_PER_DEGREE,
            30.0 * RADIANS_PER_DEGREE,
            -30.0 * RADIANS_PER_DEGREE,
            62.0 * RADIANS_PER_DEGREE,
        )?;
        let points = vec![
            GeoPoint::new_lat_lon(
                pm,
                30.4579218227 * RADIANS_PER_DEGREE,
                14.5238410082 * RADIANS_PER_DEGREE,
            )?,
            GeoPoint::new_lat_lon(
                pm,
                43.684447915 * RADIANS_PER_DEGREE,
                46.2210986329 * RADIANS_PER_DEGREE,
            )?,
            GeoPoint::new_lat_lon(
                pm,
                66.2465299717 * RADIANS_PER_DEGREE,
                -29.1786158537 * RADIANS_PER_DEGREE,
            )?,
        ];
        let path = GeoPolygonFactory::make_geo_polygon(pm, points)?
            .expect("java.lang.NullPointerException: makeGeoPolygon returned null");
        let point = GeoPoint::new_lat_lon(
            pm,
            34.2730264413182 * RADIANS_PER_DEGREE,
            82.75500168892472 * RADIANS_PER_DEGREE,
        )?;
        // Apparently the rectangle thinks the polygon is completely within it... "shape inside
        // rectangle"
        Assert::assert_true(WITHIN == rect.get_relationship(path.as_geo_shape())?);

        // Point is within path? Apparently not...
        Assert::assert_false(path.is_within(&point));

        // If it is within the path, it must be within the rectangle, and similarly visa versa
        Assert::assert_false(rect.is_within(&point));
        Ok(())
    }

    /// `testFailure2_LUCENE6475()`.
    fn test_failure2_lucene6475(&mut self) -> Result<()> {
        let geo3d_circle = GeoCircleFactory::make_geo_circle(
            &self.planet_model,
            1.6282053147165243E-4 * RADIANS_PER_DEGREE,
            -70.1600629789353 * RADIANS_PER_DEGREE,
            86.0 * RADIANS_PER_DEGREE,
        )?;
        let geo3d_shape = Geo3dShape::new(geo3d_circle, Arc::clone(&self.srr.ctx));
        let rect = self
            .srr
            .ctx
            .get_shape_factory()
            .rect(-118.0, -114.0, -2.0, 32.0)?;
        Assert::assert_true(geo3d_shape.relate(rect.as_shape()).intersects());
        // thus the bounding box must intersect too
        Assert::assert_true(
            geo3d_shape
                .get_bounding_box()
                .relate(rect.as_shape())
                .intersects(),
        );
        Ok(())
    }

    /// `pointBearingTest()`.
    fn point_bearing_test(&mut self) -> Result<()> {
        point_bearing_test(&self.srr)
    }
}

/// `pointBearingTest()` (declared by both `TestGeo3dShape*RectRelation` classes).
pub fn point_bearing_test(srr: &ShapeRectRelationFixture) -> Result<()> {
    let ctx = &srr.ctx;
    let radius = 136.0;
    let distance = 135.97;
    let bearing = 188.0;
    let p = ctx.get_shape_factory().point_xy(35.0, 85.0)?;
    let circle = ctx.get_shape_factory().circle_at(Arc::clone(&p), radius)?;
    let b_point = ctx
        .get_dist_calc()
        .point_on_bearing(&p, distance, bearing, ctx, None)?;

    let d = ctx.get_dist_calc().distance(&*p, &*b_point);
    Assert::assert_equals_double_msg(None, d, distance, f64::from(10 - 8));

    Assert::assert_equals(circle.relate(b_point.as_shape()), SpatialRelation::Contains);
    Ok(())
}
