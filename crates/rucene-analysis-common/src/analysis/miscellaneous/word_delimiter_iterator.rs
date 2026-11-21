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
//! Breaks a term into subwords, ported from
//! `org.apache.lucene.analysis.miscellaneous.WordDelimiterIterator`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use crate::jdk::character::{
    get_type as java_get_type, is_digit as java_is_digit, is_lower_case_latin1,
    is_upper_case_latin1, COMBINING_SPACING_MARK, DECIMAL_DIGIT_NUMBER, ENCLOSING_MARK,
    LETTER_NUMBER, LOWERCASE_LETTER, MODIFIER_LETTER, NON_SPACING_MARK, OTHER_LETTER, OTHER_NUMBER,
    SURROGATE, TITLECASE_LETTER, UPPERCASE_LETTER,
};

/// Equivalent to `WordDelimiterIterator.LOWER`.
pub(crate) const LOWER: u8 = 0x01;
/// Equivalent to `WordDelimiterIterator.UPPER`.
pub(crate) const UPPER: u8 = 0x02;
/// Equivalent to `WordDelimiterIterator.DIGIT`.
pub(crate) const DIGIT: u8 = 0x04;
/// Equivalent to `WordDelimiterIterator.SUBWORD_DELIM`.
pub(crate) const SUBWORD_DELIM: u8 = 0x08;

// combinations: for testing, not for setting bits
/// Equivalent to `WordDelimiterIterator.ALPHA`.
pub const ALPHA: u8 = 0x03;
/// Equivalent to `WordDelimiterIterator.ALPHANUM`.
pub const ALPHANUM: u8 = 0x07;

/// Indicates the end of iteration.
///
/// Equivalent to `WordDelimiterIterator.DONE`.
pub const DONE: i32 = -1;

/// The default character type table: Latin-1 lower case letters are
/// `LOWER`, upper case letters `UPPER`, digits `DIGIT`, anything else
/// `SUBWORD_DELIM`.
///
/// Equivalent to `WordDelimiterIterator.DEFAULT_WORD_DELIM_TABLE`, built by
/// its static initializer with `Character.isLowerCase(int)`,
/// `isUpperCase(int)` and `isDigit(int)`.
pub static DEFAULT_WORD_DELIM_TABLE: LazyLock<Arc<[u8]>> = LazyLock::new(|| {
    let mut tab = vec![0u8; 256];
    for (i, slot) in tab.iter_mut().enumerate() {
        let i = i as i32;
        let mut code = 0u8;
        if is_lower_case_latin1(i) {
            code |= LOWER;
        } else if is_upper_case_latin1(i) {
            code |= UPPER;
        } else if java_is_digit(i) {
            code |= DIGIT;
        }
        if code == 0 {
            code = SUBWORD_DELIM;
        }
        *slot = code;
    }
    Arc::from(tab)
});

/// A BreakIterator-like API for iterating over subwords in text, according
/// to `WordDelimiterGraphFilter` rules.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.WordDelimiterIterator`
/// (`@lucene.internal`). The text is UTF-16 units, as Java's `char[] text`
/// is, and positions count units; the iterator owns a copy of the text the
/// filters hand it (Java keeps a reference to their buffer, which they do
/// not change while it is read).
#[derive(Clone)]
pub struct WordDelimiterIterator {
    pub(crate) text: Vec<u16>,
    pub(crate) length: i32,

    /// start position of text, excluding leading delimiters
    pub(crate) start_bounds: i32,
    /// end position of text, excluding trailing delimiters
    pub(crate) end_bounds: i32,

    /// Beginning of subword
    pub(crate) current: i32,
    /// End of subword
    pub(crate) end: i32,

    /// does this string end with a possessive such as 's
    has_final_possessive: bool,

    /// If false, causes case changes to be ignored (subwords will only be
    /// generated given SUBWORD_DELIM tokens). (Defaults to true)
    pub(crate) split_on_case_change: bool,

    /// If false, causes numeric changes to be ignored (subwords will only be
    /// generated given SUBWORD_DELIM tokens). (Defaults to true)
    pub(crate) split_on_numerics: bool,

    /// If true, causes trailing "'s" to be removed for each subword.
    /// (Defaults to true)
    ///
    /// "O'Neil's" => "O", "Neil"
    pub(crate) stem_english_possessive: bool,

    char_type_table: Arc<[u8]>,

    /// if true, need to skip over a possessive found in the last call to
    /// next()
    skip_possessive: bool,
}

impl fmt::Debug for WordDelimiterIterator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl WordDelimiterIterator {
    /// Create a new WordDelimiterIterator operating with the supplied rules:
    /// `char_type_table` the table containing character types,
    /// `split_on_case_change` whether to split on case changes, and
    /// `split_on_numerics` on numeric changes, `stem_english_possessive`
    /// whether to strip trailing English possessives.
    ///
    /// Equivalent to the package-private `WordDelimiterIterator(byte[],
    /// boolean, boolean, boolean)`.
    pub(crate) fn new(
        char_type_table: Arc<[u8]>,
        split_on_case_change: bool,
        split_on_numerics: bool,
        stem_english_possessive: bool,
    ) -> Self {
        Self {
            text: Vec::new(),
            length: 0,
            start_bounds: 0,
            end_bounds: 0,
            current: 0,
            end: 0,
            has_final_possessive: false,
            split_on_case_change,
            split_on_numerics,
            stem_english_possessive,
            char_type_table,
            skip_possessive: false,
        }
    }

    /// Advance to the next subword in the string. Returns index of the next
    /// subword, or [`DONE`] if all subwords have been returned.
    ///
    /// Equivalent to `WordDelimiterIterator.next()`.
    pub(crate) fn next(&mut self) -> i32 {
        self.current = self.end;
        if self.current == DONE {
            return DONE;
        }

        if self.skip_possessive {
            self.current += 2;
            self.skip_possessive = false;
        }

        let mut last_type = 0;

        while self.current < self.end_bounds && {
            last_type = self.char_type(self.text[self.current as usize]);
            is_subword_delim(last_type)
        } {
            self.current += 1;
        }

        if self.current >= self.end_bounds {
            self.end = DONE;
            return DONE;
        }

        self.end = self.current + 1;
        while self.end < self.end_bounds {
            let type_ = self.char_type(self.text[self.end as usize]);
            if self.is_break(last_type, type_) {
                break;
            }
            last_type = type_;
            self.end += 1;
        }

        if self.end < self.end_bounds - 1 && self.ends_with_possessive(self.end + 2) {
            self.skip_possessive = true;
        }

        self.end
    }

    /// Return the type of the current subword. This currently uses the type
    /// of the first character in the subword.
    ///
    /// Equivalent to `WordDelimiterIterator.type()`.
    pub(crate) fn type_(&self) -> u8 {
        if self.end == DONE {
            return 0;
        }

        let type_ = self.char_type(self.text[self.current as usize]);
        match type_ {
            // return ALPHA word type for both lower and upper
            LOWER | UPPER => ALPHA,
            _ => type_,
        }
    }

    /// Reset the text to a new value, and reset all state: `text` holds the
    /// new text (Java's `char[]`, UTF-16 units), `length` the length of the
    /// text.
    ///
    /// Equivalent to `WordDelimiterIterator.setText(char[], int)`.
    pub(crate) fn set_text(&mut self, text: &[u16], length: usize) {
        self.text.clear();
        self.text.extend_from_slice(&text[..length]);
        self.length = length as i32;
        self.end_bounds = length as i32;
        self.current = 0;
        self.start_bounds = 0;
        self.end = 0;
        self.skip_possessive = false;
        self.has_final_possessive = false;
        self.set_bounds();
    }

    // ================================================= Helper Methods ================================================

    /// Determines whether the transition from lastType to type indicates a
    /// break.
    ///
    /// Equivalent to the private `WordDelimiterIterator.isBreak(int, int)`.
    fn is_break(&self, last_type: u8, type_: u8) -> bool {
        if (type_ & last_type) != 0 {
            return false;
        }

        if !self.split_on_case_change && is_alpha(last_type) && is_alpha(type_) {
            // ALPHA->ALPHA: always ignore if case isn't considered.
            false
        } else if is_upper(last_type) && is_alpha(type_) {
            // UPPER->letter: Don't split
            false
        } else if !self.split_on_numerics
            && ((is_alpha(last_type) && is_digit(type_))
                || (is_digit(last_type) && is_alpha(type_)))
        {
            // ALPHA->NUMERIC, NUMERIC->ALPHA :Don't split
            false
        } else {
            true
        }
    }

    /// Determines if the current word contains only one subword. Note, it
    /// could be potentially surrounded by delimiters.
    ///
    /// Equivalent to `WordDelimiterIterator.isSingleWord()`.
    pub(crate) fn is_single_word(&self) -> bool {
        if self.has_final_possessive {
            self.current == self.start_bounds && self.end == self.end_bounds - 2
        } else {
            self.current == self.start_bounds && self.end == self.end_bounds
        }
    }

    /// Set the internal word bounds (remove leading and trailing delimiters).
    /// Note, if a possessive is found, don't remove it yet, simply note it.
    ///
    /// Equivalent to the private `WordDelimiterIterator.setBounds()`.
    fn set_bounds(&mut self) {
        while self.start_bounds < self.length
            && is_subword_delim(self.char_type(self.text[self.start_bounds as usize]))
        {
            self.start_bounds += 1;
        }

        while self.end_bounds > self.start_bounds
            && is_subword_delim(self.char_type(self.text[(self.end_bounds - 1) as usize]))
        {
            self.end_bounds -= 1;
        }
        if self.ends_with_possessive(self.end_bounds) {
            self.has_final_possessive = true;
        }
        self.current = self.start_bounds;
    }

    /// Determines if the text at the given position indicates an English
    /// possessive which should be removed.
    ///
    /// Equivalent to the private `WordDelimiterIterator.endsWithPossessive(int)`.
    fn ends_with_possessive(&self, pos: i32) -> bool {
        let at = |i: i32| self.text[i as usize];
        self.stem_english_possessive
            && pos > 2
            && at(pos - 2) == u16::from(b'\'')
            && (at(pos - 1) == u16::from(b's') || at(pos - 1) == u16::from(b'S'))
            && is_alpha(self.char_type(at(pos - 3)))
            && (pos == self.end_bounds || is_subword_delim(self.char_type(at(pos))))
    }

    /// Determines the type of the given character.
    ///
    /// Equivalent to the private `WordDelimiterIterator.charType(int)`.
    fn char_type(&self, ch: u16) -> u8 {
        if (ch as usize) < self.char_type_table.len() {
            return self.char_type_table[ch as usize];
        }
        get_type(i32::from(ch))
    }
}

impl fmt::Display for WordDelimiterIterator {
    /// Equivalent to `WordDelimiterIterator.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.end == DONE {
            return f.write_str("DONE");
        }
        let text = String::from_utf16_lossy(&self.text[self.current as usize..self.end as usize]);
        // `String.format(Locale.ROOT, "%#02x", type())`
        write!(
            f,
            "{text} [{}-{}] type={:#x}",
            self.current,
            self.end,
            self.type_()
        )
    }
}

/// Computes the type of the given character.
///
/// Equivalent to the static `WordDelimiterIterator.getType(int)`, over
/// `Character.getType(int)`.
pub fn get_type(ch: i32) -> u8 {
    match java_get_type(ch) {
        UPPERCASE_LETTER => UPPER,
        LOWERCASE_LETTER => LOWER,

        TITLECASE_LETTER | MODIFIER_LETTER | OTHER_LETTER | NON_SPACING_MARK
        | ENCLOSING_MARK // depends what it encloses?
        | COMBINING_SPACING_MARK => ALPHA,

        DECIMAL_DIGIT_NUMBER | LETTER_NUMBER | OTHER_NUMBER => DIGIT,

        // case Character.SPACE_SEPARATOR:
        // case Character.LINE_SEPARATOR:
        // case Character.PARAGRAPH_SEPARATOR:
        // case Character.CONTROL:
        // case Character.FORMAT:
        // case Character.PRIVATE_USE:
        SURROGATE => ALPHA | DIGIT, // prevent splitting

        // case Character.DASH_PUNCTUATION:
        // case Character.START_PUNCTUATION:
        // case Character.END_PUNCTUATION:
        // case Character.CONNECTOR_PUNCTUATION:
        // case Character.OTHER_PUNCTUATION:
        // case Character.MATH_SYMBOL:
        // case Character.CURRENCY_SYMBOL:
        // case Character.MODIFIER_SYMBOL:
        // case Character.OTHER_SYMBOL:
        // case Character.INITIAL_QUOTE_PUNCTUATION:
        // case Character.FINAL_QUOTE_PUNCTUATION:
        _ => SUBWORD_DELIM,
    }
}

/// Checks if the given word type includes `ALPHA`.
///
/// Equivalent to `WordDelimiterIterator.isAlpha(int)`.
pub(crate) fn is_alpha(type_: u8) -> bool {
    (type_ & ALPHA) != 0
}

/// Checks if the given word type includes `DIGIT`.
///
/// Equivalent to `WordDelimiterIterator.isDigit(int)`.
pub(crate) fn is_digit(type_: u8) -> bool {
    (type_ & DIGIT) != 0
}

/// Checks if the given word type includes `SUBWORD_DELIM`.
///
/// Equivalent to `WordDelimiterIterator.isSubwordDelim(int)`.
pub(crate) fn is_subword_delim(type_: u8) -> bool {
    (type_ & SUBWORD_DELIM) != 0
}

/// Checks if the given word type includes `UPPER`.
///
/// Equivalent to `WordDelimiterIterator.isUpper(int)`.
pub(crate) fn is_upper(type_: u8) -> bool {
    (type_ & UPPER) != 0
}
