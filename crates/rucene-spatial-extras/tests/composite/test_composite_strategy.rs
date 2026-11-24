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

//! Port of `TestCompositeStrategy.java` (package
//! `org.apache.lucene.spatial.composite`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Test;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene_spatial_extras::composite::composite_spatial_strategy::CompositeSpatialStrategy;
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::RecursivePrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::serialized::serialized_dv_strategy::SerializedDVStrategy;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::{
    SpatialContextFactory, SpatialContextFactoryOps,
};
use spatial4j::shape::r#impl::rectangle_impl::RectangleImpl;
use spatial4j::shape::shape::Shape;

use crate::prefix::random_spatial_op_strategy_test_case::RandomSpatialOpStrategyTestCaseOps;
use crate::spatial::spatial_test_case::{
    random_spatial_op_strategy_test_case_class, spatial_class, ti, SpatialCase, SpatialFixture,
    SpatialTestCaseOps,
};
use crate::spatial_support::random;

const NAME: &str = "org.apache.lucene.spatial.composite.TestCompositeStrategy";
/// `getClass().getSimpleName()`.
const SIMPLE_NAME: &str = "TestCompositeStrategy";

/// `TestCompositeStrategy`.
pub fn class_def() -> Arc<Class> {
    type T = TestCompositeStrategy;
    spatial_class::<T>(
        NAME,
        &random_spatial_op_strategy_test_case_class(),
        Constructor::no_args(|| TestCompositeStrategy {
            fx: SpatialFixture::new(NAME),
            grid: None,
            rpt_strategy: None,
        }),
        vec![ti::<T>("testOperations", T::test_operations).annotate(Test::new())],
    )
}

/// The `TestCompositeStrategy` instance.
pub struct TestCompositeStrategy {
    fx: SpatialFixture,
    /// `grid`.
    grid: Option<Arc<dyn SpatialPrefixTree>>,
    /// `rptStrategy`.
    rpt_strategy: Option<Arc<RecursivePrefixTreeStrategy>>,
}

impl SpatialCase for TestCompositeStrategy {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `randomIndexedShape()`.
    fn random_indexed_shape(&mut self) -> Result<Arc<dyn Shape>> {
        self.random_shape()
    }

    /// `randomQueryShape()`.
    fn random_query_shape(&mut self) -> Result<Arc<dyn Shape>> {
        self.random_shape()
    }
}

impl TestCompositeStrategy {
    /// `setupQuadGrid(int maxLevels)`.
    fn setup_quad_grid(&mut self, mut max_levels: i32) -> Result<()> {
        // non-geospatial makes this test a little easier (in gridSnap), and using boundary values 2^X
        // raises
        // the prospect of edge conditions we want to test, plus makes for simpler numbers (no
        // decimals).
        let mut factory = SpatialContextFactory::new();
        factory.geo = false;
        factory.world_bounds = Some(RectangleImpl::new(0.0, 256.0, -128.0, 128.0, None));
        self.fx.ctx = Some(factory.new_spatial_context()?);
        // A fairly shallow grid
        if max_levels == -1 {
            max_levels = RandomizedTest::random_int_between(1, 8); // max 64k cells (4^8), also 256*256
        }
        self.grid = Some(QuadPrefixTree::new(self.fx.ctx(), max_levels));
        self.rpt_strategy = Some(self.new_rpt()?);
        Ok(())
    }

    /// `setupGeohashGrid(int maxLevels)`.
    fn setup_geohash_grid(&mut self, mut max_levels: i32) -> Result<()> {
        self.fx.ctx = Some(SpatialContext::geo());
        // A fairly shallow grid
        if max_levels == -1 {
            max_levels = RandomizedTest::random_int_between(1, 3); // max 16k cells (32^3)
        }
        self.grid = Some(GeohashPrefixTree::new(self.fx.ctx(), max_levels)?);
        self.rpt_strategy = Some(self.new_rpt()?);
        Ok(())
    }

    /// `newRPT()`.
    fn new_rpt(&self) -> Result<Arc<RecursivePrefixTreeStrategy>> {
        let rpt = RecursivePrefixTreeStrategy::new(
            Arc::clone(self.grid.as_ref().expect("grid")),
            &format!("{SIMPLE_NAME}_rpt"),
        )?;
        rpt.set_dist_err_pct(0.10); // not too many cells
        Ok(rpt)
    }

    /// `testOperations()`.
    fn test_operations(&mut self) -> Result<()> {
        // setup
        if RandomizedTest::random_boolean() {
            self.setup_quad_grid(-1)?;
        } else {
            self.setup_geohash_grid(-1)?;
        }
        let serialized_dv_strategy = Arc::new(SerializedDVStrategy::new(
            self.fx.ctx(),
            &format!("{SIMPLE_NAME}_sdv"),
        )?);
        self.fx.strategy = Some(Arc::new(CompositeSpatialStrategy::new(
            &format!("composite_{SIMPLE_NAME}"),
            self.rpt_strategy.clone().expect("rptStrategy"),
            serialized_dv_strategy,
        )?));

        // Do it!

        for pred in SpatialOperation::values() {
            if pred == SpatialOperation::BBoxIntersects || pred == SpatialOperation::BBoxWithin {
                continue;
            }
            if pred == SpatialOperation::IsDisjointTo {
                // TODO
                continue;
            }
            self.test_operation_random_shapes(pred)?;
            self.delete_all()?;
            self.commit()?;
        }
        Ok(())
    }

    /// `randomShape()`.
    fn random_shape(&self) -> Result<Arc<dyn Shape>> {
        if random().next_boolean() {
            self.random_circle()
        } else {
            Ok(self.random_rectangle()?)
        }
    }

    // TODO move up
    /// `randomCircle()`.
    fn random_circle(&self) -> Result<Arc<dyn Shape>> {
        let ctx = self.fx.ctx();
        let point = self.random_point()?;
        // TODO pick using gaussian
        let radius = if ctx.is_geo() {
            RandomizedTest::random_double() * 100.0
        } else {
            // find distance to closest edge
            let world_bounds = ctx.get_world_bounds();
            let mut max_rad = point.get_x() - world_bounds.get_min_x();
            max_rad = max_rad.min(world_bounds.get_max_x() - point.get_x());
            max_rad = max_rad.min(point.get_y() - world_bounds.get_min_y());
            max_rad = max_rad.min(world_bounds.get_max_y() - point.get_y());
            RandomizedTest::random_double() * max_rad
        };
        Ok(ctx.make_circle_at(point, radius)?)
    }
}
