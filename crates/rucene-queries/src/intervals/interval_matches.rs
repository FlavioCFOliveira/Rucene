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

//! Port of `IntervalMatches.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;

use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;

/// Conversions between interval iterators and matches iterators.
///
/// Equivalent to the final class
/// `org.apache.lucene.queries.intervals.IntervalMatches`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IntervalMatches;

impl IntervalMatches {
    /// Positions `iterator` on `doc` and its first interval, and returns a
    /// matches iterator over its intervals that reads offsets, sub-matches and
    /// the query from `source`; `None` when `source` is `None` or the
    /// iterator has no interval on `doc`.
    ///
    /// Equivalent to the package-private static
    /// `IntervalMatches.asMatches(IntervalIterator, IntervalMatchesIterator,
    /// int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while positioning the iterator.
    pub(crate) fn as_matches(
        mut iterator: Box<dyn IntervalIterator>,
        source: Option<Box<dyn IntervalMatchesIterator>>,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let Some(source) = source else {
            return Ok(None);
        };
        if iterator.advance(doc)? != doc {
            return Ok(None);
        }
        if iterator.next_interval()? == NO_MORE_INTERVALS {
            return Ok(None);
        }
        Ok(Some(Box::new(AsMatches {
            iterator,
            source,
            cached: true,
        })))
    }

    /// Returns an interval iterator over the matches of a single document.
    ///
    /// Equivalent to the package-private static
    /// `IntervalMatches.wrapMatches(IntervalMatchesIterator, int)`.
    pub(crate) fn wrap_matches(
        mi: Box<dyn IntervalMatchesIterator>,
        doc: i32,
    ) -> Box<dyn IntervalIterator> {
        Box::new(WrappedMatches {
            mi,
            doc,
            state: State::UNPOSITIONED,
        })
    }
}

/// The anonymous `IntervalMatchesIterator` of `IntervalMatches.asMatches`.
struct AsMatches {
    iterator: Box<dyn IntervalIterator>,
    source: Box<dyn IntervalMatchesIterator>,
    cached: bool,
}

impl MatchesIterator for AsMatches {
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
        self.source.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.source.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.source.get_sub_matches()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.source.get_query()
    }
}

impl IntervalMatchesIterator for AsMatches {
    fn gaps(&self) -> i32 {
        self.iterator.gaps()
    }

    fn width(&self) -> i32 {
        self.iterator.width()
    }
}

/// The states of the interval iterator `IntervalMatches.wrapMatches` builds.
///
/// Equivalent to the package-private enum
/// `org.apache.lucene.queries.intervals.IntervalMatches.State`.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// Before the first `nextDoc` or `advance`.
    UNPOSITIONED,
    /// On the document, iterating its intervals.
    ITERATING,
    /// On the document, its intervals exhausted.
    NO_MORE_INTERVALS,
    /// Past the document.
    EXHAUSTED,
}

/// The anonymous `IntervalIterator` of `IntervalMatches.wrapMatches`.
struct WrappedMatches {
    mi: Box<dyn IntervalMatchesIterator>,
    doc: i32,
    state: State,
}

impl DocIdSetIterator for WrappedMatches {
    fn doc_id(&self) -> i32 {
        match self.state {
            State::UNPOSITIONED => -1,
            State::ITERATING | State::NO_MORE_INTERVALS => self.doc,
            State::EXHAUSTED => NO_MORE_DOCS,
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        match self.state {
            State::UNPOSITIONED => {
                self.state = State::ITERATING;
                return Ok(self.doc);
            }
            State::ITERATING | State::NO_MORE_INTERVALS => {
                self.state = State::EXHAUSTED;
            }
            State::EXHAUSTED => {}
        }
        Ok(NO_MORE_DOCS)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target == self.doc {
            self.state = State::ITERATING;
            return Ok(self.doc);
        }
        self.state = State::EXHAUSTED;
        Ok(NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        1
    }
}

impl IntervalIterator for WrappedMatches {
    fn start(&self) -> i32 {
        if self.state == State::NO_MORE_INTERVALS {
            return NO_MORE_INTERVALS;
        }
        debug_assert!(self.state == State::ITERATING);
        self.mi.start_position()
    }

    fn end(&self) -> i32 {
        if self.state == State::NO_MORE_INTERVALS {
            return NO_MORE_INTERVALS;
        }
        debug_assert!(self.state == State::ITERATING);
        self.mi.end_position()
    }

    fn gaps(&self) -> i32 {
        debug_assert!(self.state == State::ITERATING);
        self.mi.gaps()
    }

    fn width(&self) -> i32 {
        debug_assert!(self.state == State::ITERATING);
        self.mi.width()
    }

    fn next_interval(&mut self) -> Result<i32> {
        debug_assert!(self.state == State::ITERATING);
        if self.mi.next()? {
            return Ok(self.mi.start_position());
        }
        self.state = State::NO_MORE_INTERVALS;
        Ok(NO_MORE_INTERVALS)
    }

    fn match_cost(&self) -> f32 {
        1.0
    }
}
