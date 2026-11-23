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

//! Port of `TestRandomSpatialOpFuzzyPrefixTree.java` (package
//! `org.apache.lucene.spatial.prefix`): randomized PrefixTree test that
//! considers the fuzziness of the results introduced by grid approximation.
//!
//! The inner class `ShapePair extends ShapeCollection<Shape>` is
//! [`ShapePair`], a shape of its own that delegates to a `ShapeCollection`
//! of its two shapes and overrides `relate`; it captures the enclosing
//! instance's `ctx` and `ctx2D` when it is made, as its Java constructor
//! reads them (and `relate` reads `ctx2D`, which a test does not reassign
//! after making its pairs).

use std::fmt;
use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Assert, Test};
use randomizedtesting::RandomizedTest;
use rucene::document::{Document, Store, StoredField, StringField};
use rucene::error::Result;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::{
    RecursivePrefixTreeStrategy, RecursivePrefixTreeStrategyOps,
};
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::packed_quad_prefix_tree::PackedQuadPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::{
    SpatialContextFactory, SpatialContextFactoryOps,
};
use spatial4j::shape::point::Point;
use spatial4j::shape::r#impl::rectangle_impl::RectangleImpl;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::shape_collection::ShapeCollection;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::prefix::test_random_spatial_op_fuzzy_prefix_tree50::RecursivePrefixTreeStrategy50;
use crate::spatial::spatial_test_case::{
    spatial_class, strategy_test_case_class, ti, SpatialCase, SpatialFixture, SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;
use crate::spatial_support::{random, LinkedHashSet};

const NAME: &str = "org.apache.lucene.spatial.prefix.TestRandomSpatialOpFuzzyPrefixTree";

/// `ITERATIONS`.
const ITERATIONS: i32 = 10;

/// `TestRandomSpatialOpFuzzyPrefixTree`.
pub fn class_def() -> Arc<Class> {
    fuzzy_class()
}

/// The class `TestRandomSpatialOpFuzzyPrefixTree`, which
/// `TestRandomSpatialOpFuzzyPrefixTree50` extends.
pub fn fuzzy_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        type T = TestRandomSpatialOpFuzzyPrefixTree;
        let repeat = || Repeat {
            iterations: ITERATIONS,
            use_constant_seed: false,
        };
        spatial_class::<T>(
            NAME,
            &strategy_test_case_class(),
            Constructor::no_args(|| {
                TestRandomSpatialOpFuzzyPrefixTree::new(
                    NAME,
                    "TestRandomSpatialOpFuzzyPrefixTree",
                    false,
                )
            }),
            vec![
                ti::<T>("testIntersects", T::test_intersects)
                    .annotate(Test::new())
                    .annotate(repeat()),
                ti::<T>("testWithin", T::test_within)
                    .annotate(Test::new())
                    .annotate(repeat()),
                ti::<T>("testContains", T::test_contains)
                    .annotate(Test::new())
                    .annotate(repeat()),
                ti::<T>(
                    "testPackedQuadPointsOnlyBug",
                    T::test_packed_quad_points_only_bug,
                )
                .annotate(Test::new()),
                ti::<T>("testPointsOnlyOptBug", T::test_points_only_opt_bug).annotate(Test::new()),
                ti::<T>("testContainsPairOverlap", T::test_contains_pair_overlap)
                    .annotate(Test::new()),
                ti::<T>("testWithinDisjointParts", T::test_within_disjoint_parts)
                    .annotate(Test::new()),
                ti::<T>("testWithinLeafApproxRule", T::test_within_leaf_approx_rule)
                    .annotate(Test::new()),
                ti::<T>("testShapePair", T::test_shape_pair).annotate(Test::new()),
            ],
        )
    });
    Arc::clone(&CLASS)
}

/// The `TestRandomSpatialOpFuzzyPrefixTree` instance (also the instance of
/// its subclass `TestRandomSpatialOpFuzzyPrefixTree50`).
pub struct TestRandomSpatialOpFuzzyPrefixTree {
    fx: SpatialFixture,
    /// `getClass().getSimpleName()`.
    simple_name: &'static str,
    /// Whether this is a `TestRandomSpatialOpFuzzyPrefixTree50`, whose
    /// `newRPT()` overrides this class's.
    format50: bool,
    /// `grid` (`protected`).
    grid: Option<Arc<dyn SpatialPrefixTree>>,
    /// `ctx2D` (`private`).
    ctx2d: Option<Arc<SpatialContext>>,
    /// `(RecursivePrefixTreeStrategy) strategy`.
    rpt: Option<Arc<dyn RecursivePrefixTreeStrategyOps>>,
}

impl SpatialCase for TestRandomSpatialOpFuzzyPrefixTree {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `newDoc(String id, Shape shape)`: overridden so we can index parts of
    /// a pair separately, resulting in the detailLevel being independent for
    /// each shape vs the whole thing.
    fn new_doc(&self, id: &str, shape: Option<&Arc<dyn Shape>>) -> Result<Document> {
        let mut doc = Document::new();
        doc.add(StringField::new("id", id.to_string(), Store::YES)?);
        if let Some(shape) = shape {
            let shapes: Vec<Arc<dyn Shape>> =
                if let Some(pair) = shape.as_any().downcast_ref::<ShapePair>() {
                    vec![Arc::clone(&pair.shape1), Arc::clone(&pair.shape2)]
                } else {
                    vec![Arc::clone(shape)]
                };
            let strategy = self.fx.strategy();
            for shapei in &shapes {
                for f in strategy.create_indexable_fields(shapei)? {
                    doc.add_shared(f);
                }
            }
            if self.fx.store_shape {
                // just for diagnostics
                doc.add(StoredField::new_string(
                    strategy.get_field_name().to_string(),
                    shape.to_string(),
                )?);
            }
        }
        Ok(doc)
    }
}

impl TestRandomSpatialOpFuzzyPrefixTree {
    /// The field initializers of the class named `class_name`.
    pub fn new(class_name: &str, simple_name: &'static str, format50: bool) -> Self {
        Self {
            fx: SpatialFixture::new(class_name),
            simple_name,
            format50,
            grid: None,
            ctx2d: None,
            rpt: None,
        }
    }

    fn grid(&self) -> &Arc<dyn SpatialPrefixTree> {
        self.grid
            .as_ref()
            .expect("java.lang.NullPointerException: grid")
    }

    fn rpt(&self) -> &Arc<dyn RecursivePrefixTreeStrategyOps> {
        self.rpt
            .as_ref()
            .expect("java.lang.NullPointerException: strategy")
    }

    /// `strategy = newRPT()`.
    fn set_strategy_new_rpt(&mut self) -> Result<()> {
        let rpt = self.new_rpt()?;
        self.fx.strategy = Some(rpt.clone());
        self.rpt = Some(rpt);
        Ok(())
    }

    /// `setupGrid(int maxLevels)`.
    pub fn setup_grid(&mut self, max_levels: i32) -> Result<()> {
        if RandomizedTest::random_boolean() {
            self.setup_quad_grid(max_levels, RandomizedTest::random_boolean())?;
        } else {
            self.setup_geohash_grid(max_levels)?;
        }
        self.setup_ctx2d(&self.fx.ctx())?;

        // set prune independently on strategy & grid randomly; should work
        self.rpt()
            .set_prune_leafy_branches(RandomizedTest::random_boolean());
        if let Some(packed) = self.grid().as_any().downcast_ref::<PackedQuadPrefixTree>() {
            packed.set_prune_leafy_branches(RandomizedTest::random_boolean());
        }

        if max_levels == -1 && LuceneTestCase::rarely() {
            self.rpt().set_points_only(true);
        }

        self.fx
            .log
            .info(&format!("Strategy: {}", self.fx.strategy())); // nowarn
        Ok(())
    }

    /// `setupCtx2D(SpatialContext ctx)`.
    fn setup_ctx2d(&mut self, ctx: &Arc<SpatialContext>) -> Result<()> {
        if !ctx.is_geo() {
            self.ctx2d = Some(Arc::clone(ctx));
        }
        // A non-geo version of ctx.
        let mut ctx_factory = SpatialContextFactory::new();
        ctx_factory.geo = false;
        ctx_factory.world_bounds = Some(ctx.get_world_bounds());
        self.ctx2d = Some(ctx_factory.new_spatial_context()?);
        Ok(())
    }

    /// `setupQuadGrid(int maxLevels, boolean packedQuadPrefixTree)`.
    fn setup_quad_grid(
        &mut self,
        mut max_levels: i32,
        packed_quad_prefix_tree: bool,
    ) -> Result<()> {
        // non-geospatial makes this test a little easier (in gridSnap), and using boundary values 2^X
        // raises
        // the prospect of edge conditions we want to test, plus makes for simpler numbers (no
        // decimals).
        let mut factory = SpatialContextFactory::new();
        factory.geo = false;
        factory.world_bounds = Some(RectangleImpl::new(0.0, 256.0, -128.0, 128.0, None));
        self.fx.ctx = Some(factory.new_spatial_context()?);
        // A fairly shallow grid, and default 2.5% distErrPct
        if max_levels == -1 {
            max_levels = RandomizedTest::random_int_between(1, 8); // max 64k cells (4^8), also 256*256
        }
        self.grid = Some(if packed_quad_prefix_tree {
            PackedQuadPrefixTree::new(self.fx.ctx(), max_levels)?
        } else {
            QuadPrefixTree::new(self.fx.ctx(), max_levels)
        });
        self.set_strategy_new_rpt()
    }

    /// `setupGeohashGrid(int maxLevels)`.
    pub fn setup_geohash_grid(&mut self, mut max_levels: i32) -> Result<()> {
        self.fx.ctx = Some(SpatialContext::geo());
        // A fairly shallow grid, and default 2.5% distErrPct
        if max_levels == -1 {
            max_levels = RandomizedTest::random_int_between(1, 3); // max 16k cells (32^3)
        }
        self.grid = Some(GeohashPrefixTree::new(self.fx.ctx(), max_levels)?);
        self.set_strategy_new_rpt()
    }

    /// `newRPT()` (overridden by `TestRandomSpatialOpFuzzyPrefixTree50`).
    fn new_rpt(&self) -> Result<Arc<dyn RecursivePrefixTreeStrategyOps>> {
        let grid = Arc::clone(self.grid());
        if self.format50 {
            Ok(RecursivePrefixTreeStrategy50::new(grid, self.simple_name)?)
        } else {
            Ok(RecursivePrefixTreeStrategy::new(grid, self.simple_name)?)
        }
    }

    /// `testIntersects()`.
    fn test_intersects(&mut self) -> Result<()> {
        self.setup_grid(-1)?;
        self.do_test(SpatialOperation::Intersects)
    }

    /// `testWithin()`.
    fn test_within(&mut self) -> Result<()> {
        self.setup_grid(-1)?;
        self.do_test(SpatialOperation::IsWithin)
    }

    /// `testContains()`.
    fn test_contains(&mut self) -> Result<()> {
        self.setup_grid(-1)?;
        self.do_test(SpatialOperation::Contains)
    }

    /// `testPackedQuadPointsOnlyBug()`.
    fn test_packed_quad_points_only_bug(&mut self) -> Result<()> {
        self.setup_quad_grid(1, true)?; // packed quad.  maxLevels doesn't matter.
        self.setup_ctx2d(&self.fx.ctx())?;
        self.rpt().set_points_only(true);
        let point: Arc<dyn Shape> = self.fx.ctx().make_point(169.0, 107.0)?;
        self.adoc("0", Some(Arc::clone(&point)))?;
        self.commit()?;
        let query = self
            .fx
            .strategy()
            .make_query(&SpatialArgs::new(SpatialOperation::Intersects, point))?;
        Assert::assert_equals(1i64, self.execute_query(query, 1)?.num_found);
        Ok(())
    }

    /// `testPointsOnlyOptBug()`.
    fn test_points_only_opt_bug(&mut self) -> Result<()> {
        self.setup_quad_grid(8, false)?;
        self.setup_ctx2d(&self.fx.ctx())?;
        self.rpt().set_points_only(true);
        let point = self.fx.ctx().make_point(86.0, -127.44362190053255)?;
        self.adoc("0", Some(point.clone()))?;
        self.commit()?;
        let query = self.fx.strategy().make_query(&SpatialArgs::new(
            SpatialOperation::Intersects,
            self.fx.ctx().make_rectangle_from_points(&*point, &*point)?,
        ))?;
        Assert::assert_equals(1i64, self.execute_query(query, 1)?.num_found);
        Ok(())
    }

    /// `testContainsPairOverlap()`: see LUCENE-5062,
    /// `ContainsPrefixTreeQuery#multiOverlappingIndexedShapes`.
    fn test_contains_pair_overlap(&mut self) -> Result<()> {
        self.setup_quad_grid(3, RandomizedTest::random_boolean())?;
        let ctx = self.fx.ctx();
        let pair = self.shape_pair(
            ctx.make_rectangle(0.0, 33.0, -128.0, 128.0)?,
            ctx.make_rectangle(33.0, 128.0, -128.0, 128.0)?,
            true,
        )?;
        self.adoc("0", Some(pair))?;
        self.commit()?;
        let query = self.fx.strategy().make_query(&SpatialArgs::new(
            SpatialOperation::Contains,
            ctx.make_rectangle(0.0, 128.0, -16.0, 128.0)?,
        ))?;
        let search_results = self.execute_query(query, 1)?;
        Assert::assert_equals(1i64, search_results.num_found);
        Ok(())
    }

    /// `testWithinDisjointParts()`.
    fn test_within_disjoint_parts(&mut self) -> Result<()> {
        self.setup_quad_grid(7, RandomizedTest::random_boolean())?;
        let ctx = self.fx.ctx();
        // one shape comprised of two parts, quite separated apart
        let pair = self.shape_pair(
            ctx.make_rectangle(0.0, 10.0, -120.0, -100.0)?,
            ctx.make_rectangle(220.0, 240.0, 110.0, 125.0)?,
            false,
        )?;
        self.adoc("0", Some(pair))?;
        self.commit()?;
        // query surrounds only the second part of the indexed shape
        let query = self.fx.strategy().make_query(&SpatialArgs::new(
            SpatialOperation::IsWithin,
            ctx.make_rectangle(210.0, 245.0, 105.0, 128.0)?,
        ))?;
        let search_results = self.execute_query(query, 1)?;
        // we shouldn't find it because it's not completely within
        Assert::assert_true(search_results.num_found == 0);
        Ok(())
    }

    /// `testWithinLeafApproxRule()` (LUCENE-4916).
    fn test_within_leaf_approx_rule(&mut self) -> Result<()> {
        self.setup_quad_grid(2, RandomizedTest::random_boolean())?; // 4x4 grid
        let ctx = self.fx.ctx();
        // indexed shape will simplify to entire right half (2 top cells)
        self.adoc("0", Some(ctx.make_rectangle(192.0, 204.0, -128.0, 128.0)?))?;
        self.commit()?;

        // spatial4j calls `randomInt(int)`, which randomizedtesting deprecates.
        #[allow(deprecated)]
        let scan_level = RandomizedTest::random_int_max(2);
        self.rpt().set_prefix_grid_scan_level(scan_level);

        // query does NOT contain it; both indexed cells are leaves to the query, and
        // when expanded to the full grid cells, the top one's top row is disjoint
        // from the query and thus not a match.
        let q = self.fx.strategy().make_query(&SpatialArgs::new(
            SpatialOperation::IsWithin,
            ctx.make_rectangle(38.0, 192.0, -72.0, 56.0)?,
        ))?;
        Assert::assert_true(self.execute_query(q, 1)?.num_found == 0); // no-match

        // this time the rect is a little bigger and is considered a match. It's
        // an acceptable false-positive because of the grid approximation.
        let q = self.fx.strategy().make_query(&SpatialArgs::new(
            SpatialOperation::IsWithin,
            ctx.make_rectangle(38.0, 192.0, -72.0, 80.0)?,
        ))?;
        Assert::assert_true(self.execute_query(q, 1)?.num_found == 1); // match
        Ok(())
    }

    /// `testShapePair()`.
    fn test_shape_pair(&mut self) -> Result<()> {
        self.fx.ctx = Some(SpatialContext::geo());
        self.setup_ctx2d(&self.fx.ctx())?;
        let ctx = self.fx.ctx();

        let left_shape = self.shape_pair(
            ctx.make_rectangle(-74.0, -56.0, -8.0, 1.0)?,
            ctx.make_rectangle(-180.0, 134.0, -90.0, 90.0)?,
            true,
        )?;
        let query_shape = ctx.make_rectangle(-180.0, 180.0, -90.0, 90.0)?;
        Assert::assert_equals(
            SpatialRelation::Within,
            left_shape.relate(query_shape.as_shape()),
        );
        Ok(())
    }

    /// `doTest(SpatialOperation operation)`.
    fn do_test(&mut self, operation: SpatialOperation) -> Result<()> {
        // first show that when there's no data, a query will result in no results
        {
            let rect = self.random_rectangle()?;
            let query = self
                .fx
                .strategy()
                .make_query(&SpatialArgs::new(operation, rect))?;
            let search_results = self.execute_query(query, 1)?;
            Assert::assert_equals(0i64, search_results.num_found);
        }

        let bias_contains = operation == SpatialOperation::Contains;

        // Main index loop:
        let mut indexed_shapes: Vec<(String, Option<Arc<dyn Shape>>)> = Vec::new();
        let mut indexed_shapes_gs: Vec<(String, Option<Arc<dyn Shape>>)> = Vec::new(); // grid snapped
        let num_indexed_shapes = RandomizedTest::random_int_between(1, 6);
        let points_only = self.rpt().is_points_only();
        for i in 0..num_indexed_shapes {
            let id = i.to_string();
            let r = random().next_int_bound(12);
            let indexed_shape: Option<Arc<dyn Shape>> = if r == 0 {
                // 1 in 12
                None
            } else if r == 1 || points_only {
                // 1 in 12
                Some(self.random_point()?) // just one point
            } else if r <= 4 {
                // 3 in 12
                // comprised of more than one shape
                Some(self.random_shape_pair_rect(bias_contains)?)
            } else {
                Some(self.random_rectangle()?) // just one rect
            };

            put(&mut indexed_shapes, &id, indexed_shape.clone());
            let gs = self.grid_snap(indexed_shape.as_ref())?;
            put(&mut indexed_shapes_gs, &id, gs);

            self.adoc(&id, indexed_shape)?;

            if random().next_int_bound(10) == 0 {
                self.commit()?; // intermediate commit, produces extra segments
            }
        }
        // delete some documents randomly
        let mut i = 0;
        while i < indexed_shapes.len() {
            let id = indexed_shapes[i].0.clone();
            if random().next_int_bound(10) == 0 {
                self.delete_doc(&id)?;
                indexed_shapes.remove(i);
                indexed_shapes_gs.retain(|(k, _)| *k != id);
            } else {
                i += 1;
            }
        }

        self.commit()?;

        // Main query loop:
        let num_query_shapes = LuceneTestCase::at_least(20);
        for _ in 0..num_query_shapes {
            // spatial4j calls `randomInt(int)`, which randomizedtesting deprecates.
            #[allow(deprecated)]
            let scan_level = RandomizedTest::random_int_max(self.grid().get_max_levels());
            self.rpt().set_prefix_grid_scan_level(scan_level);

            #[allow(deprecated)]
            let choice = RandomizedTest::random_int_max(10);
            let query_shape: Arc<dyn Shape> = 'pick: {
                match choice {
                    0 => break 'pick self.random_point()?,
                    // LUCENE-5549
                    // TODO debug: -Dtests.method=testWithin -Dtests.multiplier=3
                    // -Dtests.seed=5F5294CE2E075A3E:AAD2F0F79288CA64
                    //        case 1:case 2:case 3:
                    //          if (!indexedAtLeastOneShapePair) {
                    // // avoids ShapePair.relate(ShapePair), which isn't reliable
                    //            queryShape = randomShapePairRect(!biasContains);
                    // // invert biasContains for query side
                    //            break;
                    //          }
                    4 => {
                        // choose an existing indexed shape
                        if let Some((_, Some(tmp))) = indexed_shapes.first() {
                            if tmp.as_point().is_some() || tmp.as_rectangle().is_some() {
                                // avoids null and shapePair
                                break 'pick Arc::clone(tmp);
                            }
                        }
                        // fall-through
                    }
                    _ => {}
                }
                self.random_rectangle()?
            };
            let query_shape_gs = self.grid_snap(Some(&query_shape))?;

            let op_is_disjoint = operation == SpatialOperation::IsDisjointTo;

            // Generate truth via brute force:
            // We ensure true-positive matches (if the predicate on the raw shapes match
            //  then the search should find those same matches).
            // approximations, false-positive matches
            let mut expected_ids = LinkedHashSet::new(); // true-positives
            let mut secondary_ids = LinkedHashSet::new(); // false-positives (unless disjoint)
            for (id, indexed_shape_compare) in &indexed_shapes {
                let Some(mut indexed_shape_compare) = indexed_shape_compare.clone() else {
                    continue;
                };
                let mut query_shape_compare: Option<Arc<dyn Shape>> =
                    Some(Arc::clone(&query_shape));

                if operation.evaluate(
                    &*indexed_shape_compare,
                    &**query_shape_compare.as_ref().expect("set"),
                ) {
                    expected_ids.add(id.clone());
                    if op_is_disjoint {
                        // if no longer intersect after buffering them, for disjoint, remember this
                        indexed_shape_compare = get(&indexed_shapes_gs, id).expect("snapped");
                        query_shape_compare = query_shape_gs.clone();
                        if !operation.evaluate(
                            &*indexed_shape_compare,
                            &**query_shape_compare.as_ref().expect("snapped"),
                        ) {
                            secondary_ids.add(id.clone());
                        }
                    }
                } else if !op_is_disjoint {
                    // buffer either the indexed or query shape (via gridSnap) and try again
                    if operation == SpatialOperation::Intersects {
                        indexed_shape_compare = get(&indexed_shapes_gs, id).expect("snapped");
                        query_shape_compare = query_shape_gs.clone();
                        // TODO Unfortunately, grid-snapping both can result in intersections that otherwise
                        // wouldn't happen when the grids are adjacent. Not a big deal but our test is just a
                        // bit more lenient.
                    } else if operation == SpatialOperation::Contains {
                        indexed_shape_compare = get(&indexed_shapes_gs, id).expect("snapped");
                    } else if operation == SpatialOperation::IsWithin {
                        query_shape_compare = query_shape_gs.clone();
                    }
                    if operation.evaluate(
                        &*indexed_shape_compare,
                        &**query_shape_compare.as_ref().expect("snapped"),
                    ) {
                        secondary_ids.add(id.clone());
                    }
                }
            }

            // Search and verify results
            let args = SpatialArgs::new(operation, Arc::clone(&query_shape));
            if query_shape.as_any().downcast_ref::<ShapePair>().is_some() {
                args.set_dist_err_pct(Some(0.0)); // a hack; we want to be more detailed than gridSnap(queryShape)
            }
            let query = self.fx.strategy().make_query(&args)?;
            let got = self.execute_query(query, 100)?;
            let mut remaining_expected_ids = expected_ids.clone();
            for result in &got.results {
                let id = result.get_id().unwrap_or_else(|| "null".to_string());
                let removed = remaining_expected_ids.remove(&id);
                if !removed && (!op_is_disjoint && !secondary_ids.contains(&id)) {
                    self.fail(
                        "Shouldn't match",
                        &id,
                        &indexed_shapes,
                        &indexed_shapes_gs,
                        &query_shape,
                    )?;
                }
            }
            if op_is_disjoint {
                remaining_expected_ids.remove_all(&secondary_ids);
            }
            if let Some(id) = remaining_expected_ids.first().cloned() {
                self.fail(
                    "Should have matched",
                    &id,
                    &indexed_shapes,
                    &indexed_shapes_gs,
                    &query_shape,
                )?;
            }
        }
        Ok(())
    }

    /// `randomShapePairRect(boolean biasContains)`.
    fn random_shape_pair_rect(&self, bias_contains: bool) -> Result<Arc<dyn Shape>> {
        let shape1 = self.random_rectangle()?;
        let shape2 = self.random_rectangle()?;
        self.shape_pair(shape1, shape2, bias_contains)
    }

    /// `fail(String label, String id, Map<String, Shape> indexedShapes,
    /// Map<String, Shape> indexedShapesGS, Shape queryShape)`.
    fn fail(
        &self,
        label: &str,
        id: &str,
        indexed_shapes: &[(String, Option<Arc<dyn Shape>>)],
        indexed_shapes_gs: &[(String, Option<Arc<dyn Shape>>)],
        query_shape: &Arc<dyn Shape>,
    ) -> Result<()> {
        system::err_println(&format!(
            "Ig:{} Qg:{}",
            shape_str(get(indexed_shapes_gs, id).as_ref()),
            shape_str(self.grid_snap(Some(query_shape))?.as_ref())
        ));
        Assert::fail(&format!(
            "{label} I#{id}:{} Q:{query_shape}",
            shape_str(get(indexed_shapes, id).as_ref())
        ));
    }

    /// `gridSnap(Shape snapMe)`.
    fn grid_snap(&self, snap_me: Option<&Arc<dyn Shape>>) -> Result<Option<Arc<dyn Shape>>> {
        let Some(snap_me) = snap_me else {
            return Ok(None);
        };
        if let Some(me) = snap_me.as_any().downcast_ref::<ShapePair>() {
            let s1 = self.grid_snap(Some(&me.shape1))?.expect("not null");
            let s2 = self.grid_snap(Some(&me.shape2))?.expect("not null");
            return Ok(Some(self.shape_pair(
                s1,
                s2,
                me.bias_contains_then_within,
            )?));
        }
        let mut snap_me: Arc<dyn Shape> = Arc::clone(snap_me);
        if snap_me.as_point().is_some() {
            snap_me = snap_me.get_bounding_box();
        }
        // The next 4 lines mimic PrefixTreeStrategy.createIndexableFields()
        let ctx = self.fx.ctx();
        let dist_err_pct = self.rpt().get_dist_err_pct();
        let dist_err = SpatialArgs::calc_distance_from_err_pct(&*snap_me, dist_err_pct, &ctx)?;
        let detail_level = self.grid().get_level_for_distance(dist_err);
        let mut cells = self.grid().get_tree_cell_iterator(snap_me, detail_level)?;

        // calc bounding box of cells.
        let mut cell_shapes: Vec<Arc<dyn Shape>> = Vec::with_capacity(1024);
        while cells.has_next() {
            let cell = cells.next();
            if !cell.is_leaf() {
                continue;
            }
            cell_shapes.push(cell.get_shape());
        }
        Ok(Some(
            ShapeCollection::new(cell_shapes, &ctx)?.get_bounding_box(),
        ))
    }

    /// `new ShapePair(Shape shape1, Shape shape2, boolean containsThenWithin)`.
    fn shape_pair(
        &self,
        shape1: Arc<dyn Shape>,
        shape2: Arc<dyn Shape>,
        contains_then_within: bool,
    ) -> Result<Arc<dyn Shape>> {
        Ok(ShapePair::new(
            shape1,
            shape2,
            contains_then_within,
            self.fx.ctx(),
            self.ctx2d.clone(),
        )?)
    }
}

/// `map.put(key, value)` over an insertion-ordered map.
fn put(map: &mut Vec<(String, Option<Arc<dyn Shape>>)>, key: &str, value: Option<Arc<dyn Shape>>) {
    match map.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => entry.1 = value,
        None => map.push((key.to_string(), value)),
    }
}

/// `map.get(key)` over an insertion-ordered map.
fn get(map: &[(String, Option<Arc<dyn Shape>>)], key: &str) -> Option<Arc<dyn Shape>> {
    map.iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.clone())
}

/// `String.valueOf(shape)`.
fn shape_str(shape: Option<&Arc<dyn Shape>>) -> String {
    shape.map_or_else(|| "null".to_string(), |s| s.to_string())
}

/// `ShapePair` (`private class ShapePair extends ShapeCollection<Shape>`):
/// an aggregate of 2 shapes. Unfortunately we can't simply use a
/// ShapeCollection because: (a) ambiguity between CONTAINS and WITHIN for
/// equal shapes, and (b) adjacent pairs could as a whole contain the input
/// shape. The tests here are sensitive to these matters, although in
/// practice ShapeCollection is fine.
#[derive(Debug)]
pub struct ShapePair {
    /// The `ShapeCollection` this class extends.
    collection: Arc<ShapeCollection>,
    shape1: Arc<dyn Shape>,
    shape2: Arc<dyn Shape>,
    shape1_2d: Arc<dyn Shape>, // not geo (bit of a hack)
    shape2_2d: Arc<dyn Shape>,
    bias_contains_then_within: bool,
    /// The enclosing instance's `ctx2D`.
    ctx2d: Option<Arc<SpatialContext>>,
}

impl ShapePair {
    /// `ShapePair(Shape shape1, Shape shape2, boolean containsThenWithin)`,
    /// with the enclosing instance's `ctx` and `ctx2D`.
    fn new(
        shape1: Arc<dyn Shape>,
        shape2: Arc<dyn Shape>,
        contains_then_within: bool,
        ctx: Arc<SpatialContext>,
        ctx2d: Option<Arc<SpatialContext>>,
    ) -> Result<Arc<ShapePair>> {
        let collection =
            ShapeCollection::new(vec![Arc::clone(&shape1), Arc::clone(&shape2)], &ctx)?;
        let shape1_2d = to_non_geo(&shape1, &ctx, &ctx2d, contains_then_within)?;
        let shape2_2d = to_non_geo(&shape2, &ctx, &ctx2d, contains_then_within)?;
        Ok(Arc::new(ShapePair {
            collection,
            shape1,
            shape2,
            shape1_2d,
            shape2_2d,
            bias_contains_then_within: contains_then_within,
            ctx2d,
        }))
    }

    fn ctx2d(&self) -> &Arc<SpatialContext> {
        self.ctx2d
            .as_ref()
            .expect("java.lang.NullPointerException: ctx2D")
    }

    /// `cornerContainsNonGeo(double x, double y)`.
    fn corner_contains_non_geo(&self, x: f64, y: f64) -> bool {
        let pt = self
            .ctx2d()
            .make_point(x, y)
            .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e));
        self.shape1_2d.relate(pt.as_shape()).intersects()
            || self.shape2_2d.relate(pt.as_shape()).intersects()
    }

    /// `relateApprox(Shape other)`.
    fn relate_approx(&self, other: &dyn Shape) -> SpatialRelation {
        use SpatialRelation::{Contains, Disjoint, Intersects, Within};
        if self.bias_contains_then_within {
            if self.shape1.relate(other) == Contains
                || self.shape1.equals(other)
                || self.shape2.relate(other) == Contains
                || self.shape2.equals(other)
            {
                return Contains;
            }

            if self.shape1.relate(other) == Within && self.shape2.relate(other) == Within {
                return Within;
            }
        } else {
            if (self.shape1.relate(other) == Within || self.shape1.equals(other))
                && (self.shape2.relate(other) == Within || self.shape2.equals(other))
            {
                return Within;
            }

            if self.shape1.relate(other) == Contains || self.shape2.relate(other) == Contains {
                return Contains;
            }
        }

        if self.shape1.relate(other).intersects() || self.shape2.relate(other).intersects() {
            // might actually be 'CONTAINS' if the pair are adjacent but we handle
            // that later
            return Intersects;
        }
        Disjoint
    }
}

/// `toNonGeo(Shape shape)` (`private`).
fn to_non_geo(
    shape: &Arc<dyn Shape>,
    ctx: &Arc<SpatialContext>,
    ctx2d: &Option<Arc<SpatialContext>>,
    bias_contains_then_within: bool,
) -> Result<Arc<dyn Shape>> {
    if !ctx.is_geo() {
        return Ok(Arc::clone(shape)); // already non-geo
    }
    if let Some(rect) = shape.as_rectangle() {
        let ctx2d_ref = ctx2d
            .as_ref()
            .expect("java.lang.NullPointerException: ctx2D");
        if rect.get_crosses_date_line() {
            return Ok(ShapePair::new(
                ctx2d_ref.make_rectangle(
                    rect.get_min_x(),
                    180.0,
                    rect.get_min_y(),
                    rect.get_max_y(),
                )?,
                ctx2d_ref.make_rectangle(
                    -180.0,
                    rect.get_max_x(),
                    rect.get_min_y(),
                    rect.get_max_y(),
                )?,
                bias_contains_then_within,
                Arc::clone(ctx),
                ctx2d.clone(),
            )?);
        } else {
            return Ok(ctx2d_ref.make_rectangle(
                rect.get_min_x(),
                rect.get_max_x(),
                rect.get_min_y(),
                rect.get_max_y(),
            )?);
        }
    }
    // no need to do others; this addresses the -180/+180 ambiguity corner test problem
    Ok(Arc::clone(shape))
}

impl fmt::Display for ShapePair {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ShapePair({} , {})", self.shape1, self.shape2)
    }
}

impl Shape for ShapePair {
    /// `relate(Shape other)`.
    fn relate(&self, other: &dyn Shape) -> SpatialRelation {
        use SpatialRelation::{Contains, Disjoint, Within};
        let r = self.relate_approx(other);
        if r == Disjoint {
            return r;
        }
        if r == Contains {
            return r;
        }
        if r == Within && !self.bias_contains_then_within {
            return r;
        }

        // See if the correct answer is actually Contains, when the indexed shapes are adjacent,
        // creating a larger shape that contains the input shape.
        let pair_touches = self.shape1.relate(self.shape2.as_shape()).intersects();
        if !pair_touches {
            return r;
        }
        // test all 4 corners
        // Note: awkwardly, we use a non-geo context for this because in geo, -180 & +180 are the same
        // place, which means
        //  that "other" might wrap the world horizontally and yet all its corners could be in shape1
        // (or shape2) even
        //  though shape1 is only adjacent to the dateline. I couldn't think of a better way to handle
        // this.
        let o_rect = other.as_rectangle().unwrap_or_else(|| {
            panic!(
                "java.lang.ClassCastException: class {} cannot be cast to class org.locationtech.spatial4j.shape.Rectangle",
                other.java_class_name()
            )
        });
        if self.corner_contains_non_geo(o_rect.get_min_x(), o_rect.get_min_y())
            && self.corner_contains_non_geo(o_rect.get_min_x(), o_rect.get_max_y())
            && self.corner_contains_non_geo(o_rect.get_max_x(), o_rect.get_min_y())
            && self.corner_contains_non_geo(o_rect.get_max_x(), o_rect.get_max_y())
        {
            return Contains;
        }
        r
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        self.collection.get_bounding_box()
    }

    fn has_area(&self) -> bool {
        self.collection.has_area()
    }

    fn get_area(&self, ctx: Option<&SpatialContext>) -> f64 {
        self.collection.get_area(ctx)
    }

    fn get_center(&self) -> Arc<dyn Point> {
        self.collection.get_center()
    }

    fn get_buffered(&self, distance: f64, ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        self.collection.get_buffered(distance, ctx)
    }

    fn is_empty(&self) -> bool {
        Shape::is_empty(&*self.collection)
    }

    /// `ShapeCollection.equals`: the same class and equal shape lists.
    fn equals(&self, other: &dyn Shape) -> bool {
        match other.as_any().downcast_ref::<ShapePair>() {
            Some(that) => {
                std::ptr::eq(self, that) || self.collection.equals(that.collection.as_shape())
            }
            None => false,
        }
    }

    fn hash_code(&self) -> i32 {
        self.collection.hash_code()
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.collection.get_context()
    }

    spatial4j::spatial4j_shape_boilerplate!(
        "org.apache.lucene.spatial.prefix.TestRandomSpatialOpFuzzyPrefixTree$ShapePair"
    );

    /// `this instanceof ShapeCollection`.
    fn as_shape_collection(&self) -> Option<&ShapeCollection> {
        Some(&self.collection)
    }
}
