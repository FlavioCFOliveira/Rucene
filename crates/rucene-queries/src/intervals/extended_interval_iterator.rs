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

//! Port of `ExtendedIntervalIterator.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;

use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};

/// Wraps an interval iterator and extends the bounds of its intervals.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.ExtendedIntervalIterator`. Useful for
/// specifying gaps in an ordered iterator: to match `a b [2 spaces] c`, search
/// for `phrase(a, extended(b, 0, 2), c)`. An interval with prefix bounds
/// extended by `n` will skip over matches that appear in positions lower than
/// `n`.
pub(crate) struct ExtendedIntervalIterator {
    in_: Box<dyn IntervalIterator>,
    before: i32,
    after: i32,
    positioned: bool,
}

impl ExtendedIntervalIterator {
    /// Creates a new iterator extending `in_` by `before` positions before
    /// and `after` positions beyond each interval.
    ///
    /// Equivalent to `ExtendedIntervalIterator(IntervalIterator, int, int)`.
    pub(crate) fn new(in_: Box<dyn IntervalIterator>, before: i32, after: i32) -> Self {
        Self {
            in_,
            before,
            after,
            positioned: false,
        }
    }
}

impl DocIdSetIterator for ExtendedIntervalIterator {
    fn doc_id(&self) -> i32 {
        self.in_.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.positioned = false;
        self.in_.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.positioned = false;
        self.in_.advance(target)
    }

    fn cost(&self) -> i64 {
        self.in_.cost()
    }
}

impl IntervalIterator for ExtendedIntervalIterator {
    fn start(&self) -> i32 {
        if !self.positioned {
            return -1;
        }
        let start = self.in_.start();
        if start == NO_MORE_INTERVALS {
            return NO_MORE_INTERVALS;
        }
        0.max(start.wrapping_sub(self.before))
    }

    fn end(&self) -> i32 {
        if !self.positioned {
            return -1;
        }
        let mut end = self.in_.end();
        if end == NO_MORE_INTERVALS {
            return NO_MORE_INTERVALS;
        }
        end = end.wrapping_add(self.after);
        if end < 0 || end == NO_MORE_INTERVALS {
            // overflow
            end = NO_MORE_INTERVALS - 1;
        }
        end
    }

    fn gaps(&self) -> i32 {
        self.in_.gaps()
    }

    fn next_interval(&mut self) -> Result<i32> {
        self.positioned = true;
        self.in_.next_interval()?;
        Ok(self.start())
    }

    fn match_cost(&self) -> f32 {
        self.in_.match_cost()
    }
}
