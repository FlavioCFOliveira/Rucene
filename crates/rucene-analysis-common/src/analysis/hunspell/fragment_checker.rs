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

//! The oracle of impossible word fragments, ported from
//! `org.apache.lucene.analysis.hunspell.FragmentChecker`.

use std::sync::{Arc, LazyLock};

/// An oracle for quickly checking that a specific part of a word can never
/// be a valid word. This allows speeding up the "Modification" part of
/// [`Suggester`](super::suggester::Suggester) by avoiding expensive checks on
/// impossible words. Implementations may use character case, n-grams or
/// whatever they like.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.FragmentChecker`; the
/// word, a Java `CharSequence`, is given as its UTF-16 units.
pub trait FragmentChecker: Send + Sync {
    /// Check if the given word range intersects any fragment which is
    /// impossible in the current language. For example, if the word is
    /// "aaax", and there are no "aaa" combinations in words accepted by the
    /// spellchecker (but "aax" is valid), then `true` can be returned for all
    /// ranges in `0..3`, but not for `3..4`.
    ///
    /// The implementation must be monotonic: if some range is considered
    /// impossible, larger ranges encompassing it should also produce `true`.
    ///
    /// `word` is the whole word being checked for impossible substrings,
    /// `start` the start of the range in question, inclusive, and `end` the
    /// end of the range in question, inclusive, not smaller than `start`.
    ///
    /// Equivalent to `hasImpossibleFragmentAround(CharSequence, int, int)`.
    fn has_impossible_fragment_around(&self, word: &[u16], start: usize, end: usize) -> bool;
}

/// The checker for which everything is possible.
struct EverythingPossible;

impl FragmentChecker for EverythingPossible {
    fn has_impossible_fragment_around(&self, _word: &[u16], _start: usize, _end: usize) -> bool {
        false
    }
}

/// Equivalent to `FragmentChecker.EVERYTHING_POSSIBLE`.
pub fn everything_possible() -> Arc<dyn FragmentChecker> {
    static EVERYTHING_POSSIBLE: LazyLock<Arc<dyn FragmentChecker>> =
        LazyLock::new(|| Arc::new(EverythingPossible));
    Arc::clone(&EVERYTHING_POSSIBLE)
}
