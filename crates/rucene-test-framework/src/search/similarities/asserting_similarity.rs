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

//! Port of `org.apache.lucene.tests.search.similarities.AssertingSimilarity`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::FieldInvertState;
use rucene::search::similarities::{
    BulkSimScorer, CollectionStatistics, Explanation, SimScorer, Similarity, TermStatistics,
};

/// Wraps a [`Similarity`] and checks the inputs and the results of its norms,
/// scores and explanations.
///
/// Equivalent to
/// `org.apache.lucene.tests.search.similarities.AssertingSimilarity`.
///
/// The checks of methods that cannot fail in Rust ([`SimScorer::score`],
/// [`Similarity::scorer`], …) fail by panicking with Java's
/// `AssertionError`.
///
/// **Adaptation.** Java's `toString()` renders the delegate's `toString()`;
/// Rust's [`Similarity`] has no string form, so the constructor takes the
/// delegate's rendering as a function (the delegate's [`fmt::Display`] for
/// [`new`](Self::new)), read each time this similarity is rendered.
pub struct AssertingSimilarity {
    /// `delegate`.
    delegate: Arc<dyn Similarity>,
    /// `delegate.toString()`.
    delegate_to_string: Arc<dyn Fn() -> String + Send + Sync>,
}

impl fmt::Debug for AssertingSimilarity {
    /// Starts with the type name: the core's explanations read a similarity's
    /// simple class name from its `Debug` rendering.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingSimilarity({self})")
    }
}

impl fmt::Display for AssertingSimilarity {
    /// `"Asserting(" + delegate + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Asserting({})", (self.delegate_to_string)())
    }
}

impl AssertingSimilarity {
    /// `new AssertingSimilarity(delegate)`.
    pub fn new<S: Similarity + fmt::Display + 'static>(delegate: Arc<S>) -> AssertingSimilarity {
        let rendered = Arc::clone(&delegate);
        AssertingSimilarity {
            delegate,
            delegate_to_string: Arc::new(move || rendered.to_string()),
        }
    }

    /// `new AssertingSimilarity(delegate)` over a delegate rendered by
    /// `delegate_to_string`.
    pub fn with_rendering(
        delegate: Arc<dyn Similarity>,
        delegate_to_string: Arc<dyn Fn() -> String + Send + Sync>,
    ) -> AssertingSimilarity {
        AssertingSimilarity {
            delegate,
            delegate_to_string,
        }
    }
}

impl Similarity for AssertingSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.search.similarities.AssertingSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        java_assert!(state.length() > 0);
        java_assert!(state.position() >= 0);
        java_assert!(state.offset() >= 0);
        java_assert!(state.max_term_frequency() >= 0); // TODO: seems to be 0 for omitTFAP?
        java_assert!(state.max_term_frequency() <= state.length());
        java_assert!(state.num_overlap() >= 0);
        java_assert!(state.num_overlap() < state.length());
        java_assert!(state.unique_term_count() > 0);
        java_assert!(state.unique_term_count() <= state.length());
        let norm = self.delegate.compute_norm(state)?;
        java_assert!(norm != 0);
        Ok(norm)
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        java_assert_unchecked!(boost >= 0.0);
        java_assert_unchecked!(!term_stats.is_empty());
        // TODO: check that TermStats is in bounds with respect to collection?
        // e.g. docFreq <= maxDoc
        let scorer = self.delegate.scorer(boost, collection_stats, term_stats);
        Box::new(AssertingSimScorer { delegate: scorer })
    }
}

/// `AssertingSimilarity.AssertingSimScorer`. Java also keeps the `boost`,
/// which nothing reads; it is not kept here.
struct AssertingSimScorer<'a> {
    delegate: Box<dyn SimScorer + 'a>,
}

impl SimScorer for AssertingSimScorer<'_> {
    fn score(&self, freq: f32, norm: i64) -> f32 {
        // freq in bounds
        java_assert_unchecked!(freq.is_finite());
        java_assert_unchecked!(freq > 0.0);
        // result in bounds
        let score = self.delegate.score(freq, norm);
        java_assert_unchecked!(score.is_finite());
        if cfg!(debug_assertions) {
            java_assert_unchecked!(score <= self.delegate.score(freq, 1));
        }
        java_assert_unchecked!(score >= 0.0);
        score
    }

    fn explain(&self, freq: &Explanation, norm: i64) -> Explanation {
        // freq in bounds
        java_assert_unchecked!(freq.value().float_value().is_finite());
        // result in bounds
        let explanation = self.delegate.explain(freq, norm);
        java_assert_unchecked!(explanation.value().float_value().is_finite());
        // result matches score exactly
        if cfg!(debug_assertions) {
            java_assert_unchecked!(
                explanation.value().float_value()
                    == self.delegate.score(freq.value().float_value(), norm)
            );
        }
        explanation
    }

    fn as_bulk_sim_scorer(&self) -> Box<dyn BulkSimScorer + '_> {
        Box::new(AssertingBulkSimScorer {
            bulk_scorer: self.delegate.as_bulk_sim_scorer(),
        })
    }
}

/// The anonymous `BulkSimScorer` of `asBulkSimScorer()`.
struct AssertingBulkSimScorer<'a> {
    bulk_scorer: Box<dyn BulkSimScorer + 'a>,
}

impl BulkSimScorer for AssertingBulkSimScorer<'_> {
    fn score(&mut self, size: usize, freqs: &[f32], norms: &[i64], scores: &mut [f32]) {
        for i in 0..size {
            java_assert_unchecked!(freqs[i] > 0.0);
            java_assert_unchecked!(norms[i] != 0);
        }
        self.bulk_scorer.score(size, freqs, norms, scores);
        for &score in &scores[..size] {
            java_assert_unchecked!(score.is_finite());
            java_assert_unchecked!(score >= 0.0);
        }
    }
}
