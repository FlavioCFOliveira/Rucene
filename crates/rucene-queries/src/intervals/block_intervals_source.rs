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

//! Port of `BlockIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::conjunction_interval_iterator::{
    conjunction_interval_iterator_disi, ConjunctionIntervalIterator,
};
use crate::intervals::conjunction_intervals_source::{self, ConjunctionIntervalsSource};
use crate::intervals::disjunctions::Disjunctions;
use crate::intervals::interval_iterator::{
    IntervalIterator, SharedIntervalIterator, NO_MORE_INTERVALS,
};
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::intervals::Intervals;
use crate::intervals::intervals_source::{sources_list_eq, sources_list_hash, IntervalsSource};

/// A phrase: the intervals in which every sub-source appears, in order and
/// with no gap.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.BlockIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct BlockIntervalsSource {
    sub_sources: Vec<Arc<dyn IntervalsSource>>,
}

impl BlockIntervalsSource {
    /// Equivalent to the package-private static
    /// `BlockIntervalsSource.build(List<IntervalsSource>)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub(crate) fn build(
        sub_sources: Vec<Arc<dyn IntervalsSource>>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        if sub_sources.len() == 1 {
            return Ok(Arc::clone(&sub_sources[0]));
        }
        Intervals::or(Disjunctions::pull_up(&sub_sources, |sources| {
            Arc::new(BlockIntervalsSource::new(sources)) as Arc<dyn IntervalsSource>
        })?)
    }

    /// Equivalent to the private static
    /// `BlockIntervalsSource.flatten(List<IntervalsSource>)`: block sources
    /// can be flattened because they do not increase the gap (gap = 0).
    fn flatten(sources: Vec<Arc<dyn IntervalsSource>>) -> Vec<Arc<dyn IntervalsSource>> {
        let mut flattened = Vec::with_capacity(sources.len());
        for s in sources {
            if let Some(block) = s.as_any().downcast_ref::<BlockIntervalsSource>() {
                flattened.extend(block.sub_sources.iter().cloned());
            } else {
                flattened.push(s);
            }
        }
        flattened
    }

    /// Equivalent to the private `BlockIntervalsSource(List<IntervalsSource>)`.
    fn new(sources: Vec<Arc<dyn IntervalsSource>>) -> Self {
        let sub_sources = Self::flatten(sources);
        debug_assert!(sub_sources.len() > 1);
        Self { sub_sources }
    }
}

impl ConjunctionIntervalsSource for BlockIntervalsSource {
    fn sub_sources(&self) -> &[Arc<dyn IntervalsSource>] {
        &self.sub_sources
    }

    fn combine(
        &self,
        iterators: Vec<Box<dyn IntervalIterator>>,
    ) -> Result<Box<dyn IntervalIterator>> {
        Ok(Box::new(BlockIntervalIterator::new(iterators)?))
    }
}

impl IntervalsSource for BlockIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        conjunction_intervals_source::intervals(self, field, ctx)
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        conjunction_intervals_source::matches(self, field, ctx, doc)
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        conjunction_intervals_source::visit(self, field, visitor);
    }

    fn min_extent(&self) -> i32 {
        let mut min_extent = 0i32;
        for sub_source in &self.sub_sources {
            min_extent = min_extent.wrapping_add(sub_source.min_extent());
        }
        min_extent
    }

    /// Disjunctions already pulled up in `build()`.
    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    /// `Objects.hash(subSources)`.
    fn source_hash(&self) -> i32 {
        31i32.wrapping_add(sources_list_hash(&self.sub_sources))
    }

    /// `other instanceof BlockIntervalsSource` with equal sub-sources.
    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<BlockIntervalsSource>()
            .is_some_and(|b| sources_list_eq(&self.sub_sources, &b.sub_sources))
    }

    fn to_source_string(&self) -> String {
        let parts: Vec<String> = self
            .sub_sources
            .iter()
            .map(|s| s.to_source_string())
            .collect();
        format!("BLOCK({})", parts.join(","))
    }
}

/// The iterator of a [`BlockIntervalsSource`].
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.BlockIntervalsSource.BlockIntervalIterator`.
pub(crate) struct BlockIntervalIterator {
    base: ConjunctionIntervalIterator,
    start: i32,
    end: i32,
}

impl BlockIntervalIterator {
    /// Equivalent to `BlockIntervalIterator(List<IntervalIterator>)`.
    fn new(sub_iterators: Vec<Box<dyn IntervalIterator>>) -> Result<Self> {
        let sub_iterators = sub_iterators
            .into_iter()
            .map(SharedIntervalIterator::new)
            .collect();
        Ok(Self {
            base: ConjunctionIntervalIterator::new(sub_iterators)?,
            start: -1,
            end: -1,
        })
    }

    /// Equivalent to the protected `BlockIntervalIterator.reset()`.
    ///
    /// Equivalent to the abstract `ConjunctionIntervalIterator.reset()` this subclass
    /// implements, which `conjunction_interval_iterator_disi!` calls on each document.
    fn reset(&mut self) -> Result<()> {
        self.start = -1;
        self.end = -1;
        Ok(())
    }

    /// `start = end = NO_MORE_INTERVALS`, returned.
    fn exhaust(&mut self) -> i32 {
        self.start = NO_MORE_INTERVALS;
        self.end = NO_MORE_INTERVALS;
        NO_MORE_INTERVALS
    }
}

conjunction_interval_iterator_disi!(BlockIntervalIterator);

impl IntervalIterator for BlockIntervalIterator {
    fn start(&self) -> i32 {
        self.start
    }

    fn end(&self) -> i32 {
        self.end
    }

    fn gaps(&self) -> i32 {
        0
    }

    fn next_interval(&mut self) -> Result<i32> {
        if self.base.sub_iterators[0].next_interval()? == NO_MORE_INTERVALS {
            return Ok(self.exhaust());
        }
        let mut i = 1;
        while i < self.base.sub_iterators.len() {
            while self.base.sub_iterators[i].start() <= self.base.sub_iterators[i - 1].end() {
                if self.base.sub_iterators[i].next_interval()? == NO_MORE_INTERVALS {
                    return Ok(self.exhaust());
                }
            }
            if self.base.sub_iterators[i].start()
                == self.base.sub_iterators[i - 1].end().wrapping_add(1)
            {
                i += 1;
            } else {
                if self.base.sub_iterators[0].next_interval()? == NO_MORE_INTERVALS {
                    return Ok(self.exhaust());
                }
                i = 1;
            }
        }
        self.start = self.base.sub_iterators[0].start();
        self.end = self.base.sub_iterators[self.base.sub_iterators.len() - 1].end();
        Ok(self.start)
    }

    fn match_cost(&self) -> f32 {
        self.base.match_cost()
    }
}
