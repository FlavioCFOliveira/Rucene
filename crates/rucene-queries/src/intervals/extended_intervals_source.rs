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

//! Port of `ExtendedIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::extended_interval_iterator::ExtendedIntervalIterator;
use crate::intervals::interval_iterator::IntervalIterator;
use crate::intervals::interval_matches::IntervalMatches;
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::intervals_source::{
    objects_equals, objects_hash, same_object, to_java_hash_set, IntervalsSource,
};

/// A source whose intervals are those of another, extended by a number of
/// positions before and after.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.ExtendedIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct ExtendedIntervalsSource {
    /// The package-private `source` field.
    source: Arc<dyn IntervalsSource>,
    before: i32,
    after: i32,
}

impl ExtendedIntervalsSource {
    /// Equivalent to `ExtendedIntervalsSource(IntervalsSource, int, int)`.
    pub(crate) fn new(source: Arc<dyn IntervalsSource>, before: i32, after: i32) -> Self {
        Self {
            source,
            before,
            after,
        }
    }
}

/// The anonymous `IntervalMatchesIterator` of
/// `ExtendedIntervalsSource.matches`, which hides the offsets of the
/// extended matches.
struct NoOffsetsMatchesIterator {
    delegate: Box<dyn IntervalMatchesIterator>,
}

impl MatchesIterator for NoOffsetsMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.delegate.next()
    }

    fn start_position(&self) -> i32 {
        self.delegate.start_position()
    }

    fn end_position(&self) -> i32 {
        self.delegate.end_position()
    }

    /// We could return `before > 0 ? -1 : in.startOffset()`, but keep it
    /// consistent for start/end offset.
    fn start_offset(&self) -> Result<i32> {
        Ok(-1)
    }

    /// We could return `after > 0 ? -1 : in.startOffset()`, but keep it
    /// consistent for start/end offset.
    fn end_offset(&self) -> Result<i32> {
        Ok(-1)
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.delegate.get_sub_matches()
    }

    fn get_query(&self) -> Option<Arc<dyn Query>> {
        self.delegate.get_query()
    }
}

impl IntervalMatchesIterator for NoOffsetsMatchesIterator {
    fn gaps(&self) -> i32 {
        self.delegate.gaps()
    }

    fn width(&self) -> i32 {
        self.delegate.width()
    }
}

impl IntervalsSource for ExtendedIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let Some(in_) = self.source.intervals(field, ctx)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(ExtendedIntervalIterator::new(
            in_,
            self.before,
            self.after,
        ))))
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let Some(in_) = self.source.matches(field, ctx, doc)? else {
            return Ok(None);
        };
        let in_no_offsets =
            SharedIntervalMatchesIterator::new(Box::new(NoOffsetsMatchesIterator {
                delegate: in_,
            }));
        let wrapped = ExtendedIntervalIterator::new(
            IntervalMatches::wrap_matches(Box::new(in_no_offsets.clone()), doc),
            self.before,
            self.after,
        );
        IntervalMatches::as_matches(Box::new(wrapped), Some(Box::new(in_no_offsets)), doc)
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        Arc::clone(&self.source).visit(field, visitor);
    }

    fn min_extent(&self) -> i32 {
        let min_extent = self
            .before
            .wrapping_add(self.source.min_extent())
            .wrapping_add(self.after);
        if min_extent < 0 {
            return i32::MAX;
        }
        min_extent
    }

    /// `Collections.singleton(this)` when the source pulls up nothing, a
    /// `HashSet` of extended sources over its disjuncts otherwise.
    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let inner = Arc::clone(&self.source).pull_up_disjunctions()?;
        if inner.is_empty() {
            return Ok(vec![self]);
        }
        let (before, after) = (self.before, self.after);
        Ok(to_java_hash_set(inner.into_iter().map(|s| {
            Arc::new(ExtendedIntervalsSource::new(s, before, after)) as Arc<dyn IntervalsSource>
        })))
    }

    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<ExtendedIntervalsSource>()
                .is_some_and(|that| {
                    self.before == that.before
                        && self.after == that.after
                        && objects_equals(&self.source, &that.source)
                })
    }

    /// `Objects.hash(source, before, after)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.source.source_hash(), self.before, self.after])
    }

    fn to_source_string(&self) -> String {
        format!("EXTEND({},{},{})", self.source, self.before, self.after)
    }
}
