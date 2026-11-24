// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UnicodeSetIterator`.

use std::borrow::Cow;

use rucene::error::Result;

use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// `IS_STRING`: the value of `codepoint` when the iterator is at a string.
pub const IS_STRING: i32 = -1;

/// Iterates over the contents of a `UnicodeSet`: its code points (singly
/// or by range), then its strings.
///
/// Equivalent to `com.ibm.icu.text.UnicodeSetIterator`. The strings are
/// visited from a snapshot taken at `reset`, where Java holds an iterator
/// over the set's `TreeSet` (the same sequence while the set is unchanged;
/// Java fails on a concurrent change).
#[derive(Debug, Clone)]
pub struct UnicodeSetIterator<'a> {
    /// `codepoint`: the current code point, or `IS_STRING`.
    pub codepoint: i32,
    /// `codepointEnd`: the end of the current range (`nextRange`).
    pub codepoint_end: i32,
    /// `string`: the current string when `codepoint == IS_STRING`.
    pub string: Option<Vec<u16>>,
    set: Cow<'a, UnicodeSet>,
    end_range: i32,
    range: i32,
    end_element: i32,
    next_element: i32,
    strings: Option<Vec<Vec<u16>>>,
    string_index: usize,
}

impl<'a> UnicodeSetIterator<'a> {
    /// `UnicodeSetIterator(UnicodeSet set)`.
    pub fn new(set: &'a UnicodeSet) -> Self {
        let mut it = Self::blank(Cow::Borrowed(set));
        it.reset();
        it
    }

    /// `UnicodeSetIterator()`: over an empty set.
    pub fn new_empty() -> UnicodeSetIterator<'static> {
        let mut it = UnicodeSetIterator::blank(Cow::Owned(UnicodeSet::new()));
        it.reset();
        it
    }

    fn blank(set: Cow<'a, UnicodeSet>) -> Self {
        Self {
            codepoint: 0,
            codepoint_end: 0,
            string: None,
            set,
            end_range: 0,
            range: 0,
            end_element: 0,
            next_element: 0,
            strings: None,
            string_index: 0,
        }
    }

    /// `skipToStrings()`.
    pub fn skip_to_strings(&mut self) -> &mut Self {
        // Finish code point/range iteration.
        self.range = self.end_range;
        self.end_element = -1;
        self.next_element = 0;
        self
    }

    fn next_string(&mut self) -> bool {
        let Some(strings) = &self.strings else {
            return false;
        };
        self.codepoint = IS_STRING;
        self.string = Some(strings[self.string_index].clone());
        self.string_index += 1;
        if self.string_index >= strings.len() {
            self.strings = None;
        }
        true
    }

    /// `next()`: advances to the next code point or string.
    #[allow(clippy::should_implement_trait)] // Java method name
    pub fn next(&mut self) -> bool {
        if self.next_element <= self.end_element {
            self.codepoint = self.next_element;
            self.codepoint_end = self.next_element;
            self.next_element += 1;
            return true;
        }
        if self.range < self.end_range {
            self.range += 1;
            self.load_range(self.range);
            self.codepoint = self.next_element;
            self.codepoint_end = self.next_element;
            self.next_element += 1;
            return true;
        }
        self.next_string()
    }

    /// `nextRange()`: advances to the next code point range or string.
    pub fn next_range(&mut self) -> bool {
        if self.next_element <= self.end_element {
            self.codepoint_end = self.end_element;
            self.codepoint = self.next_element;
            self.next_element = self.end_element + 1;
            return true;
        }
        if self.range < self.end_range {
            self.range += 1;
            self.load_range(self.range);
            self.codepoint_end = self.end_element;
            self.codepoint = self.next_element;
            self.next_element = self.end_element + 1;
            return true;
        }
        self.next_string()
    }

    /// `reset(UnicodeSet uset)`.
    pub fn reset_to(&mut self, uset: &'a UnicodeSet) {
        self.set = Cow::Borrowed(uset);
        self.reset();
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.end_range = self.set.get_range_count() - 1;
        self.range = 0;
        self.end_element = -1;
        self.next_element = 0;
        if self.end_range >= 0 {
            self.load_range(self.range);
        }
        if self.set.has_strings() {
            self.strings = Some(self.set.strings().cloned().collect());
            self.string_index = 0;
        } else {
            self.strings = None;
        }
    }

    /// `getString()`: the current string, or the current code point as a
    /// string.
    ///
    /// # Errors
    ///
    /// The errors of `UTF16.valueOf` for an invalid code point.
    pub fn get_string(&self) -> Result<Vec<u16>> {
        if self.codepoint != IS_STRING {
            return UTF16::value_of(self.codepoint);
        }
        Ok(self.string.clone().unwrap_or_default())
    }

    fn load_range(&mut self, a_range: i32) {
        self.next_element = self.set.get_range_start(a_range);
        self.end_element = self.set.get_range_end(a_range);
    }
}
