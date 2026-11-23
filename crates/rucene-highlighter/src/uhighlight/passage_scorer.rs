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

//! Port of `PassageScorer.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use rucene::util::byte_block_pool::{ByteBlockPool, DirectAllocator};
use rucene::util::bytes_ref_hash::BytesRefHash;

use crate::uhighlight::passage::Passage;

/// The public methods of `PassageScorer` a Java subclass overrides
/// (`PassageScorer.java:80-133`). Each default is the base class's body,
/// reachable as the scorer's `super_*` method.
pub trait PassageScorerOverrides: Send + Sync {
    /// Equivalent to an override of `weight(int, int)`.
    fn weight(&self, this: &PassageScorer, content_length: i32, total_term_freq: i32) -> f32 {
        this.super_weight(content_length, total_term_freq)
    }

    /// Equivalent to an override of `tf(int, int)`.
    fn tf(&self, this: &PassageScorer, freq: i32, passage_len: i32) -> f32 {
        this.super_tf(freq, passage_len)
    }

    /// Equivalent to an override of `norm(int)`.
    fn norm(&self, this: &PassageScorer, passage_start: i32) -> f32 {
        this.super_norm(passage_start)
    }

    /// Equivalent to an override of `score(Passage, int)`.
    fn score(&self, this: &PassageScorer, passage: &Passage, content_length: i32) -> f32 {
        this.super_score(passage, content_length)
    }
}

/// Ranks passages found by `UnifiedHighlighter`.
///
/// Each passage is scored as a miniature document within the document. The
/// final score is computed as `norm` * ∑ (`weight` * `tf`). The default
/// implementation is `norm` * BM25.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.PassageScorer`; a
/// subclass's overrides are a [`PassageScorerOverrides`]. The scorer holds
/// no mutable state, so a clone (which shares the overrides) behaves as the
/// Java instance it was cloned from.
#[derive(Clone)]
pub struct PassageScorer {
    // TODO: this formula is completely made up. It might not provide
    // relevant snippets!

    /// BM25 k1 parameter, controls term frequency normalization
    pub(crate) k1: f32,
    /// BM25 b parameter, controls length normalization.
    pub(crate) b: f32,
    /// A pivot used for length normalization.
    pub(crate) pivot: f32,
    /// The subclass's overrides; `None` for the base class.
    overrides: Option<Arc<dyn PassageScorerOverrides>>,
}

impl std::fmt::Debug for PassageScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassageScorer")
            .field("k1", &self.k1)
            .field("b", &self.b)
            .field("pivot", &self.pivot)
            .field("overridden", &self.overrides.is_some())
            .finish()
    }
}

impl Default for PassageScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl PassageScorer {
    /// Creates PassageScorer with these default values:
    ///
    /// - `k1 = 1.2`,
    /// - `b = 0.75`.
    /// - `pivot = 87`
    ///
    /// Equivalent to `new PassageScorer()`.
    pub fn new() -> Self {
        // 1.2 and 0.75 are well-known bm25 defaults (but maybe not the best
        // here) ? 87 is typical average english sentence length.
        Self::with_params(1.2, 0.75, 87.0)
    }

    /// Creates PassageScorer with specified scoring parameters.
    ///
    /// Equivalent to `new PassageScorer(float, float, float)`.
    pub fn with_params(k1: f32, b: f32, pivot: f32) -> Self {
        Self {
            k1,
            b,
            pivot,
            overrides: None,
        }
    }

    /// Equivalent to `new PassageScorer(float, float, float)` of a subclass
    /// whose overrides are `overrides`.
    pub fn with_overrides(k1: f32, b: f32, pivot: f32, overrides: Arc<dyn PassageScorerOverrides>) -> Self {
        Self {
            k1,
            b,
            pivot,
            overrides: Some(overrides),
        }
    }

    /// Computes term importance, given its in-document statistics.
    ///
    /// Equivalent to `weight(int, int)`.
    pub fn weight(&self, content_length: i32, total_term_freq: i32) -> f32 {
        match &self.overrides {
            Some(o) => o.weight(self, content_length, total_term_freq),
            None => self.super_weight(content_length, total_term_freq),
        }
    }

    /// The base class's `weight`.
    pub fn super_weight(&self, content_length: i32, total_term_freq: i32) -> f32 {
        // approximate #docs from content length
        let num_docs = 1.0 + content_length as f32 / self.pivot;
        // numDocs not numDocs - docFreq (ala DFR), since we approximate
        // numDocs
        (self.k1 + 1.0)
            * (1.0 + (num_docs as f64 + 0.5) / (total_term_freq as f64 + 0.5)).ln() as f32
    }

    /// Computes term weight, given the frequency within the passage and the
    /// passage's length.
    ///
    /// Equivalent to `tf(int, int)`.
    pub fn tf(&self, freq: i32, passage_len: i32) -> f32 {
        match &self.overrides {
            Some(o) => o.tf(self, freq, passage_len),
            None => self.super_tf(freq, passage_len),
        }
    }

    /// The base class's `tf`.
    pub fn super_tf(&self, freq: i32, passage_len: i32) -> f32 {
        let norm = self.k1 * ((1.0 - self.b) + self.b * (passage_len as f32 / self.pivot));
        freq as f32 / (freq as f32 + norm)
    }

    /// Normalize a passage according to its position in the document.
    ///
    /// Typically passages towards the beginning of the document are more
    /// useful for summarizing the contents.
    ///
    /// The default implementation is `1 + 1/log(pivot + passageStart)`
    ///
    /// Equivalent to `norm(int)`.
    pub fn norm(&self, passage_start: i32) -> f32 {
        match &self.overrides {
            Some(o) => o.norm(self, passage_start),
            None => self.super_norm(passage_start),
        }
    }

    /// The base class's `norm`.
    pub fn super_norm(&self, passage_start: i32) -> f32 {
        1.0 + 1.0 / ((self.pivot + passage_start as f32) as f64).ln() as f32
    }

    /// Equivalent to `score(Passage, int)`.
    pub fn score(&self, passage: &Passage, content_length: i32) -> f32 {
        match &self.overrides {
            Some(o) => o.score(self, passage, content_length),
            None => self.super_score(passage, content_length),
        }
    }

    /// The base class's `score`.
    pub fn super_score(&self, passage: &Passage, content_length: i32) -> f32 {
        let mut score = 0f64;
        // `new BytesRefHash()`: a pool over a DirectAllocator
        let mut pool = ByteBlockPool::with_allocator(Box::new(DirectAllocator::new()));
        let mut terms_hash = BytesRefHash::new(Arc::new(AtomicI64::new(0)));
        let hit_count = passage.get_num_matches() as usize;
        let mut term_freqs_in_passage = vec![0i32; hit_count]; // maximum size
        let mut term_freqs_in_doc = vec![0i32; hit_count];

        for i in 0..hit_count {
            let mut term_index = terms_hash
                .add(&mut pool, passage.get_match_terms()[i].slice())
                .expect("INVARIANT: a term fits the hash");
            if term_index < 0 {
                term_index = -(term_index + 1);
            } else {
                term_freqs_in_doc[term_index as usize] = passage.get_match_term_freqs_in_doc()[i];
            }
            term_freqs_in_passage[term_index as usize] += 1;
        }

        for i in 0..terms_hash.size() as usize {
            score += (self.tf(term_freqs_in_passage[i], passage.get_length())
                * self.weight(content_length, term_freqs_in_doc[i])) as f64;
        }
        score *= self.norm(passage.get_start_offset()) as f64;
        score as f32
    }
}