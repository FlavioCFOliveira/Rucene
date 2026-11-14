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

//! Port of `DisjunctionIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::java_collections::{java_compare_to, JavaObjectHashSet};

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
use crate::intervals::intervals_source::{IntervalsSource, IntervalsSourceKey};

/// The disjunction of a set of sources.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.DisjunctionIntervalsSource`. Its
/// sub-sources are a `java.util.HashSet`, kept here as a
/// [`JavaObjectHashSet`], so that they are iterated, compared and hashed as
/// Java does.
#[derive(Debug)]
pub(crate) struct DisjunctionIntervalsSource {
    /// The package-private `subSources` field.
    sub_sources: JavaObjectHashSet<IntervalsSourceKey>,
    /// The package-private `pullUpDisjunctions` field.
    pull_up_disjunctions: bool,
}

impl DisjunctionIntervalsSource {
    /// Builds the disjunction of `sub_sources`, or the single source it
    /// simplifies to.
    ///
    /// Equivalent to the package-private static
    /// `DisjunctionIntervalsSource.create(Collection<IntervalsSource>,
    /// boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates the error of a sub-disjunction's `pullUpDisjunctions`.
    pub(crate) fn create(
        sub_sources: Vec<Arc<dyn IntervalsSource>>,
        pull_up_disjunctions: bool,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let simplified = Self::simplify(sub_sources)?;
        if simplified.len() == 1 {
            let only = simplified
                .into_elements()
                .next()
                .expect("INVARIANT: the set holds one element");
            return Ok(only.0);
        }
        let sub_sources: Vec<Arc<dyn IntervalsSource>> =
            simplified.into_elements().map(|k| k.0).collect();
        Ok(Arc::new(Self::new(sub_sources, pull_up_disjunctions)?))
    }

    /// Equivalent to the private `DisjunctionIntervalsSource(Collection,
    /// boolean)`, which simplifies the sources a second time.
    fn new(sub_sources: Vec<Arc<dyn IntervalsSource>>, pull_up_disjunctions: bool) -> Result<Self> {
        Ok(Self {
            sub_sources: Self::simplify(sub_sources)?,
            pull_up_disjunctions,
        })
    }

    /// Equivalent to the private static
    /// `DisjunctionIntervalsSource.simplify(Collection<IntervalsSource>)`: a
    /// new `HashSet` of the sources, sub-disjunctions replaced by what they
    /// pull up.
    fn simplify(
        sources: Vec<Arc<dyn IntervalsSource>>,
    ) -> Result<JavaObjectHashSet<IntervalsSourceKey>> {
        let mut simplified = JavaObjectHashSet::new();
        for source in sources {
            if source.as_any().is::<DisjunctionIntervalsSource>() {
                for pulled in source.pull_up_disjunctions()? {
                    simplified.insert(IntervalsSourceKey(pulled));
                }
            } else {
                simplified.insert(IntervalsSourceKey(source));
            }
        }
        Ok(simplified)
    }

    /// The sub-sources, in the set's iteration order.
    fn sources(&self) -> impl Iterator<Item = &Arc<dyn IntervalsSource>> + '_ {
        self.sub_sources.iter().map(|k| &k.0)
    }
}

impl IntervalsSource for DisjunctionIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let mut sub_iterators: Vec<Box<dyn IntervalIterator>> = Vec::new();
        for sub_source in self.sources() {
            if let Some(it) = sub_source.intervals(field, ctx)? {
                sub_iterators.push(it);
            }
        }
        if sub_iterators.is_empty() {
            return Ok(None);
        }
        Ok(Some(Box::new(DisjunctionIntervalIterator::new(
            sub_iterators,
        )?)))
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let mut sub_matches: Vec<SharedIntervalMatchesIterator> = Vec::new();
        for sub_source in self.sources() {
            if let Some(mi) = sub_source.matches(field, ctx, doc)? {
                sub_matches.push(SharedIntervalMatchesIterator::new(mi));
            }
        }
        if sub_matches.is_empty() {
            return Ok(None);
        }
        let mut it = DisjunctionIntervalIterator::new(
            sub_matches
                .iter()
                .map(|m| IntervalMatches::wrap_matches(Box::new(m.clone()), doc))
                .collect(),
        )?;
        if it.advance(doc)? != doc {
            return Ok(None);
        }
        Ok(Some(Box::new(DisjunctionMatchesIterator {
            it,
            subs: sub_matches,
        })))
    }

    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<DisjunctionIntervalsSource>()
            .is_some_and(|that| self.sub_sources == that.sub_sources)
    }

    /// `Objects.hash(subSources)`: `31 + subSources.hashCode()`.
    fn source_hash(&self) -> i32 {
        31i32.wrapping_add(self.sub_sources.java_hash_code())
    }

    /// The sorted renderings of the sub-sources, joined as `or(a,b,...)`.
    fn to_source_string(&self) -> String {
        let mut strings: Vec<String> = self.sources().map(|s| s.to_source_string()).collect();
        strings.sort_by(|a, b| java_compare_to(a, b));
        format!("or({})", strings.join(","))
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        let sources: Vec<Arc<dyn IntervalsSource>> = self.sources().cloned().collect();
        let parent = IntervalQuery::new(field, self);
        let mut v = visitor.get_sub_visitor(Occur::SHOULD, &parent);
        for source in sources {
            source.visit(field, &mut *v);
        }
    }

    fn min_extent(&self) -> i32 {
        let mut min_extent = i32::MAX;
        for sub_source in self.sources() {
            min_extent = min_extent.min(sub_source.min_extent());
        }
        min_extent
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        if self.pull_up_disjunctions {
            return Ok(self.sources().cloned().collect());
        }
        Ok(vec![self])
    }
}

/// `lessThan(a, b)`: `a.end() < b.end() || (a.end() == b.end() && a.start()
/// >= b.start())`.
#[derive(Debug, Clone, Copy)]
struct EndThenStart;

impl PriorityQueueComparator<SharedIntervalIterator> for EndThenStart {
    fn less_than(&self, a: &SharedIntervalIterator, b: &SharedIntervalIterator) -> bool {
        a.end() < b.end() || (a.end() == b.end() && a.start() >= b.start())
    }
}

/// The iterator a disjunction is positioned on: one of its sub-iterators, or
/// one of the two sentinels.
#[derive(Clone)]
enum Current {
    /// The private static `EMPTY` sentinel.
    Empty,
    /// The private static `EXHAUSTED` sentinel.
    Exhausted,
    /// A sub-iterator.
    It(SharedIntervalIterator),
}

impl Current {
    fn start(&self) -> i32 {
        match self {
            Current::Empty => -1,
            Current::Exhausted => NO_MORE_INTERVALS,
            Current::It(it) => it.start(),
        }
    }

    fn end(&self) -> i32 {
        match self {
            Current::Empty => -1,
            Current::Exhausted => NO_MORE_INTERVALS,
            Current::It(it) => it.end(),
        }
    }

    /// **Divergence.** The sentinels' `gaps()` throws
    /// `UnsupportedOperationException`, which the infallible signature
    /// raises as a panic; the iterator contract forbids the call.
    fn gaps(&self) -> i32 {
        match self {
            Current::Empty | Current::Exhausted => {
                panic!("java.lang.UnsupportedOperationException: gaps() of an unpositioned disjunction")
            }
            Current::It(it) => it.gaps(),
        }
    }

    fn is_sentinel(&self) -> bool {
        matches!(self, Current::Empty | Current::Exhausted)
    }
}

/// The disjunction of interval iterators: the minimal intervals of all of
/// them.
///
/// Equivalent to the package-private static class
/// `org.apache.lucene.queries.intervals.DisjunctionIntervalsSource.DisjunctionIntervalIterator`.
/// Its sub-iterators are shared between the approximation's queue, the
/// `iterators` list and the interval queue, as Java's references are.
pub(crate) struct DisjunctionIntervalIterator {
    approximation: DisjunctionDISIApproximation,
    interval_queue: PriorityQueue<SharedIntervalIterator, EndThenStart>,
    iterators: Vec<SharedIntervalIterator>,
    match_cost: f32,
    current: Current,
}

impl DisjunctionIntervalIterator {
    /// Equivalent to `DisjunctionIntervalIterator(List<IntervalIterator>)`.
    ///
    /// # Errors
    ///
    /// The error of sizing the interval queue.
    pub(crate) fn new(iterators: Vec<Box<dyn IntervalIterator>>) -> Result<Self> {
        let iterators: Vec<SharedIntervalIterator> = iterators
            .into_iter()
            .map(SharedIntervalIterator::new)
            .collect();
        let mut disi_queue = DisiPriorityQueue::new(iterators.len());
        for it in &iterators {
            disi_queue.add(DisiWrapper::new(it.clone()));
        }
        let approximation = DisjunctionDISIApproximation::new(disi_queue);
        let interval_queue = PriorityQueue::new(iterators.len(), EndThenStart)?;
        let mut costsum = 0f32;
        for it in &iterators {
            costsum += it.cost() as f32;
        }
        Ok(Self {
            approximation,
            interval_queue,
            iterators,
            match_cost: costsum,
            current: Current::Empty,
        })
    }

    /// Equivalent to the private `DisjunctionIntervalIterator.reset()`.
    fn reset(&mut self) -> Result<()> {
        self.interval_queue.clear();
        let queue = &mut self.approximation.sub_iterators;
        for position in queue.top_list() {
            let dw = queue.wrapper_mut(position);
            dw.intervals.next_interval()?;
            self.interval_queue.add(dw.intervals.clone());
        }
        self.current = Current::Empty;
        Ok(())
    }

    /// The position of the current sub-iterator in the list of iterators.
    ///
    /// Equivalent to the package-private
    /// `DisjunctionIntervalIterator.currentOrd()`.
    ///
    /// # Errors
    ///
    /// The `IllegalStateException` Java throws when the current iterator is
    /// not in the list.
    pub(crate) fn current_ord(&self) -> Result<usize> {
        debug_assert!(!self.current.is_sentinel());
        if let Current::It(current) = &self.current {
            for (i, it) in self.iterators.iter().enumerate() {
                if it.ptr_eq(current) {
                    return Ok(i);
                }
            }
        }
        Err(LuceneError::illegal_state(String::new()))
    }

    /// Equivalent to the private
    /// `DisjunctionIntervalIterator.contains(IntervalIterator, int, int)`.
    fn contains(it: &SharedIntervalIterator, start: i32, end: i32) -> bool {
        start >= it.start() && start <= it.end() && end >= it.start() && end <= it.end()
    }
}

impl DocIdSetIterator for DisjunctionIntervalIterator {
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

impl IntervalIterator for DisjunctionIntervalIterator {
    fn match_cost(&self) -> f32 {
        self.match_cost
    }

    fn start(&self) -> i32 {
        self.current.start()
    }

    fn end(&self) -> i32 {
        self.current.end()
    }

    fn gaps(&self) -> i32 {
        self.current.gaps()
    }

    fn next_interval(&mut self) -> Result<i32> {
        if self.current.is_sentinel() {
            if let Some(top) = self.interval_queue.top() {
                self.current = Current::It(top.clone());
            }
            return Ok(self.current.start());
        }
        let start = self.current.start();
        let end = self.current.end();
        while self
            .interval_queue
            .top()
            .is_some_and(|top| Self::contains(top, start, end))
        {
            if let Some(mut it) = self.interval_queue.pop() {
                if it.next_interval()? != NO_MORE_INTERVALS {
                    self.interval_queue.add(it);
                }
            }
        }
        match self.interval_queue.top() {
            None => {
                self.current = Current::Exhausted;
                Ok(NO_MORE_INTERVALS)
            }
            Some(top) => {
                self.current = Current::It(top.clone());
                Ok(self.current.start())
            }
        }
    }
}

/// The matches of a disjunction: the positions of the disjunction iterator,
/// the offsets, sub-matches and query of the sub-matches it is positioned
/// on.
///
/// Equivalent to the private record
/// `org.apache.lucene.queries.intervals.DisjunctionIntervalsSource.DisjunctionMatchesIterator`.
pub(crate) struct DisjunctionMatchesIterator {
    it: DisjunctionIntervalIterator,
    subs: Vec<SharedIntervalMatchesIterator>,
}

impl MatchesIterator for DisjunctionMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        Ok(self.it.next_interval()? != NO_MORE_INTERVALS)
    }

    fn start_position(&self) -> i32 {
        self.it.start()
    }

    fn end_position(&self) -> i32 {
        self.it.end()
    }

    fn start_offset(&self) -> Result<i32> {
        let ord = self.it.current_ord()?;
        self.subs[ord].start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        let ord = self.it.current_ord()?;
        self.subs[ord].end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        let ord = self.it.current_ord()?;
        self.subs[ord].get_sub_matches()
    }

    /// **Divergence.** `currentOrd()` throws `IllegalStateException` for an
    /// unpositioned disjunction; the infallible signature panics.
    fn get_query(&self) -> Arc<dyn Query> {
        match self.it.current_ord() {
            Ok(ord) => self.subs[ord].get_query(),
            Err(e) => panic!("{e}"),
        }
    }
}

impl IntervalMatchesIterator for DisjunctionMatchesIterator {
    fn gaps(&self) -> i32 {
        self.it.gaps()
    }

    fn width(&self) -> i32 {
        self.it.width()
    }
}
