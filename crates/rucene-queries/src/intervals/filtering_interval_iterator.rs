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

//! Port of `FilteringIntervalIterator.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use rucene::error::Result;

use crate::intervals::conjunction_interval_iterator::{
    conjunction_interval_iterator_disi, ConjunctionIntervalIterator,
};
use crate::intervals::interval_iterator::{
    IntervalIterator, SharedIntervalIterator, NO_MORE_INTERVALS,
};

/// The `nextInterval()` of a [`FilteringIntervalIterator`] subclass.
pub(crate) type FilteringNextInterval = fn(&mut FilteringIntervalIterator) -> Result<i32>;

/// A conjunction of two interval iterators whose intervals are those of `a`
/// that pass a filter against `b`.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.FilteringIntervalIterator`, whose
/// subclasses are anonymous classes of `OverlappingIntervalsSource`,
/// `ContainingIntervalsSource` and `ContainedByIntervalsSource` that only
/// implement `nextInterval()`; each is the [`FilteringNextInterval`] the
/// iterator is built with.
pub(crate) struct FilteringIntervalIterator {
    pub(crate) base: ConjunctionIntervalIterator,
    /// The package-private `a` field.
    pub(crate) a: SharedIntervalIterator,
    /// The package-private `b` field.
    pub(crate) b: SharedIntervalIterator,
    /// The package-private `bpos` field.
    pub(crate) bpos: bool,
    next_interval: FilteringNextInterval,
}

impl FilteringIntervalIterator {
    /// Equivalent to the protected
    /// `FilteringIntervalIterator(IntervalIterator, IntervalIterator)`, with
    /// the subclass's `nextInterval()`.
    ///
    /// # Errors
    ///
    /// The error of building the conjunction.
    pub(crate) fn new(
        a: Box<dyn IntervalIterator>,
        b: Box<dyn IntervalIterator>,
        next_interval: FilteringNextInterval,
    ) -> Result<Self> {
        let a = SharedIntervalIterator::new(a);
        let b = SharedIntervalIterator::new(b);
        Ok(Self {
            base: ConjunctionIntervalIterator::new(vec![a.clone(), b.clone()])?,
            a,
            b,
            bpos: false,
            next_interval,
        })
    }

    /// Equivalent to the protected `FilteringIntervalIterator.reset()`.
    ///
    /// Equivalent to the abstract `ConjunctionIntervalIterator.reset()` this subclass
    /// implements, which `conjunction_interval_iterator_disi!` calls on each document.
    fn reset(&mut self) -> Result<()> {
        self.bpos = self.b.next_interval()? != NO_MORE_INTERVALS;
        Ok(())
    }
}

conjunction_interval_iterator_disi!(FilteringIntervalIterator);

impl IntervalIterator for FilteringIntervalIterator {
    fn start(&self) -> i32 {
        if !self.bpos {
            return NO_MORE_INTERVALS;
        }
        self.a.start()
    }

    fn end(&self) -> i32 {
        if !self.bpos {
            return NO_MORE_INTERVALS;
        }
        self.a.end()
    }

    fn gaps(&self) -> i32 {
        self.a.gaps()
    }

    fn next_interval(&mut self) -> Result<i32> {
        (self.next_interval)(self)
    }

    fn match_cost(&self) -> f32 {
        self.base.match_cost()
    }
}
