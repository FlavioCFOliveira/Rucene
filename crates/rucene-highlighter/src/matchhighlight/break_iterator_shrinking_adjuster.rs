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

//! Port of `BreakIteratorShrinkingAdjuster.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::error::Result;
use rucene_analysis_common::jdk::break_iterator::{get_word_instance, BreakIterator, DONE};
use rucene_analysis_common::jdk::character::is_whitespace;
use rucene_analysis_common::jdk::locale::Locale;

use crate::matchhighlight::char_sequence_iterator::CharSequenceIterator;
use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::passage::Passage;
use crate::matchhighlight::passage_adjuster::PassageAdjuster;

#[derive(Debug)]
struct State {
    bi: Box<dyn BreakIterator>,
    value: Arc<[u16]>,
}

/// A `PassageAdjuster` that adjusts the `Passage` range to word boundaries
/// hinted by the given `BreakIterator`.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.BreakIteratorShrinkingAdjuster`.
#[derive(Debug)]
pub struct BreakIteratorShrinkingAdjuster {
    state: Mutex<State>,
}

impl Default for BreakIteratorShrinkingAdjuster {
    fn default() -> Self {
        Self::new()
    }
}

impl BreakIteratorShrinkingAdjuster {
    /// Equivalent to `new BreakIteratorShrinkingAdjuster()`: a word instance
    /// of the root locale.
    pub fn new() -> Self {
        Self::with_break_iterator(get_word_instance(&Locale::of_parts("", "", "", "")))
    }

    /// Equivalent to `new BreakIteratorShrinkingAdjuster(BreakIterator)`.
    pub fn with_break_iterator(bi: Box<dyn BreakIterator>) -> Self {
        Self {
            state: Mutex::new(State {
                bi,
                value: Arc::from(Vec::new()),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PassageAdjuster for BreakIteratorShrinkingAdjuster {
    fn current_value(&self, value: &str) {
        let mut state = self.state();
        state.value = Arc::from(value.encode_utf16().collect::<Vec<u16>>());
        let iter = CharSequenceIterator::new(Arc::clone(&state.value));
        state.bi.set_text(Box::new(iter));
    }

    fn adjust(&self, passage: &Passage) -> Result<OffsetRange> {
        let mut state = self.state();
        let state = &mut *state;
        let value = &state.value;
        let value_length = value.len() as i32;
        let bi = &mut state.bi;
        let mut from = passage.from;
        if from > 0 {
            while !bi.is_boundary(from)?
                || (from < value_length && is_whitespace(value[from as usize] as i32))
            {
                from = bi.following(from)?;
                if from == DONE {
                    from = passage.from;
                    break;
                }
            }
            if from == value_length {
                from = passage.from;
            }
        }

        let mut to = passage.to;
        if to != value_length {
            while !bi.is_boundary(to)? || (to > 0 && is_whitespace(value[(to - 1) as usize] as i32)) {
                to = bi.preceding(to)?;
                if to == DONE {
                    to = passage.to;
                    break;
                }
            }
            if to == 0 {
                to = passage.to;
            }
        }

        for r in &passage.markers {
            from = from.min(r.from);
            to = to.max(r.to);
        }

        if from > to {
            from = to;
        }

        Ok(OffsetRange::new(from, to))
    }
}
