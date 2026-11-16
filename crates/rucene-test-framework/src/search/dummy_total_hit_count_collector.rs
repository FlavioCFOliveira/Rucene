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

//! Port of `org.apache.lucene.tests.search.DummyTotalHitCountCollector`.

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::{
    CollectionResult, Collector, CollectorManager, LeafCollector, Scorable, ScoreMode,
};

/// A collector that only counts the hits, without the optimisations of
/// [`TotalHitCountCollector`](rucene::search::TotalHitCountCollector).
///
/// Equivalent to `org.apache.lucene.tests.search.DummyTotalHitCountCollector`.
#[derive(Debug, Default)]
pub struct DummyTotalHitCountCollector {
    /// `totalHits`.
    total_hits: i32,
}

impl DummyTotalHitCountCollector {
    /// `new DummyTotalHitCountCollector()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getTotalHits()`: the number of hits collected so far.
    pub fn get_total_hits(&self) -> i32 {
        self.total_hits
    }

    /// `createManager()`: a manager that sums the hits of its collectors.
    pub fn create_manager() -> DummyTotalHitCountCollectorManager {
        DummyTotalHitCountCollectorManager
    }
}

/// The anonymous leaf collector of `getLeafCollector`.
struct Leaf<'a> {
    total_hits: &'a mut i32,
}

impl LeafCollector for Leaf<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        // `totalHits++`: Java's int arithmetic wraps.
        *self.total_hits = self.total_hits.wrapping_add(1);
        Ok(())
    }
}

impl Collector for DummyTotalHitCountCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(Leaf {
            total_hits: &mut self.total_hits,
        }))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

/// The anonymous manager of `DummyTotalHitCountCollector.createManager()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct DummyTotalHitCountCollectorManager;

impl CollectorManager for DummyTotalHitCountCollectorManager {
    type Collector = DummyTotalHitCountCollector;
    type Output = i32;

    fn new_collector(&self) -> Result<DummyTotalHitCountCollector> {
        Ok(DummyTotalHitCountCollector::new())
    }

    fn reduce(&self, collectors: Vec<DummyTotalHitCountCollector>) -> Result<i32> {
        let mut sum = 0i32;
        for coll in &collectors {
            sum = sum.wrapping_add(coll.total_hits);
        }
        Ok(sum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scorable the collector never reads.
    struct NoScore;

    impl Scorable for NoScore {
        fn score(&mut self) -> Result<f32> {
            Ok(0.0)
        }
    }

    /// Java's `totalHits++` wraps from `Integer.MAX_VALUE` to
    /// `Integer.MIN_VALUE` instead of failing.
    #[test]
    fn total_hits_increment_wraps_as_java_int() {
        let mut total_hits = i32::MAX;
        let mut leaf = Leaf {
            total_hits: &mut total_hits,
        };
        assert!(leaf.collect(0, &mut NoScore).is_ok());
        assert_eq!(total_hits, i32::MIN);
    }
}
