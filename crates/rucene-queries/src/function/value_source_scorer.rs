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

//! Port of `ValueSourceScorer.java`, of the Java package
//! `org.apache.lucene.queries.function`.

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::{
    self, AllDocIdSetIterator, DocIdSetIterator, NO_MORE_DOCS,
};
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::two_phase_iterator::TwoPhaseIterator;

use crate::function::function_values::FunctionValues;

/// Fixed cost for a single iteration of the two-phase iterator.
///
/// Equivalent to the private `ValueSourceScorer.DEF_COST`.
const DEF_COST: i32 = 5;

/// What a [`ValueSourceScorer`] subclass decides: which documents match, and
/// at what cost.
///
/// Equivalent to the overridable members of the abstract
/// `ValueSourceScorer`: its abstract `matches(int)` and its protected
/// `matchCost()`. Every Java subclass of `ValueSourceScorer` is anonymous and
/// overrides only these two; it reads the values through the `values` field
/// it inherits, which the methods receive as a parameter.
pub trait ValueSourceScorerMatcher {
    /// Override to decide if this document matches. It is called by
    /// [`TwoPhaseIterator::matches`].
    ///
    /// Equivalent to the abstract `ValueSourceScorer.matches(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    fn matches(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<bool>;

    /// Cost evaluation function which defines the cost of access for the
    /// two-phase iterator of the scorer.
    ///
    /// Equivalent to the protected `ValueSourceScorer.matchCost()`:
    /// the fixed cost plus the cost exposed by the delegated values.
    fn match_cost(&self, values: &dyn FunctionValues) -> f32 {
        DEF_COST as f32 + values.cost()
    }
}

/// A [`Scorer`] which returns the result of `FunctionValues::float_val` as the
/// score for a document, and which filters out documents that do not match
/// [`matches`](Self::matches). This scorer has a [`TwoPhaseIterator`].
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.ValueSourceScorer`. It is similar to
/// `FunctionQuery`, with an added filter. If the scores are needed, the
/// underlying value will probably be fetched or computed twice — once to
/// filter and next to return the score.
///
/// **Divergence.** Java's subclasses override `matches(int)` and
/// `matchCost()`; here a subclass is the [`ValueSourceScorerMatcher`] the
/// scorer is built with (see that trait). Java keeps the two-phase iterator
/// and its `asDocIdSetIterator` view as two fields over one approximation;
/// this scorer is both views itself, with the same behaviour.
pub struct ValueSourceScorer<'a> {
    /// The protected `values` field.
    values: Box<dyn FunctionValues + 'a>,
    /// The subclass's `matches`/`matchCost`.
    matcher: Box<dyn ValueSourceScorerMatcher + 'a>,
    /// `DocIdSetIterator.all(readerContext.reader().maxDoc())`: no
    /// approximation.
    approximation: AllDocIdSetIterator,
}

impl std::fmt::Debug for ValueSourceScorer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValueSourceScorer")
            .field("doc", &self.approximation.doc_id())
            .finish_non_exhaustive()
    }
}

impl<'a> ValueSourceScorer<'a> {
    /// Creates the scorer over the values of a leaf.
    ///
    /// Equivalent to the protected
    /// `ValueSourceScorer(LeafReaderContext, FunctionValues)`, with the
    /// subclass behaviour supplied as `matcher`.
    ///
    /// # Errors
    ///
    /// Propagates the error of building the all-documents approximation.
    pub fn new(
        reader_context: &LeafReaderContext,
        values: Box<dyn FunctionValues + 'a>,
        matcher: Box<dyn ValueSourceScorerMatcher + 'a>,
    ) -> Result<Self> {
        let approximation = doc_id_set_iterator::all(reader_context.leaf_reader().max_doc())?;
        Ok(Self {
            values,
            matcher,
            approximation,
        })
    }

    /// Decides if this document matches.
    ///
    /// Equivalent to `ValueSourceScorer.matches(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    pub fn matches(&mut self, doc: i32) -> Result<bool> {
        self.matcher.matches(&mut *self.values, doc)
    }

    /// The match cost of the two-phase iterator.
    ///
    /// Equivalent to the protected `ValueSourceScorer.matchCost()`.
    pub fn match_cost(&self) -> f32 {
        self.matcher.match_cost(&*self.values)
    }

    /// The values the scorer reads.
    ///
    /// Equivalent to the protected `values` field.
    pub fn values(&mut self) -> &mut (dyn FunctionValues + 'a) {
        &mut *self.values
    }

    /// Confirms the candidates of the approximation, which is what
    /// `TwoPhaseIterator.asDocIdSetIterator(twoPhaseIterator)` does for the
    /// `disi` field.
    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if TwoPhaseIterator::matches(self)? {
                return Ok(doc);
            }
            doc = self.approximation.next_doc()?;
        }
    }
}

impl DocIdSetIterator for ValueSourceScorer<'_> {
    fn doc_id(&self) -> i32 {
        self.approximation.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.approximation.next_doc()?;
        self.confirm(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.approximation.advance(target)?;
        self.confirm(doc)
    }

    fn cost(&self) -> i64 {
        self.approximation.cost()
    }

    fn wraps_two_phase_iterator(&self) -> bool {
        true
    }
}

impl TwoPhaseIterator for ValueSourceScorer<'_> {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.approximation.doc_id();
        ValueSourceScorer::matches(self, doc)
    }

    fn match_cost(&self) -> f32 {
        ValueSourceScorer::match_cost(self)
    }
}

impl Scorable for ValueSourceScorer<'_> {
    /// Equivalent to `ValueSourceScorer.score()`: the value, with `NaN` and
    /// `-Infinity` mapped to `-Float.MAX_VALUE`, since priority queues cannot
    /// handle them.
    fn score(&mut self) -> Result<f32> {
        // (same as FunctionQuery, but no qWeight)
        let doc = self.approximation.doc_id();
        let score = self.values.float_val(doc)?;
        // This conditional handles both -infinity and NaN since comparisons
        // with NaN are always false.
        Ok(if score > f32::NEG_INFINITY {
            score
        } else {
            -f32::MAX
        })
    }
}

impl Scorer for ValueSourceScorer<'_> {
    fn doc_id(&self) -> i32 {
        self.approximation.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(self)
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}
