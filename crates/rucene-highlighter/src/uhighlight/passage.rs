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

//! Port of `Passage.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt;

use rucene::util::java_fmt::float_to_string;
use rucene::util::{ArrayUtil, BytesRef};

/// Represents a passage (typically a sentence of the document).
///
/// A passage contains `get_num_matches()` highlights from the query, and the
/// offsets and query terms that correspond with each match.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.Passage`.
#[derive(Debug, Clone)]
pub struct Passage {
    start_offset: i32,
    end_offset: i32,
    score: f32,

    match_starts: Vec<i32>,
    match_ends: Vec<i32>,
    match_terms: Vec<BytesRef>,
    match_term_freq_in_doc: Vec<i32>,
    num_matches: i32,
}

impl Default for Passage {
    fn default() -> Self {
        Self::new()
    }
}

impl Passage {
    /// Equivalent to `new Passage()`.
    pub fn new() -> Self {
        Self {
            start_offset: -1,
            end_offset: -1,
            score: 0.0,
            match_starts: vec![0; 8],
            match_ends: vec![0; 8],
            match_terms: vec![BytesRef::default(); 8],
            match_term_freq_in_doc: vec![0; 8],
            num_matches: 0,
        }
    }

    /// Equivalent to `addMatch(int, int, BytesRef, int)`.
    pub fn add_match(&mut self, start_offset: i32, end_offset: i32, term: BytesRef, term_freq_in_doc: i32) {
        debug_assert!(start_offset >= self.start_offset && start_offset <= self.end_offset);
        if self.num_matches as usize == self.match_starts.len() {
            let new_length = ArrayUtil::oversize(self.num_matches as usize + 1, 8);
            self.match_starts.resize(new_length, 0);
            self.match_ends.resize(new_length, 0);
            self.match_terms.resize(new_length, BytesRef::default());
            self.match_term_freq_in_doc.resize(new_length, 0);
        }
        debug_assert!(
            self.match_starts.len() == self.match_ends.len()
                && self.match_ends.len() == self.match_terms.len()
        );
        let n = self.num_matches as usize;
        self.match_starts[n] = start_offset;
        self.match_ends[n] = end_offset;
        self.match_terms[n] = term;
        self.match_term_freq_in_doc[n] = term_freq_in_doc;
        self.num_matches += 1;
    }

    /// Equivalent to `reset()`.
    pub fn reset(&mut self) {
        self.start_offset = -1;
        self.end_offset = -1;
        self.score = 0.0;
        self.num_matches = 0;
    }

    /// Start offset of this passage.
    ///
    /// Equivalent to `getStartOffset()`.
    pub fn get_start_offset(&self) -> i32 {
        self.start_offset
    }

    /// End offset of this passage.
    ///
    /// Equivalent to `getEndOffset()`.
    pub fn get_end_offset(&self) -> i32 {
        self.end_offset
    }

    /// Equivalent to `getLength()`.
    pub fn get_length(&self) -> i32 {
        self.end_offset - self.start_offset
    }

    /// Passage's score.
    ///
    /// Equivalent to `getScore()`.
    pub fn get_score(&self) -> f32 {
        self.score
    }

    /// Equivalent to `setScore(float)`.
    pub fn set_score(&mut self, score: f32) {
        self.score = score;
    }

    /// Number of term matches available in `get_match_starts`,
    /// `get_match_ends`, `get_match_terms`.
    ///
    /// Equivalent to `getNumMatches()`.
    pub fn get_num_matches(&self) -> i32 {
        self.num_matches
    }

    /// Start offsets of the term matches, in increasing order. Only
    /// `get_num_matches` are valid.
    ///
    /// Equivalent to `getMatchStarts()`.
    pub fn get_match_starts(&self) -> &[i32] {
        &self.match_starts
    }

    /// End offsets of the term matches, corresponding with
    /// `get_match_starts`.
    ///
    /// Equivalent to `getMatchEnds()`.
    pub fn get_match_ends(&self) -> &[i32] {
        &self.match_ends
    }

    /// BytesRef (term text) of the matches, corresponding with
    /// `get_match_starts()`.
    ///
    /// Equivalent to `getMatchTerms()`.
    pub fn get_match_terms(&self) -> &[BytesRef] {
        &self.match_terms
    }

    /// Equivalent to `getMatchTermFreqsInDoc()`.
    pub fn get_match_term_freqs_in_doc(&self) -> &[i32] {
        &self.match_term_freq_in_doc
    }

    /// Equivalent to `setStartOffset(int)`.
    pub fn set_start_offset(&mut self, start_offset: i32) {
        self.start_offset = start_offset;
    }

    /// Equivalent to `setEndOffset(int)`.
    pub fn set_end_offset(&mut self, end_offset: i32) {
        debug_assert!(self.start_offset <= end_offset);
        self.end_offset = end_offset;
    }
}

impl fmt::Display for Passage {
    /// For debugging. ex: Passage[0-22]{yin[0-3],yang[4-8],yin[10-13]}score=2.4964213
    ///
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Passage[{}-{}]{{", self.start_offset, self.end_offset)?;
        for i in 0..self.num_matches as usize {
            if i != 0 {
                f.write_str(",")?;
            }
            let term = self.match_terms[i]
                .utf8_to_string()
                .unwrap_or_else(|_| String::from_utf8_lossy(self.match_terms[i].slice()).into_owned());
            write!(
                f,
                "{}[{}-{}]",
                term,
                self.match_starts[i] - self.start_offset,
                self.match_ends[i] - self.start_offset
            )?;
        }
        write!(f, "}}score={}", float_to_string(self.score))
    }
}