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

//! Port of `TestNumberRangeFacets.java` (package
//! `org.apache.lucene.spatial.prefix`).

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Test};
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene::search::{Query, TermInSetQuery};
use rucene::util::{Bits, BytesRef};
use rucene_queryparser::jdk::calendar::{self as calendar, Calendar};
use rucene_spatial_extras::prefix::number_range_prefix_tree_strategy::NumberRangePrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::date_range_prefix_tree::{
    DateRangePrefixTree, DEFAULT_CAL,
};
use rucene_spatial_extras::prefix::tree::number_range_prefix_tree::{
    as_unit_nr_shape, NRShape, NumberRangePrefixTreeImpl,
};
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_test_framework::search::FixedBitSetCollector;
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_test_case::{
    java_assert, spatial_class, spatial_set_up, strategy_test_case_class, ti, SpatialCase,
    SpatialFixture, SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;
use crate::spatial_support::{cal, random};

const NAME: &str = "org.apache.lucene.spatial.prefix.TestNumberRangeFacets";

/// `TestNumberRangeFacets`.
pub fn class_def() -> Arc<Class> {
    type T = TestNumberRangeFacets;
    spatial_class::<T>(
        NAME,
        &strategy_test_case_class(),
        Constructor::no_args(|| TestNumberRangeFacets {
            fx: SpatialFixture::new(NAME),
            tree: None,
            nr: None,
            random_cal_window_field: 0,
            random_cal_window_ms: 0,
        }),
        vec![ti::<T>("test", T::test)
            .annotate(Repeat {
                iterations: 20,
                use_constant_seed: false,
            })
            .annotate(Test::new())],
    )
}

/// The `TestNumberRangeFacets` instance.
pub struct TestNumberRangeFacets {
    fx: SpatialFixture,
    /// `tree`.
    tree: Option<Arc<DateRangePrefixTree>>,
    /// `(NumberRangePrefixTreeStrategy) strategy`.
    nr: Option<Arc<NumberRangePrefixTreeStrategy>>,
    /// `randomCalWindowField`.
    random_cal_window_field: i32,
    /// `randomCalWindowMs`.
    random_cal_window_ms: i64,
}

impl SpatialCase for TestNumberRangeFacets {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `setUp()`.
    fn set_up(&mut self) -> Result<()> {
        spatial_set_up(&mut self.fx)?;
        let tree = DateRangePrefixTree::new(&DEFAULT_CAL)?;
        let nr = NumberRangePrefixTreeStrategy::new(tree.clone(), "dateRange")?;
        self.fx.strategy = Some(nr.clone());
        let mut tmp_cal = tree.new_cal();
        self.random_cal_window_field =
            RandomizedTest::random_int_between(1, calendar::ZONE_OFFSET as i32 - 1); // we're not allowed to add zone offset
        tmp_cal.add(self.random_cal_window_field as usize, 2_000)?;
        self.random_cal_window_ms = 2000i64.max(tmp_cal.get_time_in_millis()?);
        self.tree = Some(tree);
        self.nr = Some(nr);
        Ok(())
    }
}

impl TestNumberRangeFacets {
    fn tree(&self) -> &Arc<DateRangePrefixTree> {
        self.tree.as_ref().expect("setUp() sets tree")
    }

    /// `test()`.
    fn test(&mut self) -> Result<()> {
        // generate test data
        let mut indexed_shapes: Vec<Option<Arc<dyn NRShape>>> = Vec::new();
        let num_indexed_shapes = random().next_int_bound(15);
        for _ in 0..num_indexed_shapes {
            indexed_shapes.push(Some(self.random_shape()?));
        }

        // Main index loop:
        for i in 0..indexed_shapes.len() {
            let shape: Option<Arc<dyn Shape>> =
                indexed_shapes[i].clone().map(|s| s as Arc<dyn Shape>);
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

        // Main query loop:
        for _query_idx in 0..10 {
            self.pre_query_havoc();
            // We need to have a facet range window to do the facets between (a start time & end time). We
            // randomly
            // pick a date, decide the level we want to facet on, and then pick a right end time that is
            // up to 2 thousand
            // values later.
            let mut cal_field_facet = self.random_cal_window_field - 1;
            if cal_field_facet > 1 && LuceneTestCase::rarely() {
                cal_field_facet -= 1;
            }
            let mut left_cal = self.random_calendar()?;
            left_cal.add(
                cal_field_facet as usize,
                -RandomizedTest::random_int_between(0, 1000),
            )?;
            let mut right_cal = left_cal.clone();
            right_cal.add(
                cal_field_facet as usize,
                RandomizedTest::random_int_between(0, 2000),
            )?;
            // Pick facet detail level based on cal field.
            let mut detail_level = self
                .tree()
                .get_tree_level_for_calendar_field(cal_field_facet)?;
            if detail_level < 0 {
                // no exact match
                detail_level = -detail_level;
            }

            // Randomly pick a filter/acceptDocs
            let mut top_accept_docs: Option<Box<dyn Bits>> = None;
            let mut accept_field_ids: Vec<i32> = Vec::new();
            if LuceneTestCase::usually() {
                // get all possible IDs into a list, random shuffle it, then randomly choose how many of the
                // first we use to
                // replace the list.
                for (i, s) in indexed_shapes.iter().enumerate() {
                    if s.is_none() {
                        // we deleted this one
                        continue;
                    }
                    accept_field_ids.push(i as i32);
                }
                randomizedtesting::jdk::util::shuffle(&mut accept_field_ids, &*random());
                let n = RandomizedTest::random_int_between(0, accept_field_ids.len() as i32);
                accept_field_ids.truncate(n as usize);
                if !accept_field_ids.is_empty() {
                    let mut terms: Vec<BytesRef> = Vec::new();
                    for accept_doc_id in &accept_field_ids {
                        terms.push(BytesRef::from_char_sequence(&accept_doc_id.to_string()));
                    }

                    top_accept_docs = Some(
                        self.search_for_doc_bits(Arc::new(TermInSetQuery::new("id", terms)?))?,
                    );
                }
            }

            // Lets do it!
            let left_shape = self.tree().to_shape(&mut left_cal)?;
            let right_shape = self.tree().to_shape(&mut right_cal)?;
            let facet_range: Arc<dyn NRShape> =
                self.tree().to_range_shape(left_shape, right_shape)?;
            let facet_range_shape: Arc<dyn Shape> = facet_range.clone();
            let mut facets = self.nr.as_ref().expect("strategy").calc_nr_facets(
                self.fx.index_searcher().get_top_reader_context(),
                top_accept_docs.as_deref(),
                &facet_range_shape,
                detail_level,
            )?;

            // System.out.println("Q: " + queryIdx + " " + facets);

            // Verify results. We do it by looping over indexed shapes and reducing the facet counts.
            let facet_shape_rounded = facet_range.round_to_level(detail_level)?;
            for (indexed_shape_id, indexed_shape) in indexed_shapes.iter().enumerate() {
                if top_accept_docs.is_some()
                    && !accept_field_ids.contains(&(indexed_shape_id as i32))
                {
                    continue; // this doc was filtered out via acceptDocs
                }
                let Some(indexed_shape) = indexed_shape else {
                    // was deleted
                    continue;
                };
                let indexed_shape_rounded = indexed_shape.round_to_level(detail_level)?;
                if !indexed_shape_rounded
                    .relate(facet_shape_rounded.as_shape())
                    .intersects()
                {
                    // no intersection at all
                    continue;
                }
                // walk the cells
                let mut cell_iterator = self
                    .tree()
                    .get_tree_cell_iterator(indexed_shape.clone(), detail_level)?;
                while cell_iterator.has_next() {
                    let cell = cell_iterator.next();
                    if !cell
                        .get_shape()
                        .relate(facet_shape_rounded.as_shape())
                        .intersects()
                    {
                        cell_iterator.remove(); // no intersection; prune
                        continue;
                    }
                    java_assert(cell.get_level() <= detail_level);

                    if cell.get_level() == detail_level {
                        // count it
                        let shape = as_unit_nr_shape(&*cell.get_shape()).expect("a UnitNRShape");
                        let parent_shape = shape.get_shape_at_level(detail_level - 1); // get parent
                        let facet_parent_val = facets.get_parent_mut(&*parent_shape);
                        Assert::assert_true(facet_parent_val.is_some()); // assertNotNull
                        let facet_parent_val = facet_parent_val.expect("not null");
                        let index = shape.get_val_at_level(shape.get_unit_level());
                        Assert::assert_true(facet_parent_val.child_counts.is_some()); // assertNotNull
                        let child_counts =
                            facet_parent_val.child_counts.as_mut().expect("not null");
                        java_assert(child_counts[index as usize] > 0);
                        child_counts[index as usize] -= 1;
                    } else if cell.is_leaf() {
                        // count it, and remove/prune.
                        if cell.get_level() < detail_level - 1 {
                            java_assert(facets.top_leaves > 0);
                            facets.top_leaves -= 1;
                        } else {
                            let shape =
                                as_unit_nr_shape(&*cell.get_shape()).expect("a UnitNRShape");
                            let parent_shape = shape.get_shape_at_level(detail_level - 1); // get parent
                            let facet_parent_val = facets.get_parent_mut(&*parent_shape);
                            Assert::assert_true(facet_parent_val.is_some()); // assertNotNull
                            let facet_parent_val = facet_parent_val.expect("not null");
                            java_assert(facet_parent_val.parent_leaves > 0);
                            facet_parent_val.parent_leaves -= 1;
                        }

                        cell_iterator.remove();
                    }
                }
            }
            // At this point; all counts should be down to zero.
            Assert::assert_true(facets.top_leaves == 0);
            for (_, facet_parent_val) in &facets.parents {
                Assert::assert_true(facet_parent_val.parent_leaves == 0);
                if let Some(child_counts) = &facet_parent_val.child_counts {
                    for &child_count in child_counts {
                        Assert::assert_true(child_count == 0);
                    }
                }
            }
        }
        Ok(())
    }

    /// `searchForDocBits(Query query)`.
    fn search_for_doc_bits(&self, query: Arc<dyn Query>) -> Result<Box<dyn Bits>> {
        let searcher = self.fx.index_searcher();
        let bits = searcher.search_with_manager(
            query,
            &FixedBitSetCollector::create_manager(searcher.get_index_reader().max_doc()),
        )?;
        Ok(Box::new(bits))
    }

    /// `preQueryHavoc()` (`private`).
    fn pre_query_havoc(&mut self) {
        if let Some(rpts) = self.fx.strategy().as_recursive_prefix_tree_strategy() {
            let scan_level =
                RandomizedTest::random_int_between(0, rpts.get_grid().get_max_levels());
            rpts.set_prefix_grid_scan_level(scan_level);
        }
    }

    /// `randomShape()` (`protected`).
    fn random_shape(&self) -> Result<Arc<dyn NRShape>> {
        let mut cal1 = self.random_calendar()?;
        let s1 = self.tree().to_shape(&mut cal1)?;
        if LuceneTestCase::rarely() {
            return Ok(s1);
        }
        let mut cal2 = self.random_calendar()?;
        let s2 = self.tree().to_shape(&mut cal2)?;
        let range = if cal::compare_to(&cal1, &cal2) < 0 {
            self.tree().to_range_shape(Arc::clone(&s1), s2)
        } else {
            self.tree().to_range_shape(s2, Arc::clone(&s1))
        };
        match range {
            Ok(r) => Ok(r),
            Err(e) if e.is_illegal_argument_exception() => {
                java_assert(e.to_string().contains("Differing precision"));
                Ok(s1)
            }
            Err(e) => Err(e),
        }
    }

    /// `randomCalendar()`.
    fn random_calendar(&self) -> Result<Calendar> {
        let mut cal = self.tree().new_cal();
        cal.set_time_in_millis(random().next_long() % self.random_cal_window_ms);
        let field = random().next_int_bound(calendar::FIELD_COUNT as i32 + 1) - 1;
        cal::clear_fields_after_catching_underflow(self.tree(), &mut cal, field);
        Ok(cal)
    }
}
