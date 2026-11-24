// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.StringMatcher`.

use std::any::Any;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use rucene::error::Result;

use crate::r#impl::utility::Utility;
use crate::text::replaceable::Replaceable;
use crate::text::rule_based_transliterator::Data;
use crate::text::unicode_matcher::{UnicodeMatcher, U_MATCH, U_MISMATCH, U_PARTIAL_MATCH};
use crate::text::unicode_replacer::UnicodeReplacer;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// An object that matches a fixed input string, implementing the
/// `UnicodeMatcher` API. It can also act as a `UnicodeReplacer`, emitting
/// the text it matched (a segment).
///
/// Equivalent to the package-private class `com.ibm.icu.text.StringMatcher`.
///
/// **Divergence.** Java holds the `RuleBasedTransliterator.Data` that owns
/// this matcher (`StringMatcher.java:57`); the data also holds the matcher
/// (through its rules and variables), a cycle Java's garbage collector
/// handles. Here the reference is a [`Weak`]: the data outlives every use
/// of its matchers (see [`Data`] for how earlier sections are kept alive).
/// `matchStart`/`matchLimit` are atomics because the matcher is shared;
/// Java mutates them under `synchronized(data)`, which the data's rule-set
/// lock reproduces.
pub struct StringMatcher {
    /// The text to be matched.
    pattern: Vec<u16>,
    /// Context offset of the start of the last match, or -1.
    match_start: AtomicI32,
    /// Context offset of the limit of the last match, or -1.
    match_limit: AtomicI32,
    /// The segment number, 1-based, or 0 if not a segment.
    segment_number: i32,
    /// Context object that maps stand-ins to matcher and replacer objects.
    data: Weak<Data>,
}

impl std::fmt::Debug for StringMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StringMatcher")
            .field("pattern", &String::from_utf16_lossy(&self.pattern))
            .field("segment_number", &self.segment_number)
            .finish()
    }
}

impl StringMatcher {
    /// `StringMatcher(String theString, int segmentNum, Data theData)`.
    pub(crate) fn new(the_string: Vec<u16>, segment_num: i32, the_data: &Arc<Data>) -> Self {
        Self {
            data: Arc::downgrade(the_data),
            pattern: the_string,
            match_start: AtomicI32::new(-1),
            match_limit: AtomicI32::new(-1),
            segment_number: segment_num,
        }
    }

    /// `StringMatcher(String theString, int start, int limit, int
    /// segmentNum, Data theData)`: matches `theString.substring(start,
    /// limit)`.
    ///
    /// # Panics
    ///
    /// Where Java throws `StringIndexOutOfBoundsException`: an invalid range
    /// (`TransliteratorParser` checks it first, as its `catch` does).
    pub(crate) fn new_range(
        the_string: &[u16],
        start: i32,
        limit: i32,
        segment_num: i32,
        the_data: &Arc<Data>,
    ) -> Self {
        Self::new(
            the_string[start as usize..limit as usize].to_vec(),
            segment_num,
            the_data,
        )
    }

    fn data(&self) -> Arc<Data> {
        self.data
            .upgrade()
            .expect("RuleBasedTransliterator.Data released while its StringMatcher is in use")
    }

    /// `resetMatch()`: removes any match information.
    pub fn reset_match(&self) {
        self.match_start.store(-1, Ordering::Relaxed);
        self.match_limit.store(-1, Ordering::Relaxed);
    }
}

/// `Utility.appendToRule` for a value that is a UTF-16 unit or -1, which
/// never fails.
fn append_to_rule_unit(
    rule: &mut Vec<u16>,
    c: i32,
    is_literal: bool,
    escape_unprintable: bool,
    quote_buf: &mut Vec<u16>,
) {
    Utility::append_to_rule(rule, c, is_literal, escape_unprintable, quote_buf)
        .expect("a UTF-16 unit is a valid code point");
}

/// `Utility.appendToRule(StringBuffer, String, ...)` for UTF-16 text, which
/// never fails.
fn append_to_rule_text(
    rule: &mut Vec<u16>,
    text: &[u16],
    is_literal: bool,
    escape_unprintable: bool,
    quote_buf: &mut Vec<u16>,
) {
    Utility::append_to_rule_str(rule, text, is_literal, escape_unprintable, quote_buf)
        .expect("a UTF-16 unit is a valid code point");
}

impl UnicodeMatcher for StringMatcher {
    /// `matches(Replaceable text, int[] offset, int limit, boolean
    /// incremental)`.
    fn matches(
        &self,
        text: &dyn Replaceable,
        offset: &mut [i32],
        limit: i32,
        incremental: bool,
    ) -> Result<i32> {
        let data = self.data();
        // Note (1): We process text in 16-bit code units, rather than
        // 32-bit code points.  This works because stand-ins are
        // always in the BMP and because we are doing a literal match
        // operation, which can be done 16-bits at a time.
        let mut cursor = [offset[0]];
        if limit < cursor[0] {
            // Match in the reverse direction
            let mut i = self.pattern.len() as i32 - 1;
            while i >= 0 {
                let key_char = self.pattern[i as usize]; // OK; see note (1) above
                match data.lookup_matcher(key_char as i32) {
                    None => {
                        if cursor[0] > limit && key_char == text.char_at(cursor[0]) {
                            // OK; see note (1) above
                            cursor[0] -= 1;
                        } else {
                            return Ok(U_MISMATCH);
                        }
                    }
                    Some(subm) => {
                        let m = subm.matches(text, &mut cursor, limit, incremental)?;
                        if m != U_MATCH {
                            return Ok(m);
                        }
                    }
                }
                i -= 1;
            }
            // Record the match position, but adjust for a normal
            // forward start, limit, and only if a prior match does not
            // exist -- we want the rightmost match.
            if self.match_start.load(Ordering::Relaxed) < 0 {
                self.match_start.store(cursor[0] + 1, Ordering::Relaxed);
                self.match_limit.store(offset[0] + 1, Ordering::Relaxed);
            }
        } else {
            for &key_char in &self.pattern {
                if incremental && cursor[0] == limit {
                    // We've reached the context limit without a mismatch
                    // and without completing our match.
                    return Ok(U_PARTIAL_MATCH);
                }
                // OK; see note (1) above
                match data.lookup_matcher(key_char as i32) {
                    None => {
                        // Don't need the cursor < limit check if
                        // incremental is true (because it's done above);
                        // do need it otherwise.
                        if cursor[0] < limit && key_char == text.char_at(cursor[0]) {
                            // OK; see note (1) above
                            cursor[0] += 1;
                        } else {
                            return Ok(U_MISMATCH);
                        }
                    }
                    Some(subm) => {
                        let m = subm.matches(text, &mut cursor, limit, incremental)?;
                        if m != U_MATCH {
                            return Ok(m);
                        }
                    }
                }
            }
            // Record the match position
            self.match_start.store(offset[0], Ordering::Relaxed);
            self.match_limit.store(cursor[0], Ordering::Relaxed);
        }

        offset[0] = cursor[0];
        Ok(U_MATCH)
    }

    /// `toPattern(boolean escapeUnprintable)`.
    fn to_pattern(&self, escape_unprintable: bool) -> Vec<u16> {
        let data = self.data();
        let mut result = Vec::new();
        let mut quote_buf = Vec::new();
        if self.segment_number > 0 {
            // i.e., if this is a segment
            result.push(b'(' as u16);
        }
        for &key_char in &self.pattern {
            match data.lookup_matcher(key_char as i32) {
                None => append_to_rule_unit(
                    &mut result,
                    key_char as i32,
                    false,
                    escape_unprintable,
                    &mut quote_buf,
                ),
                Some(m) => append_to_rule_text(
                    &mut result,
                    &m.to_pattern(escape_unprintable),
                    true,
                    escape_unprintable,
                    &mut quote_buf,
                ),
            }
        }
        if self.segment_number > 0 {
            // i.e., if this is a segment
            result.push(b')' as u16);
        }
        // Flush quoteBuf out to result
        append_to_rule_unit(&mut result, -1, true, escape_unprintable, &mut quote_buf);
        result
    }

    /// `matchesIndexValue(int v)`.
    fn matches_index_value(&self, v: i32) -> bool {
        if self.pattern.is_empty() {
            return true;
        }
        let c = UTF16::char_at(&self.pattern, 0);
        let data = self.data();
        match data.lookup_matcher(c) {
            None => (c & 0xFF) == v,
            Some(m) => m.matches_index_value(v),
        }
    }

    /// `addMatchSetTo(UnicodeSet toUnionTo)`.
    fn add_match_set_to(&self, to_union_to: &mut UnicodeSet) -> Result<()> {
        let data = self.data();
        let mut i = 0;
        while i < self.pattern.len() as i32 {
            let ch = UTF16::char_at(&self.pattern, i);
            match data.lookup_matcher(ch) {
                None => {
                    to_union_to.add(ch)?;
                }
                Some(matcher) => matcher.add_match_set_to(to_union_to)?,
            }
            i += UTF16::get_char_count(ch);
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl UnicodeReplacer for StringMatcher {
    /// `replace(Replaceable text, int start, int limit, int[] cursor)`.
    fn replace(
        &self,
        text: &dyn Replaceable,
        start: i32,
        limit: i32,
        _cursor: &mut [i32],
    ) -> Result<i32> {
        let mut out_len = 0;

        // Copy segment with out-of-band data
        let dest = limit;
        // If there was no match, that means that a quantifier
        // matched zero-length.  E.g., x (a)* y matched "xy".
        let match_start = self.match_start.load(Ordering::Relaxed);
        let match_limit = self.match_limit.load(Ordering::Relaxed);
        if match_start >= 0 && match_start != match_limit {
            text.copy(match_start, match_limit, dest);
            out_len = match_limit - match_start;
        }

        text.replace(start, limit, &[]); // delete original text

        Ok(out_len)
    }

    /// `toReplacerPattern(boolean escapeUnprintable)`.
    fn to_replacer_pattern(&self, _escape_unprintable: bool) -> Result<Vec<u16>> {
        // assert(segmentNumber > 0);
        let mut rule = vec![b'$' as u16];
        Utility::append_number(&mut rule, self.segment_number, 10, 1)?;
        Ok(rule)
    }

    /// `addReplacementSetTo(UnicodeSet toUnionTo)`: a segment emits only
    /// what it matched, nothing fixed.
    fn add_replacement_set_to(&self, _to_union_to: &mut UnicodeSet) -> Result<()> {
        // The output of this replacer varies; it is the source text between
        // matchStart and matchLimit.  Since this varies depending on the
        // input text, we can't compute it here.  We can either do nothing
        // or we can add ALL characters to the set.  It's probably more
        // useful to do nothing.
        Ok(())
    }
}
