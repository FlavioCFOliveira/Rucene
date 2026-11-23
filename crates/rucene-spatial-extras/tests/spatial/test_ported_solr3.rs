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

//! Port of `TestPortedSolr3.java` (package `org.apache.lucene.spatial`):
//! based off of Solr 3's SpatialFilterTest.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Value};
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::RecursivePrefixTreeStrategy;
use rucene_spatial_extras::prefix::term_query_prefix_tree_strategy::TermQueryPrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_spatial_extras::vector::point_vector_strategy::PointVectorStrategy;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::shape_factory::ShapeFactory;

use crate::spatial::spatial_test_case::{
    spatial_class, strategy_test_case_class, ti, SpatialCase, SpatialFixture, SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;
use crate::spatial_support::{parameters_method, strategy_arg, strategy_value};

const NAME: &str = "org.apache.lucene.spatial.TestPortedSolr3";

/// `TestPortedSolr3`.
pub fn class_def() -> Arc<Class> {
    type T = TestPortedSolr3;
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
            ti::<T>("testIntersections", T::test_intersections).annotate(Test::new()),
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

        grid = GeohashPrefixTree::new(Arc::clone(&ctx), 12)?;
        strategy = RecursivePrefixTreeStrategy::new(grid, "recursive_geohash")?;
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

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

        strategy = PointVectorStrategy::new_instance(Arc::clone(&ctx), "pointvector")?;
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

        strategy = PointVectorStrategy::new_instance(Arc::clone(&ctx), "pointvector_legacy")?;
        ctor_args.push(vec![
            Value::Str(strategy.get_field_name().to_string()),
            strategy_value(strategy),
        ]);

        Ok(ctor_args)
    };
    build().unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e))
}

/// The `TestPortedSolr3` instance.
pub struct TestPortedSolr3 {
    fx: SpatialFixture,
    /// `shapeFactory`.
    shape_factory: Arc<dyn ShapeFactory>,
}

impl SpatialCase for TestPortedSolr3 {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }
}

impl TestPortedSolr3 {
    /// `TestPortedSolr3(String suiteName, SpatialStrategy strategy)`.
    fn new(strategy: Arc<dyn SpatialStrategy>) -> Self {
        let mut fx = SpatialFixture::new(NAME);
        fx.ctx = Some(strategy.get_spatial_context());
        fx.strategy = Some(strategy);
        let shape_factory = fx.ctx().get_shape_factory();
        Self { fx, shape_factory }
    }

    fn pt(&self, x: f64, y: f64) -> Result<Option<Arc<dyn Shape>>> {
        Ok(Some(self.shape_factory.point_xy(x, y)?))
    }

    /// `setupDocs()`.
    fn setup_docs(&mut self) -> Result<()> {
        self.delete_all()?;
        self.adoc("1", self.pt(-79.9289094, 32.7693246)?)?;
        self.adoc("2", self.pt(-80.9289094, 33.7693246)?)?;
        self.adoc("3", self.pt(50.9289094, -32.7693246)?)?;
        self.adoc("4", self.pt(60.9289094, -50.7693246)?)?;
        self.adoc("5", self.pt(0.0, 0.0)?)?;
        self.adoc("6", self.pt(0.1, 0.1)?)?;
        self.adoc("7", self.pt(-0.1, -0.1)?)?;
        self.adoc("8", self.pt(179.9, 0.0)?)?;
        self.adoc("9", self.pt(-179.9, 0.0)?)?;
        self.adoc("10", self.pt(50.0, 89.9)?)?;
        self.adoc("11", self.pt(-130.0, 89.9)?)?;
        self.adoc("12", self.pt(50.0, -89.9)?)?;
        self.adoc("13", self.pt(-130.0, -89.9)?)?;
        self.commit()
    }

    /// `testIntersections()`.
    fn test_intersections(&mut self) -> Result<()> {
        self.setup_docs()?;
        let sf = Arc::clone(&self.shape_factory);
        // Try some edge cases
        // NOTE: 2nd arg is distance in kilometers
        self.check_hits_circle(sf.point_xy(1.0, 1.0)?, 175.0, 3, &[5, 6, 7])?;
        self.check_hits_circle(sf.point_xy(179.8, 0.0)?, 200.0, 2, &[8, 9])?;
        self.check_hits_circle(sf.point_xy(50.0, 89.8)?, 200.0, 2, &[10, 11])?; // this goes over the north pole
        self.check_hits_circle(sf.point_xy(50.0, -89.8)?, 200.0, 2, &[12, 13])?; // this goes over the south pole
                                                                                 // try some normal cases
        self.check_hits_circle(sf.point_xy(-80.0, 33.0)?, 300.0, 2, &[])?;
        // large distance
        self.check_hits_circle(sf.point_xy(1.0, 1.0)?, 5000.0, 3, &[5, 6, 7])?;
        // Because we are generating a box based on the west/east longitudes and the south/north
        // latitudes, which then
        // translates to a range query, which is slightly more inclusive.  Thus, even though 0.0 is
        // 15.725 kms away,
        // it will be included, b/zScaling of the box calculation.
        self.check_hits_bbox(sf.point_xy(0.1, 0.1)?, 15.0, 2, &[5, 6])?;
        // try some more
        self.delete_all()?;
        self.adoc("14", self.pt(5.0, 0.0)?)?;
        self.adoc("15", self.pt(15.0, 0.0)?)?;
        // 3000KM from 0,0, see http://www.movable-type.co.uk/scripts/latlong.html
        self.adoc("16", self.pt(19.79750, 18.71111)?)?;
        self.adoc("17", self.pt(-95.436643, 44.043900)?)?;
        self.commit()?;

        self.check_hits_circle(sf.point_xy(0.0, 0.0)?, 1000.0, 1, &[14])?;
        self.check_hits_circle(sf.point_xy(0.0, 0.0)?, 2000.0, 2, &[14, 15])?;
        self.check_hits_bbox(sf.point_xy(0.0, 0.0)?, 3000.0, 3, &[14, 15, 16])?;
        self.check_hits_circle(sf.point_xy(0.0, 0.0)?, 3001.0, 3, &[14, 15, 16])?;
        self.check_hits_circle(sf.point_xy(0.0, 0.0)?, 3000.1, 3, &[14, 15, 16])?;

        // really fine grained distance and reflects some of the vagaries of how we are calculating the
        // box
        self.check_hits_circle(sf.point_xy(-96.789603, 43.517030)?, 109.0, 0, &[])?;

        // falls outside of the real distance, but inside the bounding box
        self.check_hits_circle(sf.point_xy(-96.789603, 43.517030)?, 110.0, 0, &[])?;
        self.check_hits_bbox(sf.point_xy(-96.789603, 43.517030)?, 110.0, 1, &[17])
    }

    // ---- these are similar to Solr test methods

    /// `checkHitsCircle(Point pt, double distKM, int assertNumFound, int... assertIds)`.
    fn check_hits_circle(
        &self,
        pt: Arc<dyn Point>,
        dist_km: f64,
        assert_num_found: i32,
        assert_ids: &[i32],
    ) -> Result<()> {
        self.check_hits(false, pt, dist_km, assert_num_found, assert_ids)
    }

    /// `checkHitsBBox(Point pt, double distKM, int assertNumFound, int... assertIds)`.
    fn check_hits_bbox(
        &self,
        pt: Arc<dyn Point>,
        dist_km: f64,
        assert_num_found: i32,
        assert_ids: &[i32],
    ) -> Result<()> {
        self.check_hits(true, pt, dist_km, assert_num_found, assert_ids)
    }

    /// `_checkHits(boolean bbox, Point pt, double distKM, int assertNumFound, int... assertIds)`.
    fn check_hits(
        &self,
        bbox: bool,
        pt: Arc<dyn Point>,
        dist_km: f64,
        assert_num_found: i32,
        assert_ids: &[i32],
    ) -> Result<()> {
        let op = SpatialOperation::Intersects;
        let dist_deg = DistanceUtils::dist2_degrees(dist_km, DistanceUtils::EARTH_MEAN_RADIUS_KM);
        let mut shape: Arc<dyn Shape> = self.shape_factory.circle_at(pt, dist_deg)?;
        if bbox {
            shape = shape.get_bounding_box();
        }

        let args = SpatialArgs::new(op, Arc::clone(&shape));
        // args.setDistPrecision(0.025);
        let query = self.fx.strategy().make_query(&args)?;
        let results = self.execute_query(query, 100)?;
        Assert::assert_equals_msg(
            Some(&shape.to_string()),
            i64::from(assert_num_found),
            results.num_found,
        );
        // (`assertIds` is never null)
        let mut result_ids: HashSet<i32> = HashSet::new();
        for result in &results.results {
            result_ids.insert(crate::spatial_support::java_parse_int(
                &result.document.get("id").unwrap_or_default(),
            )?);
        }
        for assert_id in assert_ids {
            Assert::assert_true_msg(&format!("has {assert_id}"), result_ids.contains(assert_id));
        }
        Ok(())
    }
}
