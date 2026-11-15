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

//! Port of `MinimumShouldMatchIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::matches::{MatchesIterator, MatchesUtils};
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

use crate::intervals::caching_matches_iterator::CachingMatchesIterator;
use crate::intervals::disi_priority_queue::DisiPriorityQueue;
use crate::intervals::disi_wrapper::DisiWrapper;
use crate::intervals::disjunction_disi_approximation::DisjunctionDISIApproximation;
use crate::intervals::interval_iterator::{
    IntervalIterator, SharedIntervalIterator, NO_MORE_INTERVALS,
};
use crate::intervals::interval_matches::IntervalMatches;
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::interval_query::IntervalQuery;
use crate::intervals::intervals_source::{
    same_object, sources_list_eq, sources_list_hash, IntervalsSource,
};
use crate::intervals::minimizing_conjunction_intervals_source::{
    cache_iterators, MatchCallback, NoOpMatchCallback,
};
use crate::intervals::unordered_intervals_source::StartThenEnd;

/// The intervals that span combinations of intervals from at least
/// `min_should_match` of the sources.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.MinimumShouldMatchIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct MinimumShouldMatchIntervalsSource {
    sources: Vec<Arc<dyn IntervalsSource>>,
    min_should_match: i32,
}

impl MinimumShouldMatchIntervalsSource {
    /// Equivalent to `MinimumShouldMatchIntervalsSource(IntervalsSource[],
    /// int)`.
    pub(crate) fn new(sources: Vec<Arc<dyn IntervalsSource>>, min_should_match: i32) -> Self {
        debug_assert!((min_should_match as usize) < sources.len());
        Self {
            sources,
            min_should_match,
        }
    }
}

impl IntervalsSource for MinimumShouldMatchIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let mut iterators = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            if let Some(it) = source.intervals(field, ctx)? {
                iterators.push(SharedIntervalIterator::new(it));
            }
        }
        if (iterators.len() as i32) < self.min_should_match {
            return Ok(None);
        }
        Ok(Some(Box::new(MinimumShouldMatchIntervalIterator::new(
            iterators,
            self.min_should_match,
            Box::new(NoOpMatchCallback),
        )?)))
    }

    /// **Divergence.** Java keys its lookup of caching matches by the
    /// identity of the wrapping interval iterators in an `IdentityHashMap`
    /// and builds the iterator over `lookup.keySet()`, whose order follows
    /// the JVM's identity hashes and is unreproducible; this port keeps the
    /// pairs in insertion order, and looks them up by identity.
    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let mut lookup: Vec<(SharedIntervalIterator, Rc<RefCell<CachingMatchesIterator>>)> =
            Vec::new();
        for source in &self.sources {
            if let Some(mi) = source.matches(field, ctx, doc)? {
                let cmi = Rc::new(RefCell::new(CachingMatchesIterator::new(mi)));
                let shared = SharedIntervalMatchesIterator::from_rc(
                    Rc::clone(&cmi) as Rc<RefCell<dyn IntervalMatchesIterator>>
                );
                let wrapped = SharedIntervalIterator::new(IntervalMatches::wrap_matches(
                    Box::new(shared),
                    doc,
                ));
                lookup.push((wrapped, cmi));
            }
        }
        if (lookup.len() as i32) < self.min_should_match {
            return Ok(None);
        }
        let mut it = MinimumShouldMatchIntervalIterator::new(
            lookup.iter().map(|(k, _)| k.clone()).collect(),
            self.min_should_match,
            cache_iterators(lookup.iter().map(|(_, v)| Rc::clone(v)).collect()),
        )?;
        if it.advance(doc)? != doc {
            return Ok(None);
        }
        if it.next_interval()? == NO_MORE_INTERVALS {
            return Ok(None);
        }
        Ok(Some(Box::new(MinimumMatchesIterator {
            cached: true,
            iterator: it,
            lookup,
        })))
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        let sources = self.sources.clone();
        let parent = IntervalQuery::new(field, self);
        let mut v = visitor.get_sub_visitor(Occur::SHOULD, &parent);
        for source in sources {
            source.visit(field, &mut *v);
        }
    }

    fn min_extent(&self) -> i32 {
        let mut sub_extents: Vec<i32> = self.sources.iter().map(|s| s.min_extent()).collect();
        sub_extents.sort_unstable();
        let mut min_extent = 0i32;
        for extent in sub_extents
            .iter()
            .take(self.min_should_match.max(0) as usize)
        {
            min_extent = min_extent.wrapping_add(*extent);
        }
        min_extent
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    fn to_source_string(&self) -> String {
        let parts: Vec<String> = self.sources.iter().map(|s| s.to_source_string()).collect();
        format!("AtLeast({}~{})", parts.join(","), self.min_should_match)
    }

    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<MinimumShouldMatchIntervalsSource>()
                .is_some_and(|that| {
                    self.min_should_match == that.min_should_match
                        && sources_list_eq(&self.sources, &that.sources)
                })
    }

    /// `31 * Objects.hash(minShouldMatch) + Arrays.hashCode(sources)`.
    fn source_hash(&self) -> i32 {
        let result = 31i32.wrapping_add(self.min_should_match);
        result
            .wrapping_mul(31)
            .wrapping_add(sources_list_hash(&self.sources))
    }
}

/// `lessThan(a, b)` of the background queue: `a.end() < b.end() || (a.end()
/// == b.end() && a.start() >= b.start())`.
#[derive(Debug, Clone, Copy)]
struct EndThenStart;

impl PriorityQueueComparator<SharedIntervalIterator> for EndThenStart {
    fn less_than(&self, a: &SharedIntervalIterator, b: &SharedIntervalIterator) -> bool {
        a.end() < b.end() || (a.end() == b.end() && a.start() >= b.start())
    }
}

/// A combination of an unordered conjunction and a disjunction: iterators
/// are advanced through a disjunction approximation; on a document, those
/// that have intervals go into a background queue ordered as a disjunction,
/// from which the first `minShouldMatch` move into a proximity queue ordered
/// as an unordered conjunction. Iteration then proceeds as for an unordered
/// conjunction over the proximity queue, except that an iterator popped from
/// it goes back into the background queue and is replaced by the top of that
/// queue.
///
/// Equivalent to the package-private static class
/// `org.apache.lucene.queries.intervals.MinimumShouldMatchIntervalsSource.MinimumShouldMatchIntervalIterator`.
pub(crate) struct MinimumShouldMatchIntervalIterator {
    approximation: DisjunctionDISIApproximation,
    proximity_queue: PriorityQueue<SharedIntervalIterator, StartThenEnd>,
    background_queue: PriorityQueue<SharedIntervalIterator, EndThenStart>,
    match_cost: f32,
    min_should_match: i32,
    current_iterators: Vec<SharedIntervalIterator>,
    on_match: Box<dyn MatchCallback>,
    start: i32,
    end: i32,
    queue_end: i32,
    slop: i32,
    lead: Option<SharedIntervalIterator>,
}

impl MinimumShouldMatchIntervalIterator {
    /// Equivalent to
    /// `MinimumShouldMatchIntervalIterator(Collection<IntervalIterator>, int,
    /// MatchCallback)`.
    ///
    /// # Errors
    ///
    /// The error of sizing the queues.
    pub(crate) fn new(
        subs: Vec<SharedIntervalIterator>,
        min_should_match: i32,
        on_match: Box<dyn MatchCallback>,
    ) -> Result<Self> {
        let mut disi_queue = DisiPriorityQueue::new(subs.len());
        let mut mc = 0f32;
        for it in &subs {
            disi_queue.add(DisiWrapper::new(it.clone()));
            mc += it.match_cost();
        }
        Ok(Self {
            approximation: DisjunctionDISIApproximation::new(disi_queue),
            proximity_queue: PriorityQueue::new(min_should_match.max(0) as usize, StartThenEnd)?,
            background_queue: PriorityQueue::new(subs.len(), EndThenStart)?,
            match_cost: mc,
            min_should_match,
            current_iterators: Vec::new(),
            on_match,
            start: 0,
            end: 0,
            queue_end: 0,
            slop: 0,
            lead: None,
        })
    }

    /// Equivalent to the private
    /// `MinimumShouldMatchIntervalIterator.updateRightExtreme(IntervalIterator)`.
    fn update_right_extreme(&mut self, it_end: i32) {
        if it_end > self.queue_end {
            self.queue_end = it_end;
        }
    }

    /// Moves the top of the background queue into the proximity queue, when
    /// there is one.
    fn promote_background_top(&mut self) {
        if let Some(next) = self.background_queue.pop() {
            let next_end = next.end();
            self.proximity_queue.add(next);
            self.update_right_extreme(next_end);
        }
    }

    /// The iterators that make up the current match.
    ///
    /// Equivalent to the package-private
    /// `MinimumShouldMatchIntervalIterator.getCurrentIterators()`.
    pub(crate) fn get_current_iterators(&mut self) -> &[SharedIntervalIterator] {
        self.current_iterators.clear();
        if let Some(lead) = &self.lead {
            self.current_iterators.push(lead.clone());
        }
        let end = self.end;
        for it in self.proximity_queue.iter() {
            if it.end() <= end {
                self.current_iterators.push(it.clone());
            }
        }
        &self.current_iterators
    }

    /// Equivalent to the private `MinimumShouldMatchIntervalIterator.reset()`.
    fn reset(&mut self) -> Result<()> {
        self.proximity_queue.clear();
        self.background_queue.clear();
        // First we populate the background queue
        let queue = &mut self.approximation.sub_iterators;
        for position in queue.top_list() {
            let dw = queue.wrapper_mut(position);
            if dw.intervals.next_interval()? != NO_MORE_INTERVALS {
                self.background_queue.add(dw.intervals.clone());
            }
        }
        // Then we pop the first minShouldMatch entries and add them to the
        // proximity queue
        self.queue_end = -1;
        for _ in 0..self.min_should_match {
            let Some(it) = self.background_queue.pop() else {
                break;
            };
            let it_end = it.end();
            self.proximity_queue.add(it);
            self.update_right_extreme(it_end);
        }
        self.start = -1;
        self.end = -1;
        Ok(())
    }
}

impl DocIdSetIterator for MinimumShouldMatchIntervalIterator {
    fn doc_id(&self) -> i32 {
        self.approximation.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.approximation.next_doc()?;
        self.reset()?;
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.approximation.advance(target)?;
        self.reset()?;
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.approximation.cost()
    }
}

impl IntervalIterator for MinimumShouldMatchIntervalIterator {
    fn start(&self) -> i32 {
        self.start
    }

    fn end(&self) -> i32 {
        self.end
    }

    fn gaps(&self) -> i32 {
        self.slop
    }

    fn next_interval(&mut self) -> Result<i32> {
        self.lead = None;
        let msm = self.min_should_match.max(0) as usize;
        // first, find a matching interval beyond the current start
        while self.proximity_queue.size() == msm
            && self
                .proximity_queue
                .top()
                .is_some_and(|top| top.start() == self.start)
        {
            if let Some(mut it) = self.proximity_queue.pop() {
                if it.next_interval()? != NO_MORE_INTERVALS {
                    self.background_queue.add(it);
                    let next = self.background_queue.pop();
                    debug_assert!(next.is_some()); // it's just been added!
                    if let Some(next) = next {
                        let next_end = next.end();
                        self.proximity_queue.add(next);
                        self.update_right_extreme(next_end);
                    }
                }
            }
        }
        if self.proximity_queue.size() < msm {
            self.start = NO_MORE_INTERVALS;
            self.end = NO_MORE_INTERVALS;
            return Ok(NO_MORE_INTERVALS);
        }
        // then, minimize it
        loop {
            self.on_match.on_match()?;
            self.start = self
                .proximity_queue
                .top()
                .map_or(NO_MORE_INTERVALS, |top| top.start());
            self.end = self.queue_end;
            let mut slop = self.width();
            for it in self.proximity_queue.iter() {
                slop = slop.wrapping_sub(it.width());
            }
            self.slop = slop;
            if self
                .proximity_queue
                .top()
                .is_some_and(|top| top.end() == self.end)
            {
                return Ok(self.start);
            }
            self.lead = self.proximity_queue.pop();
            if let Some(mut lead) = self.lead.clone() {
                if lead.next_interval()? != NO_MORE_INTERVALS {
                    self.background_queue.add(lead);
                }
                self.promote_background_top();
            }
            if !(self.proximity_queue.size() == msm && self.end == self.queue_end) {
                break;
            }
        }
        Ok(self.start)
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }
}

/// The matches of a [`MinimumShouldMatchIntervalsSource`].
///
/// Equivalent to the package-private static class
/// `org.apache.lucene.queries.intervals.MinimumShouldMatchIntervalsSource.MinimumMatchesIterator`.
/// Its `getQuery()` returns `null`, [`MatchesIterator::get_query`] `None`.
pub(crate) struct MinimumMatchesIterator {
    cached: bool,
    iterator: MinimumShouldMatchIntervalIterator,
    lookup: Vec<(SharedIntervalIterator, Rc<RefCell<CachingMatchesIterator>>)>,
}

impl MinimumMatchesIterator {
    /// `lookup.get(it)`: the caching matches of an interval iterator, found
    /// by identity.
    fn lookup(&self, it: &SharedIntervalIterator) -> &Rc<RefCell<CachingMatchesIterator>> {
        &self
            .lookup
            .iter()
            .find(|(k, _)| k.ptr_eq(it))
            .expect("INVARIANT: every current iterator wraps a cached matches iterator")
            .1
    }

    /// `iterator.getCurrentIterators()`, which updates the iterator's
    /// reusable list; the list is rebuilt here from the same state.
    fn current_iterators(&self) -> Vec<SharedIntervalIterator> {
        let mut current = Vec::new();
        if let Some(lead) = &self.iterator.lead {
            current.push(lead.clone());
        }
        for it in self.iterator.proximity_queue.iter() {
            if it.end() <= self.iterator.end {
                current.push(it.clone());
            }
        }
        current
    }
}

impl MatchesIterator for MinimumMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        if self.cached {
            self.cached = false;
            return Ok(true);
        }
        Ok(self.iterator.next_interval()? != NO_MORE_INTERVALS)
    }

    fn start_position(&self) -> i32 {
        self.iterator.start()
    }

    fn end_position(&self) -> i32 {
        self.iterator.end()
    }

    fn start_offset(&self) -> Result<i32> {
        let mut start = i32::MAX;
        for it in self.current_iterators() {
            let cms = self.lookup(&it);
            start = start.min(cms.borrow().start_offset()?);
        }
        Ok(start)
    }

    fn end_offset(&self) -> Result<i32> {
        let mut end = 0;
        for it in self.current_iterators() {
            let cms = self.lookup(&it);
            end = end.max(cms.borrow().end_offset()?);
        }
        Ok(end)
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        let mut mis: Vec<Box<dyn MatchesIterator>> = Vec::new();
        for it in self.current_iterators() {
            let cms = self.lookup(&it);
            let mi = cms.borrow().get_sub_matches()?;
            mis.push(match mi {
                Some(mi) => mi,
                None => Box::new(SharedIntervalMatchesIterator::from_rc(
                    Rc::clone(cms) as Rc<RefCell<dyn IntervalMatchesIterator>>
                )),
            });
        }
        MatchesUtils::disjunction(mis)
    }

    /// `getQuery()` returns `null` (`MinimumShouldMatchIntervalsSource.java:406-408`).
    fn get_query(&self) -> Option<Arc<dyn Query>> {
        None
    }
}

impl IntervalMatchesIterator for MinimumMatchesIterator {
    fn gaps(&self) -> i32 {
        self.iterator.gaps()
    }

    fn width(&self) -> i32 {
        self.iterator.width()
    }
}
