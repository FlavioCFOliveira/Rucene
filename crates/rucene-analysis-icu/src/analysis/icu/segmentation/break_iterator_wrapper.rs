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

//! Wraps RuleBasedBreakIterator, making object reuse convenient and
//! emitting a rule status for emoji sequences, ported from
//! `org.apache.lucene.analysis.icu.segmentation.BreakIteratorWrapper`.

use std::sync::{Arc, LazyLock};

use icu4j::text::break_iterator::{BreakIterator, DONE, WORD_NONE};
use icu4j::text::rule_based_break_iterator::RuleBasedBreakIterator;
use icu4j::text::unicode_set::UnicodeSet;
use icu4j::text::utf16::UTF16;
use rucene::util::IOUtils;

use super::char_array_iterator::CharArrayIterator;
use super::icu_tokenizer_config::EMOJI_SEQUENCE_STATUS;

/// A frozen set built from `pattern`, as the class's static initializer
/// builds `EMOJI_RK` and `EMOJI` (a failure there is Java's
/// `ExceptionInInitializerError`).
fn frozen_set(pattern: &str) -> UnicodeSet {
    let units: Vec<u16> = pattern.encode_utf16().collect();
    let mut set = UnicodeSet::from_pattern(&units).unwrap_or_else(|e| IOUtils::rethrow_always(e));
    set.freeze();
    set
}

/// See unicode doc L2/16-315 for rationale.
/// basically for us the ambiguous cases (keycap/etc) as far as types go.
///
/// Equivalent to the static field `EMOJI_RK`, `new
/// UnicodeSet("[*#0-9©®™〰〽]").freeze()` (the Java source's
/// `*` and `#` are `*` and `#`).
pub static EMOJI_RK: LazyLock<UnicodeSet> =
    LazyLock::new(|| frozen_set("[\u{2a}\u{23}0-9©®™〰〽]"));

/// faster than doing hasBinaryProperty() checks, at the cost of 1KB ram
///
/// Equivalent to the static field `EMOJI`, `new
/// UnicodeSet("[[:Emoji:][:Extended_Pictographic:]]").freeze()`.
pub static EMOJI: LazyLock<UnicodeSet> =
    LazyLock::new(|| frozen_set("[[:Emoji:][:Extended_Pictographic:]]"));

/// Wraps RuleBasedBreakIterator, making object reuse convenient and
/// emitting a rule status for emoji sequences.
///
/// Equivalent to `org.apache.lucene.analysis.icu.segmentation.BreakIteratorWrapper`
/// (package-private, `@lucene.experimental`). The text is shared as an
/// `Arc<[u16]>`, as for [`CharArrayIterator`]; the break iterator is given
/// a copy of the text iterator (`rbbi.setText(textIterator)` hands Java's
/// iterator the wrapper's own object, which the wrapper never reads again).
#[derive(Debug)]
pub struct BreakIteratorWrapper {
    text_iterator: CharArrayIterator,
    rbbi: RuleBasedBreakIterator,
    text: Arc<[u16]>,
    start: i32,
    status: i32,
}

impl BreakIteratorWrapper {
    /// Equivalent to `BreakIteratorWrapper(RuleBasedBreakIterator)`.
    pub fn new(rbbi: RuleBasedBreakIterator) -> Self {
        Self {
            text_iterator: CharArrayIterator::new(),
            rbbi,
            text: Arc::from(Vec::new()),
            start: 0,
            status: 0,
        }
    }

    /// Equivalent to `current()`.
    pub fn current(&self) -> i32 {
        self.rbbi.current()
    }

    /// Equivalent to `getRuleStatus()`.
    pub fn get_rule_status(&self) -> i32 {
        self.status
    }

    /// Equivalent to `next()`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> i32 {
        let current = self.rbbi.current();
        let next = self.rbbi.next();
        self.status = self.calc_status(current, next);
        next
    }

    /// Returns current rule status for the text between breaks. (determines token type)
    ///
    /// Equivalent to `calcStatus(int, int)`.
    fn calc_status(&self, current: i32, next: i32) -> i32 {
        // to support presentation selectors, we need to handle alphanum, num, and none at least, so
        // currently not worth optimizing.
        // https://unicode.org/cldr/utility/list-unicodeset.jsp?a=%5B%3AEmoji%3A%5D-%5B%3AEmoji_Presentation%3A%5D&g=Word_Break&i=
        if next != DONE && self.is_emoji(current, next) {
            EMOJI_SEQUENCE_STATUS
        } else {
            self.rbbi.get_rule_status()
        }
    }

    /// Returns true if the current text represents emoji character or
    /// sequence.
    ///
    /// Equivalent to `isEmoji(int, int)`.
    fn is_emoji(&self, current: i32, next: i32) -> bool {
        let begin = self.start + current;
        let end = self.start + next;
        let codepoint = UTF16::char_at_chars(&self.text, 0, end, begin);
        if EMOJI
            .contains(codepoint)
            .expect("INVARIANT: a code point read from UTF-16 text is a valid code point")
        {
            if EMOJI_RK
                .contains(codepoint)
                .expect("INVARIANT: a code point read from UTF-16 text is a valid code point")
            {
                // if its in EmojiRK, we don't treat it as emoji unless there is evidence it forms emoji
                // sequence,
                // an emoji presentation selector or keycap follows.
                let trailer = begin + if codepoint >= 0x10000 { 2 } else { 1 };
                trailer < end
                    && (self.text[trailer as usize] == 0xFE0F || self.text[trailer as usize] == 0x20E3)
            } else {
                true
            }
        } else {
            false
        }
    }

    /// Equivalent to `setText(char[], int, int)`.
    pub fn set_text(&mut self, text: Arc<[u16]>, start: i32, length: i32) {
        self.text = Arc::clone(&text);
        self.start = start;
        self.text_iterator.set_text(text, start, length);
        self.rbbi.set_text(Some(Box::new(self.text_iterator.clone())));
        self.status = WORD_NONE;
    }
}
