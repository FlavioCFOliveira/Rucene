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

//! Port of `SpanScorer.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::error::{LuceneError, Result};
use rucene::index::NumericDocValues;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::sim_scorer_source::SharedSimScorer;
use rucene::search::two_phase_iterator::TwoPhaseIterator;

use crate::spans::spans::{Spans, NO_MORE_POSITIONS};

/// A basic [`Scorer`] over [`Spans`].
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanScorer`.
///
/// The spans type is a parameter, `Box<dyn Spans>` by default, so that a
/// scorer that extends this one — `PayloadScoreQuery`'s `PayloadSpanScorer` —
/// reaches the concrete spans it built, which Java keeps in a second field of
/// the subclass.
pub struct SpanScorer<S: Spans = Box<dyn Spans>> {
    /// The spans of this scorer; the protected `spans` field.
    spans: S,
    /// The similarity scorer, `None` when scores are not needed; the protected
    /// `scorer` field.
    scorer: Option<SharedSimScorer>,
    /// The norms of the field, if any; the protected `norms` field.
    norms: Option<Box<dyn NumericDocValues>>,
    /// Accumulated sloppy freq (computed in
    /// [`set_freq_current_doc`](Self::set_freq_current_doc)).
    freq: f32,
    /// Last doc we called [`set_freq_current_doc`](Self::set_freq_current_doc)
    /// for.
    last_scored_doc: i32,
}

impl<S: Spans> std::fmt::Debug for SpanScorer<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanScorer")
            .field("spans", &self.spans.java_to_string())
            .field("freq", &self.freq)
            .field("last_scored_doc", &self.last_scored_doc)
            .finish_non_exhaustive()
    }
}

impl<S: Spans> SpanScorer<S> {
    /// Sole constructor.
    ///
    /// Equivalent to `SpanScorer(Spans, SimScorer, NumericDocValues)`.
    pub fn new(
        spans: S,
        scorer: Option<SharedSimScorer>,
        norms: Option<Box<dyn NumericDocValues>>,
    ) -> Self {
        Self {
            spans,
            scorer,
            norms,
            freq: 0.0,
            last_scored_doc: -1,
        }
    }

    /// Returns the spans of this scorer.
    ///
    /// Equivalent to `SpanScorer.getSpans()`.
    pub fn get_spans(&self) -> &S {
        &self.spans
    }

    /// Returns the spans of this scorer, mutably.
    pub fn get_spans_mut(&mut self) -> &mut S {
        &mut self.spans
    }

    /// Scores the current doc with the similarity, using the slop-adjusted
    /// frequency.
    ///
    /// Equivalent to the protected `SpanScorer.scoreCurrentDoc()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the norm, and the
    /// `NullPointerException` Java raises without a similarity scorer.
    pub fn score_current_doc(&mut self) -> Result<f32> {
        debug_assert!(self.scorer.is_some(), "SpanScorer has a null docScorer!");
        let mut norm = 1i64;
        let doc = self.doc_id();
        if let Some(norms) = self.norms.as_mut() {
            if norms.advance_exact(doc)? {
                norm = norms.long_value()?;
            }
        }
        let scorer = self.scorer.as_ref().ok_or_else(LuceneError::null_pointer)?;
        Ok(scorer.score(self.freq, norm))
    }

    /// Sets the frequency for the current document.
    ///
    /// Equivalent to the protected final `SpanScorer.setFreqCurrentDoc()`. This
    /// is called at most once per document.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading positions.
    pub fn set_freq_current_doc(&mut self) -> Result<()> {
        self.freq = 0.0;

        self.spans.do_start_current_doc()?;

        debug_assert!(
            self.spans.start_position() == -1,
            "incorrect initial start position, {}",
            self.spans.java_to_string()
        );
        debug_assert!(
            self.spans.end_position() == -1,
            "incorrect initial end position, {}",
            self.spans.java_to_string()
        );
        let mut prev_start_pos = -1;
        let mut prev_end_pos = -1;

        let mut start_pos = self.spans.next_start_position()?;
        debug_assert!(
            start_pos != NO_MORE_POSITIONS,
            "initial startPos NO_MORE_POSITIONS, {}",
            self.spans.java_to_string()
        );
        loop {
            debug_assert!(start_pos >= prev_start_pos);
            let end_pos = self.spans.end_position();
            debug_assert!(end_pos != NO_MORE_POSITIONS);
            // This assertion can fail for Or spans on the same term:
            // assert (startPos != prevStartPos) || (endPos > prevEndPos) : "non increased
            // endPos="+endPos;
            debug_assert!(
                (start_pos != prev_start_pos) || (end_pos >= prev_end_pos),
                "decreased endPos={end_pos}"
            );
            if self.scorer.is_none() {
                // scores not required, break out here
                self.freq = 1.0;
                return Ok(());
            }
            // `freq += (1.0 / (1.0 + spans.width()))`: a compound assignment of
            // a double to a float field, `freq = (float) (freq + ...)`.
            self.freq =
                (f64::from(self.freq) + (1.0 / (1.0 + f64::from(self.spans.width())))) as f32;
            self.spans.do_current_spans()?;
            prev_start_pos = start_pos;
            prev_end_pos = end_pos;
            start_pos = self.spans.next_start_position()?;
            if start_pos == NO_MORE_POSITIONS {
                break;
            }
        }

        debug_assert!(
            self.spans.start_position() == NO_MORE_POSITIONS,
            "incorrect final start position, {}",
            self.spans.java_to_string()
        );
        debug_assert!(
            self.spans.end_position() == NO_MORE_POSITIONS,
            "incorrect final end position, {}",
            self.spans.java_to_string()
        );
        Ok(())
    }

    /// Ensures [`set_freq_current_doc`](Self::set_freq_current_doc) is called,
    /// if not already called for the current doc.
    ///
    /// Equivalent to the private `SpanScorer.ensureFreq()`; it is visible to
    /// the crate because `PayloadSpanScorer`, which Java derives from this
    /// class, reaches it through the inherited final `score()`.
    pub(crate) fn ensure_freq(&mut self) -> Result<()> {
        let current_doc = self.doc_id();
        if self.last_scored_doc != current_doc {
            self.set_freq_current_doc()?;
            self.last_scored_doc = current_doc;
        }
        Ok(())
    }

    /// Returns the intermediate "sloppy freq" adjusted for edit distance.
    ///
    /// Equivalent to the package-private final
    /// `SpanScorer.sloppyFreq()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading positions.
    pub fn sloppy_freq(&mut self) -> Result<f32> {
        self.ensure_freq()?;
        Ok(self.freq)
    }

    /// The current doc ID; `SpanScorer.docID()`.
    pub fn doc_id(&self) -> i32 {
        self.spans.doc_id()
    }
}

impl<S: Spans> Scorable for SpanScorer<S> {
    /// Equivalent to the final `SpanScorer.score()`.
    fn score(&mut self) -> Result<f32> {
        self.ensure_freq()?;
        self.score_current_doc()
    }
}

impl<S: Spans> Scorer for SpanScorer<S> {
    fn doc_id(&self) -> i32 {
        self.spans.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.spans
    }

    fn iterator_ref(&self) -> Option<&dyn DocIdSetIterator> {
        Some(&self.spans)
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.spans.as_two_phase_iterator()
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}
