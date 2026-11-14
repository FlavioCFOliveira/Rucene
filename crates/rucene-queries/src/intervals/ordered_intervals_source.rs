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

//! Port of `OrderedIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::conjunction_interval_iterator::{
    conjunction_interval_iterator_disi, ConjunctionIntervalIterator,
};
use crate::intervals::disjunctions::Disjunctions;
use crate::intervals::interval_iterator::{
    IntervalIterator, SharedIntervalIterator, NO_MORE_INTERVALS,
};
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::intervals_source::{sources_list_eq, sources_list_hash, IntervalsSource};
use crate::intervals::minimizing_conjunction_intervals_source::{
    self, MatchCallback, MinimizingConjunctionIntervalsSource,
};
use crate::intervals::repeating_intervals_source::RepeatingIntervalsSource;

/// The minimal intervals in which every sub-source appears, in order.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.OrderedIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct OrderedIntervalsSource {
    sub_sources: Vec<Arc<dyn IntervalsSource>>,
}

impl OrderedIntervalsSource {
    /// Equivalent to the package-private static
    /// `OrderedIntervalsSource.build(List<IntervalsSource>)`.
    pub(crate) fn build(sources: Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource> {
        if sources.len() == 1 {
            return Arc::clone(&sources[0]);
        }
        let rewritten = Self::deduplicate(sources);
        if rewritten.len() == 1 {
            return Arc::clone(&rewritten[0]);
        }
        Arc::new(Self::new(rewritten))
    }

    /// Equivalent to the private static
    /// `OrderedIntervalsSource.deduplicate(List<IntervalsSource>)`: runs of
    /// equal consecutive sources become one repeating source.
    fn deduplicate(sources: Vec<Arc<dyn IntervalsSource>>) -> Vec<Arc<dyn IntervalsSource>> {
        let mut deduplicated: Vec<Arc<dyn IntervalsSource>> = Vec::new();
        let mut current: Vec<Arc<dyn IntervalsSource>> = Vec::new();
        for source in sources {
            if current.is_empty() || current[0].source_eq(&*source) {
                current.push(source);
            } else {
                deduplicated.push(RepeatingIntervalsSource::build(
                    Arc::clone(&current[0]),
                    current.len() as i32,
                ));
                current.clear();
                current.push(source);
            }
        }
        deduplicated.push(RepeatingIntervalsSource::build(
            Arc::clone(&current[0]),
            current.len() as i32,
        ));
        if deduplicated.len() == 1 {
            if let Some(repeating) = deduplicated[0]
                .as_any()
                .downcast_ref::<RepeatingIntervalsSource>()
            {
                repeating.set_name("ORDERED");
            }
        }
        deduplicated
    }

    /// Equivalent to the private `OrderedIntervalsSource(List<IntervalsSource>)`.
    fn new(sources: Vec<Arc<dyn IntervalsSource>>) -> Self {
        debug_assert!(sources.len() > 1);
        Self {
            sub_sources: sources,
        }
    }
}

impl MinimizingConjunctionIntervalsSource for OrderedIntervalsSource {
    fn sub_sources(&self) -> &[Arc<dyn IntervalsSource>] {
        &self.sub_sources
    }

    fn combine(
        &self,
        iterators: Vec<Box<dyn IntervalIterator>>,
        on_match: Box<dyn MatchCallback>,
    ) -> Result<Box<dyn IntervalIterator>> {
        Ok(Box::new(OrderedIntervalIterator::new(iterators, on_match)?))
    }
}

impl IntervalsSource for OrderedIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        minimizing_conjunction_intervals_source::intervals(self, field, ctx)
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        minimizing_conjunction_intervals_source::matches(self, field, ctx, doc)
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        minimizing_conjunction_intervals_source::visit(self, field, visitor);
    }

    fn min_extent(&self) -> i32 {
        let mut min_extent = 0i32;
        for sub_source in &self.sub_sources {
            min_extent = min_extent.wrapping_add(sub_source.min_extent());
        }
        min_extent
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Disjunctions::pull_up(&self.sub_sources, |sources| {
            Arc::new(OrderedIntervalsSource::new(sources)) as Arc<dyn IntervalsSource>
        })
    }

    /// `Objects.hashCode(subSources)`.
    fn source_hash(&self) -> i32 {
        sources_list_hash(&self.sub_sources)
    }

    /// `other instanceof OrderedIntervalsSource` with equal sub-sources.
    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<OrderedIntervalsSource>()
            .is_some_and(|s| sources_list_eq(&self.sub_sources, &s.sub_sources))
    }

    fn to_source_string(&self) -> String {
        let parts: Vec<String> = self
            .sub_sources
            .iter()
            .map(|s| s.to_source_string())
            .collect();
        format!("ORDERED({})", parts.join(","))
    }
}

/// The iterator of an [`OrderedIntervalsSource`].
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.OrderedIntervalsSource.OrderedIntervalIterator`.
pub(crate) struct OrderedIntervalIterator {
    base: ConjunctionIntervalIterator,
    start: i32,
    end: i32,
    i: usize,
    slop: i32,
    on_match: Box<dyn MatchCallback>,
}

impl OrderedIntervalIterator {
    /// Equivalent to the private `OrderedIntervalIterator(List<IntervalIterator>,
    /// MatchCallback)`.
    fn new(
        sub_intervals: Vec<Box<dyn IntervalIterator>>,
        on_match: Box<dyn MatchCallback>,
    ) -> Result<Self> {
        let sub_intervals = sub_intervals
            .into_iter()
            .map(SharedIntervalIterator::new)
            .collect();
        Ok(Self {
            base: ConjunctionIntervalIterator::new(sub_intervals)?,
            start: -1,
            end: -1,
            i: 1,
            slop: 0,
            on_match,
        })
    }

    /// Equivalent to the protected `OrderedIntervalIterator.reset()`.
    fn reset(&mut self) -> Result<()> {
        self.base.sub_iterators[0].next_interval()?;
        self.i = 1;
        self.start = -1;
        self.end = -1;
        self.slop = -1;
        Ok(())
    }
}

conjunction_interval_iterator_disi!(OrderedIntervalIterator);

impl IntervalIterator for OrderedIntervalIterator {
    fn start(&self) -> i32 {
        self.start
    }

    fn end(&self) -> i32 {
        self.end
    }

    fn next_interval(&mut self) -> Result<i32> {
        self.start = NO_MORE_INTERVALS;
        self.end = NO_MORE_INTERVALS;
        self.slop = NO_MORE_INTERVALS;
        let mut last_start = i32::MAX;
        let mut minimizing = false;
        let n = self.base.sub_iterators.len();
        let mut current_index = self.i;
        loop {
            let mut prev_end = self.base.sub_iterators[current_index - 1].end();
            loop {
                if prev_end >= last_start {
                    self.i = current_index;
                    return Ok(self.start);
                }
                if current_index == n {
                    break;
                }
                let current = &mut self.base.sub_iterators[current_index];
                if minimizing && (current.start() > prev_end) {
                    break;
                }
                loop {
                    if current.end() >= last_start {
                        self.i = current_index;
                        return Ok(self.start);
                    }
                    let current_start = current.next_interval()?;
                    if current_start == NO_MORE_INTERVALS {
                        self.i = current_index;
                        return Ok(self.start);
                    }
                    if current_start > prev_end {
                        break;
                    }
                }
                current_index += 1;
                prev_end = current.end();
            }
            let start = self.base.sub_iterators[0].start();
            self.start = start;
            if start == NO_MORE_INTERVALS {
                self.i = current_index;
                self.end = NO_MORE_INTERVALS;
                return Ok(NO_MORE_INTERVALS);
            }
            let end = self.base.sub_iterators[n - 1].end();
            self.end = end;
            let mut slop = end.wrapping_sub(start).wrapping_add(1);
            // use indexed loop since this is always a random access capable
            // list to avoid allocations in a hot nested loop
            for j in 0..n {
                slop = slop.wrapping_sub(self.base.sub_iterators[j].width());
            }
            self.slop = slop;
            self.on_match.on_match()?;
            current_index = 1;
            if self.base.sub_iterators[0].next_interval()? == NO_MORE_INTERVALS {
                self.i = current_index;
                return Ok(self.start);
            }
            last_start = self.base.sub_iterators[n - 1].start();
            minimizing = true;
        }
    }

    fn gaps(&self) -> i32 {
        self.slop
    }

    fn match_cost(&self) -> f32 {
        self.base.match_cost()
    }
}
