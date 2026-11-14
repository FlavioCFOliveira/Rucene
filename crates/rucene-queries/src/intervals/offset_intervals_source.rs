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

//! Port of `OffsetIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::java_collections::java_boolean_hash_code;

use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches::IntervalMatches;
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::interval_query::IntervalQuery;
use crate::intervals::intervals_source::{objects_hash, IntervalsSource};

/// Tracks a reference intervals source, and produces a pseudo-interval that
/// appears either one position before or one position after each interval
/// from the reference.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.OffsetIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct OffsetIntervalsSource {
    in_: Arc<dyn IntervalsSource>,
    before: bool,
}

impl OffsetIntervalsSource {
    /// Equivalent to `OffsetIntervalsSource(IntervalsSource, boolean)`.
    pub(crate) fn new(in_: Arc<dyn IntervalsSource>, before: bool) -> Self {
        Self { in_, before }
    }

    /// Equivalent to the private `OffsetIntervalsSource.offset(IntervalIterator)`.
    fn offset(&self, it: Box<dyn IntervalIterator>) -> Box<dyn IntervalIterator> {
        Box::new(OffsetIntervalIterator {
            in_: it,
            before: self.before,
        })
    }
}

/// The pseudo-interval iterator of an [`OffsetIntervalsSource`].
///
/// Equivalent to the private abstract static class
/// `org.apache.lucene.queries.intervals.OffsetIntervalsSource.OffsetIntervalIterator`;
/// its two anonymous subclasses, which only differ in `start()`, are selected
/// by `before`.
pub(crate) struct OffsetIntervalIterator {
    /// The package-private `in` field.
    in_: Box<dyn IntervalIterator>,
    /// Which anonymous subclass `offset(it)` created.
    before: bool,
}

impl DocIdSetIterator for OffsetIntervalIterator {
    fn doc_id(&self) -> i32 {
        self.in_.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.in_.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.in_.advance(target)
    }

    fn cost(&self) -> i64 {
        self.in_.cost()
    }
}

impl IntervalIterator for OffsetIntervalIterator {
    fn start(&self) -> i32 {
        if self.before {
            let pos = self.in_.start();
            if pos == -1 {
                return -1;
            }
            if pos == NO_MORE_INTERVALS {
                return NO_MORE_INTERVALS;
            }
            0.max(pos - 1)
        } else {
            let pos = self.in_.end().wrapping_add(1);
            if pos == 0 {
                return -1;
            }
            if pos < 0 {
                // overflow
                return i32::MAX;
            }
            if pos == i32::MAX {
                return i32::MAX - 1;
            }
            pos
        }
    }

    fn end(&self) -> i32 {
        self.start()
    }

    fn gaps(&self) -> i32 {
        0
    }

    fn next_interval(&mut self) -> Result<i32> {
        self.in_.next_interval()?;
        Ok(self.start())
    }

    fn match_cost(&self) -> f32 {
        self.in_.match_cost()
    }
}

impl IntervalsSource for OffsetIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let Some(it) = self.in_.intervals(field, ctx)? else {
            return Ok(None);
        };
        Ok(Some(self.offset(it)))
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let Some(mi) = self.in_.matches(field, ctx, doc)? else {
            return Ok(None);
        };
        let mi = SharedIntervalMatchesIterator::new(mi);
        let offset = self.offset(IntervalMatches::wrap_matches(Box::new(mi.clone()), doc));
        IntervalMatches::as_matches(offset, Some(Box::new(mi)), doc)
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        let in_ = Arc::clone(&self.in_);
        let query = IntervalQuery::new(field, self);
        let mut v = visitor.get_sub_visitor(Occur::MUST, &query);
        in_.visit(field, &mut *v);
    }

    fn min_extent(&self) -> i32 {
        1
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<OffsetIntervalsSource>()
            .is_some_and(|that| self.before == that.before && self.in_.source_eq(&*that.in_))
    }

    /// `Objects.hash(in, before)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.in_.source_hash(), java_boolean_hash_code(self.before)])
    }

    fn to_source_string(&self) -> String {
        if self.before {
            return format!("PRECEDING({})", self.in_);
        }
        format!("FOLLOWING({})", self.in_)
    }
}
