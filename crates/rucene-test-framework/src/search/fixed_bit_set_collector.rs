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

//! Port of `org.apache.lucene.tests.search.FixedBitSetCollector`.

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::{
    CollectionResult, Collector, CollectorManager, LeafCollector, Scorable, ScoreMode,
};
use rucene::util::FixedBitSet;

/// Test utility collector that uses [`FixedBitSet`] to record hits.
///
/// Equivalent to `org.apache.lucene.tests.search.FixedBitSetCollector`, a
/// `SimpleCollector`: [`Collector::get_leaf_collector`] runs
/// `doSetNextReader` and returns the collector itself, as
/// `SimpleCollector.getLeafCollector` does.
#[derive(Debug)]
pub struct FixedBitSetCollector {
    /// `bitSet`.
    bit_set: FixedBitSet,
    /// `docBase`.
    doc_base: i32,
}

impl FixedBitSetCollector {
    /// The package-private `new FixedBitSetCollector(maxDoc)`.
    fn new(max_doc: i32) -> Self {
        FixedBitSetCollector {
            bit_set: FixedBitSet::new(max_doc as usize),
            doc_base: 0,
        }
    }

    /// `createManager(maxDoc)`: a manager that ORs the bit sets of its
    /// collectors.
    pub fn create_manager(max_doc: i32) -> FixedBitSetCollectorManager {
        FixedBitSetCollectorManager { max_doc }
    }

    /// `doSetNextReader(context)`.
    fn do_set_next_reader(&mut self, context: &LeafReaderContext) {
        self.doc_base = context.doc_base();
    }
}

impl LeafCollector for FixedBitSetCollector {
    /// `SimpleCollector.setScorer`: ignored.
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.bit_set.set((self.doc_base + doc) as usize);
        Ok(())
    }
}

impl Collector for FixedBitSetCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        self.do_set_next_reader(context);
        Ok(Box::new(self))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

/// The anonymous manager of `FixedBitSetCollector.createManager(maxDoc)`.
#[derive(Debug, Clone, Copy)]
pub struct FixedBitSetCollectorManager {
    max_doc: i32,
}

impl CollectorManager for FixedBitSetCollectorManager {
    type Collector = FixedBitSetCollector;
    type Output = FixedBitSet;

    fn new_collector(&self) -> Result<FixedBitSetCollector> {
        Ok(FixedBitSetCollector::new(self.max_doc))
    }

    fn reduce(&self, collectors: Vec<FixedBitSetCollector>) -> Result<FixedBitSet> {
        let mut reduced = FixedBitSet::new(self.max_doc as usize);
        for collector in &collectors {
            reduced.or_set(&collector.bit_set);
        }
        Ok(reduced)
    }
}
