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

//! Port of `RepeatingIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::matches::{MatchesIterator, MatchesUtils};
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::conjunction_matches_iterator::SingletonMatchesIterator;
use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::intervals_source::{objects_hash, IntervalsSource};

/// Generates an iterator that spans repeating instances of a sub-iterator,
/// avoiding minimisation. This is useful for repeated terms within an
/// unordered interval, for example, ensuring that multiple iterators do not
/// match on a single term.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.RepeatingIntervalsSource`. The
/// generated iterators have a specialised `width()` that sums up the widths of
/// the individual sub-iterators, rather than returning the full span of the
/// iterator.
///
/// The `name` field, which `setName` changes after construction, is behind a
/// lock so that the shared source can be renamed as Java renames it.
#[derive(Debug)]
pub(crate) struct RepeatingIntervalsSource {
    /// The package-private `in` field.
    in_: Arc<dyn IntervalsSource>,
    /// The package-private `childCount` field.
    child_count: i32,
    /// The package-private `name` field, `None` for `null`.
    name: Mutex<Option<String>>,
}

impl RepeatingIntervalsSource {
    /// Equivalent to the package-private static
    /// `RepeatingIntervalsSource.build(IntervalsSource, int)`: the source
    /// itself for a single child.
    pub(crate) fn build(
        in_: Arc<dyn IntervalsSource>,
        child_count: i32,
    ) -> Arc<dyn IntervalsSource> {
        if child_count == 1 {
            return in_;
        }
        debug_assert!(child_count > 0);
        Arc::new(Self {
            in_,
            child_count,
            name: Mutex::new(None),
        })
    }

    /// Equivalent to `RepeatingIntervalsSource.setName(String)`.
    pub(crate) fn set_name(&self, name: &str) {
        *self.name.lock().unwrap_or_else(|e| e.into_inner()) = Some(name.to_string());
    }
}

impl IntervalsSource for RepeatingIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let Some(it) = self.in_.intervals(field, ctx)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(DuplicateIntervalIterator::new(
            it,
            self.child_count,
        ))))
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let mut subs = Vec::with_capacity(self.child_count.max(0) as usize);
        for _ in 0..self.child_count {
            let Some(mi) = self.in_.matches(field, ctx, doc)? else {
                return Ok(None);
            };
            subs.push(SharedIntervalMatchesIterator::new(mi));
        }
        DuplicateMatchesIterator::build(subs)
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        Arc::clone(&self.in_).visit(field, visitor);
    }

    fn min_extent(&self) -> i32 {
        self.in_.min_extent()
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    /// `Objects.hash(in, childCount)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.in_.source_hash(), self.child_count])
    }

    /// `other instanceof RepeatingIntervalsSource` with an equal source and
    /// child count; the name takes no part.
    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<RepeatingIntervalsSource>()
            .is_some_and(|o| self.in_.source_eq(&*o.in_) && self.child_count == o.child_count)
    }

    fn to_source_string(&self) -> String {
        let s = self.in_.to_source_string();
        let mut out = s.clone();
        for _ in 1..self.child_count {
            out.push(',');
            out.push_str(&s);
        }
        match self
            .name
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
        {
            Some(name) => format!("{name}({out})"),
            None => out,
        }
    }
}

/// The iterator of a [`RepeatingIntervalsSource`]: windows of `copies`
/// consecutive intervals of the primary iterator.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.RepeatingIntervalsSource.DuplicateIntervalIterator`.
pub(crate) struct DuplicateIntervalIterator {
    in_: Box<dyn IntervalIterator>,
    cache: Vec<i32>,
    cache_length: i32,
    cache_base: i32,
    started: bool,
    exhausted: bool,
}

impl DuplicateIntervalIterator {
    /// Equivalent to `DuplicateIntervalIterator(IntervalIterator, int)`.
    fn new(primary: Box<dyn IntervalIterator>, copies: i32) -> Self {
        Self {
            in_: primary,
            cache: vec![0; (copies.max(0) * 2) as usize],
            cache_length: copies,
            cache_base: 0,
            started: false,
            exhausted: false,
        }
    }

    /// Equivalent to the private
    /// `DuplicateIntervalIterator.cacheNextInterval(int)`.
    fn cache_next_interval(&mut self, line_pos: i32) -> Result<i32> {
        if self.in_.next_interval()? == NO_MORE_INTERVALS {
            self.exhausted = true;
            return Ok(NO_MORE_INTERVALS);
        }
        self.cache[(line_pos * 2) as usize] = self.in_.start();
        self.cache[(line_pos * 2 + 1) as usize] = self.in_.end();
        Ok(self.start())
    }

    /// `started = exhausted = false; Arrays.fill(cache, -1)`.
    fn clear(&mut self) {
        self.started = false;
        self.exhausted = false;
        self.cache.fill(-1);
    }
}

impl DocIdSetIterator for DuplicateIntervalIterator {
    fn doc_id(&self) -> i32 {
        self.in_.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.clear();
        self.in_.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.clear();
        self.in_.advance(target)
    }

    fn cost(&self) -> i64 {
        self.in_.cost()
    }
}

impl IntervalIterator for DuplicateIntervalIterator {
    fn start(&self) -> i32 {
        if self.exhausted {
            NO_MORE_INTERVALS
        } else {
            self.cache[((self.cache_base % self.cache_length) * 2) as usize]
        }
    }

    fn end(&self) -> i32 {
        if self.exhausted {
            NO_MORE_INTERVALS
        } else {
            self.cache[((((self.cache_base + self.cache_length - 1) % self.cache_length) * 2) + 1)
                as usize]
        }
    }

    /// The sum of `start - end + 1` over the cached intervals, as Java
    /// computes it.
    fn width(&self) -> i32 {
        let mut width = 0i32;
        for i in 0..self.cache_length {
            let pos = ((self.cache_base + i) % self.cache_length) as usize;
            width = width.wrapping_add(
                self.cache[pos * 2]
                    .wrapping_sub(self.cache[pos * 2 + 1])
                    .wrapping_add(1),
            );
        }
        width
    }

    /// `super.width() - width()`.
    fn gaps(&self) -> i32 {
        let super_width = self.end().wrapping_sub(self.start()).wrapping_add(1);
        super_width.wrapping_sub(self.width())
    }

    fn next_interval(&mut self) -> Result<i32> {
        if self.exhausted {
            return Ok(NO_MORE_INTERVALS);
        }
        if !self.started {
            for i in 0..self.cache_length {
                if self.cache_next_interval(i)? == NO_MORE_INTERVALS {
                    return Ok(NO_MORE_INTERVALS);
                }
            }
            self.cache_base = 0;
            self.started = true;
            Ok(self.start())
        } else {
            let insert = (self.cache_base + self.cache_length) % self.cache_length;
            self.cache_base = (self.cache_base + 1) % self.cache_length;
            self.cache_next_interval(insert)
        }
    }

    fn match_cost(&self) -> f32 {
        self.in_.match_cost()
    }
}

/// The matches of a [`RepeatingIntervalsSource`]: windows of consecutive
/// matches of copies of the sub-source.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.RepeatingIntervalsSource.DuplicateMatchesIterator`.
///
/// **Divergence.** Java's `getQuery()` throws
/// `UnsupportedOperationException`; the core
/// [`MatchesIterator::get_query`] cannot fail, so it panics with that
/// exception's name.
pub(crate) struct DuplicateMatchesIterator {
    subs: Vec<SharedIntervalMatchesIterator>,
    cached: bool,
}

impl DuplicateMatchesIterator {
    /// Positions the copies on consecutive matches, or returns `None` when
    /// there are not enough of them.
    ///
    /// Equivalent to the static
    /// `DuplicateMatchesIterator.build(List<IntervalMatchesIterator>)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while advancing the copies.
    fn build(
        mut subs: Vec<SharedIntervalMatchesIterator>,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let mut count = subs.len();
        while count > 0 {
            for _ in 0..count {
                if !subs[count - 1].next()? {
                    return Ok(None);
                }
            }
            count -= 1;
        }
        Ok(Some(Box::new(DuplicateMatchesIterator {
            subs,
            cached: false,
        })))
    }
}

impl MatchesIterator for DuplicateMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        if !self.cached {
            self.cached = true;
            return Ok(true);
        }
        let last = self.subs.len() - 1;
        if !self.subs[last].next()? {
            return Ok(false);
        }
        for i in 0..last {
            self.subs[i].next()?;
        }
        Ok(true)
    }

    fn start_position(&self) -> i32 {
        self.subs[0].start_position()
    }

    fn end_position(&self) -> i32 {
        self.subs[self.subs.len() - 1].end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.subs[0].start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.subs[self.subs.len() - 1].end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        let mut sub_matches: Vec<Box<dyn MatchesIterator>> = Vec::with_capacity(self.subs.len());
        for mi in &self.subs {
            let sub = match mi.get_sub_matches()? {
                Some(sub) => sub,
                None => Box::new(SingletonMatchesIterator::new(Box::new(mi.clone()))),
            };
            sub_matches.push(sub);
        }
        MatchesUtils::disjunction(sub_matches)
    }

    fn get_query(&self) -> Arc<dyn Query> {
        panic!("java.lang.UnsupportedOperationException: DuplicateMatchesIterator.getQuery()")
    }
}

impl IntervalMatchesIterator for DuplicateMatchesIterator {
    fn gaps(&self) -> i32 {
        let mut width = self
            .end_position()
            .wrapping_sub(self.start_position())
            .wrapping_add(1);
        for mi in &self.subs {
            width = width.wrapping_sub(
                mi.end_position()
                    .wrapping_sub(mi.start_position())
                    .wrapping_add(1),
            );
        }
        width
    }

    fn width(&self) -> i32 {
        let mut width = 0i32;
        for mi in &self.subs {
            width = width.wrapping_add(
                mi.end_position()
                    .wrapping_sub(mi.start_position())
                    .wrapping_add(1),
            );
        }
        width
    }
}
