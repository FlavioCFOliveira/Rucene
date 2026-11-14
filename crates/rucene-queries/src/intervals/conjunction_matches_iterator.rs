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

//! Port of `ConjunctionMatchesIterator.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::matches::{MatchesIterator, MatchesUtils};
use rucene::search::query::Query;

use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};

/// The matches of a conjunction: the positions of the combined iterator, the
/// offsets and sub-matches of the sub-matches.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.ConjunctionMatchesIterator`.
///
/// **Divergence.** Java's `getQuery()` throws
/// `UnsupportedOperationException`; the core
/// [`MatchesIterator::get_query`] cannot fail, so it panics with that
/// exception's name.
pub(crate) struct ConjunctionMatchesIterator {
    /// The package-private `iterator` field, positioned on its first interval.
    iterator: Box<dyn IntervalIterator>,
    /// The package-private `subs` field: handles on the sub-matches the
    /// iterator was built over.
    subs: Vec<SharedIntervalMatchesIterator>,
    /// The package-private `cached` field.
    cached: bool,
}

impl ConjunctionMatchesIterator {
    /// Equivalent to `ConjunctionMatchesIterator(IntervalIterator, List<?
    /// extends IntervalMatchesIterator>)`.
    pub(crate) fn new(
        iterator: Box<dyn IntervalIterator>,
        subs: Vec<SharedIntervalMatchesIterator>,
    ) -> Self {
        Self {
            iterator,
            subs,
            cached: true,
        }
    }
}

impl MatchesIterator for ConjunctionMatchesIterator {
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
        for s in &self.subs {
            let v = s.start_offset()?;
            if v == -1 {
                return Ok(-1);
            }
            start = start.min(v);
        }
        Ok(start)
    }

    fn end_offset(&self) -> Result<i32> {
        let mut end = -1;
        for s in &self.subs {
            let v = s.end_offset()?;
            if v == -1 {
                return Ok(-1);
            }
            end = end.max(v);
        }
        Ok(end)
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
        panic!("java.lang.UnsupportedOperationException: ConjunctionMatchesIterator.getQuery()")
    }
}

impl IntervalMatchesIterator for ConjunctionMatchesIterator {
    fn gaps(&self) -> i32 {
        self.iterator.gaps()
    }

    fn width(&self) -> i32 {
        self.iterator.width()
    }
}

/// A matches iterator over the current match of another, returned once.
///
/// Equivalent to the package-private static class
/// `org.apache.lucene.queries.intervals.ConjunctionMatchesIterator.SingletonMatchesIterator`,
/// a `FilterMatchesIterator`.
pub(crate) struct SingletonMatchesIterator {
    /// The `in` field.
    in_: Box<dyn MatchesIterator>,
    /// The package-private `exhausted` field.
    exhausted: bool,
}

impl SingletonMatchesIterator {
    /// Equivalent to `SingletonMatchesIterator(MatchesIterator)`.
    pub(crate) fn new(in_: Box<dyn MatchesIterator>) -> Self {
        Self {
            in_,
            exhausted: false,
        }
    }
}

impl MatchesIterator for SingletonMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        if self.exhausted {
            return Ok(false);
        }
        self.exhausted = true;
        Ok(true)
    }

    fn start_position(&self) -> i32 {
        self.in_.start_position()
    }

    fn end_position(&self) -> i32 {
        self.in_.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.in_.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.in_.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.in_.get_sub_matches()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.in_.get_query()
    }
}
