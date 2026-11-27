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

//! Port of `TestBBoxStrategy.java` (package `org.apache.lucene.spatial.bbox`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Ignore, Test};
use rucene::document::FieldType;
use rucene::error::Result;
use rucene::index::DocValuesType;
use rucene_spatial_extras::bbox::b_box_overlap_ratio_value_source::BBoxOverlapRatioValueSource;
use rucene_spatial_extras::bbox::b_box_similarity_value_source::BBoxSimilarityValueSource;
use rucene_spatial_extras::bbox::b_box_strategy::{BBoxStrategy, DEFAULT_FIELDTYPE};
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_spatial_extras::util::shape_area_value_source::ShapeAreaValueSource;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::{
    SpatialContextFactory, SpatialContextFactoryOps,
};
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::r#impl::rectangle_impl::RectangleImpl;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;

use crate::prefix::random_spatial_op_strategy_test_case::RandomSpatialOpStrategyTestCaseOps;
use crate::spatial::spatial_match_concern::SpatialMatchConcern;
use crate::spatial::spatial_test_case::{
    java_assert_msg, random_spatial_op_strategy_test_case_class, spatial_class, ti, SpatialCase,
    SpatialFixture, SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::{
    StrategyTestCaseOps, DATA_SIMPLE_BBOX, DATA_STATES_BBOX, DATA_WORLD_CITIES_POINTS,
    QTEST_CITIES_INTERSECTS_BBOX, QTEST_SIMPLE_QUERIES_BBOX, QTEST_STATES_INTERSECTS_BBOX,
    QTEST_STATES_IS_WITHIN_BBOX,
};
use crate::spatial_support::random;

const NAME: &str = "org.apache.lucene.spatial.bbox.TestBBoxStrategy";

/// `TestBBoxStrategy`.
pub fn class_def() -> Arc<Class> {
    type T = TestBBoxStrategy;
    spatial_class::<T>(
        NAME,
        &random_spatial_op_strategy_test_case_class(),
        Constructor::no_args(|| TestBBoxStrategy {
            fx: SpatialFixture::new(NAME),
        }),
        vec![
            ti::<T>("testOperations", T::test_operations).annotate(Test::new()),
            ti::<T>(
                "testIntersectsBugDatelineEdge",
                T::test_intersects_bug_dateline_edge,
            )
            .annotate(Test::new()),
            ti::<T>(
                "testIntersectsWorldDatelineEdge",
                T::test_intersects_world_dateline_edge,
            )
            .annotate(Test::new()),
            ti::<T>(
                "testWithinBugDatelineEdge",
                T::test_within_bug_dateline_edge,
            )
            .annotate(Test::new()),
            ti::<T>(
                "testContainsBugDatelineEdge",
                T::test_contains_bug_dateline_edge,
            )
            .annotate(Test::new()),
            ti::<T>("testWorldContainsXDL", T::test_world_contains_xdl).annotate(Test::new()),
            ti::<T>(
                "testAlongDatelineOppositeSign",
                T::test_along_dateline_opposite_sign,
            )
            .annotate(Test::new()),
            ti::<T>("testBasicOperaions", T::test_basic_operaions)
                .annotate(Test::new())
                .annotate(Ignore {
                    value: "Overlaps not supported".to_string(),
                }),
            ti::<T>("testStatesBBox", T::test_states_bbox).annotate(Test::new()),
            ti::<T>("testCitiesIntersectsBBox", T::test_cities_intersects_bbox)
                .annotate(Test::new()),
            ti::<T>("testOverlapRatio", T::test_overlap_ratio),
            ti::<T>("testAreaValueSource", T::test_area_value_source),
        ],
    )
}

/// The `TestBBoxStrategy` instance.
pub struct TestBBoxStrategy {
    fx: SpatialFixture,
}

impl SpatialCase for TestBBoxStrategy {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `randomIndexedShape()`.
    fn random_indexed_shape(&mut self) -> Result<Arc<dyn Shape>> {
        let ctx = self.fx.ctx();
        let world = ctx.get_world_bounds();
        if random().next_int_bound(10) == 0 {
            // increased chance of getting one of these
            return Ok(world);
        }

        let world_width = randomizedtesting::jdk::math::round(world.get_width()) as i32;
        let delta_left = next_int_inclusive(world_width);
        let delta_right = next_int_inclusive(world_width - delta_left);
        let world_height = randomizedtesting::jdk::math::round(world.get_height()) as i32;
        let delta_top = next_int_inclusive(world_height);
        let delta_bottom = next_int_inclusive(world_height - delta_top);
        if ctx.is_geo() && (delta_left != 0 || delta_right != 0) {
            // if geo & doesn't world-wrap, we shift randomly to potentially cross dateline
            let shift = next_int_inclusive(360);
            Ok(ctx.get_shape_factory().rect(
                DistanceUtils::norm_lon_deg(
                    world.get_min_x() + f64::from(delta_left) + f64::from(shift),
                ),
                DistanceUtils::norm_lon_deg(
                    world.get_max_x() - f64::from(delta_right) + f64::from(shift),
                ),
                world.get_min_y() + f64::from(delta_bottom),
                world.get_max_y() - f64::from(delta_top),
            )?)
        } else {
            Ok(ctx.get_shape_factory().rect(
                world.get_min_x() + f64::from(delta_left),
                world.get_max_x() - f64::from(delta_right),
                world.get_min_y() + f64::from(delta_bottom),
                world.get_max_y() - f64::from(delta_top),
            )?)
        }
    }

    /// `randomQueryShape()`.
    fn random_query_shape(&mut self) -> Result<Arc<dyn Shape>> {
        self.random_indexed_shape()
    }

    /// `convertShapeFromGetDocuments(Shape shape)`: convert
    /// DATA_WORLD_CITIES_POINTS to bbox.
    fn convert_shape_from_get_documents(&self, shape: Arc<dyn Shape>) -> Option<Arc<dyn Shape>> {
        Some(shape.get_bounding_box())
    }
}

/// `nextIntInclusive(int toInc)`: next int, inclusive, rounds to multiple
/// of 10 if given evenly divisible.
fn next_int_inclusive(to_inc: i32) -> i32 {
    const DIVIS: i32 = 10;
    if to_inc % DIVIS == 0 {
        random().next_int_bound(to_inc / DIVIS + 1) * DIVIS
    } else {
        random().next_int_bound(to_inc + 1)
    }
}

impl TestBBoxStrategy {
    /// `testOperations()`.
    fn test_operations(&mut self) -> Result<()> {
        // setup
        if random().next_int_bound(4) > 0 {
            // 75% of the time choose geo (more interesting to test)
            self.fx.ctx = Some(SpatialContext::geo());
        } else {
            let mut factory = SpatialContextFactory::new();
            factory.geo = false;
            factory.world_bounds = Some(RectangleImpl::new(-300.0, 300.0, -100.0, 100.0, None));
            self.fx.ctx = Some(factory.new_spatial_context()?);
        }
        let bbox = BBoxStrategy::new_instance(self.fx.ctx(), "bbox")?;
        self.fx.strategy = Some(bbox.clone());
        // test we can disable docValues for predicate tests
        if random().next_boolean() {
            let field_type = FieldType::new_from(bbox.get_field_type());
            field_type.set_doc_values_type(DocValuesType::NONE)?;
            self.fx.strategy = Some(BBoxStrategy::new(
                self.fx.ctx(),
                bbox.get_field_name(),
                &field_type,
            )?);
        }
        for operation in SpatialOperation::values() {
            if operation == SpatialOperation::Overlaps {
                continue; // unsupported
            }
            self.test_operation_random_shapes(operation)?;

            self.delete_all()?;
            self.commit()?;
        }
        Ok(())
    }

    /// `testIntersectsBugDatelineEdge()`.
    fn test_intersects_bug_dateline_edge(&mut self) -> Result<()> {
        self.setup_geo()?;
        let sf = self.fx.ctx().get_shape_factory();
        self.test_operation(
            sf.rect(160.0, 180.0, -10.0, 10.0)?,
            SpatialOperation::Intersects,
            sf.rect(-180.0, -160.0, -10.0, 10.0)?,
            true,
        )
    }

    /// `testIntersectsWorldDatelineEdge()`.
    fn test_intersects_world_dateline_edge(&mut self) -> Result<()> {
        self.setup_geo()?;
        let sf = self.fx.ctx().get_shape_factory();
        self.test_operation(
            sf.rect(-180.0, 180.0, -10.0, 10.0)?,
            SpatialOperation::Intersects,
            sf.rect(180.0, 180.0, -10.0, 10.0)?,
            true,
        )
    }

    /// `testWithinBugDatelineEdge()`.
    fn test_within_bug_dateline_edge(&mut self) -> Result<()> {
        self.setup_geo()?;
        let sf = self.fx.ctx().get_shape_factory();
        self.test_operation(
            sf.rect(180.0, 180.0, -10.0, 10.0)?,
            SpatialOperation::IsWithin,
            sf.rect(-180.0, -100.0, -10.0, 10.0)?,
            true,
        )
    }

    /// `testContainsBugDatelineEdge()`.
    fn test_contains_bug_dateline_edge(&mut self) -> Result<()> {
        self.setup_geo()?;
        let sf = self.fx.ctx().get_shape_factory();
        self.test_operation(
            sf.rect(-180.0, -150.0, -10.0, 10.0)?,
            SpatialOperation::Contains,
            sf.rect(180.0, 180.0, -10.0, 10.0)?,
            true,
        )
    }

    /// `testWorldContainsXDL()`.
    fn test_world_contains_xdl(&mut self) -> Result<()> {
        self.setup_geo()?;
        let sf = self.fx.ctx().get_shape_factory();
        self.test_operation(
            sf.rect(-180.0, 180.0, -10.0, 10.0)?,
            SpatialOperation::Contains,
            sf.rect(170.0, -170.0, -10.0, 10.0)?,
            true,
        )
    }

    /// `testAlongDatelineOppositeSign()`: see
    /// <https://github.com/spatial4j/spatial4j/issues/85>.
    fn test_along_dateline_opposite_sign(&mut self) -> Result<()> {
        // Due to Spatial4j bug #85, we can't simply do:
        //    testOperation(indexedShape,
        //        SpatialOperation.IsWithin,
        //        queryShape, true);

        // both on dateline but expressed using opposite signs
        self.setup_geo()?;
        let sf = self.fx.ctx().get_shape_factory();
        let indexed_shape: Arc<dyn Rectangle> = sf.rect(180.0, 180.0, -10.0, 10.0)?;
        let query_shape: Arc<dyn Rectangle> = sf.rect(-180.0, -180.0, -20.0, 20.0)?;
        let operation = SpatialOperation::IsWithin;
        // the rest is super.testOperation without leading assert:

        self.adoc("0", Some(indexed_shape.clone()))?;
        self.commit()?;
        let query = self
            .fx
            .strategy()
            .make_query(&SpatialArgs::new(operation, query_shape.clone()))?;
        let got = self.execute_query(query, 1)?;
        java_assert_msg(got.num_found <= 1, "unclean test env");
        if got.num_found != 1 {
            Assert::fail(&format!("{operation} I:{indexed_shape} Q:{query_shape}"));
        }
        self.delete_all() // clean up after ourselves
    }

    /// `setupGeo()`.
    fn setup_geo(&mut self) -> Result<()> {
        self.fx.ctx = Some(SpatialContext::geo());
        self.fx.strategy = Some(BBoxStrategy::new_instance(self.fx.ctx(), "bbox")?);
        Ok(())
    }

    // OLD STATIC TESTS (worthless?)

    /// `testBasicOperaions()` (`@Ignore("Overlaps not supported")`).
    fn test_basic_operaions(&mut self) -> Result<()> {
        self.setup_geo()?;
        self.get_add_and_verify_indexed_documents(DATA_SIMPLE_BBOX)?;

        self.execute_queries(SpatialMatchConcern::EXACT, &[QTEST_SIMPLE_QUERIES_BBOX])
    }

    /// `testStatesBBox()`.
    fn test_states_bbox(&mut self) -> Result<()> {
        self.setup_geo()?;
        self.get_add_and_verify_indexed_documents(DATA_STATES_BBOX)?;

        self.execute_queries(SpatialMatchConcern::FILTER, &[QTEST_STATES_IS_WITHIN_BBOX])?;
        self.execute_queries(SpatialMatchConcern::FILTER, &[QTEST_STATES_INTERSECTS_BBOX])
    }

    /// `testCitiesIntersectsBBox()`.
    fn test_cities_intersects_bbox(&mut self) -> Result<()> {
        self.setup_geo()?;
        self.get_add_and_verify_indexed_documents(DATA_WORLD_CITIES_POINTS)?;

        self.execute_queries(SpatialMatchConcern::FILTER, &[QTEST_CITIES_INTERSECTS_BBOX])
    }

    /// `setupNeedsDocValuesOnly()`.
    fn setup_needs_doc_values_only(&mut self) -> Result<Arc<BBoxStrategy>> {
        self.fx.ctx = Some(SpatialContext::geo());
        // random  legacy or not legacy
        let field_prefix = "bbox";
        let field_type = FieldType::new_from(&*DEFAULT_FIELDTYPE);
        if random().next_boolean() {
            field_type.set_dimensions(0, 0)?;
        }
        let strategy = BBoxStrategy::new(self.fx.ctx(), field_prefix, &field_type)?;
        self.fx.strategy = Some(strategy.clone());
        Ok(strategy)
    }

    /// `testOverlapRatio()`.
    fn test_overlap_ratio(&mut self) -> Result<()> {
        let bbox_strategy = self.setup_needs_doc_values_only()?;

        // Simply assert null shape results in 0
        self.adoc("999", None)?;
        self.commit()?;
        self.check_value_source(
            bbox_strategy.make_overlap_ratio_value_source(self.random_rectangle()?, 0.0)?,
            &[0f32],
            0f32,
        )?;

        // we test raw BBoxOverlapRatioValueSource without actual indexing
        let mut shift = 0;
        while shift < 360 {
            let query_box = self.shifted_rect(0.0, 40.0, -20.0, 20.0, shift)?; // 40x40, 1600 area

            let msl = random().next_boolean();
            let min_side_length = if msl { 0.1 } else { 0.0 };
            let mut sim =
                BBoxOverlapRatioValueSource::new(None, true, query_box, 0.5, min_side_length)?;
            let nudge = if shift == 0 {
                0
            } else {
                random().next_int_bound(3) * 10 - 10
            }; // -10, 0, or 10.  Keep 0 on first round.

            const EPS: f64 = 0.0000001;

            assert_eq_d(
                "within",
                (200.0 / 1600.0 * 0.5) + (0.5),
                sim.score(
                    &*self.shifted_rect(10.0, 30.0, 0.0, 10.0, shift + nudge)?,
                    None,
                ),
                EPS,
            );

            assert_eq_d(
                "in25%",
                0.25,
                sim.score(&*self.shifted_rect(30.0, 70.0, -20.0, 20.0, shift)?, None),
                EPS,
            );

            assert_eq_d(
                "wrap",
                0.2794117,
                sim.score(
                    &*self.shifted_rect(30.0, 10.0, -20.0, 20.0, shift + nudge)?,
                    None,
                ),
                EPS,
            );

            assert_eq_d(
                "no intersection H",
                0.0,
                sim.score(&*self.shifted_rect(-10.0, -10.0, -20.0, 20.0, shift)?, None),
                EPS,
            );
            assert_eq_d(
                "no intersection V",
                0.0,
                sim.score(&*self.shifted_rect(0.0, 20.0, -30.0, -30.0, shift)?, None),
                EPS,
            );

            assert_eq_d(
                "point",
                0.5 + if msl { 0.1 * 0.1 / 1600.0 / 2.0 } else { 0.0 },
                sim.score(&*self.shifted_rect(0.0, 0.0, 0.0, 0.0, shift)?, None),
                EPS,
            );

            assert_eq_d(
                "line 25% intersection",
                0.25 / 2.0 + if msl { 10.0 * 0.1 / 1600.0 / 2.0 } else { 0.0 },
                sim.score(&*self.shifted_rect(-30.0, 10.0, 0.0, 0.0, shift)?, None),
                EPS,
            );

            // test with point query
            sim = BBoxOverlapRatioValueSource::new(
                None,
                true,
                self.shifted_rect(0.0, 0.0, 0.0, 0.0, shift)?,
                0.5,
                min_side_length,
            )?;
            assert_eq_d(
                "same",
                1.0,
                sim.score(&*self.shifted_rect(0.0, 0.0, 0.0, 0.0, shift)?, None),
                EPS,
            );
            assert_eq_d(
                "contains",
                0.5 + if msl {
                    0.1 * 0.1 / (30.0 * 10.0) / 2.0
                } else {
                    0.0
                },
                sim.score(&*self.shifted_rect(0.0, 30.0, 0.0, 10.0, shift)?, None),
                EPS,
            );

            // test with line query (vertical this time)
            sim = BBoxOverlapRatioValueSource::new(
                None,
                true,
                self.shifted_rect(0.0, 0.0, 20.0, 40.0, shift)?,
                0.5,
                min_side_length,
            )?;
            assert_eq_d(
                "line 50%",
                0.5,
                sim.score(&*self.shifted_rect(0.0, 0.0, 10.0, 30.0, shift)?, None),
                EPS,
            );
            assert_eq_d(
                "point",
                0.5 + if msl {
                    0.1 * 0.1 / (20.0 * 0.1) / 2.0
                } else {
                    0.0
                },
                sim.score(&*self.shifted_rect(0.0, 0.0, 30.0, 30.0, shift)?, None),
                EPS,
            );
            shift += 10;
        }
        Ok(())
    }

    /// `shiftedRect(double minX, double maxX, double minY, double maxY, int xShift)`.
    fn shifted_rect(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        x_shift: i32,
    ) -> Result<Arc<dyn Rectangle>> {
        self.fx.ctx().get_shape_factory().rect(
            DistanceUtils::norm_lon_deg(min_x + f64::from(x_shift)),
            DistanceUtils::norm_lon_deg(max_x + f64::from(x_shift)),
            min_y,
            max_y,
        )
    }

    /// `testAreaValueSource()`.
    fn test_area_value_source(&mut self) -> Result<()> {
        let bbox_strategy = self.setup_needs_doc_values_only()?;

        self.adoc(
            "100",
            Some(
                self.fx
                    .ctx()
                    .get_shape_factory()
                    .rect(0.0, 20.0, 40.0, 80.0)?,
            ),
        )?;
        self.adoc("999", None)?;
        self.commit()?;
        self.check_value_source(
            Arc::new(ShapeAreaValueSource::new(
                bbox_strategy.make_shape_value_source(),
                self.fx.ctx(),
                false,
                1.0,
            )),
            &[800f32, 0f32],
            0f32,
        )?;
        self.check_value_source(
            Arc::new(ShapeAreaValueSource::new(
                bbox_strategy.make_shape_value_source(),
                self.fx.ctx(),
                true,
                1.0,
            )), // geo
            &[391.93f32, 0f32],
            0.01f32,
        )?;
        self.check_value_source(
            Arc::new(ShapeAreaValueSource::new(
                bbox_strategy.make_shape_value_source(),
                self.fx.ctx(),
                true,
                2.0,
            )),
            &[783.86f32, 0f32],
            0.01f32,
        ) // testing with a different multiplier
    }
}

/// `assertEquals(String message, double expected, double actual, double delta)`.
fn assert_eq_d(message: &str, expected: f64, actual: f64, delta: f64) {
    Assert::assert_equals_double_msg(Some(message), expected, actual, delta);
}
