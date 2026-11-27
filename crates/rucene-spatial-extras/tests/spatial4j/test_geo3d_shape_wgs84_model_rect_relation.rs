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

//! Port of `TestGeo3dShapeWGS84ModelRectRelation.java` (package
//! `org.apache.lucene.spatial.spatial4j`).

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene_spatial3d::geom::geo_area::{DISJOINT, OVERLAPS};
use rucene_spatial3d::geom::geo_b_box_factory::GeoBBoxFactory;
use rucene_spatial3d::geom::geo_circle_factory::GeoCircleFactory;
use rucene_spatial3d::geom::geo_path_factory::GeoPathFactory;
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::planet_model::{PlanetModel, CLARKE_1866, WGS84};
use rucene_spatial_extras::spatial4j::geo3d_shape_factory::Geo3dShapeFactory;
use rucene_spatial_extras::spatial4j::geo3d_spatial_context_factory::Geo3dSpatialContextFactory;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::Nightly;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;

use crate::spatial4j::shape_rect_relation_test_case::{
    shape_rect_relation_class, tm, ShapeRectRelationCase, ShapeRectRelationFixture,
    RADIANS_PER_DEGREE,
};
use crate::spatial4j::test_geo3d_shape_sphere_model_rect_relation::point_bearing_test;
use crate::spatial_support::random;

const NAME: &str = "org.apache.lucene.spatial.spatial4j.TestGeo3dShapeWGS84ModelRectRelation";

/// `TestGeo3dShapeWGS84ModelRectRelation`: overrides `testGeoCircleRect`,
/// `testGeoPolygonRect` and `testGeoPathRect` as `@Nightly` ("very slow,
/// test sources are not all here, no clue how to fix it"), each calling
/// `super`.
pub fn class_def() -> Arc<Class> {
    type T = TestGeo3dShapeWGS84ModelRectRelation;
    shape_rect_relation_class::<T>(
        NAME,
        Constructor::no_args(T::new),
        vec![
            tm::<T>("testFailure1", T::test_failure1).annotate(Test::new()),
            tm::<T>("testFailure2", T::test_failure2).annotate(Test::new()),
            tm::<T>("testFailure3", T::test_failure3).annotate(Test::new()),
            tm::<T>("pointBearingTest", T::point_bearing_test).annotate(Test::new()),
        ],
        |name, m| match name {
            "testGeoCircleRect" | "testGeoPolygonRect" | "testGeoPathRect" => m.annotate(Nightly),
            _ => m,
        },
    )
}

/// The `TestGeo3dShapeWGS84ModelRectRelation` instance.
pub struct TestGeo3dShapeWGS84ModelRectRelation {
    srr: ShapeRectRelationFixture,
    /// `planetModel`.
    planet_model: Arc<PlanetModel>,
}

impl ShapeRectRelationCase for TestGeo3dShapeWGS84ModelRectRelation {
    fn srr(&self) -> &ShapeRectRelationFixture {
        &self.srr
    }
}

impl TestGeo3dShapeWGS84ModelRectRelation {
    /// The field initializers and `TestGeo3dShapeWGS84ModelRectRelation()`.
    fn new() -> Self {
        let mut srr = ShapeRectRelationFixture::new();
        let planet_model = Arc::clone(RandomPicks::random_from(
            &*random(),
            &[Arc::clone(&WGS84), Arc::clone(&CLARKE_1866)],
        ));
        let mut factory = Geo3dSpatialContextFactory::new();
        factory.planet_model = Some(Arc::clone(&planet_model));
        srr.ctx = factory
            .new_spatial_context()
            .unwrap_or_else(|e| rethrow_java(e));
        srr.max_radius = 175;
        srr.ctx
            .get_shape_factory()
            .as_any()
            .downcast_ref::<Geo3dShapeFactory>()
            .expect("java.lang.ClassCastException: the shape factory is a Geo3dShapeFactory")
            .set_circle_accuracy(1e-12);
        Self { srr, planet_model }
    }

    /// `testFailure1()`.
    fn test_failure1(&mut self) -> Result<()> {
        let pm = &self.planet_model;
        let rect = GeoBBoxFactory::make_geo_bbox(
            pm,
            90.0 * RADIANS_PER_DEGREE,
            74.0 * RADIANS_PER_DEGREE,
            40.0 * RADIANS_PER_DEGREE,
            60.0 * RADIANS_PER_DEGREE,
        )?;
        let path_points = [GeoPoint::new_lat_lon(
            pm,
            84.4987594274 * RADIANS_PER_DEGREE,
            -22.8345484402 * RADIANS_PER_DEGREE,
        )?];
        let path = GeoPathFactory::make_geo_path(pm, 4.0 * RADIANS_PER_DEGREE, &path_points)?;
        Assert::assert_true(DISJOINT == rect.get_relationship(path.as_geo_shape())?);
        // This is what the test failure claimed...
        // assertTrue(GeoArea.CONTAINS == rect.getRelationship(path));
        // final GeoBBox bbox = getBoundingBox(path);
        // assertFalse(GeoArea.DISJOINT == rect.getRelationship(bbox));
        Ok(())
    }

    /// `testFailure2()`.
    fn test_failure2(&mut self) -> Result<()> {
        let pm = &self.planet_model;
        let rect = GeoBBoxFactory::make_geo_bbox(
            pm,
            -74.0 * RADIANS_PER_DEGREE,
            -90.0 * RADIANS_PER_DEGREE,
            0.0 * RADIANS_PER_DEGREE,
            26.0 * RADIANS_PER_DEGREE,
        )?;
        let circle = GeoCircleFactory::make_geo_circle(
            pm,
            -87.3647352103 * RADIANS_PER_DEGREE,
            52.3769709972 * RADIANS_PER_DEGREE,
            1.0 * RADIANS_PER_DEGREE,
        )?;
        Assert::assert_true(DISJOINT == rect.get_relationship(circle.as_geo_shape())?);
        // This is what the test failure claimed...
        // assertTrue(GeoArea.CONTAINS == rect.getRelationship(circle));
        // final GeoBBox bbox = getBoundingBox(circle);
        // assertFalse(GeoArea.DISJOINT == rect.getRelationship(bbox));
        Ok(())
    }

    /// `testFailure3()`.
    fn test_failure3(&mut self) -> Result<()> {
        /*
         [junit4]   1> S-R Rel: {}, Shape {}, Rectangle {}    lap# {} [CONTAINS, Geo3dShape{planetmodel=PlanetModel: {xyScaling=1.0011188180710464, zScaling=0.9977622539852008}, shape=GeoPath: {planetmodel=PlanetModel: {xyScaling=1.0011188180710464, zScaling=0.9977622539852008}, width=1.53588974175501(87.99999999999999),
          points={[[X=0.12097657665150223, Y=-0.6754177666095532, Z=0.7265376136709238], [X=-0.3837892785614207, Y=0.4258049113530899, Z=0.8180007850434892]]}}},
          Rect(minX=4.0,maxX=36.0,minY=16.0,maxY=16.0), 6981](no slf4j subst; sorry)
         [junit4] FAILURE 0.59s | Geo3dWGS84ShapeRectRelationTest.testGeoPathRect <<<
         [junit4]    > Throwable #1: java.lang.AssertionError: Geo3dShape{planetmodel=PlanetModel: {xyScaling=1.0011188180710464, zScaling=0.9977622539852008}, shape=GeoPath: {planetmodel=PlanetModel: {xyScaling=1.0011188180710464, zScaling=0.9977622539852008}, width=1.53588974175501(87.99999999999999),
          points={[[X=0.12097657665150223, Y=-0.6754177666095532, Z=0.7265376136709238], [X=-0.3837892785614207, Y=0.4258049113530899, Z=0.8180007850434892]]}}} intersect Pt(x=23.81626064835212,y=16.0)
         [junit4]    >  at __randomizedtesting.SeedInfo.seed([2595268DA3F13FEA:6CC30D8C83453E5D]:0)
         [junit4]    >  at org.apache.lucene.spatial.spatial4j.RandomizedShapeTestCase._assertIntersect(RandomizedShapeTestCase.java:168)
         [junit4]    >  at org.apache.lucene.spatial.spatial4j.RandomizedShapeTestCase.assertRelation(RandomizedShapeTestCase.java:153)
         [junit4]    >  at org.apache.lucene.spatial.spatial4j.RectIntersectionTestHelper.testRelateWithRectangle(RectIntersectionTestHelper.java:128)
         [junit4]    >  at org.apache.lucene.spatial.spatial4j.Geo3dWGS84ShapeRectRelationTest.testGeoPathRect(Geo3dWGS84ShapeRectRelationTest.java:265)
        */
        let pm = &self.planet_model;
        let rect = GeoBBoxFactory::make_geo_bbox(
            pm,
            16.0 * RADIANS_PER_DEGREE,
            16.0 * RADIANS_PER_DEGREE,
            4.0 * RADIANS_PER_DEGREE,
            36.0 * RADIANS_PER_DEGREE,
        )?;
        let path_points = [
            GeoPoint::new_lat_lon(
                pm,
                46.6369060853 * RADIANS_PER_DEGREE,
                -79.8452213228 * RADIANS_PER_DEGREE,
            )?,
            GeoPoint::new_lat_lon(
                pm,
                54.9779334519 * RADIANS_PER_DEGREE,
                132.029177424 * RADIANS_PER_DEGREE,
            )?,
        ];
        let path = GeoPathFactory::make_geo_path(pm, 88.0 * RADIANS_PER_DEGREE, &path_points)?;
        system::out_println(&format!("rect={rect}"));
        // Rectangle is within path (this is wrong; it's on the other side.  Should be OVERLAPS)
        Assert::assert_true(OVERLAPS == rect.get_relationship(path.as_geo_shape())?);
        // Rectangle contains point
        // assertTrue(rect.isWithin(pt));
        // Path contains point (THIS FAILS)
        // assertTrue(path.isWithin(pt));
        // What happens: (1) The center point of the horizontal line is within the path, in fact within
        // a radius of one of the endpoints.
        // (2) The point mentioned is NOT inside either SegmentEndpoint.
        // (3) The point mentioned is NOT inside the path segment, either.  (I think it should be...)
        Ok(())
    }

    /// `pointBearingTest()`.
    fn point_bearing_test(&mut self) -> Result<()> {
        point_bearing_test(&self.srr)
    }
}
