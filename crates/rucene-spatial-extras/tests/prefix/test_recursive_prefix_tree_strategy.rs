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

//! Port of `TestRecursivePrefixTreeStrategy.java` (package
//! `org.apache.lucene.spatial.prefix`).

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::{
    RecursivePrefixTreeStrategy, RecursivePrefixTreeStrategyOps,
};
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_test_framework::util::lucene_test_case::SuppressCodecs;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_match_concern::SpatialMatchConcern;
use crate::spatial::spatial_test_case::{
    spatial_class_annotated, strategy_test_case_class, ti, SpatialCase, SpatialFixture,
    SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::{
    StrategyTestCaseOps, DATA_WORLD_CITIES_POINTS, QTEST_CITIES_INTERSECTS_BBOX,
};

const NAME: &str = "org.apache.lucene.spatial.prefix.TestRecursivePrefixTreeStrategy";
/// `getClass().getSimpleName()`.
const SIMPLE_NAME: &str = "TestRecursivePrefixTreeStrategy";

/// `TestRecursivePrefixTreeStrategy` (`@LuceneTestCase.SuppressCodecs("SimpleText")`).
pub fn class_def() -> Arc<Class> {
    type T = TestRecursivePrefixTreeStrategy;
    spatial_class_annotated::<T>(
        NAME,
        &strategy_test_case_class(),
        Constructor::no_args(|| TestRecursivePrefixTreeStrategy {
            fx: SpatialFixture::new(NAME),
            max_length: 0,
            rpt: None,
        }),
        vec![
            ti::<T>(
                "testFilterWithVariableScanLevel",
                T::test_filter_with_variable_scan_level,
            )
            .annotate(Test::new()),
            ti::<T>("testOneMeterPrecision", T::test_one_meter_precision).annotate(Test::new()),
            ti::<T>("testPrecision", T::test_precision).annotate(Test::new()),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// The `TestRecursivePrefixTreeStrategy` instance.
pub struct TestRecursivePrefixTreeStrategy {
    fx: SpatialFixture,
    /// `maxLength`.
    max_length: i32,
    /// `(RecursivePrefixTreeStrategy) strategy`.
    rpt: Option<Arc<RecursivePrefixTreeStrategy>>,
}

impl SpatialCase for TestRecursivePrefixTreeStrategy {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }
}

impl TestRecursivePrefixTreeStrategy {
    /// `init(int maxLength)`: tests should call this first.
    fn init(&mut self, max_length: i32) -> Result<()> {
        self.max_length = max_length;
        self.fx.ctx = Some(SpatialContext::geo());
        let grid = GeohashPrefixTree::new(self.fx.ctx(), max_length)?;
        let rpt = RecursivePrefixTreeStrategy::new(grid, SIMPLE_NAME)?;
        self.fx.strategy = Some(rpt.clone());
        self.rpt = Some(rpt);
        Ok(())
    }

    /// `(RecursivePrefixTreeStrategy) strategy`.
    fn rpt(&self) -> &Arc<RecursivePrefixTreeStrategy> {
        self.rpt.as_ref().expect("init() sets the strategy")
    }

    /// `testFilterWithVariableScanLevel()`.
    fn test_filter_with_variable_scan_level(&mut self) -> Result<()> {
        self.init(GeohashPrefixTree::get_max_levels_possible())?;
        self.get_add_and_verify_indexed_documents(DATA_WORLD_CITIES_POINTS)?;

        // execute queries for each prefix grid scan level
        for i in 0..=self.max_length {
            self.rpt().set_prefix_grid_scan_level(i);
            self.execute_queries(SpatialMatchConcern::FILTER, &[QTEST_CITIES_INTERSECTS_BBOX])?;
        }
        Ok(())
    }

    /// `testOneMeterPrecision()`.
    fn test_one_meter_precision(&mut self) -> Result<()> {
        self.init(GeohashPrefixTree::get_max_levels_possible())?;
        let grid = self.rpt().get_grid();
        Assert::assert_true(grid.as_any().downcast_ref::<GeohashPrefixTree>().is_some());
        // DWS: I know this to be true.  11 is needed for one meter
        let degrees = DistanceUtils::dist2_degrees(0.001, DistanceUtils::EARTH_MEAN_RADIUS_KM);
        Assert::assert_equals(11, grid.get_level_for_distance(degrees));
        Ok(())
    }

    /// `testPrecision()`.
    fn test_precision(&mut self) -> Result<()> {
        self.init(GeohashPrefixTree::get_max_levels_possible())?;

        let ctx = self.fx.ctx();
        let i_pt = ctx
            .get_shape_factory()
            .point_xy(2.8028712999999925, 48.3708044)?; // lon, lat
        let doc = self.new_doc("iPt", Some(&(i_pt.clone() as Arc<dyn Shape>)))?;
        self.add_document(&doc)?;
        self.commit()?;

        let q_pt = ctx
            .get_shape_factory()
            .point_xy(2.4632387000000335, 48.6003516)?;

        let km2deg = DistanceUtils::dist2_degrees(1.0, DistanceUtils::EARTH_MEAN_RADIUS_KM);
        let deg2km = 1.0 / km2deg;
        const DIST: f64 = 35.75; // 35.7499...
        Assert::assert_equals_double_msg(
            None,
            DIST,
            ctx.get_dist_calc().distance(&*i_pt, &*q_pt) * deg2km,
            0.001,
        );

        // distErrPct will affect the query shape precision. The indexed precision
        // was set to nearly zilch via init(GeohashPrefixTree.getMaxLevelsPossible());
        let dist_err_pct = 0.025; // the suggested default, by the way
        let dist_mult = 1.0 + dist_err_pct;

        Assert::assert_true(35.74 * dist_mult >= DIST);
        self.check_hits(self.q(&q_pt, 35.74 * km2deg, dist_err_pct)?, 1, None)?;

        Assert::assert_true(30.0 * dist_mult < DIST);
        self.check_hits(self.q(&q_pt, 30.0 * km2deg, dist_err_pct)?, 0, None)?;

        Assert::assert_true(33.0 * dist_mult < DIST);
        self.check_hits(self.q(&q_pt, 33.0 * km2deg, dist_err_pct)?, 0, None)?;

        Assert::assert_true(34.0 * dist_mult < DIST);
        self.check_hits(self.q(&q_pt, 34.0 * km2deg, dist_err_pct)?, 0, None)
    }

    /// `q(Point pt, double distDEG, double distErrPct)`.
    fn q(&self, pt: &Arc<dyn Point>, dist_deg: f64, dist_err_pct: f64) -> Result<SpatialArgs> {
        let shape: Arc<dyn Shape> = self
            .fx
            .ctx()
            .get_shape_factory()
            .circle_at(Arc::clone(pt), dist_deg)?;
        let args = SpatialArgs::new(SpatialOperation::Intersects, shape);
        args.set_dist_err_pct(Some(dist_err_pct));
        Ok(args)
    }

    /// `checkHits(SpatialArgs args, int assertNumFound, int[] assertIds)`.
    fn check_hits(
        &self,
        args: SpatialArgs,
        assert_num_found: i32,
        assert_ids: Option<&[i32]>,
    ) -> Result<()> {
        let got = self.execute_query(self.fx.strategy().make_query(&args)?, 100)?;
        Assert::assert_equals_msg(
            Some(&args.to_string()),
            i64::from(assert_num_found),
            got.num_found,
        );
        if let Some(assert_ids) = assert_ids {
            let mut got_ids: HashSet<i32> = HashSet::new();
            for result in &got.results {
                got_ids.insert(crate::spatial_support::java_parse_int(
                    &result.document.get("id").unwrap_or_default(),
                )?);
            }
            for assert_id in assert_ids {
                Assert::assert_true_msg(&format!("has {assert_id}"), got_ids.contains(assert_id));
            }
        }
        Ok(())
    }
}
