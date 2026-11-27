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

//! Port of `TestDistanceStrategy.java` (package `org.apache.lucene.spatial`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Value};
use randomizedtesting::junit::Test;
use rucene::error::Result;
use rucene_spatial_extras::bbox::b_box_strategy::BBoxStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::RecursivePrefixTreeStrategy;
use rucene_spatial_extras::prefix::term_query_prefix_tree_strategy::TermQueryPrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::packed_quad_prefix_tree::PackedQuadPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::serialized::serialized_dv_strategy::SerializedDVStrategy;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_spatial_extras::vector::point_vector_strategy::PointVectorStrategy;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_test_case::{
    spatial_class, strategy_test_case_class, ti, SpatialCase, SpatialFixture, SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;
use crate::spatial_support::{parameters_method, random, strategy_arg, strategy_value};

const NAME: &str = "org.apache.lucene.spatial.TestDistanceStrategy";

/// `TestDistanceStrategy`.
pub fn class_def() -> Arc<Class> {
    type T = TestDistanceStrategy;
    spatial_class::<T>(
        NAME,
        &strategy_test_case_class(),
        Constructor::with_params(
            &[
                ("java.lang.String", None),
                ("org.apache.lucene.spatial.SpatialStrategy", None),
            ],
            |args: &[Value]| T::new(strategy_arg(&args[1])),
        ),
        vec![
            parameters_method("strategy=%s", parameters),
            ti::<T>("testDistanceOrder", T::test_distance_order).annotate(Test::new()),
            ti::<T>("testRecipScore", T::test_recip_score).annotate(Test::new()),
        ],
    )
}

/// `parameters()`.
fn parameters() -> Vec<Vec<Value>> {
    let build = || -> Result<Vec<Vec<Value>>> {
        let mut ctor_args = Vec::new();

        let ctx = SpatialContext::geo();

        let mut grid: Arc<dyn SpatialPrefixTree>;
        let mut strategy: Arc<dyn SpatialStrategy>;

        grid = QuadPrefixTree::new(Arc::clone(&ctx), 25);
        strategy = RecursivePrefixTreeStrategy::new(grid, "recursive_quad")?;
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

        grid = GeohashPrefixTree::new(Arc::clone(&ctx), 12)?;
        strategy = TermQueryPrefixTreeStrategy::new(grid, "termquery_geohash")?;
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

        grid = PackedQuadPrefixTree::new(Arc::clone(&ctx), 25)?;
        strategy = RecursivePrefixTreeStrategy::new(grid, "recursive_packedquad")?;
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

        strategy = PointVectorStrategy::new_instance(Arc::clone(&ctx), "pointvector")?;
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

        //  Can't test this without un-inverting since PVS legacy config didn't have docValues.
        //    However, note that Solr's tests use UninvertingReader and thus test this.
        //    strategy = PointVectorStrategy.newLegacyInstance(ctx, "pointvector_legacy");
        //    ctorArgs.add(new Object[]{strategy.getFieldName(), strategy});

        strategy = BBoxStrategy::new_instance(Arc::clone(&ctx), "bbox")?;
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

        strategy = Arc::new(SerializedDVStrategy::new(Arc::clone(&ctx), "serialized")?);
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

        Ok(ctor_args)
    };
    build().unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e))
}

/// The `TestDistanceStrategy` instance.
pub struct TestDistanceStrategy {
    fx: SpatialFixture,
}

impl SpatialCase for TestDistanceStrategy {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }
}

impl TestDistanceStrategy {
    /// `TestDistanceStrategy(String suiteName, SpatialStrategy strategy)`.
    fn new(strategy: Arc<dyn SpatialStrategy>) -> Self {
        let mut fx = SpatialFixture::new(NAME);
        fx.ctx = Some(strategy.get_spatial_context());
        fx.strategy = Some(strategy);
        Self { fx }
    }

    /// `testDistanceOrder()`.
    fn test_distance_order(&mut self) -> Result<()> {
        let shape_factory = self.fx.ctx().get_shape_factory();
        self.adoc("100", Some(shape_factory.point_xy(2.0, 1.0)?))?;
        self.adoc("101", Some(shape_factory.point_xy(-1.0, 4.0)?))?;
        self.adoc("103", None)?; // test score for nothing
        self.commit()?;
        // FYI distances are in docid order
        self.check_dist_value_source(
            &shape_factory.point_xy(4.0, 3.0)?,
            &[2.8274937f32, 5.0898066f32, 180f32],
        )?;
        self.check_dist_value_source(
            &shape_factory.point_xy(0.0, 4.0)?,
            &[3.6043684f32, 0.9975641f32, 180f32],
        )
    }

    /// `testRecipScore()`.
    fn test_recip_score(&mut self) -> Result<()> {
        let ctx = self.fx.ctx();
        let p100 = ctx.get_shape_factory().point_xy(2.02, 0.98)?;
        self.adoc("100", Some(p100.clone()))?;
        let p101 = ctx.get_shape_factory().point_xy(-1.001, 4.001)?;
        self.adoc("101", Some(p101.clone()))?;
        self.adoc("103", None)?; // test score for nothing
        self.commit()?;

        let dist = ctx.get_dist_calc().distance(&*p100, &*p101);
        let query_shape: Arc<dyn Shape> = ctx.make_circle(2.01, 0.99, dist)?;
        self.check_value_source(
            self.fx
                .strategy()
                .make_recip_distance_value_source(&*query_shape)?,
            &[1.00f32, 0.10f32, 0f32],
            0.09f32,
        )
    }

    /// `checkDistValueSource(Point pt, float... distances)`.
    fn check_dist_value_source(&self, pt: &Arc<dyn Point>, distances: &[f32]) -> Result<()> {
        let multiplier = random().next_float() * 100f32;
        let mut dists2 = distances.to_vec();
        for d in dists2.iter_mut() {
            *d *= multiplier;
        }
        self.check_value_source(
            self.fx
                .strategy()
                .make_distance_value_source(pt, f64::from(multiplier))?,
            &dists2,
            1.0e-3f32,
        )
    }
}
