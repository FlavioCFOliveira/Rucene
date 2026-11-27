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

//! Port of `TestHeatmapFacetCounter.java` (package
//! `org.apache.lucene.spatial.prefix`).

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{After, Assert, Test};
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene::search::Query;
use rucene_spatial_extras::prefix::heatmap_facet_counter;
use rucene_spatial_extras::prefix::intersects_prefix_tree_query::IntersectsPrefixTreeQuery;
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::RecursivePrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::{
    SpatialContextFactory, SpatialContextFactoryOps,
};
use spatial4j::distance::distance_utils::DistanceUtils;
use spatial4j::shape::r#impl::rectangle_impl::RectangleImpl;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::shape_factory::ShapeFactory;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::spatial::spatial_test_case::{
    java_assert, spatial_class, spatial_set_up, strategy_test_case_class, ti, SpatialCase,
    SpatialFixture, SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;
use crate::spatial_support::random;

const NAME: &str = "org.apache.lucene.spatial.prefix.TestHeatmapFacetCounter";
/// `getTestClass().getSimpleName()`.
const SIMPLE_NAME: &str = "TestHeatmapFacetCounter";

/// `TestHeatmapFacetCounter`.
pub fn class_def() -> Arc<Class> {
    type T = TestHeatmapFacetCounter;
    spatial_class::<T>(
        NAME,
        &strategy_test_case_class(),
        Constructor::no_args(|| TestHeatmapFacetCounter {
            fx: SpatialFixture::new(NAME),
            grid: None,
            rpt: None,
            shape_factory: None,
            cells_validated: 0,
            cell_validated_non_zero: 0,
        }),
        vec![
            ti::<T>("after", T::after).annotate(After),
            ti::<T>("testStatic", T::test_static).annotate(Test::new()),
            ti::<T>("testLucene7291Dateline", T::test_lucene7291_dateline).annotate(Test::new()),
            ti::<T>("testQueryCircle", T::test_query_circle).annotate(Test::new()),
            ti::<T>("testRandom", T::test_random)
                .annotate(Test::new())
                .annotate(Repeat {
                    iterations: 20,
                    use_constant_seed: false,
                }),
        ],
    )
}

/// The `TestHeatmapFacetCounter` instance.
pub struct TestHeatmapFacetCounter {
    fx: SpatialFixture,
    /// `grid`.
    grid: Option<Arc<dyn SpatialPrefixTree>>,
    /// `(RecursivePrefixTreeStrategy) strategy`.
    rpt: Option<Arc<RecursivePrefixTreeStrategy>>,
    /// `shapeFactory`.
    shape_factory: Option<Arc<dyn ShapeFactory>>,
    /// `cellsValidated`.
    cells_validated: i32,
    /// `cellValidatedNonZero`.
    cell_validated_non_zero: i32,
}

impl SpatialCase for TestHeatmapFacetCounter {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `setUp()`.
    fn set_up(&mut self) -> Result<()> {
        spatial_set_up(&mut self.fx)?;
        self.cells_validated = 0;
        self.cell_validated_non_zero = 0;
        self.fx.ctx = Some(SpatialContext::geo());
        self.shape_factory = Some(self.fx.ctx().get_shape_factory());
        self.grid = Some(QuadPrefixTree::new(
            self.fx.ctx(),
            RandomizedTest::random_int_between(1, 8),
        ));
        self.set_rpt()?;
        if LuceneTestCase::rarely() {
            self.rpt().set_points_only(true);
        }
        Ok(())
    }
}

impl TestHeatmapFacetCounter {
    fn grid(&self) -> &Arc<dyn SpatialPrefixTree> {
        self.grid.as_ref().expect("grid")
    }

    fn rpt(&self) -> &Arc<RecursivePrefixTreeStrategy> {
        self.rpt.as_ref().expect("strategy")
    }

    fn sf(&self) -> Arc<dyn ShapeFactory> {
        Arc::clone(self.shape_factory.as_ref().expect("shapeFactory"))
    }

    /// `strategy = new RecursivePrefixTreeStrategy(grid, getTestClass().getSimpleName())`.
    fn set_rpt(&mut self) -> Result<()> {
        let rpt = RecursivePrefixTreeStrategy::new(Arc::clone(self.grid()), SIMPLE_NAME)?;
        self.fx.strategy = Some(rpt.clone());
        self.rpt = Some(rpt);
        Ok(())
    }

    /// `after()` (`@After`).
    fn after(&mut self) -> Result<()> {
        self.fx.log.info(&format!(
            "Validated {} cells, {} non-zero",
            self.cells_validated, self.cell_validated_non_zero
        )); // nowarn
        Ok(())
    }

    /// `testStatic()`.
    fn test_static(&mut self) -> Result<()> {
        let sf = self.sf();
        // Some specific tests (static, not random).
        self.adoc("0", Some(sf.rect(179.8, -170.0, -90.0, -80.0)?))?; // barely crosses equator
        self.adoc("1", Some(sf.point_xy(-180.0, -85.0)?))?; // a pt within the above rect
        self.adoc("2", Some(sf.point_xy(172.0, -85.0)?))?; // a pt to left of rect
        self.commit()?;

        self.validate_heatmap_result_loop(sf.rect(170.0, 180.0, -90.0, -85.0)?, 1, 100)?;
        self.validate_heatmap_result_loop(sf.rect(-180.0, -160.0, -89.0, -50.0)?, 1, 100)?;
        self.validate_heatmap_result_loop(sf.rect(179.0, 179.0, -89.0, -50.0)?, 1, 100)?; // line
                                                                                          // We could test anything and everything at this point... I prefer we leave that to random
                                                                                          // testing and then
                                                                                          // add specific tests if we find a bug.
        Ok(())
    }

    /// `testLucene7291Dateline()`.
    fn test_lucene7291_dateline(&mut self) -> Result<()> {
        self.grid = Some(QuadPrefixTree::new(self.fx.ctx(), 2)); // only 2, and we wind up with some big leaf cells
        self.set_rpt()?;
        let sf = self.sf();
        self.adoc("0", Some(sf.rect(-102.0, -83.0, 43.0, 52.0)?))?;
        self.commit()?;
        self.validate_heatmap_result_loop(sf.rect(179.0, -179.0, 62.0, 63.0)?, 2, 100)
        // HM crosses dateline
    }

    /// `testQueryCircle()`.
    fn test_query_circle(&mut self) -> Result<()> {
        // overwrite setUp; non-geo bounds is more straight-forward; otherwise 88,88 would actually be
        // practically north,
        let mut spatial_context_factory = SpatialContextFactory::new();
        spatial_context_factory.geo = false;
        spatial_context_factory.world_bounds =
            Some(RectangleImpl::new(-90.0, 90.0, -90.0, 90.0, None));
        self.fx.ctx = Some(spatial_context_factory.new_spatial_context()?);
        self.shape_factory = Some(self.fx.ctx().get_shape_factory());
        const LEVEL: i32 = 4;
        self.grid = Some(QuadPrefixTree::new(self.fx.ctx(), LEVEL));
        self.set_rpt()?;
        let sf = self.sf();
        let circle: Arc<dyn Shape> = sf.circle(0.0, 0.0, 89.0)?;
        self.adoc("0", Some(sf.point_xy(88.0, 88.0)?))?; // top-right, inside bbox of circle but not the circle
        self.adoc("1", Some(sf.point_xy(0.0, 0.0)?))?; // clearly inside; dead center in fact
        self.commit()?;
        let heatmap = heatmap_facet_counter::calc_facets(
            &**self.rpt(),
            self.fx.index_searcher().get_top_reader_context(),
            None,
            Some(circle),
            LEVEL,
            1000,
        )?;
        // assert that only one point is found, not 2
        let mut found_one = false;
        for &count in &heatmap.counts {
            match count {
                0 => {}
                1 => {
                    Assert::assert_false(found_one); // this is the first
                    found_one = true;
                }
                _ => Assert::fail(&format!("counts should be 0 or 1: {count}")),
            }
        }
        Assert::assert_true(found_one);
        Ok(())
    }

    /// `validateHeatmapResultLoop(Rectangle inputRange, int facetLevel, int
    /// cellCountRecursThreshold)`: recursively facet & validate at higher
    /// resolutions until we've seen enough. We assume there are some
    /// non-zero cells.
    fn validate_heatmap_result_loop(
        &mut self,
        input_range: Arc<dyn Rectangle>,
        facet_level: i32,
        cell_count_recurs_threshold: i32,
    ) -> Result<()> {
        if facet_level > self.grid().get_max_levels() {
            return Ok(());
        }
        const MAX_CELLS: i32 = 10_000;
        let heatmap = heatmap_facet_counter::calc_facets(
            &**self.rpt(),
            self.fx.index_searcher().get_top_reader_context(),
            None,
            Some(input_range.clone()),
            facet_level,
            MAX_CELLS,
        )?;
        let pre_non_zero = self.cell_validated_non_zero;
        self.validate_heatmap_result(&*input_range, facet_level, &heatmap)?;
        java_assert(self.cell_validated_non_zero - pre_non_zero > 0); // we validated more non-zero cells
        if (heatmap.counts.len() as i32) < cell_count_recurs_threshold {
            self.validate_heatmap_result_loop(
                input_range,
                facet_level + 1,
                cell_count_recurs_threshold,
            )?;
        }
        Ok(())
    }

    /// `testRandom()`.
    fn test_random(&mut self) -> Result<()> {
        // Tests using random index shapes & query shapes. This has found all sorts of edge case bugs
        // (e.g. dateline,
        // cell border, overflow(?)).

        let num_indexed_shapes = 1 + RandomizedTest::at_most(9);
        let mut indexed_shapes: Vec<Option<Arc<dyn Shape>>> =
            Vec::with_capacity(num_indexed_shapes as usize);
        for _ in 0..num_indexed_shapes {
            indexed_shapes.push(Some(self.heatmap_random_indexed_shape()?));
        }

        // Main index loop:
        for i in 0..indexed_shapes.len() {
            let shape = indexed_shapes[i].clone();
            self.adoc(&i.to_string(), shape)?;

            if random().next_int_bound(10) == 0 {
                self.commit()?; // intermediate commit, produces extra segments
            }
        }
        // delete some documents randomly
        for id in 0..indexed_shapes.len() {
            if random().next_int_bound(10) == 0 {
                self.delete_doc(&id.to_string())?;
                indexed_shapes[id] = None;
            }
        }

        self.commit()?;

        // once without dateline wrap
        let rect = self.random_rectangle()?;
        let range = if LuceneTestCase::usually() {
            self.fx.ctx().get_world_bounds()
        } else {
            rect.clone()
        };
        self.query_heatmap_recursive(range, 1)?;
        // and once with dateline wrap
        if rect.get_width() > 0.0 {
            let shift = random().next_double() % rect.get_width();
            let wrapped = self.sf().rect(
                DistanceUtils::norm_lon_deg(rect.get_min_x() - shift),
                DistanceUtils::norm_lon_deg(rect.get_max_x() - shift),
                rect.get_min_y(),
                rect.get_max_y(),
            )?;
            self.query_heatmap_recursive(wrapped, 1)?;
        }
        Ok(())
    }

    /// `queryHeatmapRecursive(Rectangle inputRange, int facetLevel)`: build
    /// heatmap, validate results, then descend recursively to another facet
    /// level.
    fn query_heatmap_recursive(
        &mut self,
        input_range: Arc<dyn Rectangle>,
        facet_level: i32,
    ) -> Result<bool> {
        if !input_range.has_area() {
            // Don't test line inputs. It's not that we don't support it but it is more challenging to
            // test if per-chance it
            // coincides with a grid line due due to edge overlap issue for some grid implementations (geo
            // & quad).
            return Ok(false);
        }
        // FYI testing filtering of underlying PrefixTreeFacetCounter is done in another test
        // (`Bits filter = null`)
        // Calculate facets
        const MAX_CELLS: i32 = 10_000;
        let heatmap = heatmap_facet_counter::calc_facets(
            &**self.rpt(),
            self.fx.index_searcher().get_top_reader_context(),
            None,
            Some(input_range.clone()),
            facet_level,
            MAX_CELLS,
        )?;

        self.validate_heatmap_result(&*input_range, facet_level, &heatmap)?;

        let mut found_non_zero_count = false;
        for &count in &heatmap.counts {
            if count > 0 {
                found_non_zero_count = true;
                break;
            }
        }

        // Test again recursively to higher facetLevel (more detailed cells)
        if found_non_zero_count
            && self.cells_validated <= 500
            && facet_level != self.grid().get_max_levels()
            && input_range.has_area()
        {
            for _ in 0..5 {
                // try multiple times until we find non-zero counts
                let sub = self.random_rectangle_in(&*input_range)?;
                if self.query_heatmap_recursive(sub, facet_level + 1)? {
                    break; // we found data here so we needn't try again
                }
            }
        }
        Ok(found_non_zero_count)
    }

    /// `validateHeatmapResult(Rectangle inputRange, int facetLevel, HeatmapFacetCounter.Heatmap heatmap)`.
    fn validate_heatmap_result(
        &mut self,
        input_range: &dyn Rectangle,
        facet_level: i32,
        heatmap: &heatmap_facet_counter::Heatmap,
    ) -> Result<()> {
        let heat_rect = &heatmap.region;
        Assert::assert_true(
            heat_rect.relate(input_range.as_shape()) == SpatialRelation::Contains
                || heat_rect.equals(input_range.as_shape()),
        );
        let cell_width = heat_rect.get_width() / f64::from(heatmap.columns);
        let cell_height = heat_rect.get_height() / f64::from(heatmap.rows);
        for c in 0..heatmap.columns {
            for r in 0..heatmap.rows {
                let facet_count = heatmap.get_count(c, r);
                let x = DistanceUtils::norm_lon_deg(
                    heat_rect.get_min_x() + f64::from(c) * cell_width + cell_width / 2.0,
                );
                let y = DistanceUtils::norm_lat_deg(
                    heat_rect.get_min_y() + f64::from(r) * cell_height + cell_height / 2.0,
                );
                let pt: Arc<dyn Shape> = self.sf().point_xy(x, y)?;
                Assert::assert_equals(
                    self.count_matching_docs_at_level(pt, facet_level)?,
                    facet_count,
                );
            }
        }
        Ok(())
    }

    /// `countMatchingDocsAtLevel(Point pt, int facetLevel)`.
    fn count_matching_docs_at_level(
        &mut self,
        pt: Arc<dyn Shape>,
        facet_level: i32,
    ) -> Result<i32> {
        // we use IntersectsPrefixTreeFilter directly so that we can specify the level to go to exactly.
        let strategy = self.rpt();
        let filter: Arc<dyn Query> = Arc::new(IntersectsPrefixTreeQuery::new(
            pt,
            strategy.get_field_name(),
            Arc::clone(self.grid()),
            facet_level,
            self.grid().get_max_levels(),
        ));
        let total_hits = self.fx.index_searcher().count(filter)?;
        self.cells_validated += 1;
        if total_hits > 0 {
            self.cell_validated_non_zero += 1;
        }
        Ok(total_hits)
    }

    /// `randomIndexedShape()` (`private`; not the abstract method of
    /// `RandomSpatialOpStrategyTestCase`, which this class does not extend).
    fn heatmap_random_indexed_shape(&self) -> Result<Arc<dyn Shape>> {
        if self.rpt().is_points_only() || random().next_boolean() {
            Ok(self.random_point()?)
        } else {
            Ok(self.random_rectangle()?)
        }
    }
}
