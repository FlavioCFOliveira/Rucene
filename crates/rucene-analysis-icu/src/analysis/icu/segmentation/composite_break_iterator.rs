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

//! An internal BreakIterator for multilingual text, ported from
//! `org.apache.lucene.analysis.icu.segmentation.CompositeBreakIterator`.

use std::sync::Arc;

use icu4j::lang::u_character::UCharacter;
use icu4j::lang::u_property::UProperty;
use icu4j::lang::u_script::UScript;
use icu4j::text::break_iterator::DONE;

use super::break_iterator_wrapper::BreakIteratorWrapper;
use super::icu_tokenizer_config::ICUTokenizerConfig;
use super::script_iterator::ScriptIterator;

/// An internal BreakIterator for multilingual text, following
/// recommendations from: UAX #29: Unicode Text Segmentation.
/// (<http://unicode.org/reports/tr29/>)
///
/// See <http://unicode.org/reports/tr29/#Tailoring> for the motivation of
/// this design.
///
/// Text is first divided into script boundaries. The processing is then
/// delegated to the appropriate break iterator for that specific script.
///
/// This break iterator also allows you to retrieve the ISO 15924 script
/// code associated with a piece of text.
///
/// See also UAX #29, UTR #24
///
/// Equivalent to `org.apache.lucene.analysis.icu.segmentation.CompositeBreakIterator`
/// (package-private, `@lucene.experimental`). Java's field `rbbi`, a
/// reference to one element of `wordBreakers`, is the index of that
/// element (`None` is Java's initial `null`).
#[derive(Debug)]
pub struct CompositeBreakIterator {
    config: Arc<dyn ICUTokenizerConfig>,
    word_breakers: Vec<Option<BreakIteratorWrapper>>,

    rbbi: Option<usize>,
    script_iterator: ScriptIterator,

    text: Arc<[u16]>,
}

impl CompositeBreakIterator {
    /// Equivalent to `CompositeBreakIterator(ICUTokenizerConfig)`.
    pub fn new(config: Arc<dyn ICUTokenizerConfig>) -> Self {
        let size = 1 + UCharacter::get_int_property_max_value(UProperty::SCRIPT);
        let script_iterator = ScriptIterator::new(config.combine_cj());
        Self {
            config,
            word_breakers: (0..size).map(|_| None).collect(),
            rbbi: None,
            script_iterator,
            text: Arc::from(Vec::new()),
        }
    }

    /// The current break iterator, Java's `rbbi`.
    ///
    /// # Panics
    ///
    /// With Java's `NullPointerException` before the first `setText`.
    fn rbbi(&self) -> &BreakIteratorWrapper {
        let index = self.rbbi.unwrap_or_else(|| {
            rucene::store::java_nio_file::throw_unchecked("java.lang.NullPointerException", None)
        });
        self.word_breakers[index]
            .as_ref()
            .expect("INVARIANT: rbbi indexes a created break iterator")
    }

    /// The current break iterator, mutably.
    fn rbbi_mut(&mut self) -> &mut BreakIteratorWrapper {
        let index = self.rbbi.unwrap_or_else(|| {
            rucene::store::java_nio_file::throw_unchecked("java.lang.NullPointerException", None)
        });
        self.word_breakers[index]
            .as_mut()
            .expect("INVARIANT: rbbi indexes a created break iterator")
    }

    /// Retrieve the next break position. If the RBBI range is exhausted
    /// within the script boundary, examine the next script boundary.
    ///
    /// Returns the next break position or `BreakIterator.DONE`.
    ///
    /// Equivalent to `next()`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> i32 {
        let mut next = self.rbbi_mut().next();
        while next == DONE && self.script_iterator.next() {
            self.rbbi = Some(self.get_break_iterator(self.script_iterator.get_script_code()));
            let text = Arc::clone(&self.text);
            let start = self.script_iterator.get_script_start();
            let length =
                self.script_iterator.get_script_limit() - self.script_iterator.get_script_start();
            self.rbbi_mut().set_text(text, start, length);
            next = self.rbbi_mut().next();
        }
        if next == DONE {
            DONE
        } else {
            next + self.script_iterator.get_script_start()
        }
    }

    /// Retrieve the current break position.
    ///
    /// Returns the current break position or `BreakIterator.DONE`.
    ///
    /// Equivalent to `current()`.
    pub fn current(&self) -> i32 {
        let current = self.rbbi().current();
        if current == DONE {
            DONE
        } else {
            current + self.script_iterator.get_script_start()
        }
    }

    /// Retrieve the rule status code (token type) from the underlying break
    /// iterator.
    ///
    /// Returns the rule status code (see `RuleBasedBreakIterator` constants).
    ///
    /// Equivalent to `getRuleStatus()`.
    pub fn get_rule_status(&self) -> i32 {
        self.rbbi().get_rule_status()
    }

    /// Retrieve the UScript script code for the current token. This code
    /// can be decoded with UScript into a name or ISO 15924 code.
    ///
    /// Returns the UScript script code for the current token.
    ///
    /// Equivalent to `getScriptCode()`.
    pub fn get_script_code(&self) -> i32 {
        self.script_iterator.get_script_code()
    }

    /// Set a new region of text to be examined by this iterator.
    ///
    /// * `text`: buffer of text
    /// * `start`: offset into buffer
    /// * `length`: maximum length to examine
    ///
    /// Equivalent to `setText(char[], int, int)`.
    pub fn set_text(&mut self, text: Arc<[u16]>, start: i32, length: i32) {
        self.text = Arc::clone(&text);
        self.script_iterator
            .set_text(Arc::clone(&text), start, length);
        if self.script_iterator.next() {
            self.rbbi = Some(self.get_break_iterator(self.script_iterator.get_script_code()));
            let start = self.script_iterator.get_script_start();
            let length =
                self.script_iterator.get_script_limit() - self.script_iterator.get_script_start();
            self.rbbi_mut().set_text(text, start, length);
        } else {
            self.rbbi = Some(self.get_break_iterator(UScript::COMMON));
            self.rbbi_mut().set_text(text, 0, 0);
        }
    }

    /// Equivalent to `getBreakIterator(int)`: the break iterator of
    /// `script_code`, created from the config on first use; here its index
    /// in `word_breakers`.
    fn get_break_iterator(&mut self, script_code: i32) -> usize {
        let index = script_code as usize;
        if self.word_breakers[index].is_none() {
            self.word_breakers[index] = Some(BreakIteratorWrapper::new(
                self.config.get_break_iterator(script_code),
            ));
        }
        index
    }
}
