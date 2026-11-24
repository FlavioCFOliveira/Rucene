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

//! Port of `RandomSpatialOpStrategyTestCase.java` (package
//! `org.apache.lucene.spatial.prefix`): base test harness, ideally for
//! SpatialStrategy impls that have exact results (not grid approximated),
//! hence "not fuzzy". Its abstract methods (`randomIndexedShape`,
//! `randomQueryShape`) are on [`SpatialCase`]; the others are
//! [`RandomSpatialOpStrategyTestCaseOps`].

use std::sync::Arc;

use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_test_case::{SpatialCase, SpatialTestCaseOps};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;
use crate::spatial_support::{random, LinkedHashSet};

/// The methods of `RandomSpatialOpStrategyTestCase` no subclass overrides.
pub trait RandomSpatialOpStrategyTestCaseOps: SpatialCase {
    // Note: this is partially redundant with StrategyTestCase.runTestQuery & testOperation

    /// `testOperationRandomShapes(SpatialOperation operation)`.
    ///
    /// # Errors
    ///
    /// The errors of the shapes made, of indexing and of searching.
    fn test_operation_random_shapes(&mut self, operation: SpatialOperation) -> Result<()> {
        let num_indexed_shapes = RandomizedTest::random_int_between(1, 6);
        let mut indexed_shapes: Vec<Option<Arc<dyn Shape>>> =
            Vec::with_capacity(num_indexed_shapes as usize);
        for _ in 0..num_indexed_shapes {
            indexed_shapes.push(Some(self.random_indexed_shape()?));
        }

        let num_query_shapes = LuceneTestCase::at_least(20);
        let mut query_shapes: Vec<Arc<dyn Shape>> = Vec::with_capacity(num_query_shapes as usize);
        for _ in 0..num_query_shapes {
            query_shapes.push(self.random_query_shape()?);
        }

        self.test_operation_shapes(operation, indexed_shapes, query_shapes, true /*havoc*/)
    }

    /// `testOperation(SpatialOperation operation, List<Shape> indexedShapes,
    /// List<Shape> queryShapes, boolean havoc)`.
    ///
    /// # Errors
    ///
    /// The errors of indexing and of searching.
    fn test_operation_shapes(
        &mut self,
        operation: SpatialOperation,
        mut indexed_shapes: Vec<Option<Arc<dyn Shape>>>,
        query_shapes: Vec<Arc<dyn Shape>>,
        havoc: bool,
    ) -> Result<()> {
        // first show that when there's no data, a query will result in no results
        {
            let shape = self.random_query_shape()?;
            let query = self
                .fx()
                .strategy()
                .make_query(&SpatialArgs::new(operation, shape))?;
            let search_results = self.execute_query(query, 1)?;
            Assert::assert_equals(0i64, search_results.num_found);
        }

        // Main index loop:
        for i in 0..indexed_shapes.len() {
            let shape = indexed_shapes[i].clone();
            self.adoc(&i.to_string(), shape)?;

            if havoc && random().next_int_bound(10) == 0 {
                self.commit()?; // intermediate commit, produces extra segments
            }
        }
        if havoc {
            // delete some documents randomly
            for id in 0..indexed_shapes.len() {
                if random().next_int_bound(10) == 0 {
                    self.delete_doc(&id.to_string())?;
                    indexed_shapes[id] = None;
                }
            }
        }

        self.commit()?;

        // Main query loop:
        for (query_idx, query_shape) in query_shapes.iter().enumerate() {
            if havoc {
                self.pre_query_havoc();
            }

            // Generate truth via brute force:
            // We ensure true-positive matches (if the predicate on the raw shapes match
            //  then the search should find those same matches).
            let mut expected_ids = LinkedHashSet::new(); // true-positives
            for (id, indexed_shape) in indexed_shapes.iter().enumerate() {
                let Some(indexed_shape) = indexed_shape else {
                    continue;
                };
                if operation.evaluate(&**indexed_shape, &**query_shape) {
                    expected_ids.add(id.to_string());
                }
            }

            // Search and verify results
            let args = SpatialArgs::new(operation, Arc::clone(query_shape));
            let query = self.fx().strategy().make_query(&args)?;
            let got = self.execute_query(query, 100)?;
            let mut remaining_expected_ids = expected_ids.clone();
            for result in &got.results {
                let id = result.get_id().unwrap_or_else(|| "null".to_string());
                if !remaining_expected_ids.remove(&id) {
                    fail(
                        &format!("qIdx:{query_idx} Shouldn't match"),
                        &id,
                        &indexed_shapes,
                        &**query_shape,
                        operation,
                    );
                }
            }
            if let Some(id) = remaining_expected_ids.first() {
                fail(
                    &format!("qIdx:{query_idx} Should have matched"),
                    id,
                    &indexed_shapes,
                    &**query_shape,
                    operation,
                );
            }
        }
        Ok(())
    }

    /// `preQueryHavoc()`.
    fn pre_query_havoc(&mut self) {
        if let Some(rpts) = self.fx().strategy().as_recursive_prefix_tree_strategy() {
            let scan_level =
                RandomizedTest::random_int_between(0, rpts.get_grid().get_max_levels());
            rpts.set_prefix_grid_scan_level(scan_level);
        }
    }
}

impl<T: SpatialCase> RandomSpatialOpStrategyTestCaseOps for T {}

/// `fail(String label, String id, List<Shape> indexedShapes, Shape
/// queryShape, SpatialOperation operation)` (`private`).
fn fail(
    label: &str,
    id: &str,
    indexed_shapes: &[Option<Arc<dyn Shape>>],
    query_shape: &dyn Shape,
    operation: SpatialOperation,
) -> ! {
    let indexed = indexed_shapes[id.parse::<usize>().expect("an id is an index")]
        .as_ref()
        .map_or_else(|| "null".to_string(), |s| s.to_string());
    Assert::fail(&format!(
        "[{operation}] {label} I#{id}:{indexed} Q:{query_shape}"
    ))
}
