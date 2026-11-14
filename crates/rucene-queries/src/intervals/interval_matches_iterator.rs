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

//! Port of `IntervalMatchesIterator.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;

/// An extension of [`MatchesIterator`] that allows it to be treated as an
/// `IntervalIterator`.
///
/// Equivalent to the interface
/// `org.apache.lucene.queries.intervals.IntervalMatchesIterator`. This is
/// necessary to get access to the gaps and the width of a match when
/// constructing matches.
pub trait IntervalMatchesIterator: MatchesIterator {
    /// The number of top-level gaps inside the current match.
    ///
    /// Equivalent to `IntervalMatchesIterator.gaps()`.
    fn gaps(&self) -> i32;

    /// The width of the current match.
    ///
    /// Equivalent to `IntervalMatchesIterator.width()`.
    fn width(&self) -> i32;
}

/// An owned matches iterator, adapted to be shared.
struct BoxedIntervalMatchesIterator(Box<dyn IntervalMatchesIterator>);

impl MatchesIterator for BoxedIntervalMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.0.next()
    }

    fn start_position(&self) -> i32 {
        self.0.start_position()
    }

    fn end_position(&self) -> i32 {
        self.0.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.0.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.0.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.0.get_sub_matches()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.0.get_query()
    }
}

impl IntervalMatchesIterator for BoxedIntervalMatchesIterator {
    fn gaps(&self) -> i32 {
        self.0.gaps()
    }

    fn width(&self) -> i32 {
        self.0.width()
    }
}

/// A handle on a matches iterator that several holders reach, as Java's
/// references do: the interval iterator `IntervalMatches.wrapMatches` builds
/// over it, and the matches iterator that reads its offsets and sub-matches.
///
/// **This has no counterpart in Lucene 10.5.0.** Every call borrows the
/// shared iterator for its duration only and forwards to it.
#[derive(Clone)]
pub(crate) struct SharedIntervalMatchesIterator(Rc<RefCell<dyn IntervalMatchesIterator>>);

impl std::fmt::Debug for SharedIntervalMatchesIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedIntervalMatchesIterator")
    }
}

impl SharedIntervalMatchesIterator {
    /// Shares an owned iterator.
    pub(crate) fn new(iterator: Box<dyn IntervalMatchesIterator>) -> Self {
        Self(Rc::new(RefCell::new(BoxedIntervalMatchesIterator(
            iterator,
        ))))
    }

    /// A handle on an iterator already shared under its concrete type.
    pub(crate) fn from_rc(iterator: Rc<RefCell<dyn IntervalMatchesIterator>>) -> Self {
        Self(iterator)
    }
}

impl MatchesIterator for SharedIntervalMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.0.borrow_mut().next()
    }

    fn start_position(&self) -> i32 {
        self.0.borrow().start_position()
    }

    fn end_position(&self) -> i32 {
        self.0.borrow().end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.0.borrow().start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.0.borrow().end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.0.borrow().get_sub_matches()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.0.borrow().get_query()
    }
}

impl IntervalMatchesIterator for SharedIntervalMatchesIterator {
    fn gaps(&self) -> i32 {
        self.0.borrow().gaps()
    }

    fn width(&self) -> i32 {
        self.0.borrow().width()
    }
}
