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

//! Port of `RelativeIterator.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use rucene::error::Result;

use crate::intervals::interval_iterator::IntervalIterator;

/// The base of the interval iterators that filter the intervals of `a` by
/// their relation to those of `b`, positioned on `a`'s documents.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.RelativeIterator`, whose fields are
/// this struct's and whose concrete members are its methods. A subclass holds
/// one as its `base` field and implements `nextInterval()`;
/// [`relative_iterator_impl!`] gives it every other member.
pub(crate) struct RelativeIterator {
    /// The package-private `a` field.
    pub(crate) a: Box<dyn IntervalIterator>,
    /// The package-private `b` field.
    pub(crate) b: Box<dyn IntervalIterator>,
    /// The package-private `bpos` field.
    pub(crate) bpos: bool,
}

impl std::fmt::Debug for RelativeIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelativeIterator")
            .field("bpos", &self.bpos)
            .finish_non_exhaustive()
    }
}

impl RelativeIterator {
    /// Equivalent to `RelativeIterator(IntervalIterator, IntervalIterator)`.
    pub(crate) fn new(a: Box<dyn IntervalIterator>, b: Box<dyn IntervalIterator>) -> Self {
        Self { a, b, bpos: false }
    }

    /// Equivalent to `RelativeIterator.docID()`.
    pub(crate) fn doc_id(&self) -> i32 {
        self.a.doc_id()
    }

    /// Equivalent to `RelativeIterator.nextDoc()`.
    pub(crate) fn next_doc(&mut self) -> Result<i32> {
        let doc = self.a.next_doc()?;
        self.reset()?;
        Ok(doc)
    }

    /// Equivalent to `RelativeIterator.advance(int)`.
    pub(crate) fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.a.advance(target)?;
        self.reset()?;
        Ok(doc)
    }

    /// Equivalent to `RelativeIterator.cost()`.
    pub(crate) fn cost(&self) -> i64 {
        self.a.cost()
    }

    /// Equivalent to the protected `RelativeIterator.reset()`.
    pub(crate) fn reset(&mut self) -> Result<()> {
        let doc = self.a.doc_id();
        self.bpos =
            self.b.doc_id() == doc || (self.b.doc_id() < doc && self.b.advance(doc)? == doc);
        Ok(())
    }

    /// Equivalent to `RelativeIterator.start()`.
    pub(crate) fn start(&self) -> i32 {
        self.a.start()
    }

    /// Equivalent to `RelativeIterator.end()`.
    pub(crate) fn end(&self) -> i32 {
        self.a.end()
    }

    /// Equivalent to `RelativeIterator.gaps()`.
    pub(crate) fn gaps(&self) -> i32 {
        self.a.gaps()
    }

    /// Equivalent to `RelativeIterator.matchCost()`.
    pub(crate) fn match_cost(&self) -> f32 {
        self.a.match_cost() + self.b.match_cost()
    }
}

/// Implements [`DocIdSetIterator`](rucene::search::doc_id_set_iterator::DocIdSetIterator)
/// and [`IntervalIterator`] for a subclass of [`RelativeIterator`] held in its
/// `base` field, whose own `next_interval_impl(&mut RelativeIterator)` is the
/// subclass's `nextInterval()`.
macro_rules! relative_iterator_impl {
    ($ty:ty) => {
        impl ::rucene::search::doc_id_set_iterator::DocIdSetIterator for $ty {
            fn doc_id(&self) -> i32 {
                self.base.doc_id()
            }

            fn next_doc(&mut self) -> ::rucene::error::Result<i32> {
                self.base.next_doc()
            }

            fn advance(&mut self, target: i32) -> ::rucene::error::Result<i32> {
                self.base.advance(target)
            }

            fn cost(&self) -> i64 {
                self.base.cost()
            }
        }

        impl $crate::intervals::interval_iterator::IntervalIterator for $ty {
            fn start(&self) -> i32 {
                self.base.start()
            }

            fn end(&self) -> i32 {
                self.base.end()
            }

            fn gaps(&self) -> i32 {
                self.base.gaps()
            }

            fn next_interval(&mut self) -> ::rucene::error::Result<i32> {
                Self::next_interval_impl(&mut self.base)
            }

            fn match_cost(&self) -> f32 {
                self.base.match_cost()
            }
        }
    };
}
pub(crate) use relative_iterator_impl;
