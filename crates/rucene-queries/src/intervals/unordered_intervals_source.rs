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

//! Port of `UnorderedIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

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

/// The minimal intervals in which every sub-source appears, in any order;
/// the sub-sources may overlap.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.UnorderedIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct UnorderedIntervalsSource {
    sub_sources: Vec<Arc<dyn IntervalsSource>>,
}

impl UnorderedIntervalsSource {
    /// Equivalent to the package-private static
    /// `UnorderedIntervalsSource.build(List<IntervalsSource>)`.
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
    /// `UnorderedIntervalsSource.deduplicate(List<IntervalsSource>)`: equal
    /// sources, counted in a `LinkedHashMap` (which preserves the order of
    /// first occurrence), become one repeating source each.
    fn deduplicate(sources: Vec<Arc<dyn IntervalsSource>>) -> Vec<Arc<dyn IntervalsSource>> {
        // preserve order for testing
        let mut counts: Vec<(Arc<dyn IntervalsSource>, i32)> = Vec::new();
        for source in sources {
            match counts.iter_mut().find(|(k, _)| k.source_eq(&*source)) {
                Some((_, v)) => *v += 1,
                None => counts.push((source, 1)),
            }
        }
        let mut deduplicated: Vec<Arc<dyn IntervalsSource>> = Vec::with_capacity(counts.len());
        for (source, count) in counts {
            deduplicated.push(RepeatingIntervalsSource::build(source, count));
        }
        if deduplicated.len() == 1 {
            if let Some(repeating) = deduplicated[0]
                .as_any()
                .downcast_ref::<RepeatingIntervalsSource>()
            {
                repeating.set_name("UNORDERED");
            }
        }
        deduplicated
    }

    /// Equivalent to the private
    /// `UnorderedIntervalsSource(List<IntervalsSource>)`.
    fn new(sources: Vec<Arc<dyn IntervalsSource>>) -> Self {
        debug_assert!(sources.len() > 1);
        Self {
            sub_sources: sources,
        }
    }
}

impl MinimizingConjunctionIntervalsSource for UnorderedIntervalsSource {
    fn sub_sources(&self) -> &[Arc<dyn IntervalsSource>] {
        &self.sub_sources
    }

    fn combine(
        &self,
        iterators: Vec<Box<dyn IntervalIterator>>,
        on_match: Box<dyn MatchCallback>,
    ) -> Result<Box<dyn IntervalIterator>> {
        Ok(Box::new(UnorderedIntervalIterator::new(
            iterators, on_match,
        )?))
    }
}

impl IntervalsSource for UnorderedIntervalsSource {
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
            Arc::new(UnorderedIntervalsSource::new(sources)) as Arc<dyn IntervalsSource>
        })
    }

    /// `Objects.hash(this.subSources)`.
    fn source_hash(&self) -> i32 {
        31i32.wrapping_add(sources_list_hash(&self.sub_sources))
    }

    /// `other instanceof UnorderedIntervalsSource` with equal sub-sources.
    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<UnorderedIntervalsSource>()
            .is_some_and(|o| sources_list_eq(&self.sub_sources, &o.sub_sources))
    }

    fn to_source_string(&self) -> String {
        let parts: Vec<String> = self
            .sub_sources
            .iter()
            .map(|s| s.to_source_string())
            .collect();
        format!("UNORDERED({})", parts.join(","))
    }
}

/// `lessThan(a, b)`: `a.start() < b.start() || (a.start() == b.start() &&
/// a.end() >= b.end())`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StartThenEnd;

impl PriorityQueueComparator<SharedIntervalIterator> for StartThenEnd {
    fn less_than(&self, a: &SharedIntervalIterator, b: &SharedIntervalIterator) -> bool {
        a.start() < b.start() || (a.start() == b.start() && a.end() >= b.end())
    }
}

/// The iterator of an [`UnorderedIntervalsSource`].
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.UnorderedIntervalsSource.UnorderedIntervalIterator`;
/// its `subIterators` array is the base's list of the same handles.
pub(crate) struct UnorderedIntervalIterator {
    base: ConjunctionIntervalIterator,
    queue: PriorityQueue<SharedIntervalIterator, StartThenEnd>,
    on_match: Box<dyn MatchCallback>,
    start: i32,
    end: i32,
    slop: i32,
    queue_end: i32,
}

impl UnorderedIntervalIterator {
    /// Equivalent to `UnorderedIntervalIterator(List<IntervalIterator>,
    /// MatchCallback)`.
    fn new(
        sub_iterators: Vec<Box<dyn IntervalIterator>>,
        on_match: Box<dyn MatchCallback>,
    ) -> Result<Self> {
        let sub_iterators: Vec<SharedIntervalIterator> = sub_iterators
            .into_iter()
            .map(SharedIntervalIterator::new)
            .collect();
        let queue = PriorityQueue::new(sub_iterators.len(), StartThenEnd)?;
        Ok(Self {
            base: ConjunctionIntervalIterator::new(sub_iterators)?,
            queue,
            on_match,
            start: -1,
            end: -1,
            slop: 0,
            queue_end: 0,
        })
    }

    /// Equivalent to the package-private
    /// `UnorderedIntervalIterator.updateRightExtreme(IntervalIterator)`.
    fn update_right_extreme(&mut self, it_end: i32) {
        if it_end > self.queue_end {
            self.queue_end = it_end;
        }
    }

    /// Pops the top iterator, advances it, and puts it back if it has another
    /// interval.
    fn advance_top(&mut self) -> Result<()> {
        if let Some(mut it) = self.queue.pop() {
            if it.next_interval()? != NO_MORE_INTERVALS {
                let it_end = it.end();
                self.queue.add(it);
                self.update_right_extreme(it_end);
            }
        }
        Ok(())
    }

    /// Equivalent to the protected `UnorderedIntervalIterator.reset()`.
    fn reset(&mut self) -> Result<()> {
        self.queue_end = -1;
        self.start = -1;
        self.end = -1;
        self.queue.clear();
        for i in 0..self.base.sub_iterators.len() {
            let mut it = self.base.sub_iterators[i].clone();
            if it.next_interval()? == NO_MORE_INTERVALS {
                break;
            }
            let it_end = it.end();
            self.queue.add(it);
            self.update_right_extreme(it_end);
        }
        Ok(())
    }
}

conjunction_interval_iterator_disi!(UnorderedIntervalIterator);

impl IntervalIterator for UnorderedIntervalIterator {
    fn start(&self) -> i32 {
        self.start
    }

    fn end(&self) -> i32 {
        self.end
    }

    fn next_interval(&mut self) -> Result<i32> {
        let length = self.base.sub_iterators.len();
        // first, find a matching interval
        while self.queue.size() == length
            && self
                .queue
                .top()
                .is_some_and(|top| top.start() == self.start)
        {
            self.advance_top()?;
        }
        if self.queue.size() < length {
            self.start = NO_MORE_INTERVALS;
            self.end = NO_MORE_INTERVALS;
            return Ok(NO_MORE_INTERVALS);
        }
        // then, minimize it
        loop {
            self.start = self
                .queue
                .top()
                .map_or(NO_MORE_INTERVALS, |top| top.start());
            self.end = self.queue_end;
            let mut slop = self.width();
            for it in &self.base.sub_iterators {
                slop = slop.wrapping_sub(it.width());
            }
            self.slop = slop;
            self.on_match.on_match()?;
            if self.queue.top().is_some_and(|top| top.end() == self.end) {
                return Ok(self.start);
            }
            self.advance_top()?;
            if !(self.queue.size() == length && self.end == self.queue_end) {
                break;
            }
        }
        Ok(self.start)
    }

    fn gaps(&self) -> i32 {
        self.slop
    }

    fn match_cost(&self) -> f32 {
        self.base.match_cost()
    }
}
