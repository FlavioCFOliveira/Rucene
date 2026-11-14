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

//! Port of `ContainedByIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::conjunction_intervals_source::{self, ConjunctionIntervalsSource};
use crate::intervals::conjunction_matches_iterator::ConjunctionMatchesIterator;
use crate::intervals::disjunctions::Disjunctions;
use crate::intervals::filtering_interval_iterator::FilteringIntervalIterator;
use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::intervals::Intervals;
use crate::intervals::intervals_source::{sources_list_eq, sources_list_hash, IntervalsSource};

/// The intervals of a small source that appear within an interval of a big source.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.ContainedByIntervalsSource`, a `ConjunctionIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct ContainedByIntervalsSource {
    /// The protected `subSources` field: `[small, big]`.
    sub_sources: Vec<Arc<dyn IntervalsSource>>,
    small: Arc<dyn IntervalsSource>,
    big: Arc<dyn IntervalsSource>,
}

impl ContainedByIntervalsSource {
    /// Equivalent to the package-private static
    /// `ContainedByIntervalsSource.build(IntervalsSource, IntervalsSource)`:
    /// disjunctions of the big source are pulled up.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub(crate) fn build(
        small: Arc<dyn IntervalsSource>,
        big: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Intervals::or(Disjunctions::pull_up_one(big, |s| {
            Arc::new(ContainedByIntervalsSource::new(Arc::clone(&small), s))
                as Arc<dyn IntervalsSource>
        })?)
    }

    /// Equivalent to the private `ContainedByIntervalsSource(IntervalsSource, IntervalsSource)`.
    fn new(small: Arc<dyn IntervalsSource>, big: Arc<dyn IntervalsSource>) -> Self {
        Self {
            sub_sources: vec![Arc::clone(&small), Arc::clone(&big)],
            small,
            big,
        }
    }
}

/// The anonymous `FilteringIntervalIterator` of `ContainedByIntervalsSource.combine`.
fn next_interval(it: &mut FilteringIntervalIterator) -> Result<i32> {
    if !it.bpos {
        return Ok(NO_MORE_INTERVALS);
    }
    while it.a.next_interval()? != NO_MORE_INTERVALS {
        while it.b.end() < it.a.end() {
            if it.b.next_interval()? == NO_MORE_INTERVALS {
                it.bpos = false;
                return Ok(NO_MORE_INTERVALS);
            }
        }
        if it.b.start() <= it.a.start() {
            return Ok(it.a.start());
        }
    }
    it.bpos = false;
    Ok(NO_MORE_INTERVALS)
}

impl ConjunctionIntervalsSource for ContainedByIntervalsSource {
    fn sub_sources(&self) -> &[Arc<dyn IntervalsSource>] {
        &self.sub_sources
    }

    fn combine(
        &self,
        iterators: Vec<Box<dyn IntervalIterator>>,
    ) -> Result<Box<dyn IntervalIterator>> {
        debug_assert!(iterators.len() == 2);
        let mut iterators = iterators.into_iter();
        let (Some(a), Some(b)) = (iterators.next(), iterators.next()) else {
            unreachable!("INVARIANT: a two-source conjunction combines two iterators");
        };
        Ok(Box::new(FilteringIntervalIterator::new(
            a,
            b,
            next_interval,
        )?))
    }

    /// The only sub-source whose matches count is the "small" source.
    fn create_matches_iterator(
        &self,
        it: Box<dyn IntervalIterator>,
        subs: Vec<SharedIntervalMatchesIterator>,
    ) -> Box<dyn IntervalMatchesIterator> {
        debug_assert!(subs.len() == 2);
        Box::new(ConjunctionMatchesIterator::new(it, vec![subs[0].clone()]))
    }
}

impl IntervalsSource for ContainedByIntervalsSource {
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
        self.small.min_extent()
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let small = Arc::clone(&self.small);
        Disjunctions::pull_up_one(Arc::clone(&self.big), |s| {
            Arc::new(ContainedByIntervalsSource::new(Arc::clone(&small), s))
                as Arc<dyn IntervalsSource>
        })
    }

    /// `Objects.hashCode(subSources)`.
    fn source_hash(&self) -> i32 {
        sources_list_hash(&self.sub_sources)
    }

    /// `other instanceof ContainedByIntervalsSource` with equal sub-sources.
    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<ContainedByIntervalsSource>()
            .is_some_and(|o| sources_list_eq(&self.sub_sources, &o.sub_sources))
    }

    fn to_source_string(&self) -> String {
        format!("CONTAINED_BY({},{})", self.small, self.big)
    }
}
