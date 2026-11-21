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

//! Port of `ConjunctionIntervalIterator.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use rucene::error::Result;
use rucene::search::conjunction_utils::ConjunctionUtils;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;

use crate::intervals::interval_iterator::{IntervalIterator, SharedIntervalIterator};

/// The base of the interval iterators that intersect their sub-iterators.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.ConjunctionIntervalIterator`, whose
/// fields are this struct's. A subclass holds one as its `base` field and
/// implements `reset()`; [`conjunction_interval_iterator_disi!`] gives it the
/// `docID`, `nextDoc`, `advance` and `cost` of the class, and its
/// `IntervalIterator` implementation answers the final `matchCost()` with
/// [`match_cost`](Self::match_cost).
///
/// **Divergence.** The approximation intersects the sub-iterators, which the
/// subclass also reads; both reach them through [`SharedIntervalIterator`]
/// handles (see the `interval_iterator` module).
pub(crate) struct ConjunctionIntervalIterator {
    /// `ConjunctionUtils.intersectIterators(subIterators)`.
    pub(crate) approximation: Box<dyn DocIdSetIterator>,
    /// The package-private `subIterators` field.
    pub(crate) sub_iterators: Vec<SharedIntervalIterator>,
    /// The package-private `cost` field: the sum of the sub-iterators' match
    /// costs.
    pub(crate) cost: f32,
}

impl std::fmt::Debug for ConjunctionIntervalIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConjunctionIntervalIterator")
            .field("sub_iterators", &self.sub_iterators.len())
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl ConjunctionIntervalIterator {
    /// Equivalent to `ConjunctionIntervalIterator(List<IntervalIterator>)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `ConjunctionUtils.intersectIterators`
    /// for fewer than two sub-iterators.
    pub(crate) fn new(sub_iterators: Vec<SharedIntervalIterator>) -> Result<Self> {
        let iterators: Vec<Box<dyn DocIdSetIterator>> = sub_iterators
            .iter()
            .map(|it| Box::new(it.clone()) as Box<dyn DocIdSetIterator>)
            .collect();
        let approximation =
            ConjunctionUtils::intersect_iterators(iterators)?.into_doc_id_set_iterator();
        let mut costsum = 0f32;
        for it in &sub_iterators {
            costsum += it.match_cost();
        }
        Ok(Self {
            approximation,
            sub_iterators,
            cost: costsum,
        })
    }

    /// Equivalent to the final `ConjunctionIntervalIterator.matchCost()`.
    pub(crate) fn match_cost(&self) -> f32 {
        self.cost
    }
}

/// Implements [`DocIdSetIterator`] for a subclass of
/// [`ConjunctionIntervalIterator`] held in its `base` field: the class's
/// `docID()`, `nextDoc()` and `advance(int)` — which call the subclass's
/// `reset()` when a document is reached — and `cost()`.
macro_rules! conjunction_interval_iterator_disi {
    ($ty:ty) => {
        impl ::rucene::search::doc_id_set_iterator::DocIdSetIterator for $ty {
            fn doc_id(&self) -> i32 {
                self.base.approximation.doc_id()
            }

            fn next_doc(&mut self) -> ::rucene::error::Result<i32> {
                let doc = self.base.approximation.next_doc()?;
                if doc != ::rucene::search::doc_id_set_iterator::NO_MORE_DOCS {
                    self.reset()?;
                }
                Ok(doc)
            }

            fn advance(&mut self, target: i32) -> ::rucene::error::Result<i32> {
                let doc = self.base.approximation.advance(target)?;
                if doc != ::rucene::search::doc_id_set_iterator::NO_MORE_DOCS {
                    self.reset()?;
                }
                Ok(doc)
            }

            fn cost(&self) -> i64 {
                self.base.approximation.cost()
            }
        }
    };
}
pub(crate) use conjunction_interval_iterator_disi;
