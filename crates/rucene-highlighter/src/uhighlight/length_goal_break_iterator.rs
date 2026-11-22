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

//! Port of `LengthGoalBreakIterator.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt;

use rucene::error::{LuceneError, Result};
use rucene::util::java_fmt::float_to_string;
use rucene_analysis_common::jdk::break_iterator::{BreakIterator, DONE};
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

/// Wraps another `BreakIterator` to skip past breaks that would result in
/// passages that are too short. It's still possible to get a short passage
/// but only at the very end of the input text.
///
/// Important: This is not a general purpose `BreakIterator`; it's only
/// designed to work in a way compatible with the `UnifiedHighlighter`. Some
/// assumptions are checked with Java assertions.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.LengthGoalBreakIterator`.
#[derive(Debug)]
pub struct LengthGoalBreakIterator {
    base_iter: Box<dyn BreakIterator>,
    length_goal: i32,
    fragment_alignment: f32, // how much text to align before match-fragment, valid in range [0, 1]
    is_minimum_length: bool, // if false then is "closest to" length
    current_cache: i32,
}

impl LengthGoalBreakIterator {
    /// Breaks will be at least `min_length` apart (to the extent possible),
    /// while trying to position the match inside the fragment according to
    /// `fragment_alignment`.
    ///
    /// Equivalent to the static `createMinLength(BreakIterator, int, float)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `fragment_alignment` is outside `[0,
    /// 1]` or not finite.
    pub fn create_min_length(
        base_iter: Box<dyn BreakIterator>,
        min_length: i32,
        fragment_alignment: f32,
    ) -> Result<Self> {
        let current = base_iter.current();
        Self::new(base_iter, min_length, fragment_alignment, true, current)
    }

    /// For behavior description see `create_min_length`; this one breaks
    /// will be on average `target_length` apart; the matched text segment
    /// (field match) may be wrapped in a fragment longer than the target
    /// length.
    ///
    /// Equivalent to the static `createClosestToLength(BreakIterator, int,
    /// float)`.
    ///
    /// # Errors
    ///
    /// As [`create_min_length`](Self::create_min_length).
    pub fn create_closest_to_length(
        base_iter: Box<dyn BreakIterator>,
        target_length: i32,
        fragment_alignment: f32,
    ) -> Result<Self> {
        let current = base_iter.current();
        Self::new(base_iter, target_length, fragment_alignment, false, current)
    }

    fn new(
        base_iter: Box<dyn BreakIterator>,
        length_goal: i32,
        fragment_alignment: f32,
        is_minimum_length: bool,
        current_cache: i32,
    ) -> Result<Self> {
        if !(0.0..=1.0).contains(&fragment_alignment) || !fragment_alignment.is_finite() {
            return Err(LuceneError::illegal_argument(
                "fragmentAlignment must be >= zero and <= one",
            ));
        }
        Ok(Self {
            base_iter,
            current_cache,
            length_goal,
            fragment_alignment,
            is_minimum_length,
        })
    }

    fn text_end_index(&mut self) -> i32 {
        self.base_iter.get_text().get_end_index()
    }

    /// Equivalent to the private `following(int, int)`.
    fn following_target(&mut self, match_end_index: i32, target_idx: i32) -> Result<i32> {
        if target_idx >= self.text_end_index() {
            if self.current_cache == self.base_iter.last() {
                return Ok(DONE);
            }
            self.current_cache = self.base_iter.last();
            return Ok(self.current_cache);
        }
        let after_idx = self.base_iter.following(target_idx - 1)?;
        if after_idx == DONE {
            self.current_cache = self.base_iter.last();
            return Ok(DONE);
        }
        if after_idx == target_idx {
            // right on the money
            self.current_cache = after_idx;
            return Ok(self.current_cache);
        }
        if self.is_minimum_length {
            // thus never undershoot
            self.current_cache = after_idx;
            return Ok(self.current_cache);
        }

        // note: it is a shame that we invoke preceding() *one more time*; BI's
        // are sometimes expensive.

        // Find closest break to target
        let before_idx = self.base_iter.preceding(target_idx)?;
        if target_idx - before_idx < after_idx - target_idx && before_idx > match_end_index {
            self.current_cache = before_idx;
            return Ok(self.current_cache);
        }
        // moving the iterator to after_idx
        self.current_cache = after_idx;
        Ok(self.current_cache)
    }
}

impl fmt::Display for LengthGoalBreakIterator {
    /// Equivalent to `toString()`; the base iterator renders as its
    /// `Debug` form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let goal_desc = if self.is_minimum_length {
            "minLen"
        } else {
            "targetLen"
        };
        write!(
            f,
            "LengthGoalBreakIterator{{{}={}, fragAlign={}, baseIter={:?}}}",
            goal_desc,
            self.length_goal,
            float_to_string(self.fragment_alignment),
            self.base_iter
        )
    }
}

impl BreakIterator for LengthGoalBreakIterator {
    fn get_text(&mut self) -> &dyn CharacterIterator {
        self.base_iter.get_text()
    }

    fn set_text_units(&mut self, new_text: &[u16]) {
        self.base_iter.set_text_units(new_text);
        self.current_cache = self.base_iter.current();
    }

    fn set_text(&mut self, new_text: Box<dyn CharacterIterator>) {
        self.base_iter.set_text(new_text);
        self.current_cache = self.base_iter.current();
    }

    fn current(&self) -> i32 {
        self.current_cache
    }

    fn first(&mut self) -> i32 {
        self.current_cache = self.base_iter.first();
        self.current_cache
    }

    fn last(&mut self) -> i32 {
        self.current_cache = self.base_iter.last();
        self.current_cache
    }

    fn next_n(&mut self, n: i32) -> i32 {
        debug_assert!(false, "Not supported");
        self.base_iter.next_n(n) // probably wrong
    }

    // Called by getSummaryPassagesNoHighlight to generate default summary.
    fn next(&mut self) -> i32 {
        let current = self.current_cache;
        self.following_target(current, current + self.length_goal)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn previous(&mut self) -> i32 {
        debug_assert!(false, "Not supported");
        self.base_iter.previous()
    }

    fn following(&mut self, match_end_index: i32) -> Result<i32> {
        let target =
            (match_end_index + 1) + (self.length_goal as f32 * (1.0 - self.fragment_alignment)) as i32;
        self.following_target(match_end_index, target)
    }

    fn preceding(&mut self, match_start_index: i32) -> Result<i32> {
        let target_idx =
            (match_start_index - 1) - (self.length_goal as f32 * self.fragment_alignment) as i32;
        if target_idx <= 0 {
            if self.current_cache == self.base_iter.first() {
                return Ok(DONE);
            }
            self.current_cache = self.base_iter.first();
            return Ok(self.current_cache);
        }
        let before_idx = self.base_iter.preceding(target_idx + 1)?;
        if before_idx == DONE {
            self.current_cache = self.base_iter.first();
            return Ok(DONE);
        }
        if before_idx == target_idx {
            // right on the money
            self.current_cache = before_idx;
            return Ok(self.current_cache);
        }
        if self.is_minimum_length {
            // thus never undershoot
            self.current_cache = before_idx;
            return Ok(self.current_cache);
        }

        // note: it is a shame that we invoke following() *one more time*; BI's
        // are sometimes expensive.

        // Find closest break to target
        let after_idx = self.base_iter.following(target_idx - 1)?;
        if after_idx - target_idx < target_idx - before_idx && after_idx < match_start_index {
            self.current_cache = after_idx;
            return Ok(self.current_cache);
        }
        self.current_cache = before_idx;
        Ok(self.current_cache)
    }

    fn is_boundary(&mut self, offset: i32) -> Result<bool> {
        debug_assert!(false, "Not supported");
        self.base_iter.is_boundary(offset)
    }

    fn clone_box(&self) -> Box<dyn BreakIterator> {
        Box::new(Self {
            base_iter: self.base_iter.clone_box(),
            length_goal: self.length_goal,
            fragment_alignment: self.fragment_alignment,
            is_minimum_length: self.is_minimum_length,
            current_cache: self.current_cache,
        })
    }
}