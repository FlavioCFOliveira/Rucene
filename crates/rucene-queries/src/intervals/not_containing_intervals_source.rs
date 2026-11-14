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

//! Port of `NotContainingIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::difference_intervals_source::{self, DifferenceIntervalsSource};
use crate::intervals::disjunctions::Disjunctions;
use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::intervals::Intervals;
use crate::intervals::intervals_source::{objects_hash, IntervalsSource};
use crate::intervals::relative_iterator::{relative_iterator_impl, RelativeIterator};

/// The intervals of the minuend that do not contain an interval of the subtrahend.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.NotContainingIntervalsSource`, a `DifferenceIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct NotContainingIntervalsSource {
    minuend: Arc<dyn IntervalsSource>,
    subtrahend: Arc<dyn IntervalsSource>,
}

impl NotContainingIntervalsSource {
    /// Equivalent to the package-private static
    /// `NotContainingIntervalsSource.build(IntervalsSource, IntervalsSource)`:
    /// disjunctions of the minuend are pulled up.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub(crate) fn build(
        minuend: Arc<dyn IntervalsSource>,
        subtrahend: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Intervals::or(Disjunctions::pull_up_one(minuend, |s| {
            Arc::new(NotContainingIntervalsSource::new(
                s,
                Arc::clone(&subtrahend),
            )) as Arc<dyn IntervalsSource>
        })?)
    }

    /// Equivalent to the private `NotContainingIntervalsSource(IntervalsSource, IntervalsSource)`.
    fn new(minuend: Arc<dyn IntervalsSource>, subtrahend: Arc<dyn IntervalsSource>) -> Self {
        Self {
            minuend,
            subtrahend,
        }
    }
}

impl DifferenceIntervalsSource for NotContainingIntervalsSource {
    fn minuend(&self) -> &Arc<dyn IntervalsSource> {
        &self.minuend
    }

    fn subtrahend(&self) -> &Arc<dyn IntervalsSource> {
        &self.subtrahend
    }

    fn combine(
        &self,
        minuend: Box<dyn IntervalIterator>,
        subtrahend: Box<dyn IntervalIterator>,
    ) -> Box<dyn IntervalIterator> {
        Box::new(NotContainingIterator {
            base: RelativeIterator::new(minuend, subtrahend),
        })
    }
}

impl IntervalsSource for NotContainingIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        difference_intervals_source::intervals(self, field, ctx)
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        difference_intervals_source::matches(self, field, ctx, doc)
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        difference_intervals_source::visit(self, field, visitor);
    }

    fn min_extent(&self) -> i32 {
        difference_intervals_source::min_extent(self)
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    /// `Objects.hash(minuend, subtrahend)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.minuend.source_hash(), self.subtrahend.source_hash()])
    }

    /// `other instanceof NotContainingIntervalsSource` with equal minuend and subtrahend.
    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<NotContainingIntervalsSource>()
            .is_some_and(|o| {
                self.minuend.source_eq(&*o.minuend) && self.subtrahend.source_eq(&*o.subtrahend)
            })
    }

    fn to_source_string(&self) -> String {
        format!("NOT_CONTAINING({},{})", self.minuend, self.subtrahend)
    }
}

/// The iterator of a [`NotContainingIntervalsSource`].
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.NotContainingIntervalsSource.NotContainingIterator`, a `RelativeIterator`.
pub(crate) struct NotContainingIterator {
    base: RelativeIterator,
}

impl NotContainingIterator {
    /// The class's `nextInterval()`.
    fn next_interval_impl(base: &mut RelativeIterator) -> Result<i32> {
        if !base.bpos {
            return base.a.next_interval();
        }
        while base.a.next_interval()? != NO_MORE_INTERVALS {
            while base.b.start() < base.a.start() && base.b.end() < base.a.end() {
                if base.b.next_interval()? == NO_MORE_INTERVALS {
                    base.bpos = false;
                    return Ok(base.a.start());
                }
            }
            if base.b.start() > base.a.end() {
                return Ok(base.a.start());
            }
        }
        Ok(NO_MORE_INTERVALS)
    }
}

relative_iterator_impl!(NotContainingIterator);
