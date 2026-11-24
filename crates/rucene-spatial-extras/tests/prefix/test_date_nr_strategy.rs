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

//! Port of `TestDateNRStrategy.java` (package `org.apache.lucene.spatial.prefix`).

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Test;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene_queryparser::jdk::calendar::{self as calendar, Calendar};
use rucene_spatial_extras::prefix::number_range_prefix_tree_strategy::NumberRangePrefixTreeStrategy;
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::date_range_prefix_tree::{
    DateRangePrefixTree, DEFAULT_CAL,
};
use rucene_spatial_extras::prefix::tree::number_range_prefix_tree::NumberRangePrefixTreeImpl;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::shape::shape::Shape;

use crate::prefix::random_spatial_op_strategy_test_case::RandomSpatialOpStrategyTestCaseOps;
use crate::spatial::spatial_test_case::{
    java_assert, random_spatial_op_strategy_test_case_class, spatial_class, spatial_set_up, ti,
    SpatialCase, SpatialFixture,
};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;
use crate::spatial_support::{cal, random};

const NAME: &str = "org.apache.lucene.spatial.prefix.TestDateNRStrategy";

/// `ITERATIONS`.
const ITERATIONS: i32 = 10;

/// `TestDateNRStrategy`.
pub fn class_def() -> Arc<Class> {
    type T = TestDateNRStrategy;
    let repeat = || Repeat {
        iterations: ITERATIONS,
        use_constant_seed: false,
    };
    spatial_class::<T>(
        NAME,
        &random_spatial_op_strategy_test_case_class(),
        Constructor::no_args(|| TestDateNRStrategy {
            fx: SpatialFixture::new(NAME),
            tree: None,
            nr: None,
            random_cal_window_ms: 0,
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
            ti::<T>("testWithinSame", T::test_within_same).annotate(Test::new()),
            ti::<T>("testWorld", T::test_world).annotate(Test::new()),
            ti::<T>(
                "testBugInitIterOptimization",
                T::test_bug_init_iter_optimization,
            )
            .annotate(Test::new()),
            ti::<T>(
                "testLastMillionYearPeriod",
                T::test_last_million_year_period,
            )
            .annotate(Test::new()),
        ],
    )
}

/// The `TestDateNRStrategy` instance.
pub struct TestDateNRStrategy {
    fx: SpatialFixture,
    /// `tree`.
    tree: Option<Arc<DateRangePrefixTree>>,
    /// `(NumberRangePrefixTreeStrategy) strategy`.
    nr: Option<Arc<NumberRangePrefixTreeStrategy>>,
    /// `randomCalWindowMs`.
    random_cal_window_ms: i64,
}

impl SpatialCase for TestDateNRStrategy {
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
        nr.set_points_only(RandomizedTest::random_int() % 5 == 0);
        let mut tmp_cal = tree.new_cal();
        let random_cal_window_field =
            RandomizedTest::random_int_between(calendar::YEAR as i32, calendar::MILLISECOND as i32);
        tmp_cal.add(random_cal_window_field as usize, 2_000)?;
        self.random_cal_window_ms = 2000i64.max(tmp_cal.get_time_in_millis()?);
        self.tree = Some(tree);
        self.nr = Some(nr);
        Ok(())
    }

    /// `randomIndexedShape()`.
    fn random_indexed_shape(&mut self) -> Result<Arc<dyn Shape>> {
        if self.nr().is_points_only() {
            let mut cal = self.tree().new_cal();
            cal.set_time_in_millis(random().next_long());
            Ok(self.tree().to_shape(&mut cal)?)
        } else {
            self.random_shape()
        }
    }

    /// `randomQueryShape()`.
    fn random_query_shape(&mut self) -> Result<Arc<dyn Shape>> {
        self.random_shape()
    }
}

impl TestDateNRStrategy {
    fn tree(&self) -> &Arc<DateRangePrefixTree> {
        self.tree.as_ref().expect("setUp() sets tree")
    }

    fn nr(&self) -> &Arc<NumberRangePrefixTreeStrategy> {
        self.nr.as_ref().expect("setUp() sets strategy")
    }

    /// `testIntersects()`.
    fn test_intersects(&mut self) -> Result<()> {
        self.test_operation_random_shapes(SpatialOperation::Intersects)
    }

    /// `testWithin()`.
    fn test_within(&mut self) -> Result<()> {
        self.test_operation_random_shapes(SpatialOperation::IsWithin)
    }

    /// `testContains()`.
    fn test_contains(&mut self) -> Result<()> {
        self.test_operation_random_shapes(SpatialOperation::Contains)
    }

    /// `testWithinSame()`.
    fn test_within_same(&mut self) -> Result<()> {
        let shape = self.random_indexed_shape()?;
        self.test_operation(Arc::clone(&shape), SpatialOperation::IsWithin, shape, true)
        // is within itself
    }

    /// `testWorld()`.
    fn test_world(&mut self) -> Result<()> {
        self.nr().set_points_only(false);
        let mut world_cal = self.tree().new_cal();
        let world: Arc<dyn Shape> = self.tree().to_shape(&mut world_cal)?; // world matches everything
        let mut random_cal = self.random_calendar()?;
        let query: Arc<dyn Shape> = self.tree().to_shape(&mut random_cal)?;
        self.test_operation(world, SpatialOperation::Contains, query, true)
    }

    /// `testBugInitIterOptimization()`.
    fn test_bug_init_iter_optimization(&mut self) -> Result<()> {
        self.nr().set_points_only(false);
        // bug due to fast path initIter() optimization
        let indexed: Arc<dyn Shape> = self
            .tree()
            .parse_shape("[2014-03-27T23 TO 2014-04-01T01]")?;
        let query: Arc<dyn Shape> = self.tree().parse_shape("[2014-04 TO 2014-04-01T02]")?;
        self.test_operation(indexed, SpatialOperation::Intersects, query, true)
    }

    /// `testLastMillionYearPeriod()`.
    fn test_last_million_year_period(&mut self) -> Result<()> {
        // a year in the last million year period (>=292M)
        let indexed: Arc<dyn Shape> = self.tree().parse_shape("+292220922-05-17T18:01:57.572")?;
        let query: Arc<dyn Shape> = self.tree().parse_shape("[1970 TO *]")?;
        self.test_operation(indexed, SpatialOperation::Intersects, query, true)
    }

    /// `randomShape()`.
    fn random_shape(&self) -> Result<Arc<dyn Shape>> {
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
