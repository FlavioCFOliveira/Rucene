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

//! Port of `IntervalScorer.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::similarities::SimScorer;
use rucene::search::two_phase_iterator::TwoPhaseIterator;

use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_score_function::IntervalScoreFunction;

/// The scorer of an `IntervalQuery`: a function of the sloppy frequency of
/// the intervals in each document.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.IntervalScorer`. Java creates a new
/// two-phase iterator over the intervals on each `twoPhaseIterator()` call and
/// `iterator()` is its `asDocIdSetIterator` view; this scorer is both views
/// itself, with the same behaviour.
pub(crate) struct IntervalScorer {
    intervals: Box<dyn IntervalIterator>,
    sim_scorer: Box<dyn SimScorer + Send + Sync>,
    boost: f32,
    min_extent: i32,
    freq: f32,
    last_scored_doc: i32,
}

impl std::fmt::Debug for IntervalScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntervalScorer")
            .field("boost", &self.boost)
            .field("min_extent", &self.min_extent)
            .finish_non_exhaustive()
    }
}

impl IntervalScorer {
    /// Equivalent to `IntervalScorer(IntervalIterator, int, float,
    /// IntervalScoreFunction)`.
    pub(crate) fn new(
        intervals: Box<dyn IntervalIterator>,
        min_extent: i32,
        boost: f32,
        score_function: &dyn IntervalScoreFunction,
    ) -> Self {
        Self {
            intervals,
            sim_scorer: score_function.scorer(boost),
            boost,
            min_extent,
            freq: 0.0,
            last_scored_doc: -1,
        }
    }

    /// The sloppy frequency of the current document.
    ///
    /// Equivalent to the package-private `IntervalScorer.freq()`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the intervals.
    pub(crate) fn freq(&mut self) -> Result<f32> {
        self.ensure_freq()?;
        Ok(self.freq)
    }

    /// Equivalent to the private `IntervalScorer.ensureFreq()`: each interval
    /// adds `1.0 / max(length - minExtent + 1, 1)`, computed in `double` and
    /// stored into the `float` sum.
    fn ensure_freq(&mut self) -> Result<()> {
        let doc = self.intervals.doc_id();
        if self.last_scored_doc != doc {
            self.last_scored_doc = doc;
            self.freq = 0.0;
            loop {
                let length = self
                    .intervals
                    .end()
                    .wrapping_sub(self.intervals.start())
                    .wrapping_add(1);
                let denominator = length.wrapping_sub(self.min_extent).wrapping_add(1).max(1);
                self.freq = (f64::from(self.freq) + 1.0 / f64::from(denominator)) as f32;
                if self.intervals.next_interval()? == NO_MORE_INTERVALS {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Confirms the candidates of the approximation, which is what
    /// `TwoPhaseIterator.asDocIdSetIterator(twoPhaseIterator())` does.
    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if TwoPhaseIterator::matches(self)? {
                return Ok(doc);
            }
            doc = self.intervals.next_doc()?;
        }
    }
}

impl DocIdSetIterator for IntervalScorer {
    fn doc_id(&self) -> i32 {
        self.intervals.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.intervals.next_doc()?;
        self.confirm(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.intervals.advance(target)?;
        self.confirm(doc)
    }

    fn cost(&self) -> i64 {
        self.intervals.cost()
    }

    fn wraps_two_phase_iterator(&self) -> bool {
        true
    }
}

impl TwoPhaseIterator for IntervalScorer {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.intervals
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &*self.intervals
    }

    fn matches(&mut self) -> Result<bool> {
        Ok(self.intervals.next_interval()? != NO_MORE_INTERVALS)
    }

    fn match_cost(&self) -> f32 {
        self.intervals.match_cost()
    }
}

impl Scorable for IntervalScorer {
    fn score(&mut self) -> Result<f32> {
        self.ensure_freq()?;
        Ok(self.sim_scorer.score(self.freq, 1))
    }
}

impl Scorer for IntervalScorer {
    fn doc_id(&self) -> i32 {
        self.intervals.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(self)
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(self.boost)
    }
}
