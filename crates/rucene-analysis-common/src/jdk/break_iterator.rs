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

//! `java.text.BreakIterator`, the boundary analysis
//! `SegmentingTokenizerBase` and `ThaiTokenizer` segment their text with.
//!
//! JDK 25 answers `BreakIterator.getWordInstance(Locale)` and
//! `getSentenceInstance(Locale)` with the iterators of
//! `sun.util.locale.provider.BreakIteratorProviderImpl`: a
//! `sun.text.RuleBasedBreakIterator` over the compiled word or sentence rules
//! of `sun.text.resources.BreakIteratorRules`, and, for a Thai locale, a
//! `sun.text.DictionaryBasedBreakIterator` over the Thai word rules and the
//! `thai_dict` dictionary (`sun.text.resources.ext.BreakIteratorInfo_th`).
//! This module emulates those iterators, written from the JDK 25 sources read
//! as specification: [`RuleBasedBreakIterator`] runs the JDK's
//! `handleNext` algorithm over the state tables of
//! [`break_iterator_rules`](super::break_iterator_rules), and its
//! dictionary mode the splitting algorithm of
//! `DictionaryBasedBreakIterator.divideUpDictionaryRange` over
//! `BreakDictionary`.
//!
//! **Divergence.** The Thai dictionary is ICU's word list, not the JDK's
//! `thai_dict` (see `break_dictionary`; approved on rmp task #447).

use std::cell::RefCell;
use std::fmt::{self, Debug};
use std::rc::Rc;

use rucene::error::{LuceneError, Result};

use super::break_dictionary::BreakDictionary;
use super::break_iterator_rules::{
    sentence_rules, thai_word_rules, word_rules, RuleTable, IGNORE, START_STATE, STOP_STATE,
};
use super::character_iterator::{CharacterIterator, StringCharacterIterator, DONE as CHAR_DONE};
use super::locale::Locale;

/// The value [`BreakIterator::next`] and the other movements answer when the
/// first or last boundary has been passed: `BreakIterator.DONE`, `-1`.
pub const DONE: i32 = -1;

/// The location of boundaries in text.
///
/// Equivalent to the abstract class `java.text.BreakIterator` of JDK 25: an
/// implementation maintains a current position in its text and moves it
/// from boundary to boundary.
///
/// **Divergence.** `clone()` is [`clone_box`](Self::clone_box), a trait
/// object being unable to return `Self`; `setText(String)` is
/// [`set_text_str`](Self::set_text_str).
pub trait BreakIterator: Debug + Send + Sync {
    /// Returns the first boundary, which is the begin index of the text, and
    /// makes it the current one: `BreakIterator.first()`.
    fn first(&mut self) -> i32;

    /// Returns the last boundary, which is the end index of the text, and
    /// makes it the current one: `BreakIterator.last()`.
    fn last(&mut self) -> i32;

    /// Returns the `n`th boundary from the current one (`n` may be
    /// negative), or [`DONE`] past either end: `BreakIterator.next(int)`.
    fn next_n(&mut self, n: i32) -> i32;

    /// Returns the boundary following the current one, or [`DONE`] when the
    /// current boundary is the last: `BreakIterator.next()`.
    fn next(&mut self) -> i32;

    /// Returns the boundary preceding the current one, or [`DONE`] when the
    /// current boundary is the first: `BreakIterator.previous()`.
    fn previous(&mut self) -> i32;

    /// Returns the first boundary following `offset`, or [`DONE`] when
    /// `offset` is at or past the last boundary: `BreakIterator.following(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `offset` is outside the text.
    fn following(&mut self, offset: i32) -> Result<i32>;

    /// Returns the last boundary preceding `offset`, or [`DONE`]:
    /// `BreakIterator.preceding(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `offset` is outside the text.
    fn preceding(&mut self, offset: i32) -> Result<i32>;

    /// Returns whether `offset` is a boundary, moving the current position
    /// to the first boundary following `offset - 1`:
    /// `BreakIterator.isBoundary(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `offset` is outside the text.
    fn is_boundary(&mut self, offset: i32) -> Result<bool>;

    /// Returns the current boundary: `BreakIterator.current()`.
    fn current(&self) -> i32;

    /// Returns the text being scanned: `BreakIterator.getText()`.
    fn get_text(&mut self) -> &dyn CharacterIterator;

    /// Sets a new text to scan, resetting the current position to its first
    /// boundary: `BreakIterator.setText(CharacterIterator)`.
    fn set_text(&mut self, new_text: Box<dyn CharacterIterator>);

    /// Sets a new string to scan: `BreakIterator.setText(String)`, `setText(new
    /// StringCharacterIterator(newText))`; `new_text` is the string's UTF-16
    /// units.
    fn set_text_units(&mut self, new_text: &[u16]) {
        self.set_text(Box::new(StringCharacterIterator::new(new_text.to_vec())));
    }

    /// [`set_text_units`](Self::set_text_units) of the UTF-16 units of
    /// `new_text`.
    fn set_text_str(&mut self, new_text: &str) {
        let units: Vec<u16> = new_text.encode_utf16().collect();
        self.set_text_units(&units);
    }

    /// Returns a copy of this iterator: `BreakIterator.clone()`.
    fn clone_box(&self) -> Box<dyn BreakIterator>;
}

/// Returns a new break iterator for word breaks in `locale`.
///
/// Equivalent to `BreakIterator.getWordInstance(Locale)` on JDK 25: for a
/// locale whose language is Thai (`th`), a dictionary-based iterator over the
/// Thai word rules; for any other, a rule-based iterator over the word rules
/// of the root locale.
pub fn get_word_instance(locale: &Locale) -> Box<dyn BreakIterator> {
    if locale.get_language() == "th" {
        Box::new(RuleBasedBreakIterator::with_dictionary(
            thai_word_rules(),
            BreakDictionary::thai(),
        ))
    } else {
        Box::new(RuleBasedBreakIterator::new(word_rules()))
    }
}

/// Returns a new break iterator for sentence breaks in `locale`.
///
/// Equivalent to `BreakIterator.getSentenceInstance(Locale)` on JDK 25: a
/// rule-based iterator over the sentence rules, the same for every locale
/// (Thai's names the root sentence rules too).
pub fn get_sentence_instance(_locale: &Locale) -> Box<dyn BreakIterator> {
    Box::new(RuleBasedBreakIterator::new(sentence_rules()))
}

/// What `DictionaryBasedBreakIterator` adds to `RuleBasedBreakIterator`.
#[derive(Debug, Clone)]
struct DictionaryState {
    dictionary: &'static BreakDictionary,
    /// `dictionaryCharCount`.
    dictionary_char_count: i32,
    /// `cachedBreakPositions`.
    cached_break_positions: Option<Vec<i32>>,
    /// `positionInCache`.
    position_in_cache: i32,
}

/// A rule-based break iterator.
///
/// Equivalent to `sun.text.RuleBasedBreakIterator` of JDK 25 over one of the
/// tables of [`break_iterator_rules`](super::break_iterator_rules), and,
/// when built `with_dictionary`, to its subclass
/// `sun.text.DictionaryBasedBreakIterator`: the rules then return ranges of
/// dictionary characters whole, and the dictionary splits each range into
/// words.
pub struct RuleBasedBreakIterator {
    table: &'static RuleTable,
    /// `text`; `None` is Java's `null`.
    text: Option<Box<dyn CharacterIterator>>,
    /// `cachedLastKnownBreak`.
    cached_last_known_break: i32,
    /// `Some` for a `DictionaryBasedBreakIterator`.
    dictionary: Option<DictionaryState>,
}

impl Debug for RuleBasedBreakIterator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuleBasedBreakIterator")
            .field("text", &self.text)
            .field("cached_last_known_break", &self.cached_last_known_break)
            .field("dictionary", &self.dictionary.is_some())
            .finish_non_exhaustive()
    }
}

impl Clone for RuleBasedBreakIterator {
    /// Equivalent to `RuleBasedBreakIterator.clone()`: a copy whose text is a
    /// clone of this one's.
    fn clone(&self) -> Self {
        Self {
            table: self.table,
            text: self.text.as_ref().map(|text| text.clone_box()),
            cached_last_known_break: self.cached_last_known_break,
            dictionary: self.dictionary.clone(),
        }
    }
}

/// `getCurrentCodePointCount()`: 2 at a surrogate pair, else 1.
fn current_code_point_count(text: &mut dyn CharacterIterator) -> i32 {
    let c1 = text.current();
    if is_high_surrogate(c1) && text.get_index() < text.get_end_index() {
        let c2 = text.next();
        text.previous();
        if is_low_surrogate(c2) {
            return 2;
        }
    }
    1
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..0xDC00).contains(&unit)
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..0xE000).contains(&unit)
}

/// `getCurrent()`: the code point at the current index (`DONE`, U+FFFF,
/// past the end).
fn get_current(text: &mut dyn CharacterIterator) -> i32 {
    let c1 = text.current();
    if is_high_surrogate(c1) && text.get_index() < text.get_end_index() {
        let c2 = text.next();
        text.previous();
        if is_low_surrogate(c2) {
            return 0x10000 + ((i32::from(c1) - 0xD800) << 10) + (i32::from(c2) - 0xDC00);
        }
    }
    i32::from(c1)
}

/// `getNext()`: moves to the next code point and returns it, or `DONE`
/// (leaving the index) when none follows.
fn get_next(text: &mut dyn CharacterIterator) -> i32 {
    let index = text.get_index();
    let end_index = text.get_end_index();
    if index == end_index {
        return i32::from(CHAR_DONE);
    }
    let index = index + current_code_point_count(text);
    if index >= end_index {
        return i32::from(CHAR_DONE);
    }
    set_index(text, index);
    get_current(text)
}

/// `getPrevious()`: moves to the previous code point and returns it.
fn get_previous(text: &mut dyn CharacterIterator) -> i32 {
    let c2 = text.previous();
    if is_low_surrogate(c2) && text.get_index() > text.get_begin_index() {
        let c1 = text.previous();
        if is_high_surrogate(c1) {
            return 0x10000 + ((i32::from(c1) - 0xD800) << 10) + (i32::from(c2) - 0xDC00);
        } else {
            text.next();
        }
    }
    i32::from(c2)
}

/// `getNextIndex()`: the index after the current code point, at most the
/// end index.
fn get_next_index(text: &mut dyn CharacterIterator) -> i32 {
    let index = text.get_index() + current_code_point_count(text);
    let end_index = text.get_end_index();
    index.min(end_index)
}

/// `text.setIndex(i)` for an index the iterator itself computed, inside the
/// text.
fn set_index(text: &mut dyn CharacterIterator, index: i32) {
    text.set_index(index)
        .expect("INVARIANT: the iterator only moves inside its text");
}

/// `checkOffset(int, CharacterIterator)`.
fn check_offset(offset: i32, text: &dyn CharacterIterator) -> Result<()> {
    if offset < text.get_begin_index() || offset > text.get_end_index() {
        return Err(LuceneError::illegal_argument("offset out of bounds"));
    }
    Ok(())
}

impl RuleBasedBreakIterator {
    /// A rule-based iterator over `table`, with no text.
    pub(crate) fn new(table: &'static RuleTable) -> Self {
        Self {
            table,
            text: None,
            cached_last_known_break: DONE,
            dictionary: None,
        }
    }

    /// A dictionary-based iterator over `table` and `dictionary`, with no
    /// text: `DictionaryBasedBreakIterator(ruleFile, ruleData,
    /// dictionaryFile, dictionaryData)`.
    pub(crate) fn with_dictionary(
        table: &'static RuleTable,
        dictionary: &'static BreakDictionary,
    ) -> Self {
        Self {
            table,
            text: None,
            cached_last_known_break: DONE,
            dictionary: Some(DictionaryState {
                dictionary,
                dictionary_char_count: 0,
                cached_break_positions: None,
                position_in_cache: 0,
            }),
        }
    }

    /// `getText()`: the text, an empty `StringCharacterIterator` when none
    /// was set.
    fn text(&mut self) -> &mut dyn CharacterIterator {
        &mut **self
            .text
            .get_or_insert_with(|| Box::new(StringCharacterIterator::new(Vec::new())))
    }

    /// `lookupCategory(int)`, which `DictionaryBasedBreakIterator` overrides
    /// to count the dictionary characters it looks up.
    fn lookup_category(&mut self, c: i32) -> i32 {
        let result = self.table.lookup_category(c);
        if let Some(dictionary) = &mut self.dictionary {
            if result != IGNORE && self.table.is_dictionary_category(result) {
                dictionary.dictionary_char_count += 1;
            }
        }
        result
    }

    /// `RuleBasedBreakIterator.handleNext()`: from the current position, runs
    /// the state table until it stops or the text ends, and moves to the end
    /// of the last accepting state reached (or to the lookahead position of
    /// a lookahead rule).
    fn rule_handle_next(&mut self) -> i32 {
        let table = self.table;
        let end = self.text().get_end_index();
        if self.text().get_index() == end {
            return DONE;
        }

        // set our result to the current index plus one
        let mut result = get_next_index(self.text());
        let mut lookahead_result = 0;

        // begin in state 1
        let mut state = START_STATE;
        let mut c = get_current(self.text());

        // loop until we reach the end of the text or transition to state 0
        while c != i32::from(CHAR_DONE) && state != STOP_STATE {
            // look up the current character's character category
            let category = self.lookup_category(c);

            // if the character isn't an ignore character, look up a state
            // transition in the state table
            if category != IGNORE {
                state = table.lookup_state(state, category);
            }

            // if the state we've just transitioned to is a lookahead state,
            // (but not also an end state), save its position.  If it's
            // both a lookahead state and an end state, update the break
            // position to the last saved lookup-state position
            if table.is_lookahead_state(state) {
                if table.is_end_state(state) {
                    result = lookahead_result;
                } else {
                    lookahead_result = get_next_index(self.text());
                }
            } else if table.is_end_state(state) {
                // otherwise, if the state we've just transitioned to is an
                // accepting state, update the break position to be the
                // current iteration position
                result = get_next_index(self.text());
            }

            c = get_next(self.text());
        }

        // if we've run off the end of the text, and the very last character
        // took us into a lookahead state, advance the break position to the
        // lookahead position
        if c == i32::from(CHAR_DONE) && lookahead_result == end {
            result = lookahead_result;
        }

        set_index(self.text(), result);
        result
    }

    /// `handlePrevious()`: from the current position, runs the backwards
    /// state table leftward until it stops (or the text begins), and moves
    /// to the position after the pair of characters it stopped on, a
    /// position the rules always break at.
    fn handle_previous(&mut self) -> i32 {
        let table = self.table;
        let mut state = START_STATE;
        let mut category = 0;
        let mut last_category = 0;
        let mut c = get_current(self.text());

        // loop until we reach the beginning of the text or transition to
        // state 0
        while c != i32::from(CHAR_DONE) && state != STOP_STATE {
            // save the last character's category and look up the current
            // character's category
            last_category = category;
            category = self.lookup_category(c);

            // if the current character isn't an ignore character, look up a
            // state transition in the backwards state table
            if category != IGNORE {
                state = table.lookup_backward_state(state, category);
            }

            // then advance one character backwards
            c = get_previous(self.text());
        }

        // if we didn't march off the beginning of the text, we're either
        // one or two positions away from the real break position.
        // (One because of the call to previous() at the end of the loop
        // above, and another because the character that takes us into the
        // stop state will always be the character BEFORE the break position.)
        if c != i32::from(CHAR_DONE) {
            if last_category != IGNORE {
                get_next(self.text());
                get_next(self.text());
            } else {
                get_next(self.text());
            }
        }
        self.text().get_index()
    }

    /// `handleNext()`: [`rule_handle_next`](Self::rule_handle_next), or, for
    /// a dictionary iterator, `DictionaryBasedBreakIterator.handleNext()`,
    /// which returns the breaks it cached for a range of dictionary
    /// characters, and divides up such a range when the rules return one.
    fn handle_next(&mut self) -> i32 {
        if self.dictionary.is_none() {
            return self.rule_handle_next();
        }

        // if there are no cached break positions, or if we've just moved
        // off the end of the range covered by the cache, we have to dump
        // and possibly regenerate the cache
        let exhausted = {
            let d = self.dictionary.as_ref().expect("INVARIANT: checked above");
            match &d.cached_break_positions {
                None => true,
                Some(cache) => d.position_in_cache == cache.len() as i32 - 1,
            }
        };
        if exhausted {
            // start by using the inherited handleNext() to find a tentative
            // return value.   dictionaryCharCount tells us how many
            // dictionary characters we passed over on our way to the
            // tentative return value
            let start_pos = self.text().get_index();
            self.dictionary_state().dictionary_char_count = 0;
            let result = self.rule_handle_next();

            // if we passed over more than one dictionary character, then we
            // use divideUpDictionaryRange() to regenerate the cached break
            // positions for the new range
            if self.dictionary_state().dictionary_char_count > 1 && result - start_pos > 1 {
                self.divide_up_dictionary_range(start_pos, result);
            } else {
                // otherwise, the value we got back from the inherited
                // fuction is our return value, and we can dump the cache
                self.dictionary_state().cached_break_positions = None;
                return result;
            }
        }

        // if the cache of break positions has been regenerated (or existed
        // all along), then just advance to the next break position in the
        // cache and return it
        let d = self.dictionary_state();
        if let Some(cache) = &d.cached_break_positions {
            d.position_in_cache += 1;
            let position = cache[d.position_in_cache as usize];
            set_index(self.text(), position);
            return position;
        }
        -9999 // SHOULD NEVER GET HERE!
    }

    /// The `DictionaryBasedBreakIterator` state.
    fn dictionary_state(&mut self) -> &mut DictionaryState {
        self.dictionary
            .as_mut()
            .expect("INVARIANT: a dictionary iterator")
    }

    /// `DictionaryBasedBreakIterator.divideUpDictionaryRange(int, int)`:
    /// divides the range `start_pos..end_pos` into words with the
    /// dictionary, caching the break positions. It finds the longest words
    /// it can, backtracking to a shorter word where a longer one leaves no
    /// word to follow it; where no division into dictionary words exists,
    /// it keeps the division that reached farthest and continues one
    /// character after it.
    fn divide_up_dictionary_range(&mut self, start_pos: i32, end_pos: i32) {
        let dictionary = self.dictionary_state().dictionary;

        // the range we're dividing may begin or end with non-dictionary
        // characters (i.e., for line breaking, we may have leading or
        // trailing punctuation that needs to be kept with the word).  Skip
        // over any leading non-dictionary characters
        set_index(self.text(), start_pos);
        let mut c = get_current(self.text());
        let mut category = self.lookup_category(c);
        while category == IGNORE || !self.table.is_dictionary_category(category) {
            c = get_next(self.text());
            category = self.lookup_category(c);
        }

        // the stacks of break positions; `current` and `best` are shared
        // lists, as Java's `Stack` references are, since `current` becomes
        // `best` itself
        let mut current_break_positions: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
        let mut possible_break_positions: Vec<i32> = Vec::new();
        let mut wrong_break_positions: Vec<i32> = Vec::new();

        // the dictionary is implemented as a trie, which is treated as a
        // state machine.  -1 represents the end of a legal word.  Every
        // word in the dictionary is represented by a path from the root
        // node to -1.  A path that ends in state 0 is an illegal
        // combination of characters.
        let mut state = 0;

        // these two variables are used for error handling.  We keep track
        // of the farthest we've gotten through the range being divided, and
        // the combination of breaks that got us that far.  If we use up all
        // possible break combinations, the text contains an error or a word
        // that's not in the dictionary.  In this case, we "bless" the break
        // positions that got us the farthest as real break positions, and
        // then start over from scratch with the character where the error
        // occurred.
        let mut farthest_end_point = self.text().get_index();
        let mut best_break_positions: Option<Rc<RefCell<Vec<i32>>>> = None;

        // initialize (we always exit the loop with a break statement)
        c = get_current(self.text());
        loop {
            // if we can transition to state "-1" from our current state,
            // we're on the last character of a legal word.  Push that
            // position onto the possible-break-positions stack
            if dictionary.end_of_word_state(state) == -1 {
                possible_break_positions.push(self.text().get_index());
            }

            // look up the new state to transition to in the dictionary
            state = dictionary.get_next_state_from_character(state, c);

            // if the character we're sitting on causes us to transition to
            // the "end of word" state, then it was a non-dictionary
            // character and we've successfully traversed the whole range.
            // Drop out of the loop.
            if state == -1 {
                let index = self.text().get_index();
                current_break_positions.borrow_mut().push(index);
                break;
            } else if state == 0 || self.text().get_index() >= end_pos {
                // if the character we're sitting on causes us to transition
                // to the error state, or if we've gone off the end of the
                // range without transitioning to the "end of word" state,
                // we've hit an error...

                // if this is the farthest we've gotten, take note of it in
                // case there's an error in the text
                let index = self.text().get_index();
                if index > farthest_end_point {
                    farthest_end_point = index;
                    let copy = current_break_positions.borrow().clone();
                    best_break_positions = Some(Rc::new(RefCell::new(copy)));
                }

                // wrongBreakPositions is a list of all break positions
                // we've tried starting that didn't allow us to traverse all
                // the way through the text.  Every time we pop a break
                // position off of currentBreakPositions, we put it into
                // wrongBreakPositions to avoid trying it again later.  If we
                // make it to this spot, we're either going to back up to a
                // break in possibleBreakPositions and try starting over from
                // there, or we've exhausted all possible break positions and
                // are going to do the fallback procedure.  This loop
                // prevents us from messing with anything in
                // possibleBreakPositions that didn't work as a starting
                // point the last time we tried it (this is to prevent a bunch
                // of repetitive checks from slowing down some extreme cases)
                while possible_break_positions
                    .last()
                    .is_some_and(|p| wrong_break_positions.contains(p))
                {
                    possible_break_positions.pop();
                }

                // if we've used up all possible break-position combinations,
                // there's an error or an unknown word in the text.  In this
                // case, we start over, treating the farthest character we've
                // reached as the beginning of the range, and "blessing" the
                // break positions that got us that far as real break
                // positions
                if possible_break_positions.is_empty() {
                    if let Some(best) = &best_break_positions {
                        current_break_positions = Rc::clone(best);
                        if farthest_end_point < end_pos {
                            set_index(self.text(), farthest_end_point + 1);
                        } else {
                            break;
                        }
                    } else {
                        let index = self.text().get_index();
                        {
                            let mut current = current_break_positions.borrow_mut();
                            if (current.is_empty()
                                || *current.last().expect("INVARIANT: not empty") != index)
                                && index != start_pos
                            {
                                current.push(index);
                            }
                        }
                        get_next(self.text());
                        let index = self.text().get_index();
                        current_break_positions.borrow_mut().push(index);
                    }
                } else {
                    // if we still have more break positions we can try, then
                    // promote the last break in possibleBreakPositions into
                    // realBreakPositions, and get rid of all entries in
                    // realBreakPositions that come after it.  Then back up to
                    // that position and start over from there (i.e., treat
                    // that position as the beginning of a new word)
                    let temp = possible_break_positions
                        .pop()
                        .expect("INVARIANT: not empty");
                    {
                        let mut current = current_break_positions.borrow_mut();
                        while current.last().is_some_and(|&top| temp < top) {
                            let temp2 = current.pop().expect("INVARIANT: not empty");
                            wrong_break_positions.push(temp2);
                        }
                        current.push(temp);
                    }
                    let top = *current_break_positions
                        .borrow()
                        .last()
                        .expect("INVARIANT: just pushed");
                    set_index(self.text(), top);
                }

                // re-sync "c" for the next go-round, and drop out of the
                // loop if we've made it off the end of the range
                c = get_current(self.text());
                if self.text().get_index() >= end_pos {
                    break;
                }
            } else {
                // if we didn't hit any exceptional conditions on this last
                // iteration, just advance to the next character and loop
                c = get_next(self.text());
            }
        }

        // dump the last break position in the list, and replace it with the
        // actual end of the range (which may be the same character, or may
        // be further on because the range actually ended with non-dictionary
        // characters we want to keep with the word)
        let mut current = current_break_positions.borrow().clone();
        if !current.is_empty() {
            current.pop();
        }
        current.push(end_pos);

        // create a regular array to hold the break positions and copy the
        // break positions from the stack to the array (in addition, our
        // starting position goes into this array as a break position).
        // This array becomes the cache of break positions used by next()
        // and previous(), so this is where we actually refresh the cache.
        let mut cached = Vec::with_capacity(current.len() + 1);
        cached.push(start_pos);
        cached.extend_from_slice(&current);
        let d = self.dictionary_state();
        d.cached_break_positions = Some(cached);
        d.position_in_cache = 0;
    }

    /// `RuleBasedBreakIterator.previous()`.
    fn rule_previous(&mut self) -> i32 {
        // if we're already sitting at the beginning of the text, return DONE
        let begin = self.text().get_begin_index();
        if self.current() == begin {
            return DONE;
        }

        // set things up.  handlePrevious() will back us up to some valid
        // break position before the current position (we back our internal
        // iterator up one step to prevent handlePrevious() from returning
        // the current position), but not necessarily the last one before
        // where we started
        let start = self.current();
        let mut last_result = self.cached_last_known_break;
        if last_result >= start || last_result <= DONE {
            get_previous(self.text());
            last_result = self.handle_previous();
        } else {
            // it might be better to check if handlePrevious() give us
            // closer safe value but handlePrevious() is slow too so
            // assume cachedLastKnownBreak is ok
            set_index(self.text(), last_result);
        }
        let mut result = last_result;

        // iterate forward from the known break position until we pass our
        // starting point.  The last break position before the starting
        // point is our return value
        while result != DONE && result < start {
            last_result = result;
            result = self.handle_next();
        }

        // set the current iteration position to be the last break position
        // before where we started, and then return that value
        set_index(self.text(), last_result);
        self.cached_last_known_break = last_result;
        last_result
    }

    /// `RuleBasedBreakIterator.following(int)`.
    fn rule_following(&mut self, offset: i32) -> Result<i32> {
        check_offset(offset, self.text())?;

        // Set our internal iteration position (temporarily)
        // to the position passed in.  If this is the _beginning_ position,
        // then we can just use next() to get our return value
        set_index(self.text(), offset);
        if offset == self.text().get_begin_index() {
            self.cached_last_known_break = self.handle_next();
            return Ok(self.cached_last_known_break);
        }

        // otherwise, we have to sync up first.  Use handlePrevious() to back
        // up to a known break position before the specified position (if
        // we can determine that the specified position is a break position,
        // we don't back up at all).  This may or may not be the last break
        // position at or before our starting position.  Advance forward
        // from here until we've passed the starting position.  The position
        // we stop on will be the first break position after the specified one.
        let mut result = self.cached_last_known_break;
        if result >= offset || result <= DONE {
            result = self.handle_previous();
        } else {
            // it might be better to check if handlePrevious() give us
            // closer safe value but handlePrevious() is slow too so
            // assume cachedLastKnownBreak is ok
            set_index(self.text(), result);
        }
        while result != DONE && result <= offset {
            result = self.handle_next();
        }
        self.cached_last_known_break = result;
        Ok(result)
    }

    /// The `DictionaryBasedBreakIterator` resets of `first()`, `last()` and
    /// `setText()`: no cached breaks.
    fn reset_dictionary(&mut self) {
        if let Some(d) = &mut self.dictionary {
            d.cached_break_positions = None;
            d.dictionary_char_count = 0;
            d.position_in_cache = 0;
        }
    }
}

impl BreakIterator for RuleBasedBreakIterator {
    /// Equivalent to `RuleBasedBreakIterator.first()` (and
    /// `DictionaryBasedBreakIterator.first()`, which first drops its
    /// cache).
    fn first(&mut self) -> i32 {
        self.reset_dictionary();
        let t = self.text();
        t.first();
        t.get_index()
    }

    /// Equivalent to `RuleBasedBreakIterator.last()` (and
    /// `DictionaryBasedBreakIterator.last()`, which first drops its cache).
    fn last(&mut self) -> i32 {
        self.reset_dictionary();
        let t = self.text();
        let end = t.get_end_index();
        set_index(t, end);
        t.get_index()
    }

    /// Equivalent to `RuleBasedBreakIterator.next(int)`.
    fn next_n(&mut self, mut n: i32) -> i32 {
        let mut result = self.current();
        while n > 0 {
            result = self.handle_next();
            n -= 1;
        }
        while n < 0 {
            result = self.previous();
            n += 1;
        }
        result
    }

    /// Equivalent to `RuleBasedBreakIterator.next()`, `handleNext()`.
    fn next(&mut self) -> i32 {
        self.handle_next()
    }

    /// Equivalent to `RuleBasedBreakIterator.previous()`, and for a
    /// dictionary iterator to `DictionaryBasedBreakIterator.previous()`,
    /// which first steps back through its cache.
    fn previous(&mut self) -> i32 {
        if self.dictionary.is_none() {
            return self.rule_previous();
        }
        // if we have cached break positions and we're still in the range
        // covered by them, just move one step backward in the cache
        let cached = {
            let d = self.dictionary_state();
            match &d.cached_break_positions {
                Some(cache) if d.position_in_cache > 0 => {
                    d.position_in_cache -= 1;
                    Some(cache[d.position_in_cache as usize])
                }
                _ => None,
            }
        };
        if let Some(position) = cached {
            set_index(self.text(), position);
            return position;
        }
        // otherwise, dump the cache and use the inherited previous() method
        // to move backward.  This may fill up the cache with new break
        // positions, in which case we have to mark our position in the
        // cache
        self.dictionary_state().cached_break_positions = None;
        let result = self.rule_previous();
        let d = self.dictionary_state();
        if let Some(cache) = &d.cached_break_positions {
            d.position_in_cache = cache.len() as i32 - 2;
        }
        result
    }

    /// Equivalent to `RuleBasedBreakIterator.following(int)`, and for a
    /// dictionary iterator to `DictionaryBasedBreakIterator.following(int)`,
    /// which answers from its cache when `offset` is inside the cached range.
    fn following(&mut self, offset: i32) -> Result<i32> {
        if self.dictionary.is_none() {
            return self.rule_following(offset);
        }
        check_offset(offset, self.text())?;
        // if we have no cached break positions, or "offset" is outside the
        // range covered by the cache, then dump the cache and call our
        // inherited following() method.  This will call other methods in
        // this class that may refresh the cache.
        let found = {
            let d = self.dictionary_state();
            match &d.cached_break_positions {
                Some(cache) if offset >= cache[0] && offset < cache[cache.len() - 1] => {
                    // on the other hand, if "offset" is within the range
                    // covered by the cache, then just search the cache for
                    // the first break position after "offset"
                    let mut position = 0usize;
                    while position < cache.len() && offset >= cache[position] {
                        position += 1;
                    }
                    d.position_in_cache = position as i32;
                    Some(cache[position])
                }
                _ => {
                    d.cached_break_positions = None;
                    None
                }
            }
        };
        match found {
            Some(position) => {
                set_index(self.text(), position);
                Ok(self.text().get_index())
            }
            None => self.rule_following(offset),
        }
    }

    /// Equivalent to `RuleBasedBreakIterator.preceding(int)`, and for a
    /// dictionary iterator to `DictionaryBasedBreakIterator.preceding(int)`,
    /// which answers from its cache when `offset` is inside the cached range.
    fn preceding(&mut self, offset: i32) -> Result<i32> {
        check_offset(offset, self.text())?;
        if let Some(d) = &mut self.dictionary {
            match &d.cached_break_positions {
                Some(cache) if offset > cache[0] && offset <= cache[cache.len() - 1] => {
                    // on the other hand, if "offset" is within the range
                    // covered by the cache, then just search the cache for
                    // the last break position before "offset"
                    let mut position = 0usize;
                    while position < cache.len() && offset > cache[position] {
                        position += 1;
                    }
                    let position = position as i32 - 1;
                    d.position_in_cache = position;
                    let value = cache[position as usize];
                    set_index(self.text(), value);
                    return Ok(self.text().get_index());
                }
                _ => {
                    // if we have no cached break positions, or "offset" is
                    // outside the range covered by the cache, we can just
                    // call the inherited routine (which will eventually call
                    // other routines in this class that may refresh the cache)
                    d.cached_break_positions = None;
                }
            }
        }
        // `RuleBasedBreakIterator.preceding(int)`: move to the offset and
        // use previous() to find the boundary before it
        set_index(self.text(), offset);
        Ok(self.previous())
    }

    /// Equivalent to `RuleBasedBreakIterator.isBoundary(int)`.
    fn is_boundary(&mut self, offset: i32) -> Result<bool> {
        check_offset(offset, self.text())?;
        if offset == self.text().get_begin_index() {
            Ok(true)
        } else {
            // to check whether this is a boundary, we can use following() on
            // the position before the specified one and return true if the
            // position we get back is the one the user specified
            Ok(self.following(offset - 1)? == offset)
        }
    }

    /// Equivalent to `RuleBasedBreakIterator.current()`: the text's index (0
    /// with no text, the index of the empty iterator `getText()` creates).
    fn current(&self) -> i32 {
        self.text.as_ref().map_or(0, |t| t.get_index())
    }

    fn get_text(&mut self) -> &dyn CharacterIterator {
        self.text()
    }

    /// Equivalent to `RuleBasedBreakIterator.setText(CharacterIterator)` (and
    /// `DictionaryBasedBreakIterator.setText`, which also drops its cache):
    /// an iterator that cannot move to its end index is wrapped in the
    /// JDK's `SafeCharIterator`.
    fn set_text(&mut self, mut new_text: Box<dyn CharacterIterator>) {
        // Test iterator to see if we need to wrap it in a SafeCharIterator.
        // The correct behavior for CharacterIterators is to allow the
        // position to be set to the endpoint of the iterator.  Many
        // CharacterIterators do not uphold this, so this is a workaround
        // to permit them to use this class.
        let end = new_text.get_end_index();
        let good_iterator = match new_text.set_index(end) {
            Ok(_) => new_text.get_index() == end,
            Err(_) => false,
        };
        if !good_iterator {
            new_text = Box::new(SafeCharIterator::new(new_text));
        }
        new_text.first();
        self.text = Some(new_text);
        self.cached_last_known_break = DONE;
        self.reset_dictionary();
    }

    fn clone_box(&self) -> Box<dyn BreakIterator> {
        Box::new(self.clone())
    }
}

/// The `CharacterIterator` `RuleBasedBreakIterator.setText` wraps an
/// iterator in when that iterator cannot be positioned at its end index.
///
/// Equivalent to the private class
/// `sun.text.RuleBasedBreakIterator.SafeCharIterator`.
#[derive(Debug)]
struct SafeCharIterator {
    base: Box<dyn CharacterIterator>,
    range_start: i32,
    range_limit: i32,
    current_index: i32,
}

impl SafeCharIterator {
    fn new(base: Box<dyn CharacterIterator>) -> Self {
        let range_start = base.get_begin_index();
        let range_limit = base.get_end_index();
        let current_index = base.get_index();
        Self {
            base,
            range_start,
            range_limit,
            current_index,
        }
    }

    /// `base.setIndex(i)` for an index inside the base's text.
    fn base_at(&mut self, index: i32) -> u16 {
        self.base.set_index(index).unwrap_or(CHAR_DONE)
    }
}

impl CharacterIterator for SafeCharIterator {
    fn first(&mut self) -> u16 {
        self.set_index(self.range_start).unwrap_or(CHAR_DONE)
    }

    fn last(&mut self) -> u16 {
        self.set_index(self.range_limit - 1).unwrap_or(CHAR_DONE)
    }

    fn current(&self) -> u16 {
        if self.current_index < self.range_start || self.current_index >= self.range_limit {
            CHAR_DONE
        } else {
            // `base.setIndex(currentIndex)`: answered without moving the
            // base, which only this wrapper positions
            let mut base = self.base.clone_box();
            base.set_index(self.current_index).unwrap_or(CHAR_DONE)
        }
    }

    fn next(&mut self) -> u16 {
        self.current_index += 1;
        if self.current_index >= self.range_limit {
            self.current_index = self.range_limit;
            CHAR_DONE
        } else {
            self.base_at(self.current_index)
        }
    }

    fn previous(&mut self) -> u16 {
        self.current_index -= 1;
        if self.current_index < self.range_start {
            self.current_index = self.range_start;
            CHAR_DONE
        } else {
            self.base_at(self.current_index)
        }
    }

    fn set_index(&mut self, position: i32) -> Result<u16> {
        if position < self.range_start || position > self.range_limit {
            return Err(LuceneError::illegal_argument("Invalid position"));
        }
        self.current_index = position;
        Ok(self.current())
    }

    fn get_begin_index(&self) -> i32 {
        self.range_start
    }

    fn get_end_index(&self) -> i32 {
        self.range_limit
    }

    fn get_index(&self) -> i32 {
        self.current_index
    }

    fn clone_box(&self) -> Box<dyn CharacterIterator> {
        Box::new(SafeCharIterator {
            base: self.base.clone_box(),
            range_start: self.range_start,
            range_limit: self.range_limit,
            current_index: self.current_index,
        })
    }
}
