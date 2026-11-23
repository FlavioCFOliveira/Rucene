// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.ReplaceableContextIterator`.

use crate::r#impl::u_case_props::ContextIterator;
use crate::text::replaceable::Replaceable;
use crate::text::utf16::UTF16;

/// Implementation of `UCaseProps.ContextIterator`, iterates over a
/// `Replaceable`. See `ustrcase.c/utf16_caseContextIterator()`. See
/// `ustrcase.c/ReplaceableContextIterator`.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.ReplaceableContextIterator`. Java keeps the text in a
/// field set by `setText`; here the field borrows it (`'a`), since the
/// iterator lives for one transliteration call.
pub struct ReplaceableContextIterator<'a> {
    rep: Option<&'a dyn Replaceable>,
    index: i32,
    limit: i32,
    cp_start: i32,
    cp_limit: i32,
    context_start: i32,
    context_limit: i32,
    dir: i32,
    reached_limit: bool,
}

impl Default for ReplaceableContextIterator<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> ReplaceableContextIterator<'a> {
    /// `ReplaceableContextIterator()`: constructor.
    pub fn new() -> Self {
        Self {
            rep: None,
            limit: 0,
            cp_start: 0,
            cp_limit: 0,
            index: 0,
            context_start: 0,
            context_limit: 0,
            dir: 0,
            reached_limit: false,
        }
    }

    fn rep(&self) -> &'a dyn Replaceable {
        self.rep
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: Cannot invoke \"com.ibm.icu.text.Replaceable.length()\" because \"this.rep\" is null"))
    }

    /// `setText(Replaceable rep)`: set the text for iteration.
    pub fn set_text(&mut self, rep: &'a dyn Replaceable) {
        self.rep = Some(rep);
        self.limit = rep.length();
        self.context_limit = self.limit;
        self.cp_start = 0;
        self.cp_limit = 0;
        self.index = 0;
        self.context_start = 0;
        self.dir = 0;
        self.reached_limit = false;
    }

    /// `setIndex(int index)`: set the index where `nextCaseMapCP()` is to
    /// start iterating.
    pub fn set_index(&mut self, index: i32) {
        self.cp_start = index;
        self.cp_limit = index;
        self.index = 0;
        self.dir = 0;
        self.reached_limit = false;
    }

    /// `getCaseMapCPStart()`: get the index of where the code point
    /// currently being case-mapped starts.
    pub fn get_case_map_cp_start(&self) -> i32 {
        self.cp_start
    }

    /// `setLimit(int lim)`: set the iteration limit for `nextCaseMapCP()`
    /// to an index within the string. If the limit parameter is negative
    /// or past the string, then the string length is restored as the
    /// iteration limit.
    pub fn set_limit(&mut self, lim: i32) {
        let len = self.rep().length();
        if 0 <= lim && lim <= len {
            self.limit = lim;
        } else {
            self.limit = len;
        }
        self.reached_limit = false;
    }

    /// `setContextLimits(int contextStart, int contextLimit)`: set the
    /// start and limit indexes for context iteration with `next()`.
    pub fn set_context_limits(&mut self, context_start: i32, context_limit: i32) {
        let len = self.rep().length();
        if context_start < 0 {
            self.context_start = 0;
        } else if context_start <= len {
            self.context_start = context_start;
        } else {
            self.context_start = len;
        }
        if context_limit < self.context_start {
            self.context_limit = self.context_start;
        } else if context_limit <= len {
            self.context_limit = context_limit;
        } else {
            self.context_limit = len;
        }
        self.reached_limit = false;
    }

    /// `nextCaseMapCP()`: iterate forward through the string to fetch the
    /// next code point to be case-mapped, and set the context indexes for
    /// it. Returns the next code point to be case-mapped, or <0 when the
    /// iteration is done.
    pub fn next_case_map_cp(&mut self) -> i32 {
        if self.cp_limit < self.limit {
            self.cp_start = self.cp_limit;
            let c = self.rep().char32_at(self.cp_limit);
            self.cp_limit += UTF16::get_char_count(c);
            c
        } else {
            -1
        }
    }

    /// `replace(String text)`: replace the current code point by its case
    /// mapping, and update the indexes. Returns the delta for the change
    /// of the text length.
    pub fn replace(&mut self, text: &[u16]) -> i32 {
        let delta = text.len() as i32 - (self.cp_limit - self.cp_start);
        self.rep().replace(self.cp_start, self.cp_limit, text);
        self.cp_limit += delta;
        self.limit += delta;
        self.context_limit += delta;
        delta
    }

    /// `didReachLimit()`: did forward context iteration with `next()` reach
    /// the iteration limit?
    pub fn did_reach_limit(&self) -> bool {
        self.reached_limit
    }
}

impl ContextIterator for ReplaceableContextIterator<'_> {
    /// `reset(int direction)`.
    fn reset(&mut self, direction: i32) {
        if direction > 0 {
            // reset for forward iteration
            self.dir = 1;
            self.index = self.cp_limit;
        } else if direction < 0 {
            // reset for backward iteration
            self.dir = -1;
            self.index = self.cp_start;
        } else {
            // not a valid direction
            self.dir = 0;
            self.index = 0;
        }
        self.reached_limit = false;
    }

    /// `next()`.
    fn next(&mut self) -> i32 {
        if self.dir > 0 {
            if self.index < self.context_limit {
                let c = self.rep().char32_at(self.index);
                self.index += UTF16::get_char_count(c);
                return c;
            } else {
                // forward context iteration reached the limit
                self.reached_limit = true;
            }
        } else if self.dir < 0 && self.index > self.context_start {
            let c = self.rep().char32_at(self.index - 1);
            self.index -= UTF16::get_char_count(c);
            return c;
        }
        -1
    }
}
