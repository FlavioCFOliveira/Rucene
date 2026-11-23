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

//! Port of `TestSerializedStrategy.java` (package
//! `org.apache.lucene.spatial.serialized`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Test;
use rucene::error::Result;
use rucene_spatial_extras::serialized::serialized_dv_strategy::SerializedDVStrategy;
use spatial4j::context::spatial_context::SpatialContext;

use crate::spatial::spatial_match_concern::SpatialMatchConcern;
use crate::spatial::spatial_test_case::{
    spatial_class, spatial_set_up, strategy_test_case_class, ti, SpatialCase, SpatialFixture,
};
use crate::spatial::strategy_test_case::{
    StrategyTestCaseOps, DATA_SIMPLE_BBOX, DATA_STATES_BBOX, DATA_WORLD_CITIES_POINTS,
    QTEST_CITIES_INTERSECTS_BBOX, QTEST_SIMPLE_QUERIES_BBOX, QTEST_STATES_INTERSECTS_BBOX,
    QTEST_STATES_IS_WITHIN_BBOX,
};

const NAME: &str = "org.apache.lucene.spatial.serialized.TestSerializedStrategy";

/// `TestSerializedStrategy`.
pub fn class_def() -> Arc<Class> {
    type T = TestSerializedStrategy;
    spatial_class::<T>(
        NAME,
        &strategy_test_case_class(),
        Constructor::no_args(|| TestSerializedStrategy {
            fx: SpatialFixture::new(NAME),
        }),
        vec![
            ti::<T>("testBasicOperaions", T::test_basic_operaions).annotate(Test::new()),
            ti::<T>("testStatesBBox", T::test_states_bbox).annotate(Test::new()),
            ti::<T>("testCitiesIntersectsBBox", T::test_cities_intersects_bbox)
                .annotate(Test::new()),
        ],
    )
}

/// The `TestSerializedStrategy` instance.
pub struct TestSerializedStrategy {
    fx: SpatialFixture,
}

impl SpatialCase for TestSerializedStrategy {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `setUp()`.
    fn set_up(&mut self) -> Result<()> {
        spatial_set_up(&mut self.fx)?;
        self.fx.ctx = Some(SpatialContext::geo());
        self.fx.strategy = Some(Arc::new(SerializedDVStrategy::new(
            self.fx.ctx(),
            "serialized",
        )?));
        Ok(())
    }
}

impl TestSerializedStrategy {
    /// `testBasicOperaions()`.
    fn test_basic_operaions(&mut self) -> Result<()> {
        self.get_add_and_verify_indexed_documents(DATA_SIMPLE_BBOX)?;

        self.execute_queries(SpatialMatchConcern::EXACT, &[QTEST_SIMPLE_QUERIES_BBOX])
    }

    /// `testStatesBBox()`.
    fn test_states_bbox(&mut self) -> Result<()> {
        self.get_add_and_verify_indexed_documents(DATA_STATES_BBOX)?;

        self.execute_queries(SpatialMatchConcern::FILTER, &[QTEST_STATES_IS_WITHIN_BBOX])?;
        self.execute_queries(SpatialMatchConcern::FILTER, &[QTEST_STATES_INTERSECTS_BBOX])
    }

    /// `testCitiesIntersectsBBox()`.
    fn test_cities_intersects_bbox(&mut self) -> Result<()> {
        self.get_add_and_verify_indexed_documents(DATA_WORLD_CITIES_POINTS)?;

        self.execute_queries(SpatialMatchConcern::FILTER, &[QTEST_CITIES_INTERSECTS_BBOX])
    }

    // sorting is tested in DistanceStrategyTest
}
