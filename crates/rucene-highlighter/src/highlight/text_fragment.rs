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

//! Port of `TextFragment.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The text a fragment indexes: Java's shared `CharSequence` (the
/// `Highlighter`'s `StringBuilder`), as UTF-16 code units.
pub type MarkedUpText = Arc<Mutex<Vec<u16>>>;

#[derive(Debug)]
struct State {
    marked_up_text: MarkedUpText,
    frag_num: i32,
    text_start_pos: i32,
    text_end_pos: i32,
    score: f32,
}

/// Low-level class used to record information about a section of a document
/// with a score.
///
/// Equivalent to `org.apache.lucene.search.highlight.TextFragment`. A value is
/// a handle onto one fragment, as a Java reference is: a clone is the same
/// fragment, so the `Highlighter` updates a fragment it has already queued.
#[derive(Debug, Clone)]
pub struct TextFragment {
    state: Arc<Mutex<State>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TextFragment {
    /// Equivalent to `new TextFragment(CharSequence, int, int)`.
    pub fn new(marked_up_text: MarkedUpText, text_start_pos: i32, frag_num: i32) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                marked_up_text,
                frag_num,
                text_start_pos,
                text_end_pos: 0,
                score: 0.0,
            })),
        }
    }

    /// Equivalent to the package-private `setScore(float)`.
    pub(crate) fn set_score(&self, score: f32) {
        lock(&self.state).score = score;
    }

    /// Equivalent to writing the package-private field `textEndPos`.
    pub(crate) fn set_text_end_pos(&self, text_end_pos: i32) {
        lock(&self.state).text_end_pos = text_end_pos;
    }

    /// Equivalent to reading the package-private field `textStartPos`.
    pub(crate) fn text_start_pos(&self) -> i32 {
        lock(&self.state).text_start_pos
    }

    // Mirrors a package-private Java member that no Rust caller reaches.
    #[allow(dead_code)]
    /// Equivalent to reading the package-private field `textEndPos`.
    pub(crate) fn text_end_pos(&self) -> i32 {
        lock(&self.state).text_end_pos
    }

    /// Equivalent to `getScore()`.
    pub fn get_score(&self) -> f32 {
        lock(&self.state).score
    }

    /// Equivalent to `merge(TextFragment)`: `frag2` is the fragment to be
    /// merged into this one.
    pub fn merge(&self, frag2: &TextFragment) {
        let (end, score2) = {
            let other = lock(&frag2.state);
            (other.text_end_pos, other.score)
        };
        let mut state = lock(&self.state);
        state.text_end_pos = end;
        state.score = state.score.max(score2);
    }

    /// Returns true if this fragment follows the one passed.
    ///
    /// Equivalent to `follows(TextFragment)`.
    pub fn follows(&self, fragment: &TextFragment) -> bool {
        let end = lock(&fragment.state).text_end_pos;
        self.text_start_pos() == end
    }

    /// Returns the fragment sequence number.
    ///
    /// Equivalent to `getFragNum()`.
    pub fn get_frag_num(&self) -> i32 {
        lock(&self.state).frag_num
    }

    /// Whether `self` and `other` are the same fragment (Java's `==`).
    pub fn same_fragment(&self, other: &TextFragment) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl fmt::Display for TextFragment {
    /// Returns the marked-up text of this text fragment.
    ///
    /// Equivalent to `toString()`: `markedUpText.subSequence(textStartPos,
    /// textEndPos)`.
    ///
    /// # Panics
    ///
    /// When the range is outside the text, where Java throws
    /// `StringIndexOutOfBoundsException`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (text, start, end) = {
            let state = lock(&self.state);
            (
                Arc::clone(&state.marked_up_text),
                state.text_start_pos,
                state.text_end_pos,
            )
        };
        let text = lock(&text);
        let s = crate::java::substring(&text, start, end).unwrap_or_else(|e| panic!("{e}"));
        f.write_str(&s)
    }
}
