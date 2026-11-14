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

//! Port of `IntervalIterator.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.
//!
//! # Shared iterators
//!
//! Several interval iterators reach one sub-iterator through two references:
//! a conjunction intersects its sub-iterators through `ConjunctionUtils` and
//! reads their intervals itself, a disjunction keeps them in a `DisiWrapper`
//! and in a priority queue. Rust cannot alias them, so the sub-iterator is
//! shared through a `SharedIntervalIterator`, whose clones are handles on
//! the very same iterator, as Java's references are.

use std::cell::RefCell;
use std::rc::Rc;

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;

/// When returned from [`IntervalIterator::next_interval`], indicates that
/// there are no more matching intervals on the current document.
///
/// Equivalent to `IntervalIterator.NO_MORE_INTERVALS`.
pub const NO_MORE_INTERVALS: i32 = i32::MAX;

/// A [`DocIdSetIterator`] that also allows iteration over matching intervals
/// in a document.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.IntervalIterator`.
///
/// Once the iterator is positioned on a document by calling
/// [`advance`](DocIdSetIterator::advance) or
/// [`next_doc`](DocIdSetIterator::next_doc), intervals may be retrieved by
/// calling [`next_interval`](Self::next_interval) until
/// [`NO_MORE_INTERVALS`] is returned. The limits of the current interval are
/// returned by [`start`](Self::start) and [`end`](Self::end). When the
/// iterator has been moved to a new document, but before `next_interval` has
/// been called, both return `-1`. It is possible for a document to return
/// [`NO_MORE_INTERVALS`] on the first call to `next_interval`.
pub trait IntervalIterator: DocIdSetIterator {
    /// The start of the current interval.
    ///
    /// Equivalent to the abstract `IntervalIterator.start()`: `-1` if
    /// `next_interval` has not yet been called and [`NO_MORE_INTERVALS`] once
    /// the iterator is exhausted.
    fn start(&self) -> i32;

    /// The end of the current interval.
    ///
    /// Equivalent to the abstract `IntervalIterator.end()`.
    fn end(&self) -> i32;

    /// The number of gaps within the current interval.
    ///
    /// Equivalent to the abstract `IntervalIterator.gaps()`: the number of
    /// gaps between the immediate sub-intervals of this interval, not
    /// including the gaps inside those sub-intervals. It should not be called
    /// before `next_interval`, or after it has returned
    /// [`NO_MORE_INTERVALS`].
    fn gaps(&self) -> i32;

    /// The width of the current interval.
    ///
    /// Equivalent to `IntervalIterator.width()`: `end() - start() + 1`.
    fn width(&self) -> i32 {
        self.end().wrapping_sub(self.start()).wrapping_add(1)
    }

    /// Advances the iterator to the next interval, returning its start or
    /// [`NO_MORE_INTERVALS`] if there are no more intervals on the current
    /// document.
    ///
    /// Equivalent to the abstract `IntervalIterator.nextInterval()`. It should
    /// not be called after `NO_MORE_DOCS` is returned by `next_doc` or
    /// `advance`; after `NO_MORE_INTERVALS` it may be called again.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading positions.
    fn next_interval(&mut self) -> Result<i32>;

    /// An indication of the average cost of iterating over all intervals in a
    /// document.
    ///
    /// Equivalent to the abstract `IntervalIterator.matchCost()`.
    fn match_cost(&self) -> f32;

    /// Equivalent to `IntervalIterator.toString()`:
    /// `docID() + ":[" + start() + "->" + end() + "]"`.
    fn interval_to_string(&self) -> String {
        format!("{}:[{}->{}]", self.doc_id(), self.start(), self.end())
    }
}

/// An owned interval iterator, adapted to be shared.
struct BoxedIntervalIterator(Box<dyn IntervalIterator>);

impl DocIdSetIterator for BoxedIntervalIterator {
    fn doc_id(&self) -> i32 {
        self.0.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.0.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.advance(target)
    }

    fn cost(&self) -> i64 {
        self.0.cost()
    }
}

impl IntervalIterator for BoxedIntervalIterator {
    fn start(&self) -> i32 {
        self.0.start()
    }

    fn end(&self) -> i32 {
        self.0.end()
    }

    fn gaps(&self) -> i32 {
        self.0.gaps()
    }

    fn width(&self) -> i32 {
        self.0.width()
    }

    fn next_interval(&mut self) -> Result<i32> {
        self.0.next_interval()
    }

    fn match_cost(&self) -> f32 {
        self.0.match_cost()
    }

    fn interval_to_string(&self) -> String {
        self.0.interval_to_string()
    }
}

/// A handle on an interval iterator that several holders reach, as Java's
/// references do; see the module documentation.
///
/// **This has no counterpart in Lucene 10.5.0.** Every call borrows the
/// shared iterator for its duration only and forwards to it, `width` and the
/// string rendering included, so a holder sees exactly the iterator Java's
/// reference sees.
#[derive(Clone)]
pub(crate) struct SharedIntervalIterator(Rc<RefCell<dyn IntervalIterator>>);

impl std::fmt::Debug for SharedIntervalIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedIntervalIterator")
    }
}

impl SharedIntervalIterator {
    /// Shares an owned iterator.
    pub(crate) fn new(iterator: Box<dyn IntervalIterator>) -> Self {
        Self(Rc::new(RefCell::new(BoxedIntervalIterator(iterator))))
    }

    /// Whether the two handles reach the same iterator: Java's `==` on
    /// interval iterators.
    pub(crate) fn ptr_eq(&self, other: &SharedIntervalIterator) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl DocIdSetIterator for SharedIntervalIterator {
    fn doc_id(&self) -> i32 {
        self.0.borrow().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.0.borrow_mut().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.borrow_mut().advance(target)
    }

    fn cost(&self) -> i64 {
        self.0.borrow().cost()
    }
}

impl IntervalIterator for SharedIntervalIterator {
    fn start(&self) -> i32 {
        self.0.borrow().start()
    }

    fn end(&self) -> i32 {
        self.0.borrow().end()
    }

    fn gaps(&self) -> i32 {
        self.0.borrow().gaps()
    }

    fn width(&self) -> i32 {
        self.0.borrow().width()
    }

    fn next_interval(&mut self) -> Result<i32> {
        self.0.borrow_mut().next_interval()
    }

    fn match_cost(&self) -> f32 {
        self.0.borrow().match_cost()
    }

    fn interval_to_string(&self) -> String {
        self.0.borrow().interval_to_string()
    }
}
