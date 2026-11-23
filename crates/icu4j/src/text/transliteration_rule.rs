// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.TransliterationRule`.

use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use crate::r#impl::utility::Utility;
use crate::text::replaceable::Replaceable;
use crate::text::rule_based_transliterator::Data;
use crate::text::string_matcher::StringMatcher;
use crate::text::string_replacer::StringReplacer;
use crate::text::transliterator::Position;
use crate::text::unicode_matcher::{UnicodeMatcher, U_MATCH, U_MISMATCH, U_PARTIAL_MATCH};
use crate::text::unicode_replacer::UnicodeReplacer;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// A transliteration rule used by `RuleBasedTransliterator`: a key, with
/// optional ante and post contexts, and an output replacer.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.TransliterationRule`.
///
/// **Divergence.** The owning `Data` is held as a [`Weak`], as in
/// [`StringMatcher`] (the data holds this rule).
pub struct TransliterationRule {
    /// The match that must occur before the key, or null if there is no
    /// preceding context.
    ante_context: Option<StringMatcher>,
    /// The matcher object for the key. If null, then the key is empty.
    key: Option<StringMatcher>,
    /// The match that must occur after the key, or null if there is no
    /// following context.
    post_context: Option<StringMatcher>,
    /// The object that performs the replacement if the key, ante, and post
    /// context match.
    output: Box<dyn UnicodeReplacer>,
    /// The string that must be matched, consisting of the anteContext, key,
    /// and postContext, concatenated together, in that order. Some
    /// components may be empty (zero length).
    pattern: Vec<u16>,
    /// An array of matcher objects corresponding to the input pattern
    /// segments. If there are no segments this is null. N.B. This is a
    /// UnicodeMatcher for generality, but in practice it is always a
    /// StringMatcher. In the future we may generalize this, but for now we
    /// sometimes cast down to StringMatcher.
    pub(crate) segments: Option<Vec<Arc<StringMatcher>>>,
    /// The length of the string that must match before the key. If zero,
    /// then there is no matching requirement before the key. Substring
    /// [0,anteContextLength) of pattern is the anteContext.
    ante_context_length: i32,
    /// The length of the key. Substring
    /// [anteContextLength,anteContextLength + keyLength) is the key.
    key_length: i32,
    /// Miscellaneous attributes.
    pub(crate) flags: u8,
    /// The data that maps stand-ins to matchers.
    data: Weak<Data>,
}

impl std::fmt::Debug for TransliterationRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransliterationRule")
            .field("pattern", &String::from_utf16_lossy(&self.pattern))
            .field("ante_context_length", &self.ante_context_length)
            .field("key_length", &self.key_length)
            .field("flags", &self.flags)
            .finish()
    }
}

/// `String.regionMatches(int toffset, String other, int ooffset, int len)`.
fn region_matches(this: &[u16], toffset: i32, other: &[u16], ooffset: i32, len: i32) -> bool {
    // Note: toffset, ooffset, or len might be near -1>>>1.
    if ooffset < 0 || toffset < 0 || toffset as i64 > this.len() as i64 - len as i64 || ooffset as i64 > other.len() as i64 - len as i64 {
        return false;
    }
    if len <= 0 {
        return true;
    }
    this[toffset as usize..(toffset + len) as usize] == other[ooffset as usize..(ooffset + len) as usize]
}

impl TransliterationRule {
    /// Flag bit: the rule is anchored at the start of the context (`^`).
    pub const ANCHOR_START: u8 = 1;
    /// Flag bit: the rule is anchored at the end of the context (`$`).
    pub const ANCHOR_END: u8 = 2;

    /// `TransliterationRule(String input, int anteContextPos, int
    /// postContextPos, String output, int cursorPos, int cursorOffset,
    /// UnicodeMatcher[] segs, boolean anchorStart, boolean anchorEnd, Data
    /// theData)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid ante context, post context
    /// or cursor position.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        input: Vec<u16>,
        ante_context_pos: i32,
        post_context_pos: i32,
        output: &[u16],
        mut cursor_pos: i32,
        cursor_offset: i32,
        segs: Option<Vec<Arc<StringMatcher>>>,
        anchor_start: bool,
        anchor_end: bool,
        the_data: &Arc<Data>,
    ) -> Result<Self> {
        let input_len = input.len() as i32;
        let ante_context_length;
        // Do range checks only when warranted to save time
        if ante_context_pos < 0 {
            ante_context_length = 0;
        } else {
            if ante_context_pos > input_len {
                return Err(LuceneError::illegal_argument("Invalid ante context"));
            }
            ante_context_length = ante_context_pos;
        }
        let key_length;
        if post_context_pos < 0 {
            key_length = input_len - ante_context_length;
        } else {
            if post_context_pos < ante_context_length || post_context_pos > input_len {
                return Err(LuceneError::illegal_argument("Invalid post context"));
            }
            key_length = post_context_pos - ante_context_length;
        }
        if cursor_pos < 0 {
            cursor_pos = output.len() as i32;
        } else if cursor_pos > output.len() as i32 {
            return Err(LuceneError::illegal_argument("Invalid cursor position"));
        }

        // We don't validate the segments array.  The caller must
        // guarantee that the segments are well-formed (that is, that
        // all $n references in the output refer to indices of this
        // array, and that no array elements are null).
        let pattern = input;
        let mut flags = 0u8;
        if anchor_start {
            flags |= Self::ANCHOR_START;
        }
        if anchor_end {
            flags |= Self::ANCHOR_END;
        }

        let mut ante_context = None;
        if ante_context_length > 0 {
            ante_context = Some(StringMatcher::new(pattern[..ante_context_length as usize].to_vec(), 0, the_data));
        }

        let mut key = None;
        if key_length > 0 {
            key = Some(StringMatcher::new(
                pattern[ante_context_length as usize..(ante_context_length + key_length) as usize].to_vec(),
                0,
                the_data,
            ));
        }

        let post_context_length = pattern.len() as i32 - key_length - ante_context_length;
        let mut post_context = None;
        if post_context_length > 0 {
            post_context = Some(StringMatcher::new(
                pattern[(ante_context_length + key_length) as usize..].to_vec(),
                0,
                the_data,
            ));
        }

        let output = Box::new(StringReplacer::new_with_cursor(output.to_vec(), cursor_pos + cursor_offset, the_data));

        Ok(Self {
            data: Arc::downgrade(the_data),
            ante_context_length,
            key_length,
            segments: segs,
            pattern,
            flags,
            ante_context,
            key,
            post_context,
            output,
        })
    }

    fn data(&self) -> Arc<Data> {
        self.data.upgrade().expect("RuleBasedTransliterator.Data released while its TransliterationRule is in use")
    }

    /// `getAnteContextLength()`: the preceding context length, plus one for
    /// an anchor at the start (it counts as a character of context).
    pub fn get_ante_context_length(&self) -> i32 {
        self.ante_context_length + if (self.flags & Self::ANCHOR_START) != 0 { 1 } else { 0 }
    }

    /// `getIndexValue()`: the low byte of the first key character, or -1
    /// when the key is empty or starts with a matcher (the rule set's
    /// index then asks [`matches_index_value`](Self::matches_index_value)).
    pub(crate) fn get_index_value(&self) -> i32 {
        if self.ante_context_length == self.pattern.len() as i32 {
            // A pattern with just ante context {such as foo)>bar} can
            // match any key.
            return -1;
        }
        let c = UTF16::char_at(&self.pattern, self.ante_context_length);
        if self.data().lookup_matcher(c).is_none() {
            c & 0xFF
        } else {
            -1
        }
    }

    /// `matchesIndexValue(int v)`: whether this rule can match text whose
    /// first character has the low byte `v`.
    pub(crate) fn matches_index_value(&self, v: i32) -> bool {
        // Delegate to the key, or if there is none, to the postContext.
        // If there is neither then we match any key; return true.
        let m: Option<&dyn UnicodeMatcher> = match (&self.key, &self.post_context) {
            (Some(k), _) => Some(k),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        };
        match m {
            Some(m) => m.matches_index_value(v),
            None => true,
        }
    }

    /// `masks(TransliterationRule r2)`: whether this rule masks `r2`, that
    /// is, matches whatever `r2` matches so that `r2` is never reached.
    pub fn masks(&self, r2: &TransliterationRule) -> bool {
        // Rule r1 masks rule r2 if the string formed of the
        // antecontext, key, and postcontext overlaps in the following
        // way:
        //
        // r1:      aakkkpppp
        // r2:     aaakkkkkpppp
        //            ^
        //
        // The strings must be aligned at the first character of the
        // key.  The length of r1 to the left of the alignment point
        // must be <= the length of r2 to the left; ditto for the
        // right.  The characters of r1 must equal (or be a superset
        // of) the corresponding characters of r2.  The superset
        // operation should be performed to check for UnicodeSet
        // masking.
        //
        // Anchors:  Two patterns that differ only in anchors only
        // mask one another if they are exactly equal, and r2 has
        // all the anchors r1 has (optionally, plus some).  Here Y
        // means the row masks the column, N means it doesn't.
        //
        //         ab   ^ab    ab$  ^ab$
        //   ab    Y     Y     Y     Y
        //  ^ab    N     Y     N     Y
        //   ab$   N     N     Y     Y
        //  ^ab$   N     N     N     Y
        //
        // Post context: {a}b masks ab, but not vice versa, since {a}b
        // matches everything ab matches, and {a}b matches {|a|}b but ab
        // does not.  Pre context is different (a{b} does not align with
        // ab).

        // LIMITATION of the current mask algorithm: Some rule
        // maskings are currently not detected.  For example,
        // "{Lu}]a>x" masks "A]a>y".  This can be added later. TODO

        let len = self.pattern.len() as i32;
        let left = self.ante_context_length;
        let left2 = r2.ante_context_length;
        let right = self.pattern.len() as i32 - left;
        let right2 = r2.pattern.len() as i32 - left2;

        // TODO Clean this up -- some logic might be combinable with the
        // next statement.

        // Test for anchor masking
        if left == left2 && right == right2 && self.key_length <= r2.key_length && region_matches(&r2.pattern, 0, &self.pattern, 0, len) {
            // The following boolean logic implements the table above
            return (self.flags == r2.flags)
                || (!((self.flags & Self::ANCHOR_START) != 0) && !((self.flags & Self::ANCHOR_END) != 0))
                || (((r2.flags & Self::ANCHOR_START) != 0) && ((r2.flags & Self::ANCHOR_END) != 0));
        }

        left <= left2
            && (right < right2 || (right == right2 && self.key_length <= r2.key_length))
            && region_matches(&r2.pattern, left2 - left, &self.pattern, 0, len)
    }

    /// `posBefore(Replaceable str, int pos)`: the position of the code
    /// point before `pos`, or `pos - 1` at the start.
    pub(crate) fn pos_before(str: &dyn Replaceable, pos: i32) -> i32 {
        if pos > 0 {
            pos - UTF16::get_char_count(str.char32_at(pos - 1))
        } else {
            pos - 1
        }
    }

    /// `posAfter(Replaceable str, int pos)`: the position after the code
    /// point at `pos`, or `pos + 1` outside the text.
    pub(crate) fn pos_after(str: &dyn Replaceable, pos: i32) -> i32 {
        if pos >= 0 && pos < str.length() {
            pos + UTF16::get_char_count(str.char32_at(pos))
        } else {
            pos + 1
        }
    }

    /// `matchAndReplace(Replaceable text, Position pos, boolean
    /// incremental)`: attempts a match and replacement at the given
    /// position; `U_MISMATCH`, `U_PARTIAL_MATCH` or `U_MATCH` (after the
    /// replacement, with `pos` updated).
    ///
    /// # Errors
    ///
    /// The errors of the matchers and the replacer.
    pub fn match_and_replace(&self, text: &dyn Replaceable, pos: &mut Position, incremental: bool) -> Result<i32> {
        // Matching and replacing are done in one method because the
        // replacement operation needs information obtained during the
        // match.  Another way to do this is to have the match method
        // create a match result struct with relevant offsets, and to pass
        // this into the replace method.

        // ============================ MATCH ===========================

        // Reset segment match data
        if let Some(segments) = &self.segments {
            for s in segments {
                s.reset_match();
            }
        }

        let mut int_ref = [0i32; 1];

        // ------------------------ Ante Context ------------------------

        // A mismatch in the ante context, or with the start anchor,
        // is an outright U_MISMATCH regardless of whether we are
        // incremental or not.
        let mut o_text; // offset into 'text'
        let ante_limit = Self::pos_before(text, pos.context_start);

        int_ref[0] = Self::pos_before(text, pos.start);

        if let Some(ante_context) = &self.ante_context {
            let m = ante_context.matches(text, &mut int_ref, ante_limit, false)?;
            if m != U_MATCH {
                return Ok(U_MISMATCH);
            }
        }

        o_text = int_ref[0];

        let min_o_text = Self::pos_after(text, o_text);

        // ------------------------ Start Anchor ------------------------

        if ((self.flags & Self::ANCHOR_START) != 0) && o_text != ante_limit {
            return Ok(U_MISMATCH);
        }

        // -------------------- Key and Post Context --------------------

        int_ref[0] = pos.start;

        if let Some(key) = &self.key {
            let m = key.matches(text, &mut int_ref, pos.limit, incremental)?;
            if m != U_MATCH {
                return Ok(m);
            }
        }

        let key_limit = int_ref[0];

        if let Some(post_context) = &self.post_context {
            if incremental && key_limit == pos.limit {
                // The key matches just before pos.limit, and there is
                // a postContext.  Since we are in incremental mode,
                // we must assume more characters may be inserted at
                // pos.limit -- this is a partial match.
                return Ok(U_PARTIAL_MATCH);
            }

            let m = post_context.matches(text, &mut int_ref, pos.context_limit, incremental)?;
            if m != U_MATCH {
                return Ok(m);
            }
        }

        o_text = int_ref[0];

        // ------------------------- Stop Anchor ------------------------

        if (self.flags & Self::ANCHOR_END) != 0 {
            if o_text != pos.context_limit {
                return Ok(U_MISMATCH);
            }
            if incremental {
                return Ok(U_PARTIAL_MATCH);
            }
        }

        // =========================== REPLACE ==========================

        // We have a full match.  The key is between pos.start and
        // keyLimit.

        let new_length = self.output.replace(text, pos.start, key_limit, &mut int_ref)?;
        let len_delta = new_length - (key_limit - pos.start);
        let new_start = int_ref[0];

        o_text += len_delta;
        pos.limit += len_delta;
        pos.context_limit += len_delta;
        // Restrict new value of start to [minOText, min(oText, pos.limit)].
        pos.start = min_o_text.max(o_text.min(pos.limit).min(new_start));
        Ok(U_MATCH)
    }

    /// `toRule(boolean escapeUnprintable)`: a string representation of
    /// this rule as rule source.
    ///
    /// # Errors
    ///
    /// The errors of `Utility.appendToRule` and of the output's
    /// `toReplacerPattern`.
    pub fn to_rule(&self, escape_unprintable: bool) -> Result<Vec<u16>> {
        let mut rule = Vec::new();

        // Accumulate special characters (and non-specials following them)
        // into quoteBuf.  Append quoteBuf, within single quotes, when
        // a non-quoted element must be inserted.
        let mut quote_buf = Vec::new();

        // Do not emit the braces '{' '}' around the pattern if there
        // is neither anteContext nor postContext.
        let emit_braces = self.ante_context.is_some() || self.post_context.is_some();

        // Emit start anchor
        if (self.flags & Self::ANCHOR_START) != 0 {
            rule.push(b'^' as u16);
        }

        // Emit the input pattern
        Utility::append_to_rule_matcher(
            &mut rule,
            self.ante_context.as_ref().map(|m| m as &dyn UnicodeMatcher),
            escape_unprintable,
            &mut quote_buf,
        )?;

        if emit_braces {
            Utility::append_to_rule(&mut rule, b'{' as i32, true, escape_unprintable, &mut quote_buf)?;
        }

        Utility::append_to_rule_matcher(&mut rule, self.key.as_ref().map(|m| m as &dyn UnicodeMatcher), escape_unprintable, &mut quote_buf)?;

        if emit_braces {
            Utility::append_to_rule(&mut rule, b'}' as i32, true, escape_unprintable, &mut quote_buf)?;
        }

        Utility::append_to_rule_matcher(
            &mut rule,
            self.post_context.as_ref().map(|m| m as &dyn UnicodeMatcher),
            escape_unprintable,
            &mut quote_buf,
        )?;

        // Emit end anchor
        if (self.flags & Self::ANCHOR_END) != 0 {
            rule.push(b'$' as u16);
        }

        Utility::append_to_rule_str(&mut rule, &[b' ' as u16, b'>' as u16, b' ' as u16], true, escape_unprintable, &mut quote_buf)?;

        // Emit the output pattern

        Utility::append_to_rule_str(&mut rule, &self.output.to_replacer_pattern(escape_unprintable)?, true, escape_unprintable, &mut quote_buf)?;

        Utility::append_to_rule(&mut rule, b';' as i32, true, escape_unprintable, &mut quote_buf)?;

        Ok(rule)
    }

    /// `toString()`: `'{' + toRule(true) + '}'`.
    ///
    /// # Errors
    ///
    /// The errors of [`to_rule`](Self::to_rule).
    pub fn to_java_string(&self) -> Result<Vec<u16>> {
        let mut s = vec![b'{' as u16];
        s.extend_from_slice(&self.to_rule(true)?);
        s.push(b'}' as u16);
        Ok(s)
    }

    /// `addSourceTargetSet(UnicodeSet filter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet, UnicodeSet revisiting)`: unions into
    /// `source_set` the characters this rule may match under `filter`, and
    /// into `target_set` those it may output.
    ///
    /// # Errors
    ///
    /// The errors of the `UnicodeSet` operations.
    pub(crate) fn add_source_target_set(
        &self,
        filter: &UnicodeSet,
        source_set: &mut UnicodeSet,
        target_set: &mut UnicodeSet,
        _revisiting: &mut UnicodeSet,
    ) -> Result<()> {
        let limit = self.ante_context_length + self.key_length;
        let mut temp_source = UnicodeSet::new();
        let mut temp = UnicodeSet::new();

        // We need to walk through the pattern.
        // Iff some of the characters at ALL of the the positions are
        // matched by the filter, then we add temp to toUnionTo.
        let data = self.data();
        let mut i = self.ante_context_length;
        while i < limit {
            let ch = UTF16::char_at(&self.pattern, i);
            i += UTF16::get_char_count(ch);
            match data.lookup_matcher(ch) {
                None => {
                    if !filter.contains(ch)? {
                        return Ok(());
                    }
                    temp_source.add(ch)?;
                }
                Some(matcher) => match matcher.as_any().downcast_ref::<UnicodeSet>() {
                    Some(set) => {
                        if !filter.contains_some(set) {
                            return Ok(());
                        }
                        matcher.add_match_set_to(&mut temp_source)?;
                    }
                    None => {
                        // catch (ClassCastException e)
                        temp.clear()?;
                        matcher.add_match_set_to(&mut temp)?;
                        if !filter.contains_some(&temp) {
                            return Ok(());
                        }
                        temp_source.add_all(&temp)?;
                    }
                },
            }
        }
        // if we made our way through the gauntlet, add to source/target
        source_set.add_all(&temp_source)?;
        self.output.add_replacement_set_to(target_set)?;
        Ok(())
    }
}
