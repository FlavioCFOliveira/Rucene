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

//! Port of `IntervalFilter.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;

use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};

/// The `accept()` of an [`IntervalFilter`] subclass: it receives the wrapped
/// iterator, Java's `in`, positioned on the candidate interval.
pub type IntervalAccept = Box<dyn FnMut(&dyn IntervalIterator) -> bool>;

/// Wraps an [`IntervalIterator`] and passes through those intervals that match
/// the `accept` function.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.IntervalFilter`.
///
/// **Divergence.** Java's subclasses override the abstract protected
/// `accept()` and read the protected `in` field; here the subclass is the
/// [`IntervalAccept`] the filter is built with, which receives `in`.
pub struct IntervalFilter {
    /// The protected `in` field.
    in_: Box<dyn IntervalIterator>,
    accept: IntervalAccept,
}

impl std::fmt::Debug for IntervalFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntervalFilter").finish_non_exhaustive()
    }
}

impl IntervalFilter {
    /// Creates a new filter.
    ///
    /// Equivalent to `IntervalFilter(IntervalIterator)`, with the subclass's
    /// `accept()`; Java's `requireNonNull` is the type itself.
    pub fn new(in_: Box<dyn IntervalIterator>, accept: IntervalAccept) -> Self {
        Self { in_, accept }
    }

    /// Returns `true` if the wrapped iterator's interval should be passed on.
    ///
    /// Equivalent to the abstract protected `IntervalFilter.accept()`.
    fn accept(&mut self) -> bool {
        (self.accept)(&*self.in_)
    }
}

impl DocIdSetIterator for IntervalFilter {
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

impl IntervalIterator for IntervalFilter {
    fn start(&self) -> i32 {
        self.in_.start()
    }

    fn end(&self) -> i32 {
        self.in_.end()
    }

    fn gaps(&self) -> i32 {
        self.in_.gaps()
    }

    fn match_cost(&self) -> f32 {
        self.in_.match_cost()
    }

    /// Equivalent to the final `IntervalFilter.nextInterval()`.
    fn next_interval(&mut self) -> Result<i32> {
        let mut next;
        loop {
            next = self.in_.next_interval()?;
            if !(next != NO_MORE_INTERVALS && !self.accept()) {
                break;
            }
        }
        Ok(next)
    }
}
