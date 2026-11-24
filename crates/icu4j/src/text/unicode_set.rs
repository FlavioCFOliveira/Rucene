// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UnicodeSet`.

use std::any::Any;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, LazyLock, RwLock};

use rucene::error::{LuceneError, Result};

use crate::jdk::{
    char_count, code_point_at, code_point_before, ParsePosition,
};
use crate::lang::char_sequences::CharSequences;
use crate::lang::character_properties::CharacterProperties;
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::lang::u_script::UScript;
use crate::r#impl::bmp_set::BMPSet;
use crate::r#impl::character_properties_impl::CharacterPropertiesImpl;
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::rule_character_iterator::{self as rci, Position, RuleCharacterIterator};
use crate::r#impl::sorted_set_relation::SortedSetRelation;
use crate::r#impl::string_range::StringRange;
use crate::r#impl::u_case_props::UCaseProps;
use crate::r#impl::u_character_property::UCharacterProperty;
use crate::r#impl::u_property_aliases::UPropertyAliases;
use crate::r#impl::unicode_set_string_span::{self as usss, UnicodeSetStringSpan};
use crate::r#impl::utility::Utility;
use crate::text::break_iterator;
use crate::text::replaceable::Replaceable;
use crate::text::symbol_table::{SymbolTable, SYMBOL_REF};
use crate::text::unicode_filter::UnicodeFilter;
use crate::text::unicode_matcher::{UnicodeMatcher, ETHER, U_MATCH, U_MISMATCH, U_PARTIAL_MATCH};
use crate::text::unicode_set_iterator::UnicodeSetIterator;
use crate::text::utf16::UTF16;
use crate::util::freezable::Freezable;
use crate::util::output_int::OutputInt;
use crate::util::u_locale::ULocale;
use crate::util::version_info::VersionInfo;

const LOW: i32 = 0x000000; // LOW <= all valid values. ZERO for codepoints
const HIGH: i32 = 0x110000; // HIGH > all valid values. 10000 for code units.
// 110000 for codepoints

/// Enough for sets with few ranges.
/// For example, White_Space has 10 ranges, list length 21.
const INITIAL_CAPACITY: i32 = 25;

/// Max list [0, 1, 2, ..., max code point, HIGH]
const MAX_LENGTH: i32 = HIGH + 1;

/// `MIN_VALUE`: minimum value that can be stored in a UnicodeSet.
pub const MIN_VALUE: i32 = LOW;

/// `MAX_VALUE`: maximum value that can be stored in a UnicodeSet.
pub const MAX_VALUE: i32 = HIGH - 1;

const ANY_ID: &str = "ANY"; // [\u0000-\U0010FFFF]
const ASCII_ID: &str = "ASCII"; // [\u0000-\u007F]
const ASSIGNED: &str = "Assigned"; // [:^Cn:]

/// `IGNORE_SPACE`: bitmask for constructor and applyPattern() indicating
/// that white space should be ignored.
pub const IGNORE_SPACE: i32 = 1;
/// `CASE`: alias for `CASE_INSENSITIVE`.
pub const CASE: i32 = 2;
/// `CASE_INSENSITIVE`: close over case-insensitivity (full case folding).
pub const CASE_INSENSITIVE: i32 = 2;
/// `ADD_CASE_MAPPINGS`: add all case mappings.
pub const ADD_CASE_MAPPINGS: i32 = 4;
/// `SIMPLE_CASE_INSENSITIVE`: close over simple case folding.
pub const SIMPLE_CASE_INSENSITIVE: i32 = 6;
const CASE_MASK: i32 = CASE_INSENSITIVE | ADD_CASE_MAPPINGS;

/// `EMPTY`: a frozen empty set.
pub static EMPTY: LazyLock<UnicodeSet> = LazyLock::new(|| {
    let mut s = UnicodeSet::new();
    s.freeze();
    s
});

/// `ALL_CODE_POINTS`: a frozen set of all code points.
pub static ALL_CODE_POINTS: LazyLock<UnicodeSet> = LazyLock::new(|| {
    let mut s = UnicodeSet::new_range_unchecked(0, 0x10FFFF);
    s.freeze();
    s
});

/// `XSYMBOL_TABLE`: the default extended symbol table.
static XSYMBOL_TABLE: RwLock<Option<Arc<dyn XSymbolTable>>> = RwLock::new(None);

fn invalid_code_point(c: i32) -> LuceneError {
    LuceneError::illegal_argument(format!("Invalid code point U+{}", Utility::hex_places(c as i64, 6)))
}

fn check_cp(c: i32) -> Result<()> {
    if !(MIN_VALUE..=MAX_VALUE).contains(&c) {
        return Err(invalid_code_point(c));
    }
    Ok(())
}

/// `Character.charCount(codePoint)` as a `usize` step.
fn step(cp: i32) -> i32 {
    char_count(cp)
}

/// A mutable set of Unicode characters and multicharacter strings.
///
/// Equivalent to `com.ibm.icu.text.UnicodeSet` (which extends
/// `UnicodeFilter` and implements `Iterable<String>`,
/// `Comparable<UnicodeSet>` and `Freezable<UnicodeSet>`). The strings are a
/// `BTreeSet<Vec<u16>>`, whose order is `String.compareTo`'s (UTF-16 code
/// units, then length), where Java shares one unmodifiable empty set
/// between sets without strings. `clone()` of a frozen set returns the set
/// itself in Java; [`Clone`] copies it (frozen sets never change, so the
/// copy behaves the same).
pub struct UnicodeSet {
    len: i32, // length used; list may be longer to minimize reallocs
    list: Vec<i32>, // MUST be terminated with HIGH
    buffer: Vec<i32>, // internal buffer, may be empty
    strings: BTreeSet<Vec<u16>>,
    /// The pattern representation of this set. This may not be the most
    /// economical pattern. It is the pattern supplied to applyPattern(),
    /// with variables substituted and whitespace removed. For sets
    /// constructed without applyPattern(), or modified using the non-pattern
    /// API, this string will be null, indicating that toPattern() must
    /// generate a pattern representation from the inversion list.
    pat: Option<Vec<u16>>,
    // Optimization for frozen sets
    bmp_set: Option<Box<BMPSet>>,
    string_span: Option<Box<UnicodeSetStringSpan>>,
}

impl Clone for UnicodeSet {
    fn clone(&self) -> Self {
        Self {
            len: self.len,
            list: self.list.clone(),
            buffer: Vec::new(),
            strings: self.strings.clone(),
            pat: self.pat.clone(),
            bmp_set: self.bmp_set.clone(),
            string_span: self.string_span.clone(),
        }
    }
}

impl fmt::Debug for UnicodeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnicodeSet({})", String::from_utf16_lossy(&self.to_pattern(true)))
    }
}

impl Default for UnicodeSet {
    fn default() -> Self {
        Self::new()
    }
}

/// `appendCodePoint(Appendable app, int c)` (private).
fn append_code_point(app: &mut Vec<u16>, c: i32) {
    debug_assert!((0..=0x10ffff).contains(&c));
    if c <= 0xffff {
        app.push(c as u16);
    } else {
        app.push(UTF16::get_lead_surrogate(c));
        app.push(UTF16::get_trail_surrogate(c));
    }
}

/// `_appendToPat(T buf, String s, boolean escapeUnprintable)`.
fn append_to_pat_str<'b>(buf: &'b mut Vec<u16>, s: &[u16], escape_unprintable: bool) -> &'b mut Vec<u16> {
    let mut i = 0;
    while i < s.len() as i32 {
        let cp = code_point_at(s, i);
        append_to_pat(buf, cp, escape_unprintable);
        i += step(cp);
    }
    buf
}

/// `_appendToPat(T buf, int c, boolean escapeUnprintable)`: appends a code
/// point, quoting it when it is special in a pattern.
fn append_to_pat(buf: &mut Vec<u16>, c: i32, escape_unprintable: bool) -> &mut Vec<u16> {
    if if escape_unprintable { Utility::is_unprintable(c) } else { Utility::should_always_be_escaped(c) } {
        // Use hex escape notation (<backslash>uxxxx or <backslash>Uxxxxxxxx) for anything
        // unprintable
        return Utility::escape(buf, c);
    }
    // Okay to let ':' pass through
    match c {
        0x5B /* [ */ | 0x5D /* ] */ | 0x2D /* - */ | 0x5E /* ^ */ | 0x26 /* & */ | 0x5C /* \ */
        | 0x7B /* { */ | 0x7D /* } */ | 0x24 /* $ */ | 0x3A /* : */ => {
            buf.push(b'\\' as u16);
        }
        _ => {
            // Escape whitespace
            if PatternProps::is_white_space(c) {
                buf.push(b'\\' as u16);
            }
        }
    }
    append_code_point(buf, c);
    buf
}

/// `_appendToPat(T result, int start, int end, boolean escapeUnprintable)`.
fn append_range_to_pat(result: &mut Vec<u16>, start: i32, end: i32, escape_unprintable: bool) -> &mut Vec<u16> {
    append_to_pat(result, start, escape_unprintable);
    if start != end {
        if (start + 1) != end ||
                // Avoid writing what looks like a lead+trail surrogate pair.
                start == 0xdbff
        {
            result.push(b'-' as u16);
        }
        append_to_pat(result, end, escape_unprintable);
    }
    result
}

fn ascii(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl UnicodeSet {
    /// `MIN_VALUE` (the class constant; also the module constant).
    pub const MIN_VALUE: i32 = MIN_VALUE;
    /// `MAX_VALUE`.
    pub const MAX_VALUE: i32 = MAX_VALUE;
    /// `IGNORE_SPACE`.
    pub const IGNORE_SPACE: i32 = IGNORE_SPACE;
    /// `CASE`.
    pub const CASE: i32 = CASE;
    /// `CASE_INSENSITIVE`.
    pub const CASE_INSENSITIVE: i32 = CASE_INSENSITIVE;
    /// `ADD_CASE_MAPPINGS`.
    pub const ADD_CASE_MAPPINGS: i32 = ADD_CASE_MAPPINGS;
    /// `SIMPLE_CASE_INSENSITIVE`.
    pub const SIMPLE_CASE_INSENSITIVE: i32 = SIMPLE_CASE_INSENSITIVE;

    //----------------------------------------------------------------
    // Public API
    //----------------------------------------------------------------

    /// `UnicodeSet()`: an empty set.
    pub fn new() -> Self {
        let mut list = vec![0i32; INITIAL_CAPACITY as usize];
        list[0] = HIGH;
        Self { len: 1, list, buffer: Vec::new(), strings: BTreeSet::new(), pat: None, bmp_set: None, string_span: None }
    }

    /// `UnicodeSet(UnicodeSet other)`: a thawed copy.
    pub fn from_set(other: &UnicodeSet) -> Self {
        let mut s = Self { len: 0, list: Vec::new(), buffer: Vec::new(), strings: BTreeSet::new(), pat: None, bmp_set: None, string_span: None };
        s.set_unchecked(other);
        s
    }

    /// `UnicodeSet(int start, int end)`: the range `start..=end` (empty
    /// when `end < start`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    pub fn new_range(start: i32, end: i32) -> Result<Self> {
        let mut s = Self::new();
        s.add_range(start, end)?;
        Ok(s)
    }

    /// `new UnicodeSet(start, end)` for code points known to be valid.
    pub(crate) fn new_range_unchecked(start: i32, end: i32) -> Self {
        let mut s = Self::new();
        s.add_unchecked_range(start, end);
        s
    }

    /// `UnicodeSet(int... pairs)`: the ranges `pairs[0]..=pairs[1]`, ...
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an odd count or ranges that are not
    /// monotonically increasing.
    pub fn from_pairs(pairs: &[i32]) -> Result<Self> {
        if (pairs.len() & 1) != 0 {
            return Err(LuceneError::illegal_argument("Must have even number of integers"));
        }
        let mut list = vec![0i32; pairs.len() + 1]; // don't allocate extra space, because it is likely that this is a fixed set.
        let len = list.len() as i32;
        let mut last = -1; // used to ensure that the results are monotonically increasing.
        let mut i = 0;
        while i < pairs.len() {
            // start of pair
            let start = pairs[i];
            if last >= start {
                return Err(LuceneError::illegal_argument("Must be monotonically increasing."));
            }
            list[i] = start;
            i += 1;
            // end of pair
            let limit = pairs[i].wrapping_add(1);
            if start >= limit {
                return Err(LuceneError::illegal_argument("Must be monotonically increasing."));
            }
            list[i] = limit;
            last = limit;
            i += 1;
        }
        list[i] = HIGH; // terminate
        Ok(Self { len, list, buffer: Vec::new(), strings: BTreeSet::new(), pat: None, bmp_set: None, string_span: None })
    }

    /// `UnicodeSet(String pattern)`: white space ignored.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed pattern.
    pub fn from_pattern(pattern: &[u16]) -> Result<Self> {
        let mut s = Self::new();
        s.apply_pattern_full(pattern, None, None, IGNORE_SPACE)?;
        Ok(s)
    }

    /// `UnicodeSet(String pattern, boolean ignoreWhitespace)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn from_pattern_ignore_ws(pattern: &[u16], ignore_whitespace: bool) -> Result<Self> {
        let mut s = Self::new();
        s.apply_pattern_full(pattern, None, None, if ignore_whitespace { IGNORE_SPACE } else { 0 })?;
        Ok(s)
    }

    /// `UnicodeSet(String pattern, int options)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn from_pattern_options(pattern: &[u16], options: i32) -> Result<Self> {
        let mut s = Self::new();
        s.apply_pattern_full(pattern, None, None, options)?;
        Ok(s)
    }

    /// `UnicodeSet(String pattern, ParsePosition pos, SymbolTable symbols)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn from_pattern_pos(pattern: &[u16], pos: &mut ParsePosition, symbols: Option<&dyn SymbolTable>) -> Result<Self> {
        let mut s = Self::new();
        s.apply_pattern_full(pattern, Some(pos), symbols, IGNORE_SPACE)?;
        Ok(s)
    }

    /// `UnicodeSet(String pattern, ParsePosition pos, SymbolTable symbols,
    /// int options)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn from_pattern_pos_options(
        pattern: &[u16],
        pos: &mut ParsePosition,
        symbols: Option<&dyn SymbolTable>,
        options: i32,
    ) -> Result<Self> {
        let mut s = Self::new();
        s.apply_pattern_full(pattern, Some(pos), symbols, options)?;
        Ok(s)
    }

    /// `set(int start, int end)`: makes this the range `start..=end`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; `IllegalArgumentException`
    /// for an invalid code point.
    pub fn set_range(&mut self, start: i32, end: i32) -> Result<&mut Self> {
        self.check_frozen()?;
        self.clear()?;
        self.complement_range(start, end)?;
        Ok(self)
    }

    /// `set(UnicodeSet other)`: makes this a copy of `other`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn set(&mut self, other: &UnicodeSet) -> Result<&mut Self> {
        self.check_frozen()?;
        self.set_unchecked(other);
        Ok(self)
    }

    fn set_unchecked(&mut self, other: &UnicodeSet) {
        self.list = other.list[..other.len as usize].to_vec();
        self.len = other.len;
        self.pat = other.pat.clone();
        self.strings = other.strings.clone();
    }

    /// `applyPattern(String pattern)`: white space ignored.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; `IllegalArgumentException`
    /// for a malformed pattern.
    pub fn apply_pattern(&mut self, pattern: &[u16]) -> Result<&mut Self> {
        self.check_frozen()?;
        self.apply_pattern_full(pattern, None, None, IGNORE_SPACE)
    }

    /// `applyPattern(String pattern, boolean ignoreWhitespace)`.
    ///
    /// # Errors
    ///
    /// As [`apply_pattern`](Self::apply_pattern).
    pub fn apply_pattern_ignore_ws(&mut self, pattern: &[u16], ignore_whitespace: bool) -> Result<&mut Self> {
        self.check_frozen()?;
        self.apply_pattern_full(pattern, None, None, if ignore_whitespace { IGNORE_SPACE } else { 0 })
    }

    /// `applyPattern(String pattern, int options)`.
    ///
    /// # Errors
    ///
    /// As [`apply_pattern`](Self::apply_pattern).
    pub fn apply_pattern_options(&mut self, pattern: &[u16], options: i32) -> Result<&mut Self> {
        self.check_frozen()?;
        self.apply_pattern_full(pattern, None, None, options)
    }

    /// `resemblesPattern(String pattern, int pos)`.
    pub fn resembles_pattern(pattern: &[u16], pos: i32) -> bool {
        ((pos + 1) < pattern.len() as i32 && pattern[pos as usize] == b'[' as u16)
            || Self::resembles_property_pattern(pattern, pos)
    }

    /// `toPattern(boolean escapeUnprintable)`.
    pub fn to_pattern(&self, escape_unprintable: bool) -> Vec<u16> {
        if let Some(pat) = &self.pat {
            if !escape_unprintable {
                return pat.clone();
            }
        }
        let mut result = Vec::new();
        self.to_pattern_into(&mut result, escape_unprintable);
        result
    }

    /// `_toPattern(T result, boolean escapeUnprintable)` (private): appends
    /// the pattern to `result`.
    fn to_pattern_into<'r>(&self, result: &'r mut Vec<u16>, escape_unprintable: bool) -> &'r mut Vec<u16> {
        let Some(pat) = &self.pat else {
            return self.append_new_pattern(result, escape_unprintable, true);
        };
        if !escape_unprintable {
            // TODO: The C++ version does not have this shortcut, and instead
            // always cleans up the pattern string,
            // which also escapes Utility.shouldAlwaysBeEscaped(c).
            // We should sync these implementations.
            result.extend_from_slice(pat);
            return result;
        }
        let mut odd_number_of_backslashes = false;
        let mut i = 0;
        while i < pat.len() as i32 {
            let c = code_point_at(pat, i);
            i += step(c);
            if Utility::is_unprintable(c) {
                // If the unprintable character is preceded by an odd
                // number of backslashes, then it has been escaped
                // and we omit the last backslash.
                Utility::escape(result, c);
                odd_number_of_backslashes = false;
            } else if !odd_number_of_backslashes && c == b'\\' as i32 {
                // Temporarily withhold an odd-numbered backslash.
                odd_number_of_backslashes = true;
            } else {
                if odd_number_of_backslashes {
                    result.push(b'\\' as u16);
                }
                append_code_point(result, c);
                odd_number_of_backslashes = false;
            }
        }
        if odd_number_of_backslashes {
            result.push(b'\\' as u16);
        }
        result
    }

    /// `_generatePattern(StringBuffer result, boolean escapeUnprintable)`
    /// (deprecated internal).
    pub fn generate_pattern<'r>(&self, result: &'r mut Vec<u16>, escape_unprintable: bool) -> &'r mut Vec<u16> {
        self.generate_pattern_strings(result, escape_unprintable, true)
    }

    /// `_generatePattern(StringBuffer result, boolean escapeUnprintable,
    /// boolean includeStrings)` (deprecated internal).
    pub fn generate_pattern_strings<'r>(&self, result: &'r mut Vec<u16>, escape_unprintable: bool, include_strings: bool) -> &'r mut Vec<u16> {
        self.append_new_pattern(result, escape_unprintable, include_strings)
    }

    // Implementation of public _generatePattern().
    // Allows other callers to use a StringBuilder while the existing API is stuck with StringBuffer.
    fn append_new_pattern<'r>(&self, result: &'r mut Vec<u16>, escape_unprintable: bool, include_strings: bool) -> &'r mut Vec<u16> {
        result.push(b'[' as u16);

        let mut i = 0;
        let mut limit = self.len & !1; // = 2 * getRangeCount()

        // If the set contains at least 2 intervals and includes both
        // MIN_VALUE and MAX_VALUE, then the inverse representation will
        // be more economical.
        //     if (getRangeCount() >= 2 &&
        //             getRangeStart(0) == MIN_VALUE &&
        //             getRangeEnd(last) == MAX_VALUE)
        // Invariant: list[len-1] == HIGH == MAX_VALUE + 1
        // If limit == len then len is even and the last range ends with MAX_VALUE.
        //
        // *But* do not write the inverse (complement) if there are strings.
        // Since ICU 70, the '^' performs a code point complement which removes all strings.
        if self.len >= 4 && self.list[0] == 0 && limit == self.len && !self.has_strings() {
            // Emit the inverse
            result.push(b'^' as u16);
            // Offsetting the inversion list index by one lets us
            // iterate over the ranges of the set complement.
            i = 1;
            limit -= 1;
        }

        // Emit the ranges as pairs.
        while i < limit {
            let mut start = self.list[i as usize];
            let end = self.list[i as usize + 1] - 1;
            if !(0xd800..=0xdbff).contains(&end) {
                append_range_to_pat(result, start, end, escape_unprintable);
                i += 2;
            } else {
                // The range ends with a lead surrogate.
                // Avoid writing what looks like a lead+trail surrogate pair.
                // 1. Postpone ranges that start with a lead surrogate code point.
                let first_lead = i;
                loop {
                    i += 2;
                    if !(i < limit && self.list[i as usize] <= 0xdbff) {
                        break;
                    }
                }
                let first_after_lead = i;
                // 2. Write following ranges that start with a trail surrogate code point.
                while i < limit && {
                    start = self.list[i as usize];
                    start <= 0xdfff
                } {
                    append_range_to_pat(result, start, self.list[i as usize + 1] - 1, escape_unprintable);
                    i += 2;
                }
                // 3. Now write the postponed ranges.
                let mut j = first_lead;
                while j < first_after_lead {
                    append_range_to_pat(result, self.list[j as usize], self.list[j as usize + 1] - 1, escape_unprintable);
                    j += 2;
                }
            }
        }

        if include_strings && self.has_strings() {
            for s in &self.strings {
                result.push(b'{' as u16);
                append_to_pat_str(result, s, escape_unprintable);
                result.push(b'}' as u16);
            }
        }
        result.push(b']' as u16);
        result
    }

    /// `size()`: the number of code points and strings.
    pub fn size(&self) -> i32 {
        let mut n = 0;
        let count = self.get_range_count();
        for i in 0..count {
            n += self.get_range_end(i) - self.get_range_start(i) + 1;
        }
        n + self.strings.len() as i32
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.len == 1 && !self.has_strings()
    }

    /// `hasStrings()`.
    pub fn has_strings(&self) -> bool {
        !self.strings.is_empty()
    }

    /// `matchRest(Replaceable text, int start, int limit, String s)`.
    fn match_rest(text: &dyn Replaceable, start: i32, limit: i32, s: &[u16]) -> i32 {
        let max_len;
        let mut slen = s.len() as i32;
        if start < limit {
            let mut m = limit - start;
            if m > slen {
                m = slen;
            }
            for i in 1..m {
                if text.char_at(start + i) != s[i as usize] {
                    return 0;
                }
            }
            max_len = m;
        } else {
            let mut m = start - limit;
            if m > slen {
                m = slen;
            }
            slen -= 1; // <=> slen = s.length() - 1;
            for i in 1..m {
                if text.char_at(start - i) != s[(slen - i) as usize] {
                    return 0;
                }
            }
            max_len = m;
        }
        max_len
    }

    /// `matchesAt(CharSequence text, int offset)` (deprecated internal):
    /// the end of the longest match at `offset`, or `offset - 1`.
    pub fn matches_at(&self, text: &[u16], offset: i32) -> i32 {
        let mut last_len = -1;
        'strings: {
            if self.has_strings() {
                let first_char = text[offset as usize];
                // TODO make this faster!
                // get a starting point
                let mut it = self.strings.iter();
                let mut trial: Option<&Vec<u16>> = None;
                for t in it.by_ref() {
                    trial = Some(t);
                    let first_string_char = t[0];
                    if first_string_char < first_char {
                        continue;
                    }
                    if first_string_char > first_char {
                        break 'strings;
                    }
                }
                // As in Java, the scan above only stops early by leaving the
                // block, so `trial` is the last string here.
                let Some(mut trial) = trial else { break 'strings };
                // now keep checking string until we get the longest one
                loop {
                    let temp_len = Self::matches_at_sub(text, offset, trial);
                    if last_len > temp_len {
                        break 'strings;
                    }
                    last_len = temp_len;
                    match it.next() {
                        None => break,
                        Some(t) => trial = t,
                    }
                }
            }
        }

        if last_len < 2 {
            let cp = UTF16::char_at(text, offset);
            if self.contains_valid(cp) {
                last_len = UTF16::get_char_count(cp);
            }
        }

        offset + last_len
    }

    /// Does one string contain another, starting at a specific offset?
    fn matches_at_sub(text: &[u16], offset_in_text: i32, substring: &[u16]) -> i32 {
        let len = substring.len() as i32;
        let text_length = text.len() as i32;
        if text_length + offset_in_text > len {
            return -1;
        }
        let mut i = 0;
        let mut j = offset_in_text;
        while i < len {
            let pc = substring[i as usize];
            let tc = text[j as usize];
            if pc != tc {
                return -1;
            }
            i += 1;
            j += 1;
        }
        i
    }

    /// `indexOf(int c)`: the index of `c` among the code points of the set,
    /// -1 when absent.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    pub fn index_of(&self, c: i32) -> Result<i32> {
        check_cp(c)?;
        let mut i = 0usize;
        let mut n = 0;
        loop {
            let start = self.list[i];
            i += 1;
            if c < start {
                return Ok(-1);
            }
            let limit = self.list[i];
            i += 1;
            if c < limit {
                return Ok(n + c - start);
            }
            n += limit - start;
        }
    }

    /// `charAt(int index)`: the code point at `index`, -1 when out of range.
    pub fn char_at(&self, mut index: i32) -> i32 {
        if index >= 0 {
            // len2 is the largest even integer <= len, that is, it is len
            // for even values and len-1 for odd values.  With odd values
            // the last entry is UNICODESET_HIGH.
            let len2 = (self.len & !1) as usize;
            let mut i = 0;
            while i < len2 {
                let start = self.list[i];
                i += 1;
                let count = self.list[i] - start;
                i += 1;
                if index < count {
                    return start + index;
                }
                index -= count;
            }
        }
        -1
    }

    /// `add(int start, int end)`: adds the range `start..=end`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; `IllegalArgumentException`
    /// for an invalid code point.
    pub fn add_range(&mut self, start: i32, end: i32) -> Result<&mut Self> {
        self.check_frozen()?;
        check_cp(start)?;
        check_cp(end)?;
        self.add_unchecked_range(start, end);
        Ok(self)
    }

    /// `addAll(int start, int end)`: as [`add_range`](Self::add_range).
    ///
    /// # Errors
    ///
    /// As [`add_range`](Self::add_range).
    pub fn add_all_range(&mut self, start: i32, end: i32) -> Result<&mut Self> {
        self.add_range(start, end)
    }

    // for internal use, after checkFrozen has been called
    fn add_unchecked_range(&mut self, start: i32, end: i32) {
        if start < end {
            let limit = end + 1;
            // Fast path for adding a new range after the last one.
            // Odd list length: [..., lastStart, lastLimit, HIGH]
            if (self.len & 1) != 0 {
                // If the list is empty, set lastLimit low enough to not be adjacent to 0.
                let last_limit = if self.len == 1 { -2 } else { self.list[self.len as usize - 2] };
                if last_limit <= start {
                    if last_limit == start {
                        // Extend the last range.
                        self.list[self.len as usize - 2] = limit;
                        if limit == HIGH {
                            self.len -= 1;
                        }
                    } else {
                        self.list[self.len as usize - 1] = start;
                        if limit < HIGH {
                            self.ensure_capacity(self.len + 2);
                            self.list[self.len as usize] = limit;
                            self.len += 1;
                            self.list[self.len as usize] = HIGH;
                            self.len += 1;
                        } else {
                            // limit == HIGH
                            self.ensure_capacity(self.len + 1);
                            self.list[self.len as usize] = HIGH;
                            self.len += 1;
                        }
                    }
                    self.pat = None;
                    return;
                }
            }
            // This is slow. Could be much faster using findCodePoint(start)
            // and modifying the list, dealing with adjacent & overlapping ranges.
            let r = range(start, end);
            self.add_list(&r, 2, 0);
        } else if start == end {
            self.add_valid(start);
        }
    }

    /// `add(int c)`: adds the code point.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; `IllegalArgumentException`
    /// for an invalid code point.
    pub fn add(&mut self, c: i32) -> Result<&mut Self> {
        self.check_frozen()?;
        check_cp(c)?;
        self.add_valid(c);
        Ok(self)
    }

    /// `add_unchecked(int c)` (private) for a valid code point, after
    /// `checkFrozen`.
    pub(crate) fn add_valid(&mut self, c: i32) {
        // find smallest i such that c < list[i]
        // if odd, then it is IN the set
        // if even, then it is OUT of the set
        let i = self.find_code_point(c) as usize;

        // already in set?
        if (i & 1) != 0 {
            return;
        }

        // HIGH is 0x110000
        // assert(list[len-1] == HIGH);

        // empty = [HIGH]
        // [start_0, limit_0, start_1, limit_1, HIGH]

        // [..., start_k-1, limit_k-1, start_k, limit_k, ..., HIGH]
        //                             ^
        //                             list[i]

        // i == 0 means c is before the first range
        // TODO: Is the "list[i]-1" a typo? Even if you pass MAX_VALUE into
        //      add_unchecked, the maximum value that "c" will be compared to
        //      is "MAX_VALUE-1" meaning that "if (c == MAX_VALUE)" will
        //      never be reached according to this logic.
        if c == self.list[i] - 1 {
            // c is before start of next range
            self.list[i] = c;
            // if we touched the HIGH mark, then add a new one
            if c == MAX_VALUE {
                self.ensure_capacity(self.len + 1);
                self.list[self.len as usize] = HIGH;
                self.len += 1;
            }
            if i > 0 && c == self.list[i - 1] {
                // collapse adjacent ranges

                // [..., start_k-1, c, c, limit_k, ..., HIGH]
                //                     ^
                //                     list[i]
                let len = self.len as usize;
                self.list.copy_within(i + 1..len, i - 1);
                self.len -= 2;
            }
        } else if i > 0 && c == self.list[i - 1] {
            // c is after end of prior range
            self.list[i - 1] += 1;
            // no need to check for collapse here
        } else {
            // At this point we know the new char is not adjacent to
            // any existing ranges, and it is not 10FFFF.

            // [..., start_k-1, limit_k-1, start_k, limit_k, ..., HIGH]
            //                             ^
            //                             list[i]

            // [..., start_k-1, limit_k-1, c, c+1, start_k, limit_k, ..., HIGH]
            //                             ^
            //                             list[i]

            // Don't use ensureCapacity() to save on copying.
            // NOTE: This has no measurable impact on performance,
            // but it might help in some usage patterns.
            let len = self.len as usize;
            if len + 2 > self.list.len() {
                let mut temp = vec![0i32; Self::next_capacity(self.len + 2) as usize];
                if i != 0 {
                    temp[..i].copy_from_slice(&self.list[..i]);
                }
                temp[i + 2..len + 2].copy_from_slice(&self.list[i..len]);
                self.list = temp;
            } else {
                self.list.copy_within(i..len, i + 2);
            }

            self.list[i] = c;
            self.list[i + 1] = c + 1;
            self.len += 2;
        }

        self.pat = None;
    }

    /// `add(CharSequence s)`: adds a string, or its code point when it has
    /// exactly one.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn add_str(&mut self, s: &[u16]) -> Result<&mut Self> {
        self.check_frozen()?;
        let cp = Self::get_single_cp(s);
        if cp < 0 {
            if !self.strings.contains(s) {
                self.add_string(s);
                self.pat = None;
            }
        } else {
            self.add_unchecked_range(cp, cp);
        }
        Ok(self)
    }

    fn add_string(&mut self, s: &[u16]) {
        self.strings.insert(s.to_vec());
    }

    /// @return a code point IF the string consists of a single one.
    /// otherwise returns -1.
    fn get_single_cp(s: &[u16]) -> i32 {
        if s.len() == 1 {
            return s[0] as i32;
        }
        if s.len() == 2 {
            let cp = code_point_at(s, 0);
            if cp > 0xFFFF {
                // is surrogate pair
                return cp;
            }
        }
        -1
    }

    /// `addAll(CharSequence s)`: adds each code point of `s`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn add_all_str(&mut self, s: &[u16]) -> Result<&mut Self> {
        self.check_frozen()?;
        let mut i = 0;
        while i < s.len() as i32 {
            let cp = UTF16::char_at(s, i);
            self.add_unchecked_range(cp, cp);
            i += UTF16::get_char_count(cp);
        }
        Ok(self)
    }

    /// `retainAll(CharSequence s)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn retain_all_str(&mut self, s: &[u16]) -> Result<&mut Self> {
        let other = Self::from_all(s);
        self.retain_all(&other)
    }

    /// `complementAll(CharSequence s)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn complement_all_str(&mut self, s: &[u16]) -> Result<&mut Self> {
        let other = Self::from_all(s);
        self.complement_all(&other)
    }

    /// `removeAll(CharSequence s)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn remove_all_str(&mut self, s: &[u16]) -> Result<&mut Self> {
        let other = Self::from_all(s);
        self.remove_all(&other)
    }

    /// `removeAllStrings()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn remove_all_strings(&mut self) -> Result<&mut Self> {
        self.check_frozen()?;
        if self.has_strings() {
            self.strings.clear();
            self.pat = None;
        }
        Ok(self)
    }

    /// `from(CharSequence s)`: a set holding `s`.
    pub fn from_str(s: &[u16]) -> Self {
        let mut set = Self::new();
        set.add_str(s).expect("a new set is not frozen");
        set
    }

    /// `fromAll(CharSequence s)`: a set of the code points of `s`.
    pub fn from_all(s: &[u16]) -> Self {
        let mut set = Self::new();
        set.add_all_str(s).expect("a new set is not frozen");
        set
    }

    /// `retain(int start, int end)`: keeps only the range.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; `IllegalArgumentException`
    /// for an invalid code point.
    pub fn retain_range(&mut self, start: i32, end: i32) -> Result<&mut Self> {
        self.check_frozen()?;
        check_cp(start)?;
        check_cp(end)?;
        if start <= end {
            let r = range(start, end);
            self.retain_list(&r, 2, 0);
        } else {
            self.clear()?;
        }
        Ok(self)
    }

    /// `retain(int c)`.
    ///
    /// # Errors
    ///
    /// As [`retain_range`](Self::retain_range).
    pub fn retain(&mut self, c: i32) -> Result<&mut Self> {
        self.retain_range(c, c)
    }

    /// `retain(CharSequence cs)`.
    ///
    /// # Errors
    ///
    /// As [`retain_range`](Self::retain_range).
    pub fn retain_str(&mut self, cs: &[u16]) -> Result<&mut Self> {
        let cp = Self::get_single_cp(cs);
        if cp < 0 {
            self.check_frozen()?;
            let is_in = self.strings.contains(cs);
            // Check for getRangeCount() first to avoid somewhat-expensive size()
            // when there are single code points.
            if is_in && self.get_range_count() == 0 && self.size() == 1 {
                return Ok(self);
            }
            self.clear()?;
            if is_in {
                self.add_string(cs);
            }
            self.pat = None;
        } else {
            self.retain_range(cp, cp)?;
        }
        Ok(self)
    }

    /// `remove(int start, int end)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; `IllegalArgumentException`
    /// for an invalid code point.
    pub fn remove_range(&mut self, start: i32, end: i32) -> Result<&mut Self> {
        self.check_frozen()?;
        check_cp(start)?;
        check_cp(end)?;
        if start <= end {
            let r = range(start, end);
            self.retain_list(&r, 2, 2);
        }
        Ok(self)
    }

    /// `remove(int c)`.
    ///
    /// # Errors
    ///
    /// As [`remove_range`](Self::remove_range).
    pub fn remove(&mut self, c: i32) -> Result<&mut Self> {
        self.remove_range(c, c)
    }

    /// `remove(CharSequence s)`.
    ///
    /// # Errors
    ///
    /// As [`remove_range`](Self::remove_range).
    pub fn remove_str(&mut self, s: &[u16]) -> Result<&mut Self> {
        let cp = Self::get_single_cp(s);
        if cp < 0 {
            self.check_frozen()?;
            if self.strings.remove(s) {
                self.pat = None;
            }
        } else {
            self.remove_range(cp, cp)?;
        }
        Ok(self)
    }

    /// `complement(int start, int end)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; `IllegalArgumentException`
    /// for an invalid code point.
    pub fn complement_range(&mut self, start: i32, end: i32) -> Result<&mut Self> {
        self.check_frozen()?;
        check_cp(start)?;
        check_cp(end)?;
        if start <= end {
            let r = range(start, end);
            self.xor(&r, 2, 0);
        }
        self.pat = None;
        Ok(self)
    }

    /// `complement(int c)`.
    ///
    /// # Errors
    ///
    /// As [`complement_range`](Self::complement_range).
    pub fn complement_cp(&mut self, c: i32) -> Result<&mut Self> {
        self.complement_range(c, c)
    }

    /// `complement()`: the code point complement (strings kept).
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn complement(&mut self) -> Result<&mut Self> {
        self.check_frozen()?;
        self.complement_unchecked();
        Ok(self)
    }

    fn complement_unchecked(&mut self) {
        let len = self.len as usize;
        if self.list[0] == LOW {
            self.list.copy_within(1..len, 0);
            self.len -= 1;
        } else {
            self.ensure_capacity(self.len + 1);
            self.list.copy_within(0..len, 1);
            self.list[0] = LOW;
            self.len += 1;
        }
        self.pat = None;
    }

    /// `complement(CharSequence s)`: toggles a string or code point.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn complement_str(&mut self, s: &[u16]) -> Result<&mut Self> {
        self.check_frozen()?;
        let cp = Self::get_single_cp(s);
        if cp < 0 {
            if !self.strings.remove(s) {
                self.add_string(s);
            }
            self.pat = None;
        } else {
            self.complement_range(cp, cp)?;
        }
        Ok(self)
    }

    /// `contains(int c)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    pub fn contains(&self, c: i32) -> Result<bool> {
        check_cp(c)?;
        Ok(self.contains_valid(c))
    }

    /// `contains(int c)` for a code point known to be valid.
    pub(crate) fn contains_valid(&self, c: i32) -> bool {
        if let Some(b) = &self.bmp_set {
            return b.contains(c);
        }
        if let Some(s) = &self.string_span {
            return s.contains(c);
        }

        /*
        // Set i to the index of the start item greater than ch
        // We know we will terminate without length test!
        int i = -1;
        while (true) {
            if (c < list[++i]) break;
        }
         */

        let i = self.find_code_point(c);

        (i & 1) != 0 // return true if odd
    }

    /// Returns the smallest value i such that c < list[i].  Caller
    /// must ensure that c is a legal value or this method will enter
    /// an infinite loop.  This method performs a binary search.
    fn find_code_point(&self, c: i32) -> i32 {
        /* Examples:
                                           findCodePoint(c)
           set              list[]         c=0 1 3 4 7 8
           ===              ==============   ===========
           []               [110000]         0 0 0 0 0 0
           [\u0000-\u0003]  [0, 4, 110000]   1 1 1 2 2 2
           [\u0004-\u0007]  [4, 8, 110000]   0 0 0 1 1 2
           [:all:]          [0, 110000]      1 1 1 1 1 1
         */

        // Return the smallest i such that c < list[i].  Assume
        // list[len - 1] == HIGH and that c is legal (0..HIGH-1).
        if c < self.list[0] {
            return 0;
        }
        // High runner test.  c is often after the last range, so an
        // initial check for this condition pays off.
        if self.len >= 2 && c >= self.list[self.len as usize - 2] {
            return self.len - 1;
        }
        let mut lo = 0;
        let mut hi = self.len - 1;
        // invariant: c >= list[lo]
        // invariant: c < list[hi]
        loop {
            let i = ((lo + hi) as u32 >> 1) as i32;
            if i == lo {
                return hi;
            }
            if c < self.list[i as usize] {
                hi = i;
            } else {
                lo = i;
            }
        }
    }

    /// `contains(int start, int end)`: whether the whole range is in the set.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    pub fn contains_range(&self, start: i32, end: i32) -> Result<bool> {
        check_cp(start)?;
        check_cp(end)?;
        //int i = -1;
        //while (true) {
        //    if (start < list[++i]) break;
        //}
        let i = self.find_code_point(start);
        Ok((i & 1) != 0 && end < self.list[i as usize])
    }

    /// `contains(CharSequence s)`.
    pub fn contains_str(&self, s: &[u16]) -> bool {
        let cp = Self::get_single_cp(s);
        if cp < 0 {
            self.strings.contains(s)
        } else {
            self.contains_valid(cp)
        }
    }

    /// `containsAll(UnicodeSet b)`.
    pub fn contains_all(&self, b: &UnicodeSet) -> bool {
        // The specified set is a subset if all of its pairs are contained in
        // this set. This implementation accesses the lists directly for speed.
        // TODO: this could be faster if size() were cached. But that would affect building speed
        // so it needs investigation.
        let list_b = &b.list;
        let mut need_a = true;
        let mut need_b = true;
        let mut a_ptr = 0usize;
        let mut b_ptr = 0usize;
        let a_len = (self.len - 1) as usize;
        let b_len = (b.len - 1) as usize;
        let (mut start_a, mut start_b, mut limit_a, mut limit_b) = (0, 0, 0, 0);
        loop {
            // double iterations are such a pain...
            if need_a {
                if a_ptr >= a_len {
                    // ran out of A. If B is also exhausted, then break;
                    if need_b && b_ptr >= b_len {
                        break;
                    }
                    return false;
                }
                start_a = self.list[a_ptr];
                limit_a = self.list[a_ptr + 1];
                a_ptr += 2;
            }
            if need_b {
                if b_ptr >= b_len {
                    // ran out of B. Since we got this far, we have an A and we are ok so far
                    break;
                }
                start_b = list_b[b_ptr];
                limit_b = list_b[b_ptr + 1];
                b_ptr += 2;
            }
            // if B doesn't overlap and is greater than A, get next A
            if start_b >= limit_a {
                need_a = true;
                need_b = false;
                continue;
            }
            // if B is wholy contained in A, then get a new B
            if start_b >= start_a && limit_b <= limit_a {
                need_a = false;
                need_b = true;
                continue;
            }
            // all other combinations mean we fail
            return false;
        }

        b.strings.is_subset(&self.strings)
    }

    /// `containsAll(String s)`: whether `s` can be built from the code
    /// points and strings of the set.
    pub fn contains_all_str(&self, s: &[u16]) -> bool {
        let mut i = 0;
        while i < s.len() as i32 {
            let cp = UTF16::char_at(s, i);
            if !self.contains_valid(cp) {
                if !self.has_strings() {
                    return false;
                }
                return self.contains_all_from(s, 0);
            }
            i += UTF16::get_char_count(cp);
        }
        true
    }

    /// Recursive routine called if we fail to find a match in containsAll,
    /// and there are strings.
    fn contains_all_from(&self, s: &[u16], i: i32) -> bool {
        if i >= s.len() as i32 {
            return true;
        }
        let cp = UTF16::char_at(s, i);
        if self.contains_valid(cp) && self.contains_all_from(s, i + UTF16::get_char_count(cp)) {
            return true;
        }
        for set_str in &self.strings {
            if !set_str.is_empty()
                && s.len() >= i as usize + set_str.len()
                && &s[i as usize..i as usize + set_str.len()] == set_str.as_slice()
                && self.contains_all_from(s, i + set_str.len() as i32)
            {
                return true;
            }
        }
        false
    }

    /// `getRegexEquivalent()` (deprecated internal): a regex alternation
    /// when the set has strings.
    pub fn get_regex_equivalent(&self) -> Vec<u16> {
        if !self.has_strings() {
            return self.to_units();
        }
        let mut result = ascii("(?:");
        self.append_new_pattern(&mut result, true, false);
        for s in &self.strings {
            result.push(b'|' as u16);
            append_to_pat_str(&mut result, s, true);
        }
        result.push(b')' as u16);
        result
    }

    /// `containsNone(int start, int end)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    pub fn contains_none_range(&self, start: i32, end: i32) -> Result<bool> {
        check_cp(start)?;
        check_cp(end)?;
        let mut i: i32 = -1;
        loop {
            i += 1;
            if start < self.list[i as usize] {
                break;
            }
        }
        Ok((i & 1) == 0 && end < self.list[i as usize])
    }

    /// `containsNone(UnicodeSet b)`.
    pub fn contains_none(&self, b: &UnicodeSet) -> bool {
        // The specified set is a subset if some of its pairs overlap with some of this set's pairs.
        // This implementation accesses the lists directly for speed.
        let list_b = &b.list;
        let mut need_a = true;
        let mut need_b = true;
        let mut a_ptr = 0usize;
        let mut b_ptr = 0usize;
        let a_len = (self.len - 1) as usize;
        let b_len = (b.len - 1) as usize;
        let (mut start_a, mut start_b, mut limit_a, mut limit_b) = (0, 0, 0, 0);
        loop {
            // double iterations are such a pain...
            if need_a {
                if a_ptr >= a_len {
                    // ran out of A: break so we test strings
                    break;
                }
                start_a = self.list[a_ptr];
                limit_a = self.list[a_ptr + 1];
                a_ptr += 2;
            }
            if need_b {
                if b_ptr >= b_len {
                    // ran out of B: break so we test strings
                    break;
                }
                start_b = list_b[b_ptr];
                limit_b = list_b[b_ptr + 1];
                b_ptr += 2;
            }
            // if B is higher than any part of A, get new A
            if start_b >= limit_a {
                need_a = true;
                need_b = false;
                continue;
            }
            // if A is higher than any part of B, get new B
            if start_a >= limit_b {
                need_a = false;
                need_b = true;
                continue;
            }
            // all other combinations mean we fail
            return false;
        }

        SortedSetRelation::has_relation(&self.strings, SortedSetRelation::DISJOINT, &b.strings)
            .expect("DISJOINT is a valid relation")
    }

    /// `containsNone(CharSequence s)`.
    pub fn contains_none_str(&self, s: &[u16]) -> bool {
        self.span(s, 0, SpanCondition::NotContained) == s.len() as i32
    }

    /// `containsSome(int start, int end)`.
    ///
    /// # Errors
    ///
    /// As [`contains_none_range`](Self::contains_none_range).
    pub fn contains_some_range(&self, start: i32, end: i32) -> Result<bool> {
        Ok(!self.contains_none_range(start, end)?)
    }

    /// `containsSome(UnicodeSet s)`.
    pub fn contains_some(&self, s: &UnicodeSet) -> bool {
        !self.contains_none(s)
    }

    /// `containsSome(CharSequence s)`.
    pub fn contains_some_str(&self, s: &[u16]) -> bool {
        !self.contains_none_str(s)
    }

    /// `addAll(UnicodeSet c)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn add_all(&mut self, c: &UnicodeSet) -> Result<&mut Self> {
        self.check_frozen()?;
        self.add_all_unchecked(c);
        Ok(self)
    }

    fn add_all_unchecked(&mut self, c: &UnicodeSet) {
        self.add_list(&c.list, c.len, 0);
        if c.has_strings() {
            self.strings.extend(c.strings.iter().cloned());
        }
    }

    /// `retainAll(UnicodeSet c)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn retain_all(&mut self, c: &UnicodeSet) -> Result<&mut Self> {
        self.check_frozen()?;
        self.retain_all_unchecked(c);
        Ok(self)
    }

    pub(crate) fn retain_all_unchecked(&mut self, c: &UnicodeSet) {
        self.retain_list(&c.list, c.len, 0);
        if self.has_strings() {
            if !c.has_strings() {
                self.strings.clear();
            } else {
                self.strings.retain(|s| c.strings.contains(s));
            }
        }
    }

    /// `removeAll(UnicodeSet c)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn remove_all(&mut self, c: &UnicodeSet) -> Result<&mut Self> {
        self.check_frozen()?;
        self.retain_list(&c.list, c.len, 2);
        if self.has_strings() && c.has_strings() {
            self.strings.retain(|s| !c.strings.contains(s));
        }
        Ok(self)
    }

    /// `complementAll(UnicodeSet c)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn complement_all(&mut self, c: &UnicodeSet) -> Result<&mut Self> {
        self.check_frozen()?;
        self.xor(&c.list, c.len, 0);
        if c.has_strings() {
            if self.strings.is_empty() {
                self.strings = c.strings.clone();
            } else {
                SortedSetRelation::do_operation(&mut self.strings, SortedSetRelation::COMPLEMENTALL, &c.strings)?;
            }
        }
        Ok(self)
    }

    /// `clear()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn clear(&mut self) -> Result<&mut Self> {
        self.check_frozen()?;
        self.clear_unchecked();
        Ok(self)
    }

    fn clear_unchecked(&mut self) {
        self.list[0] = HIGH;
        self.len = 1;
        self.pat = None;
        if self.has_strings() {
            self.strings.clear();
        }
    }

    /// `getRangeCount()`.
    pub fn get_range_count(&self) -> i32 {
        self.len / 2
    }

    /// `getRangeStart(int index)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` past the list.
    pub fn get_range_start(&self, index: i32) -> i32 {
        self.list[(index * 2) as usize]
    }

    /// `getRangeEnd(int index)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` past the list.
    pub fn get_range_end(&self, index: i32) -> i32 {
        self.list[(index * 2 + 1) as usize] - 1
    }

    /// `compact()`: reallocates the list to its length.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn compact(&mut self) -> Result<&mut Self> {
        self.check_frozen()?;
        self.compact_unchecked();
        Ok(self)
    }

    fn compact_unchecked(&mut self) {
        if (self.len + 7) < self.list.len() as i32 {
            // If we have more than a little unused capacity, shrink it to len.
            self.list.truncate(self.len as usize);
            self.list.shrink_to_fit();
        }
        self.buffer = Vec::new();
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut result = self.len;
        for i in 0..self.len as usize {
            result = result.wrapping_mul(1000003);
            result = result.wrapping_add(self.list[i]);
        }
        result
    }

    /// `toString()`: `toPattern(true)`.
    pub fn to_units(&self) -> Vec<u16> {
        self.to_pattern(true)
    }

    /// `applyPattern(String pattern, ParsePosition pos, SymbolTable symbols,
    /// int options)`: parses the pattern from `pos` (the whole pattern,
    /// which must be consumed, when `pos` is `None`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed pattern; the errors of the
    /// symbol table.
    pub fn apply_pattern_full(
        &mut self,
        pattern: &[u16],
        pos: Option<&mut ParsePosition>,
        symbols: Option<&dyn SymbolTable>,
        options: i32,
    ) -> Result<&mut Self> {
        // Need to build the pattern in a temporary string because
        // _applyPattern calls add() etc., which set pat to empty.
        let parse_position_was_null = pos.is_none();
        let mut own_pos = ParsePosition::new(0);
        let pos = match pos {
            Some(p) => p,
            None => &mut own_pos,
        };

        let mut rebuilt_pat = Vec::new();
        {
            let mut chars = RuleCharacterIterator::new(pattern, symbols, pos)?;
            self.apply_pattern_chars(&mut chars, symbols, &mut rebuilt_pat, options, 0)?;
            if chars.in_variable() {
                return Err(Self::syntax_error(&chars, "Extra chars in variable value"));
            }
        }
        self.pat = Some(rebuilt_pat);
        if parse_position_was_null {
            let mut i = pos.get_index();

            // Skip over trailing whitespace
            if (options & IGNORE_SPACE) != 0 {
                i = PatternProps::skip_white_space(pattern, i);
            }

            if i != pattern.len() as i32 {
                return Err(LuceneError::illegal_argument(format!(
                    "Parse of \"{}\" failed at {}",
                    String::from_utf16_lossy(pattern),
                    i
                )));
            }
        }
        Ok(self)
    }

    /// Parse the pattern from the given RuleCharacterIterator.  The
    /// iterator is advanced over the parsed pattern.
    fn apply_pattern_chars(
        &mut self,
        chars: &mut RuleCharacterIterator<'_>,
        symbols: Option<&dyn SymbolTable>,
        rebuilt_pat: &mut Vec<u16>,
        options: i32,
        depth: i32,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(Self::syntax_error(chars, "Pattern nested too deeply"));
        }

        // Syntax characters: [ ] ^ - & { }

        // Recognized special forms for chars, sets: c-c s-s s&s

        let mut opts = rci::PARSE_VARIABLES | rci::PARSE_ESCAPES;
        if (options & IGNORE_SPACE) != 0 {
            opts |= rci::SKIP_WHITESPACE;
        }

        let mut pat_buf: Vec<u16> = Vec::new();
        let mut buf: Vec<u16> = Vec::new();
        let mut use_pat = false;
        let mut scratch: Option<UnicodeSet> = None;
        let mut backup: Option<Position> = None;

        // mode: 0=before [, 1=between [...], 2=after ]
        // lastItem: 0=none, 1=char, 2=set
        let mut last_item = LAST0_START;
        let mut last_char = 0;
        let mut mode = MODE0_NONE;
        let mut op: u16 = 0;

        let mut invert = false;

        self.clear_unchecked();
        let mut last_string: Option<Vec<u16>> = None;

        while mode != MODE2_OUTBRACKET && !chars.at_end() {
            //Eclipse stated the following is "dead code"
            /*
            if (false) {
                // Debugging assertion
                if (!((lastItem == 0 && op == 0) ||
                        (lastItem == 1 && (op == 0 || op == '-')) ||
                        (lastItem == 2 && (op == 0 || op == '-' || op == '&')))) {
                    throw new IllegalArgumentException();
                }
            }*/

            let mut c = 0;
            let mut literal = false;
            let mut preparsed: Option<Arc<dyn UnicodeMatcher>> = None;

            // -------- Check for property pattern

            // setMode: 0=none, 1=unicodeset, 2=propertypat, 3=preparsed
            let mut set_mode = SETMODE0_NONE;
            if Self::resembles_property_pattern_chars(chars, opts)? {
                set_mode = SETMODE2_PROPERTYPAT;
            }
            // -------- Parse '[' of opening delimiter OR nested set.
            // If there is a nested set, use `setMode' to define how
            // the set should be parsed.  If the '[' is part of the
            // opening delimiter for this pattern, parse special
            // strings "[", "[^", "[-", and "[^-".  Check for stand-in
            // characters representing a nested set in the symbol
            // table.
            else {
                // Prepare to backup if necessary
                backup = Some(chars.get_pos(backup.take()));
                c = chars.next(opts)?;
                literal = chars.is_escaped();

                if c == b'[' as i32 && !literal {
                    if mode == MODE1_INBRACKET {
                        chars.set_pos(backup.as_ref().expect("set above"));
                        set_mode = SETMODE1_UNICODESET;
                    } else {
                        // Handle opening '[' delimiter
                        mode = MODE1_INBRACKET;
                        pat_buf.push(b'[' as u16);
                        backup = Some(chars.get_pos(backup.take()));
                        c = chars.next(opts)?;
                        literal = chars.is_escaped();
                        if c == b'^' as i32 && !literal {
                            invert = true;
                            pat_buf.push(b'^' as u16);
                            backup = Some(chars.get_pos(backup.take()));
                            c = chars.next(opts)?;
                            literal = chars.is_escaped();
                        }
                        // Fall through to handle special leading '-';
                        // otherwise restart loop for nested [], \p{}, etc.
                        if c == b'-' as i32 {
                            literal = true;
                            // Fall through to handle literal '-' below
                        } else {
                            chars.set_pos(backup.as_ref().expect("set above"));
                            continue;
                        }
                    }
                } else if let Some(symbols) = symbols {
                    if let Some(m) = symbols.lookup_matcher(c) {
                        if m.as_any().downcast_ref::<UnicodeSet>().is_some() {
                            preparsed = Some(m);
                            set_mode = SETMODE3_PREPARSED;
                        } else {
                            return Err(Self::syntax_error(chars, "Syntax error"));
                        }
                    }
                }
            }

            // -------- Handle a nested set.  This either is inline in
            // the pattern or represented by a stand-in that has
            // previously been parsed and was looked up in the symbol
            // table.

            if set_mode != SETMODE0_NONE {
                if last_item == LAST1_RANGE {
                    if op != 0 {
                        return Err(Self::syntax_error(chars, "Char expected after operator"));
                    }
                    self.add_unchecked_range(last_char, last_char);
                    append_to_pat(&mut pat_buf, last_char, false);
                    last_item = LAST0_START;
                    op = 0;
                }

                if op == b'-' as u16 || op == b'&' as u16 {
                    pat_buf.push(op);
                }

                match set_mode {
                    SETMODE1_UNICODESET => {
                        let nested = scratch.get_or_insert_with(UnicodeSet::new);
                        nested.apply_pattern_chars(chars, symbols, &mut pat_buf, options, depth + 1)?;
                    }
                    SETMODE2_PROPERTYPAT => {
                        chars.skip_ignored(opts);
                        let nested = scratch.get_or_insert_with(UnicodeSet::new);
                        nested.apply_property_pattern_chars(chars, &mut pat_buf, symbols)?;
                    }
                    SETMODE3_PREPARSED => {
                        // `nested` is the symbol table's set.
                        let m = preparsed.as_ref().expect("set with SETMODE3_PREPARSED");
                        let nested = m.as_any().downcast_ref::<UnicodeSet>().expect("checked above");
                        nested.to_pattern_into(&mut pat_buf, false);
                    }
                    _ => {}
                }

                use_pat = true;

                let nested: &UnicodeSet = match &preparsed {
                    Some(m) => m.as_any().downcast_ref::<UnicodeSet>().expect("checked above"),
                    None => scratch.as_ref().expect("filled above"),
                };

                if mode == MODE0_NONE {
                    // Entire pattern is a category; leave parse loop
                    self.set_unchecked(nested);
                    mode = MODE2_OUTBRACKET;
                    break;
                }

                match op {
                    0x2D /* - */ => {
                        self.retain_list(&nested.list, nested.len, 2);
                        if self.has_strings() && nested.has_strings() {
                            self.strings.retain(|s| !nested.strings.contains(s));
                        }
                    }
                    0x26 /* & */ => self.retain_all_unchecked(nested),
                    0 => self.add_all_unchecked(nested),
                    _ => {}
                }

                op = 0;
                last_item = LAST2_SET;

                continue;
            }

            if mode == MODE0_NONE {
                return Err(Self::syntax_error(chars, "Missing '['"));
            }

            // -------- Parse special (syntax) characters.  If the
            // current character is not special, or if it is escaped,
            // then fall through and handle it below.

            if !literal {
                let mut fall_through = false;
                match c {
                    0x5D /* ] */ => {
                        if last_item == LAST1_RANGE {
                            self.add_unchecked_range(last_char, last_char);
                            append_to_pat(&mut pat_buf, last_char, false);
                        }
                        // Treat final trailing '-' as a literal
                        if op == b'-' as u16 {
                            self.add_unchecked_range(op as i32, op as i32);
                            pat_buf.push(op);
                        } else if op == b'&' as u16 {
                            return Err(Self::syntax_error(chars, "Trailing '&'"));
                        }
                        pat_buf.push(b']' as u16);
                        mode = MODE2_OUTBRACKET;
                        continue;
                    }
                    0x2D /* - */ => {
                        if op == 0 {
                            if last_item != LAST0_START {
                                op = c as u16;
                                continue;
                            } else if last_string.is_some() {
                                op = c as u16;
                                continue;
                            } else {
                                // Treat final trailing '-' as a literal
                                self.add_unchecked_range(c, c);
                                c = chars.next(opts)?;
                                literal = chars.is_escaped();
                                if c == b']' as i32 && !literal {
                                    pat_buf.extend_from_slice(&ascii("-]"));
                                    mode = MODE2_OUTBRACKET;
                                    continue;
                                }
                            }
                        }
                        return Err(Self::syntax_error(chars, "'-' not after char, string, or set"));
                    }
                    0x26 /* & */ => {
                        if last_item == LAST2_SET && op == 0 {
                            op = c as u16;
                            continue;
                        }
                        return Err(Self::syntax_error(chars, "'&' not after set"));
                    }
                    0x5E /* ^ */ => {
                        return Err(Self::syntax_error(chars, "'^' not after '['"));
                    }
                    0x7B /* { */ => {
                        if op != 0 && op != b'-' as u16 {
                            return Err(Self::syntax_error(chars, "Missing operand after operator"));
                        }
                        if last_item == LAST1_RANGE {
                            self.add_unchecked_range(last_char, last_char);
                            append_to_pat(&mut pat_buf, last_char, false);
                        }
                        last_item = LAST0_START;
                        buf.clear();
                        let mut ok = false;
                        while !chars.at_end() {
                            c = chars.next(opts)?;
                            literal = chars.is_escaped();
                            if c == b'}' as i32 && !literal {
                                ok = true;
                                break;
                            }
                            append_code_point(&mut buf, c);
                        }
                        if !ok {
                            return Err(Self::syntax_error(chars, "Invalid multicharacter string"));
                        }
                        // We have new string. Add it to set and continue;
                        // we don't need to drop through to the further
                        // processing
                        let cur_string = buf.clone();
                        if op == b'-' as u16 {
                            let last_single = CharSequences::get_single_code_point(last_string.as_deref().unwrap_or(&[]));
                            let cur_single = CharSequences::get_single_code_point(&cur_string);
                            if last_single != i32::MAX && cur_single != i32::MAX {
                                self.add_range(last_single, cur_single)?;
                            } else {
                                if let Err(e) = StringRange::expand(last_string.as_deref(), Some(&cur_string), true, &mut self.strings) {
                                    let msg = match crate::util::icu_exception::ICUException::of(&e) {
                                        Some(t) => rucene::error::JavaThrowable::get_message(t).unwrap_or("null").to_string(),
                                        None => e.to_string(),
                                    };
                                    return Err(Self::syntax_error(chars, &msg));
                                }
                            }
                            last_string = None;
                            op = 0;
                        } else {
                            self.add_str(&cur_string)?;
                            last_string = Some(cur_string.clone());
                        }
                        pat_buf.push(b'{' as u16);
                        append_to_pat_str(&mut pat_buf, &cur_string, false);
                        pat_buf.push(b'}' as u16);
                        continue;
                    }
                    x if x == SYMBOL_REF as i32 => {
                        //         symbols  nosymbols
                        // [a-$]   error    error (ambiguous)
                        // [a$]    anchor   anchor
                        // [a-$x]  var "x"* literal '$'
                        // [a-$.]  error    literal '$'
                        // *We won't get here in the case of var "x"
                        backup = Some(chars.get_pos(backup.take()));
                        c = chars.next(opts)?;
                        literal = chars.is_escaped();
                        let anchor = c == b']' as i32 && !literal;
                        if symbols.is_none() && !anchor {
                            c = SYMBOL_REF as i32;
                            chars.set_pos(backup.as_ref().expect("set above"));
                            fall_through = true; // literal '$'
                        } else {
                            if anchor && op == 0 {
                                if last_item == LAST1_RANGE {
                                    self.add_unchecked_range(last_char, last_char);
                                    append_to_pat(&mut pat_buf, last_char, false);
                                }
                                self.add_valid(ETHER as i32);
                                use_pat = true;
                                pat_buf.push(SYMBOL_REF);
                                pat_buf.push(b']' as u16);
                                mode = MODE2_OUTBRACKET;
                                continue;
                            }
                            return Err(Self::syntax_error(chars, "Unquoted '$'"));
                        }
                    }
                    _ => {
                        fall_through = true;
                    }
                }
                debug_assert!(fall_through);
            }

            // -------- Parse literal characters.  This includes both
            // escaped chars ("丁") and non-syntax characters
            // ("a").

            match last_item {
                LAST0_START => {
                    if op == b'-' as u16 && last_string.is_some() {
                        return Err(Self::syntax_error(chars, "Invalid range"));
                    }
                    last_item = LAST1_RANGE;
                    last_char = c;
                    last_string = None;
                }
                LAST1_RANGE => {
                    if op == b'-' as u16 {
                        if last_string.is_some() {
                            return Err(Self::syntax_error(chars, "Invalid range"));
                        }
                        if last_char >= c {
                            // Don't allow redundant (a-a) or empty (b-a) ranges;
                            // these are most likely typos.
                            return Err(Self::syntax_error(chars, "Invalid range"));
                        }
                        self.add_unchecked_range(last_char, c);
                        append_to_pat(&mut pat_buf, last_char, false);
                        pat_buf.push(op);
                        append_to_pat(&mut pat_buf, c, false);
                        last_item = LAST0_START;
                        op = 0;
                    } else {
                        self.add_unchecked_range(last_char, last_char);
                        append_to_pat(&mut pat_buf, last_char, false);
                        last_char = c;
                    }
                }
                LAST2_SET => {
                    if op != 0 {
                        return Err(Self::syntax_error(chars, "Set expected after operator"));
                    }
                    last_char = c;
                    last_item = LAST1_RANGE;
                }
                _ => {}
            }
        }

        if mode != MODE2_OUTBRACKET {
            return Err(Self::syntax_error(chars, "Missing ']'"));
        }

        chars.skip_ignored(opts);

        /*
         * Handle global flags (invert, case insensitivity).  If this
         * pattern should be compiled case-insensitive, then we need
         * to close over case BEFORE COMPLEMENTING.  This makes
         * patterns like /[^abc]/i work.
         */
        if (options & CASE_MASK) != 0 {
            self.close_over(options)?;
        }
        if invert {
            self.complement_unchecked();
            self.strings.clear();
            self.pat = None;
        }

        // Use the rebuilt pattern (pat) only if necessary.  Prefer the
        // generated pattern.
        if use_pat {
            rebuilt_pat.extend_from_slice(&pat_buf);
        } else {
            self.append_new_pattern(rebuilt_pat, false, true);
        }
        Ok(())
    }

    fn syntax_error(chars: &RuleCharacterIterator<'_>, msg: &str) -> LuceneError {
        LuceneError::illegal_argument(format!(
            "Error: {} at \"{}\"",
            msg,
            String::from_utf16_lossy(&Utility::escape_string(&chars.to_units()))
        ))
    }

    /// `addAllTo(T target)`: appends every element (code points as strings,
    /// then strings) to `target`.
    pub fn add_all_to<'t>(&self, target: &'t mut Vec<Vec<u16>>) -> &'t mut Vec<Vec<u16>> {
        target.extend(self.iter());
        target
    }

    /// `addAllTo(String[] target)`: fills `target` from index 0.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `target` is too short.
    pub fn add_all_to_array<'t>(&self, target: &'t mut [Vec<u16>]) -> &'t mut [Vec<u16>] {
        for (i, item) in self.iter().enumerate() {
            target[i] = item;
        }
        target
    }

    /// `toArray(UnicodeSet set)`.
    pub fn to_array(set: &UnicodeSet) -> Vec<Vec<u16>> {
        set.iter().collect()
    }

    /// `add(Iterable<?> source)`: as [`add_all_iter`](Self::add_all_iter).
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn add_iter<I, S>(&mut self, source: I) -> Result<&mut Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u16]>,
    {
        self.add_all_iter(source)
    }

    /// `addAll(Iterable<?> source)`: adds each element's string form.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn add_all_iter<I, S>(&mut self, source: I) -> Result<&mut Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u16]>,
    {
        self.check_frozen()?;
        for o in source {
            self.add_str(o.as_ref())?;
        }
        Ok(self)
    }

    //----------------------------------------------------------------
    // Implementation: Fundamental operations
    //----------------------------------------------------------------

    fn next_capacity(min_capacity: i32) -> i32 {
        // Grow exponentially to reduce the frequency of allocations.
        if min_capacity < INITIAL_CAPACITY {
            min_capacity + INITIAL_CAPACITY
        } else if min_capacity <= 2500 {
            5 * min_capacity
        } else {
            let mut new_capacity = 2 * min_capacity;
            if new_capacity > MAX_LENGTH {
                new_capacity = MAX_LENGTH;
            }
            new_capacity
        }
    }

    fn ensure_capacity(&mut self, mut new_len: i32) {
        if new_len > MAX_LENGTH {
            new_len = MAX_LENGTH;
        }
        if new_len <= self.list.len() as i32 {
            return;
        }
        let new_capacity = Self::next_capacity(new_len);
        let mut temp = vec![0i32; new_capacity as usize];
        // Copy only the actual contents.
        temp[..self.len as usize].copy_from_slice(&self.list[..self.len as usize]);
        self.list = temp;
    }

    fn ensure_buffer_capacity(&mut self, mut new_len: i32) {
        if new_len > MAX_LENGTH {
            new_len = MAX_LENGTH;
        }
        if !self.buffer.is_empty() && new_len <= self.buffer.len() as i32 {
            return;
        }
        let new_capacity = Self::next_capacity(new_len);
        self.buffer = vec![0i32; new_capacity as usize];
        // The buffer does not need to be copied because it is never used
        // after a call to xor(), add(), or retain() returns.
    }

    // polarity = 0, 3 is normal: x xor y
    // polarity = 1, 2: x xor ~y == x === y

    fn xor(&mut self, other: &[i32], other_len: i32, polarity: i32) {
        self.ensure_buffer_capacity(self.len + other_len);
        let list = &self.list;
        let buffer = &mut self.buffer;
        let (mut i, mut j, mut k) = (0usize, 0usize, 0usize);
        let mut a = list[i];
        i += 1;
        let mut b;
        // TODO: Based on the call hierarchy, polarity of 1 or 2 is never used
        //      so the following if statement will not be called.
        // CLOVER:OFF
        if polarity == 1 || polarity == 2 {
            b = LOW;
            if other[j] == LOW {
                // skip base if already LOW
                j += 1;
                b = other[j];
            }
            // CLOVER:ON
        } else {
            b = other[j];
            j += 1;
        }
        // simplest of all the routines
        // sort the values, discarding identicals!
        loop {
            if a < b {
                buffer[k] = a;
                k += 1;
                a = list[i];
                i += 1;
            } else if b < a {
                buffer[k] = b;
                k += 1;
                b = other[j];
                j += 1;
            } else if a != HIGH {
                // at this point, a == b
                // discard both values!
                a = list[i];
                i += 1;
                b = other[j];
                j += 1;
            } else {
                // DONE!
                buffer[k] = HIGH;
                k += 1;
                self.len = k as i32;
                break;
            }
        }
        // swap list and buffer
        std::mem::swap(&mut self.list, &mut self.buffer);
        self.pat = None;
    }

    // polarity = 0 is normal: x union y
    // polarity = 2: x union ~y
    // polarity = 1: ~x union y
    // polarity = 3: ~x union ~y

    fn add_list(&mut self, other: &[i32], other_len: i32, mut polarity: i32) {
        self.ensure_buffer_capacity(self.len + other_len);
        let list = &self.list;
        let buffer = &mut self.buffer;
        let (mut i, mut j, mut k) = (0usize, 0usize, 0usize);
        let mut a = list[i];
        i += 1;
        let mut b = other[j];
        j += 1;
        // change from xor is that we have to check overlapping pairs
        // polarity bit 1 means a is second, bit 2 means b is.
        'main: loop {
            match polarity {
                0 => {
                    // both first; take lower if unequal
                    if a < b {
                        // take a
                        // Back up over overlapping ranges in buffer[]
                        if k > 0 && a <= buffer[k - 1] {
                            // Pick latter end value in buffer[] vs. list[]
                            k -= 1;
                            a = list[i].max(buffer[k]);
                        } else {
                            // No overlap
                            buffer[k] = a;
                            k += 1;
                            a = list[i];
                        }
                        i += 1; // Common if/else code factored out
                        polarity ^= 1;
                    } else if b < a {
                        // take b
                        if k > 0 && b <= buffer[k - 1] {
                            k -= 1;
                            b = other[j].max(buffer[k]);
                        } else {
                            buffer[k] = b;
                            k += 1;
                            b = other[j];
                        }
                        j += 1;
                        polarity ^= 2;
                    } else {
                        // a == b, take a, drop b
                        if a == HIGH {
                            break 'main;
                        }
                        // This is symmetrical; it doesn't matter if
                        // we backtrack with a or b. - liu
                        if k > 0 && a <= buffer[k - 1] {
                            k -= 1;
                            a = list[i].max(buffer[k]);
                        } else {
                            // No overlap
                            buffer[k] = a;
                            k += 1;
                            a = list[i];
                        }
                        i += 1;
                        polarity ^= 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    }
                }
                3 => {
                    // both second; take higher if unequal, and drop other
                    if b <= a {
                        // take a
                        if a == HIGH {
                            break 'main;
                        }
                        buffer[k] = a;
                        k += 1;
                    } else {
                        // take b
                        if b == HIGH {
                            break 'main;
                        }
                        buffer[k] = b;
                        k += 1;
                    }
                    a = list[i];
                    i += 1;
                    polarity ^= 1; // factored common code
                    b = other[j];
                    j += 1;
                    polarity ^= 2;
                }
                1 => {
                    // a second, b first; if b < a, overlap
                    if a < b {
                        // no overlap, take a
                        buffer[k] = a;
                        k += 1;
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                    } else if b < a {
                        // OVERLAP, drop b
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    } else {
                        // a == b, drop both!
                        if a == HIGH {
                            break 'main;
                        }
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    }
                }
                2 => {
                    // a first, b second; if a < b, overlap
                    if b < a {
                        // no overlap, take b
                        buffer[k] = b;
                        k += 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    } else if a < b {
                        // OVERLAP, drop a
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                    } else {
                        // a == b, drop both!
                        if a == HIGH {
                            break 'main;
                        }
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    }
                }
                _ => {}
            }
        }
        buffer[k] = HIGH; // terminate
        k += 1;
        self.len = k as i32;
        // swap list and buffer
        std::mem::swap(&mut self.list, &mut self.buffer);
        self.pat = None;
    }

    // polarity = 0 is normal: x intersect y
    // polarity = 2: x intersect ~y == set-minus
    // polarity = 1: ~x intersect y
    // polarity = 3: ~x intersect ~y

    fn retain_list(&mut self, other: &[i32], other_len: i32, mut polarity: i32) {
        self.ensure_buffer_capacity(self.len + other_len);
        let list = &self.list;
        let buffer = &mut self.buffer;
        let (mut i, mut j, mut k) = (0usize, 0usize, 0usize);
        let mut a = list[i];
        i += 1;
        let mut b = other[j];
        j += 1;
        // change from xor is that we have to check overlapping pairs
        // polarity bit 1 means a is second, bit 2 means b is.
        'main: loop {
            match polarity {
                0 => {
                    // both first; drop the smaller
                    if a < b {
                        // drop a
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                    } else if b < a {
                        // drop b
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    } else {
                        // a == b, take one, drop other
                        if a == HIGH {
                            break 'main;
                        }
                        buffer[k] = a;
                        k += 1;
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    }
                }
                3 => {
                    // both second; take lower if unequal
                    if a < b {
                        // take a
                        buffer[k] = a;
                        k += 1;
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                    } else if b < a {
                        // take b
                        buffer[k] = b;
                        k += 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    } else {
                        // a == b, take one, drop other
                        if a == HIGH {
                            break 'main;
                        }
                        buffer[k] = a;
                        k += 1;
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    }
                }
                1 => {
                    // a second, b first;
                    if a < b {
                        // NO OVERLAP, drop a
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                    } else if b < a {
                        // OVERLAP, take b
                        buffer[k] = b;
                        k += 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    } else {
                        // a == b, drop both!
                        if a == HIGH {
                            break 'main;
                        }
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    }
                }
                2 => {
                    // a first, b second; if a < b, overlap
                    if b < a {
                        // no overlap, drop b
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    } else if a < b {
                        // OVERLAP, take a
                        buffer[k] = a;
                        k += 1;
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                    } else {
                        // a == b, drop both!
                        if a == HIGH {
                            break 'main;
                        }
                        a = list[i];
                        i += 1;
                        polarity ^= 1;
                        b = other[j];
                        j += 1;
                        polarity ^= 2;
                    }
                }
                _ => {}
            }
        }
        buffer[k] = HIGH; // terminate
        k += 1;
        self.len = k as i32;
        // swap list and buffer
        std::mem::swap(&mut self.list, &mut self.buffer);
        self.pat = None;
    }

    //----------------------------------------------------------------
    // Generic filter-based scanning code
    //
    // NOTE: In ICU4J, these is a lot of code for sets of type properties
    // here; ICU4J's properties are handled by the Filter subclasses.
    //----------------------------------------------------------------

    /// Generic filter-based scanning code for UCD property UnicodeSets.
    fn apply_filter(&mut self, filter: &Filter, inclusions: &UnicodeSet) {
        // Logically, walk through all Unicode characters, noting the start
        // and end of each range for which filter.contain(c) is
        // true.  Add each range to a set.
        //
        // To improve performance, use an inclusions set which
        // encodes information about character ranges that are known
        // to have identical properties.
        // inclusions contains the first characters of
        // same-value ranges for the given property.

        self.clear_unchecked();

        let mut start_has_property = -1;
        let limit_range = inclusions.get_range_count();

        for j in 0..limit_range {
            // get current range
            let start = inclusions.get_range_start(j);
            let end = inclusions.get_range_end(j);

            // for all the code points in the range, process
            for ch in start..=end {
                // only add to the unicodeset on inflection points --
                // where the hasProperty value changes to false
                if filter.contains(ch) {
                    if start_has_property < 0 {
                        start_has_property = ch;
                    }
                } else if start_has_property >= 0 {
                    self.add_unchecked_range(start_has_property, ch - 1);
                    start_has_property = -1;
                }
            }
        }
        if start_has_property >= 0 {
            self.add_unchecked_range(start_has_property, 0x10FFFF);
        }
    }

    /// Remove leading and trailing Pattern_White_Space and compress
    /// internal Pattern_White_Space to a single space character.
    fn munge_char_name(source: &[u16]) -> Vec<u16> {
        let source = PatternProps::trim_white_space(source);
        let mut buf: Option<Vec<u16>> = None;
        for i in 0..source.len() {
            let mut ch = source[i];
            if PatternProps::is_white_space(ch as i32) {
                match &buf {
                    None => buf = Some(source[..i].to_vec()),
                    Some(b) => {
                        if b[b.len() - 1] == b' ' as u16 {
                            continue;
                        }
                    }
                }
                ch = b' ' as u16; // convert to ' '
            }
            if let Some(b) = &mut buf {
                b.push(ch);
            }
        }
        buf.unwrap_or(source)
    }

    //----------------------------------------------------------------
    // Property set API
    //----------------------------------------------------------------

    /// `applyIntPropertyValue(int prop, int value)`: makes this the set of
    /// code points with the property value.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("unsupported property <prop>")`; the
    /// errors of the property data.
    pub fn apply_int_property_value(&mut self, prop: i32, value: i32) -> Result<&mut Self> {
        // All of the following include checkFrozen() before modifying this set.
        if prop == UProperty::GENERAL_CATEGORY_MASK {
            self.check_frozen()?;
            let inclusions = CharacterPropertiesImpl::get_inclusions_for_property(prop)?;
            self.apply_filter(&Filter::GeneralCategoryMask(value), &inclusions);
        } else if prop == UProperty::SCRIPT_EXTENSIONS {
            self.check_frozen()?;
            let inclusions = CharacterPropertiesImpl::get_inclusions_for_property(prop)?;
            self.apply_filter(&Filter::ScriptExtensions(value), &inclusions);
        } else if prop == UProperty::IDENTIFIER_TYPE {
            self.check_frozen()?;
            let inclusions = CharacterPropertiesImpl::get_inclusions_for_property(prop)?;
            self.apply_filter(&Filter::IdentifierType(value), &inclusions);
        } else if (0..UProperty::BINARY_LIMIT).contains(&prop) {
            if value == 0 || value == 1 {
                let set = CharacterProperties::get_binary_property_set(prop)?;
                self.set(&set)?;
                if value == 0 {
                    self.complement()?.remove_all_strings()?; // code point complement
                }
            } else {
                self.clear()?;
            }
        } else if (UProperty::INT_START..UProperty::INT_LIMIT).contains(&prop) {
            self.check_frozen()?;
            let inclusions = CharacterPropertiesImpl::get_inclusions_for_property(prop)?;
            self.apply_filter(&Filter::IntProperty(prop, value), &inclusions);
        } else {
            return Err(LuceneError::illegal_argument(format!("unsupported property {prop}")));
        }
        Ok(self)
    }

    /// `applyPropertyAlias(String propertyAlias, String valueAlias)`.
    ///
    /// # Errors
    ///
    /// As [`apply_property_alias_symbols`](Self::apply_property_alias_symbols).
    pub fn apply_property_alias(&mut self, property_alias: &[u16], value_alias: &[u16]) -> Result<&mut Self> {
        self.apply_property_alias_symbols(property_alias, value_alias, None)
    }

    /// `applyPropertyAlias(String propertyAlias, String valueAlias,
    /// SymbolTable symbols)`: makes this the set of code points with the
    /// named property value.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; `IllegalArgumentException`
    /// for an unknown or unsupported property or value;
    /// `NumberFormatException` for a malformed numeric value.
    pub fn apply_property_alias_symbols(
        &mut self,
        property_alias: &[u16],
        value_alias: &[u16],
        symbols: Option<&dyn SymbolTable>,
    ) -> Result<&mut Self> {
        self.check_frozen()?;
        let mut p;
        let v;
        let mut invert = false;

        if let Some(x) = symbols.and_then(|s| s.as_x_symbol_table()) {
            if x.apply_property_alias(property_alias, value_alias, self)? {
                return Ok(self);
            }
        }

        let default_table = XSYMBOL_TABLE.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(t) = default_table {
            if t.apply_property_alias(property_alias, value_alias, self)? {
                return Ok(self);
            }
        }

        if !value_alias.is_empty() {
            p = UCharacter::get_property_enum(property_alias)?;

            // Treat gc as gcm
            if p == UProperty::GENERAL_CATEGORY {
                p = UProperty::GENERAL_CATEGORY_MASK;
            }

            if (UProperty::BINARY_START..UProperty::BINARY_LIMIT).contains(&p)
                || (UProperty::INT_START..UProperty::INT_LIMIT).contains(&p)
                || (UProperty::MASK_START..UProperty::MASK_LIMIT).contains(&p)
            {
                match UCharacter::get_property_value_enum(p, value_alias) {
                    Ok(x) => v = x,
                    Err(e) if e.is_illegal_argument_exception() => {
                        // Handle numeric CCC
                        if p == UProperty::CANONICAL_COMBINING_CLASS
                            || p == UProperty::LEAD_CANONICAL_COMBINING_CLASS
                            || p == UProperty::TRAIL_CANONICAL_COMBINING_CLASS
                        {
                            let x = crate::jdk::parse_int(&PatternProps::trim_white_space(value_alias), 10)?;
                            // If the resultant set is empty then the numeric value
                            // was invalid.
                            //if (getRangeCount() == 0) {
                            //    throw e;
                            //}
                            // 3.5 behavior:
                            // If the value is not in the range 0..255 then the numeric value
                            // was invalid.
                            if !(0..=255).contains(&x) {
                                return Err(e);
                            }
                            v = x;
                        } else {
                            return Err(e);
                        }
                    }
                    Err(e) => return Err(e),
                }
            } else {
                match p {
                    UProperty::NUMERIC_VALUE => {
                        let s = String::from_utf16_lossy(&PatternProps::trim_white_space(value_alias));
                        let value = crate::jdk::parse_double(&s)?;
                        let inclusions = CharacterPropertiesImpl::get_inclusions_for_property(p)?;
                        self.apply_filter(&Filter::NumericValue(value), &inclusions);
                        return Ok(self);
                    }
                    UProperty::NAME => {
                        // Must munge name, since
                        // UCharacter.charFromName() does not do
                        // 'loose' matching.
                        let buf = Self::munge_char_name(value_alias);
                        let ch = UCharacter::get_char_from_extended_name(&buf)?;
                        if ch == -1 {
                            return Err(LuceneError::illegal_argument("Invalid character name"));
                        }
                        self.clear()?;
                        self.add_valid(ch);
                        return Ok(self);
                    }
                    UProperty::UNICODE_1_NAME => {
                        // ICU 49 deprecates the Unicode_1_Name property APIs.
                        return Err(LuceneError::illegal_argument("Unicode_1_Name (na1) not supported"));
                    }
                    UProperty::AGE => {
                        // Must munge name, since
                        // VersionInfo.getInstance() does not do
                        // 'loose' matching.
                        let version = VersionInfo::get_instance_str(&Self::munge_char_name(value_alias))?;
                        let inclusions = CharacterPropertiesImpl::get_inclusions_for_property(p)?;
                        self.apply_filter(&Filter::Version(version), &inclusions);
                        return Ok(self);
                    }
                    UProperty::SCRIPT_EXTENSIONS => {
                        v = UCharacter::get_property_value_enum(UProperty::SCRIPT, value_alias)?;
                        // fall through to calling applyIntPropertyValue()
                    }
                    UProperty::IDENTIFIER_TYPE => {
                        v = UCharacter::get_property_value_enum(p, value_alias)?;
                    }
                    _ => {
                        // p is a non-binary, non-enumerated property that we
                        // don't support (yet).
                        return Err(LuceneError::illegal_argument("Unsupported property"));
                    }
                }
            }
        } else {
            // valueAlias is empty.  Interpret as General Category, Script,
            // Binary property, or ANY or ASCII.  Upon success, p and v will
            // be set.
            let pnames = UPropertyAliases::instance()?;
            p = UProperty::GENERAL_CATEGORY_MASK;
            let mut vv = pnames.get_property_value_enum(p, property_alias)?;
            if vv == UProperty::UNDEFINED {
                p = UProperty::SCRIPT;
                vv = pnames.get_property_value_enum(p, property_alias)?;
                if vv == UProperty::UNDEFINED {
                    p = pnames.get_property_enum(property_alias);
                    if p == UProperty::UNDEFINED {
                        p = -1;
                    }
                    if (UProperty::BINARY_START..UProperty::BINARY_LIMIT).contains(&p) {
                        vv = 1;
                    } else if p == -1 {
                        if 0 == UPropertyAliases::compare(&ascii(ANY_ID), property_alias) {
                            self.set_range(MIN_VALUE, MAX_VALUE)?;
                            return Ok(self);
                        } else if 0 == UPropertyAliases::compare(&ascii(ASCII_ID), property_alias) {
                            self.set_range(0, 0x7F)?;
                            return Ok(self);
                        } else if 0 == UPropertyAliases::compare(&ascii(ASSIGNED), property_alias) {
                            // [:Assigned:]=[:^Cn:]
                            p = UProperty::GENERAL_CATEGORY_MASK;
                            vv = 1 << UCharacter::UNASSIGNED;
                            invert = true;
                        } else {
                            // Property name was never matched.
                            return Err(LuceneError::illegal_argument(format!(
                                "Invalid property alias: {}={}",
                                String::from_utf16_lossy(property_alias),
                                String::from_utf16_lossy(value_alias)
                            )));
                        }
                    } else {
                        // Valid property name, but it isn't binary, so the value
                        // must be supplied.
                        return Err(LuceneError::illegal_argument("Missing property value"));
                    }
                }
            }
            v = vv;
        }

        self.apply_int_property_value(p, v)?;
        if invert {
            self.complement()?.remove_all_strings()?; // code point complement
        }

        Ok(self)
    }

    //----------------------------------------------------------------
    // Property set patterns
    //----------------------------------------------------------------

    /// Return true if the given position, in the given pattern, appears
    /// to be the start of a property set pattern.
    fn resembles_property_pattern(pattern: &[u16], pos: i32) -> bool {
        // Patterns are at least 5 characters long
        if (pos + 5) > pattern.len() as i32 {
            return false;
        }

        // Look for an opening [:, [:^, \p, or \P
        region_matches(pattern, pos, &ascii("[:"), false)
            || region_matches(pattern, pos, &ascii("\\p"), true)
            || region_matches(pattern, pos, &ascii("\\N"), false)
    }

    /// Return true if the given iterator appears to point at a
    /// property pattern.  Regardless of the result, return with the
    /// iterator unchanged.
    fn resembles_property_pattern_chars(chars: &mut RuleCharacterIterator<'_>, mut iter_opts: i32) -> Result<bool> {
        let mut result = false;
        iter_opts &= !rci::PARSE_ESCAPES;
        let pos = chars.get_pos(None);
        let c = chars.next(iter_opts)?;
        if c == b'[' as i32 || c == b'\\' as i32 {
            let d = chars.next(iter_opts & !rci::SKIP_WHITESPACE)?;
            result = if c == b'[' as i32 {
                d == b':' as i32
            } else {
                d == b'N' as i32 || d == b'p' as i32 || d == b'P' as i32
            };
        }
        chars.set_pos(&pos);
        Ok(result)
    }

    /// Parse the given property pattern at the given parse position.
    /// Returns false (Java's `null`) when no pattern is there.
    fn apply_property_pattern(&mut self, pattern: &[u16], ppos: &mut ParsePosition, symbols: Option<&dyn SymbolTable>) -> Result<bool> {
        let mut pos = ppos.get_index();

        // On entry, ppos should point to one of the following locations:

        // Minimum length is 5 characters, e.g. \p{L}
        if (pos + 5) > pattern.len() as i32 {
            return Ok(false);
        }

        let mut posix = false; // true for [:pat:], false for \p{pat} \P{pat} \N{pat}
        let mut is_name = false; // true for \N{pat}, o/w false
        let mut invert = false;

        // Look for an opening [:, [:^, \p, or \P
        if region_matches(pattern, pos, &ascii("[:"), false) {
            posix = true;
            pos = PatternProps::skip_white_space(pattern, pos + 2);
            if pos < pattern.len() as i32 && pattern[pos as usize] == b'^' as u16 {
                pos += 1;
                invert = true;
            }
        } else if region_matches(pattern, pos, &ascii("\\p"), true) || region_matches(pattern, pos, &ascii("\\N"), false) {
            let c = pattern[pos as usize + 1];
            invert = c == b'P' as u16;
            is_name = c == b'N' as u16;
            pos = PatternProps::skip_white_space(pattern, pos + 2);
            if pos == pattern.len() as i32 || pattern[pos as usize] != b'{' as u16 {
                // Syntax error; "\p" or "\P" not followed by "{"
                return Ok(false);
            }
            pos += 1;
        } else {
            // Open delimiter not seen
            return Ok(false);
        }

        // Look for the matching close delimiter, either :] or }
        let close = crate::jdk::index_of(pattern, &ascii(if posix { ":]" } else { "}" }), pos);
        if close < 0 {
            // Syntax error; close delimiter missing
            return Ok(false);
        }

        // Look for an '=' sign.  If this is present, we will parse a
        // medium \p{gc=Cf} or long \p{GeneralCategory=Format}
        // pattern.
        let equals = crate::jdk::index_of_unit(pattern, b'=' as u16, pos);
        let prop_name;
        let mut value_name;
        if equals >= 0 && equals < close && !is_name {
            // Equals seen; parse medium/long pattern
            prop_name = pattern[pos as usize..equals as usize].to_vec();
            value_name = pattern[equals as usize + 1..close as usize].to_vec();
        } else {
            // Handle case where no '=' is seen, and \N{}
            let mut pn = pattern[pos as usize..close as usize].to_vec();
            value_name = Vec::new();

            // Handle \N{name}
            if is_name {
                // This is a little inefficient since it means we have to
                // parse "na" back to UProperty.NAME even though we already
                // know it's UProperty.NAME.  If we refactor the API to
                // support args of (int, String) then we can remove
                // "na" and make this a little more efficient.
                value_name = pn;
                pn = ascii("na");
            }
            prop_name = pn;
        }

        self.apply_property_alias_symbols(&prop_name, &value_name, symbols)?;

        if invert {
            self.complement()?.remove_all_strings()?; // code point complement
        }

        // Move to the limit position after the close delimiter
        ppos.set_index(close + if posix { 2 } else { 1 });

        Ok(true)
    }

    /// Parse a property pattern.
    fn apply_property_pattern_chars(
        &mut self,
        chars: &mut RuleCharacterIterator<'_>,
        rebuilt_pat: &mut Vec<u16>,
        symbols: Option<&dyn SymbolTable>,
    ) -> Result<()> {
        let pat_str = chars.get_current_buffer().to_vec();
        let start = chars.get_current_buffer_pos();
        let mut pos = ParsePosition::new(start);
        self.apply_property_pattern(&pat_str, &mut pos, symbols)?;
        let length = pos.get_index() - start;
        if length == 0 {
            return Err(Self::syntax_error(chars, "Invalid property pattern"));
        }
        chars.jumpahead(length)?;
        rebuilt_pat.extend_from_slice(&pat_str[start as usize..pos.get_index() as usize]);
        Ok(())
    }

    //----------------------------------------------------------------
    // Case folding API
    //----------------------------------------------------------------

    fn add_case_mapping(set: &mut UnicodeSet, result: i32, full: &mut Vec<u16>) {
        if result >= 0 {
            if result > UCaseProps::MAX_STRING_LENGTH {
                // add a single-code point case mapping
                set.add_valid(result);
            } else {
                // add a string case mapping from full with length result
                set.add_str(full).expect("the closure set is not frozen");
                full.clear();
            }
        }
        // result < 0: the code point mapped to itself, no need to add it
        // see UCaseProps
    }

    /// For case closure on a large set, look only at code points with
    /// relevant properties.
    fn maybe_only_case_sensitive(src: &UnicodeSet) -> Result<UnicodeSet> {
        if src.size() < 30 {
            return Ok(src.clone());
        }
        // Return the intersection of the src code points with Case_Sensitive ones.
        let sensitive = CharacterProperties::get_binary_property_set(UProperty::CASE_SENSITIVE)?;
        // Start by cloning the "smaller" set. Try not to copy the strings, if there are any in src.
        if src.has_strings() || src.get_range_count() > sensitive.get_range_count() {
            let mut r = sensitive.clone_as_thawed();
            r.retain_all_unchecked(src);
            Ok(r)
        } else {
            // Java clones src (itself when frozen, then retainAll throws).
            let mut r = src.clone();
            r.retain_all(&sensitive)?;
            Ok(r)
        }
    }

    /// Per-string simple case folding. The result may not be exactly
    /// the same as the per-code point simple case folding of the string.
    fn scf_string(s: &[u16], scf: &mut Vec<u16>) -> bool {
        let length = s.len() as i32;
        // Loop while not needing modification.
        let mut i = 0;
        while i < length {
            let mut c = code_point_at(s, i);
            let mut scf_char = UCharacter::fold_case_cp_options(c, UCharacter::FOLD_CASE_DEFAULT);
            if scf_char != c {
                // Copy the characters before c.
                scf.clear();
                scf.extend_from_slice(&s[..i as usize]);
                // Loop over the rest of the string and keep case-folding.
                loop {
                    append_code_point(scf, scf_char);
                    i += char_count(c);
                    if i == length {
                        return true;
                    }
                    c = code_point_at(s, i);
                    scf_char = UCharacter::fold_case_cp_options(c, UCharacter::FOLD_CASE_DEFAULT);
                }
            }
            i += char_count(c);
        }
        false
    }

    /// `closeOver(int attribute)`: closes the set over case mappings or
    /// case-insensitive matching.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen; the errors of the case
    /// mapping and break iterator data.
    pub fn close_over(&mut self, attribute: i32) -> Result<&mut Self> {
        self.check_frozen()?;
        match attribute & CASE_MASK {
            0 => {}
            CASE_INSENSITIVE => self.close_over_case_insensitive(false)?,
            ADD_CASE_MAPPINGS => self.close_over_add_case_mappings()?,
            SIMPLE_CASE_INSENSITIVE => self.close_over_case_insensitive(true)?,
            _ => {
                // bad option (unreachable)
            }
        }
        Ok(self)
    }

    fn close_over_case_insensitive(&mut self, simple: bool) -> Result<()> {
        let csp = UCaseProps::instance();
        // Start with input set to guarantee inclusion.
        let mut fold_set = UnicodeSet::from_set(self);

        // Full case mappings closure:
        // Remove strings because the strings will actually be reduced (folded);
        // therefore, start with no strings and add only those needed.
        // Do this before processing code points, because they may add strings.
        if !simple && fold_set.has_strings() {
            fold_set.strings.clear();
        }

        let code_points = Self::maybe_only_case_sensitive(self)?;

        // Iterate over the ranges of single code points. Nested loop for each code point.
        let n = code_points.get_range_count();
        for i in 0..n {
            let start = code_points.get_range_start(i);
            let end = code_points.get_range_end(i);

            if simple {
                for cp in start..=end {
                    csp.add_simple_case_closure(cp, &mut fold_set)?;
                }
            } else {
                for cp in start..=end {
                    csp.add_case_closure(cp, &mut fold_set)?;
                }
            }
        }
        if self.has_strings() {
            let mut sb = Vec::new();
            for s in &self.strings {
                if simple {
                    if Self::scf_string(s, &mut sb) {
                        fold_set.remove_str(s)?.add_str(&sb)?;
                    }
                } else {
                    let str = UCharacter::fold_case_str_options(s, 0);
                    if !csp.add_string_case_closure(&str, &mut fold_set)? {
                        fold_set.add_str(&str)?; // does not map to code points: add the folded string itself
                    }
                }
            }
        }
        self.set_unchecked(&fold_set);
        Ok(())
    }

    fn close_over_add_case_mappings(&mut self) -> Result<()> {
        let csp = UCaseProps::instance();
        // Start with input set to guarantee inclusion.
        let mut fold_set = UnicodeSet::from_set(self);

        let code_points = Self::maybe_only_case_sensitive(self)?;

        // Iterate over the ranges of single code points. Nested loop for each code point.
        let n = code_points.get_range_count();
        let mut full = Vec::new();

        for i in 0..n {
            let start = code_points.get_range_start(i);
            let end = code_points.get_range_end(i);

            // add case mappings
            // (does not add long s for regular s, or Kelvin for k, for example)
            for cp in start..=end {
                let result = csp.to_full_lower(cp, None, &mut full, UCaseProps::LOC_ROOT);
                Self::add_case_mapping(&mut fold_set, result, &mut full);

                let result = csp.to_full_title(cp, None, &mut full, UCaseProps::LOC_ROOT);
                Self::add_case_mapping(&mut fold_set, result, &mut full);

                let result = csp.to_full_upper(cp, None, &mut full, UCaseProps::LOC_ROOT);
                Self::add_case_mapping(&mut fold_set, result, &mut full);

                let result = csp.to_full_folding(cp, &mut full, 0);
                Self::add_case_mapping(&mut fold_set, result, &mut full);
            }
        }
        if self.has_strings() {
            let root = ULocale::root();
            let mut bi = break_iterator::get_word_instance_ulocale(&root)?;
            for str in &self.strings {
                // TODO: call lower-level functions
                fold_set.add_str(&UCharacter::to_lower_case_ulocale(&root, str))?;
                fold_set.add_str(&UCharacter::to_title_case_ulocale(&root, str, Some(bi.as_mut()))?)?;
                fold_set.add_str(&UCharacter::to_upper_case_ulocale(&root, str))?;
                fold_set.add_str(&UCharacter::fold_case_str_options(str, 0))?;
            }
        }
        self.set_unchecked(&fold_set);
        Ok(())
    }

    /// `isFrozen()`.
    pub fn is_frozen(&self) -> bool {
        self.bmp_set.is_some() || self.string_span.is_some()
    }

    /// `freeze()`: makes the set immutable and builds its span structures.
    pub fn freeze(&mut self) -> &mut Self {
        if !self.is_frozen() {
            self.compact_unchecked();

            // Optimize contains() and span() and similar functions.
            if self.has_strings() {
                let strings: Vec<Vec<u16>> = self.strings.iter().cloned().collect();
                self.string_span = Some(Box::new(UnicodeSetStringSpan::new(self, strings, usss::ALL)));
            }
            if self.string_span.as_ref().is_none_or(|s| !s.needs_string_span_utf16()) {
                // Optimize for code point spans.
                // There are no strings, or
                // all strings are irrelevant for span() etc. because
                // all of each string's code points are contained in this set.
                // However, fully contained strings are relevant for spanAndCount(),
                // so we create both objects.
                self.bmp_set = Some(Box::new(BMPSet::new(&self.list, self.len)));
            }
        }
        self
    }

    /// `span(CharSequence s, SpanCondition spanCondition)`.
    pub fn span_from_start(&self, s: &[u16], span_condition: SpanCondition) -> i32 {
        self.span(s, 0, span_condition)
    }

    /// `span(CharSequence s, int start, SpanCondition spanCondition)`: the
    /// end of the span of `s` from `start`.
    pub fn span(&self, s: &[u16], mut start: i32, span_condition: SpanCondition) -> i32 {
        let end = s.len() as i32;
        if start < 0 {
            start = 0;
        } else if start >= end {
            return end;
        }
        if let Some(b) = &self.bmp_set {
            // Frozen set without strings, or no string is relevant for span().
            return b.span(s, start, span_condition, None);
        }
        if let Some(ss) = &self.string_span {
            return ss.span(s, start, span_condition);
        } else if self.has_strings() {
            let which = if span_condition == SpanCondition::NotContained {
                usss::FWD_UTF16_NOT_CONTAINED
            } else {
                usss::FWD_UTF16_CONTAINED
            };
            let str_span = UnicodeSetStringSpan::new(self, self.strings.iter().cloned().collect(), which);
            if str_span.needs_string_span_utf16() {
                return str_span.span(s, start, span_condition);
            }
        }

        self.span_code_points_and_count(s, start, span_condition, None)
    }

    /// `spanAndCount(CharSequence s, int start, SpanCondition spanCondition,
    /// OutputInt outCount)` (deprecated internal): as `span`, setting
    /// `out_count` to the number of code points and strings spanned.
    pub fn span_and_count(&self, s: &[u16], mut start: i32, span_condition: SpanCondition, out_count: &mut OutputInt) -> i32 {
        let end = s.len() as i32;
        if start < 0 {
            start = 0;
        } else if start >= end {
            return end;
        }
        if let Some(ss) = &self.string_span {
            // We might also have bmpSet != null,
            // but fully-contained strings are relevant for counting elements.
            return ss.span_and_count(s, start, span_condition, out_count);
        } else if let Some(b) = &self.bmp_set {
            return b.span(s, start, span_condition, Some(out_count));
        } else if self.has_strings() {
            let mut which = if span_condition == SpanCondition::NotContained {
                usss::FWD_UTF16_NOT_CONTAINED
            } else {
                usss::FWD_UTF16_CONTAINED
            };
            which |= usss::WITH_COUNT;
            let str_span = UnicodeSetStringSpan::new(self, self.strings.iter().cloned().collect(), which);
            return str_span.span_and_count(s, start, span_condition, out_count);
        }

        self.span_code_points_and_count(s, start, span_condition, Some(out_count))
    }

    fn span_code_points_and_count(&self, s: &[u16], start: i32, span_condition: SpanCondition, out_count: Option<&mut OutputInt>) -> i32 {
        // Pin to 0/1 values.
        let span_contained = span_condition != SpanCondition::NotContained;

        let mut next = start;
        let length = s.len() as i32;
        let mut count = 0;
        loop {
            let c = code_point_at(s, next);
            if span_contained != self.contains_valid(c) {
                break;
            }
            count += 1;
            next += char_count(c);
            if next >= length {
                break;
            }
        }
        if let Some(oc) = out_count {
            oc.value = count;
        }
        next
    }

    /// `spanBack(CharSequence s, SpanCondition spanCondition)`.
    pub fn span_back_from_end(&self, s: &[u16], span_condition: SpanCondition) -> i32 {
        self.span_back(s, s.len() as i32, span_condition)
    }

    /// `spanBack(CharSequence s, int fromIndex, SpanCondition
    /// spanCondition)`: the start of the span of `s` that ends at
    /// `from_index`.
    pub fn span_back(&self, s: &[u16], mut from_index: i32, span_condition: SpanCondition) -> i32 {
        if from_index <= 0 {
            return 0;
        }
        if from_index > s.len() as i32 {
            from_index = s.len() as i32;
        }
        if let Some(b) = &self.bmp_set {
            // Frozen set without strings, or no string is relevant for spanBack().
            return b.span_back(s, from_index, span_condition);
        }
        if let Some(ss) = &self.string_span {
            return ss.span_back(s, from_index, span_condition);
        } else if self.has_strings() {
            let which = if span_condition == SpanCondition::NotContained {
                usss::BACK_UTF16_NOT_CONTAINED
            } else {
                usss::BACK_UTF16_CONTAINED
            };
            let str_span = UnicodeSetStringSpan::new(self, self.strings.iter().cloned().collect(), which);
            if str_span.needs_string_span_utf16() {
                return str_span.span_back(s, from_index, span_condition);
            }
        }

        // Pin to 0/1 values.
        let span_contained = span_condition != SpanCondition::NotContained;

        let mut prev = from_index;
        loop {
            let c = code_point_before(s, prev);
            if span_contained != self.contains_valid(c) {
                break;
            }
            prev -= char_count(c);
            if prev <= 0 {
                break;
            }
        }
        prev
    }

    /// `cloneAsThawed()`: a mutable copy.
    pub fn clone_as_thawed(&self) -> UnicodeSet {
        let result = UnicodeSet::from_set(self);
        debug_assert!(!result.is_frozen());
        result
    }

    // internal function
    fn check_frozen(&self) -> Result<()> {
        if self.is_frozen() {
            return Err(LuceneError::unsupported_operation("Attempt to modify frozen object"));
        }
        Ok(())
    }

    /// `ranges()`: the code point ranges.
    pub fn ranges(&self) -> EntryRangeIterator<'_> {
        EntryRangeIterator { set: self, pos: 0 }
    }

    /// `iterator()`: every element as a string, code points first, then
    /// strings.
    pub fn iter(&self) -> UnicodeSetIterator2<'_> {
        UnicodeSetIterator2::new(self)
    }

    /// `containsAll(Iterable<T> collection)`.
    pub fn contains_all_iter<I, S>(&self, collection: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u16]>,
    {
        for o in collection {
            if !self.contains_str(o.as_ref()) {
                return false;
            }
        }
        true
    }

    /// `containsNone(Iterable<T> collection)`.
    pub fn contains_none_iter<I, S>(&self, collection: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u16]>,
    {
        for o in collection {
            if self.contains_str(o.as_ref()) {
                return false;
            }
        }
        true
    }

    /// `containsSome(Iterable<T> collection)`.
    pub fn contains_some_iter<I, S>(&self, collection: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u16]>,
    {
        !self.contains_none_iter(collection)
    }

    /// `addAll(T... collection)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn add_all_items<S: AsRef<[u16]>>(&mut self, collection: &[S]) -> Result<&mut Self> {
        self.check_frozen()?;
        for s in collection {
            self.add_str(s.as_ref())?;
        }
        Ok(self)
    }

    /// `removeAll(Iterable<T> collection)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn remove_all_iter<I, S>(&mut self, collection: I) -> Result<&mut Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u16]>,
    {
        self.check_frozen()?;
        for o in collection {
            self.remove_str(o.as_ref())?;
        }
        Ok(self)
    }

    /// `retainAll(Iterable<T> collection)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn retain_all_iter<I, S>(&mut self, collection: I) -> Result<&mut Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u16]>,
    {
        self.check_frozen()?;
        // TODO optimize
        let mut to_retain = UnicodeSet::new();
        to_retain.add_all_iter(collection)?;
        self.retain_all(&to_retain)?;
        Ok(self)
    }

    /// `compareTo(UnicodeSet o)`: `SHORTER_FIRST`.
    pub fn compare_to(&self, o: &UnicodeSet) -> i32 {
        self.compare_to_style(o, ComparisonStyle::ShorterFirst)
    }

    /// `compareTo(UnicodeSet o, ComparisonStyle style)`.
    pub fn compare_to_style(&self, o: &UnicodeSet, style: ComparisonStyle) -> i32 {
        if style != ComparisonStyle::Lexicographic {
            let diff = self.size() - o.size();
            if diff != 0 {
                return if (diff < 0) == (style == ComparisonStyle::ShorterFirst) { -1 } else { 1 };
            }
        }
        let mut i = 0usize;
        loop {
            let result = self.list[i] - o.list[i];
            if 0 != result {
                // if either list ran out, compare to the last string
                if self.list[i] == HIGH {
                    if !self.has_strings() {
                        return 1;
                    }
                    let item = self.strings.first().expect("has strings");
                    return Self::compare_str_cp(item, o.list[i]);
                }
                if o.list[i] == HIGH {
                    if !o.has_strings() {
                        return -1;
                    }
                    let item = o.strings.first().expect("has strings");
                    let compare_result = Self::compare_str_cp(item, self.list[i]);
                    return if compare_result > 0 { -1 } else if compare_result < 0 { 1 } else { 0 }; // Reverse the order.
                }
                // otherwise return the result if even index, or the reversal if not
                return if (i & 1) == 0 { result } else { -result };
            }
            if self.list[i] == HIGH {
                break;
            }
            i += 1;
        }
        Self::compare_iters(self.strings.iter(), o.strings.iter())
    }

    /// `compareTo(Iterable<String> other)`.
    pub fn compare_to_iter<'o, I>(&self, other: I) -> i32
    where
        I: IntoIterator<Item = &'o Vec<u16>>,
    {
        let mine: Vec<Vec<u16>> = self.iter().collect();
        let others: Vec<&Vec<u16>> = other.into_iter().collect();
        Self::compare_iters(mine.iter(), others.iter().copied())
    }

    /// `compare(CharSequence string, int codePoint)`: as if the code point
    /// were a string.
    ///
    /// # Panics
    ///
    /// Java's `IllegalArgumentException` for an invalid code point surfaces
    /// as a panic here: the set's own callers only pass code points.
    pub fn compare_str_cp(string: &[u16], code_point: i32) -> i32 {
        CharSequences::compare_cp(string, code_point).expect("java.lang.IllegalArgumentException")
    }

    /// `compare(int codePoint, CharSequence string)`.
    pub fn compare_cp_str(code_point: i32, string: &[u16]) -> i32 {
        -Self::compare_str_cp(string, code_point)
    }

    /// `compare(Iterable<T> collection1, Iterable<T> collection2)` and
    /// `compare(Iterator<T> first, Iterator<T> other)`.
    pub fn compare_iters<T, A, B>(first: A, other: B) -> i32
    where
        T: Ord,
        A: IntoIterator<Item = T>,
        B: IntoIterator<Item = T>,
    {
        let mut first = first.into_iter();
        let mut other = other.into_iter();
        loop {
            match (first.next(), other.next()) {
                (None, None) => return 0,
                (None, Some(_)) => return -1,
                (Some(_), None) => return 1,
                (Some(item1), Some(item2)) => {
                    let result = item1.cmp(&item2);
                    if !result.is_eq() {
                        return if result.is_lt() { -1 } else { 1 };
                    }
                }
            }
        }
    }

    /// `compare(Collection<T> collection1, Collection<T> collection2,
    /// ComparisonStyle style)`.
    pub fn compare_collections<T: Ord>(collection1: &[T], collection2: &[T], style: ComparisonStyle) -> i32 {
        if style != ComparisonStyle::Lexicographic {
            let diff = collection1.len() as i32 - collection2.len() as i32;
            if diff != 0 {
                return if (diff < 0) == (style == ComparisonStyle::ShorterFirst) { -1 } else { 1 };
            }
        }
        Self::compare_iters(collection1.iter(), collection2.iter())
    }

    /// `strings()`: the strings, in order.
    pub fn strings(&self) -> std::collections::btree_set::Iter<'_, Vec<u16>> {
        self.strings.iter()
    }

    /// `getSingleCodePoint(CharSequence s)` (deprecated internal).
    pub fn get_single_code_point(s: &[u16]) -> i32 {
        CharSequences::get_single_code_point(s)
    }

    /// `addBridges(UnicodeSet dontCare)` (deprecated internal): adds the
    /// ranges between ranges of this set that `dont_care` fully contains.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when frozen.
    pub fn add_bridges(&mut self, dont_care: &UnicodeSet) -> Result<&mut Self> {
        let mut not_in_input = UnicodeSet::from_set(self);
        not_in_input.complement_unchecked();
        not_in_input.strings.clear();
        let mut it = UnicodeSetIterator::new(&not_in_input);
        while it.next_range() {
            if it.codepoint != 0
                && it.codepoint_end != 0x10FFFF
                && dont_care.contains_range(it.codepoint, it.codepoint_end).unwrap_or(false)
            {
                self.add_range(it.codepoint, it.codepoint_end)?;
            }
        }
        Ok(self)
    }

    /// `findIn(CharSequence value, int fromIndex, boolean findNot)`
    /// (deprecated internal).
    pub fn find_in(&self, value: &[u16], mut from_index: i32, find_not: bool) -> i32 {
        //TODO add strings, optimize, using ICU4C algorithms
        while from_index < value.len() as i32 {
            let cp = UTF16::char_at(value, from_index);
            if self.contains_valid(cp) != find_not {
                break;
            }
            from_index += UTF16::get_char_count(cp);
        }
        from_index
    }

    /// `findLastIn(CharSequence value, int fromIndex, boolean findNot)`
    /// (deprecated internal).
    pub fn find_last_in(&self, value: &[u16], mut from_index: i32, find_not: bool) -> i32 {
        //TODO add strings, optimize, using ICU4C algorithms
        from_index -= 1;
        while from_index >= 0 {
            let cp = UTF16::char_at(value, from_index);
            if self.contains_valid(cp) != find_not {
                break;
            }
            from_index -= UTF16::get_char_count(cp);
        }
        if from_index < 0 {
            -1
        } else {
            from_index
        }
    }

    /// `stripFrom(CharSequence source, boolean matches)` (deprecated
    /// internal).
    pub fn strip_from(&self, source: &[u16], matches: bool) -> Vec<u16> {
        let mut result = Vec::new();
        let mut pos = 0;
        while pos < source.len() as i32 {
            let inside = self.find_in(source, pos, !matches);
            result.extend_from_slice(&source[pos as usize..inside as usize]);
            pos = self.find_in(source, inside, matches); // get next start
        }
        result
    }

    /// `getDefaultXSymbolTable()` (internal).
    pub fn get_default_x_symbol_table() -> Option<Arc<dyn XSymbolTable>> {
        XSYMBOL_TABLE.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// `setDefaultXSymbolTable(XSymbolTable xSymbolTable)` (internal).
    pub fn set_default_x_symbol_table(x_symbol_table: Option<Arc<dyn XSymbolTable>>) {
        // If the properties override inclusions, these have to be regenerated.
        // TODO: Check if the Unicode Tools or Unicode Utilities really need this.
        CharacterPropertiesImpl::clear();
        *XSYMBOL_TABLE.write().unwrap_or_else(|e| e.into_inner()) = x_symbol_table;
    }

    /// `rangeStream()`.
    pub fn range_stream(&self) -> EntryRangeIterator<'_> {
        self.ranges()
    }

    /// `stringStream()`.
    pub fn string_stream(&self) -> std::collections::btree_set::Iter<'_, Vec<u16>> {
        self.strings()
    }

    /// `codePointStream()`.
    pub fn code_point_stream(&self) -> CodePointSpliterator<'_> {
        CodePointSpliterator::new(self)
    }

    /// `stream()`.
    pub fn stream(&self) -> UnicodeSetIterator2<'_> {
        self.iter()
    }

    /// `codePoints()`.
    pub fn code_points(&self) -> CodePointIteratorInt<'_> {
        CodePointIteratorInt::new(self)
    }
}

/// `range(int start, int end)` (private): `[start, end + 1, HIGH]`
/// (Java caches the array in `rangeList`).
fn range(start: i32, end: i32) -> [i32; 3] {
    [start, end + 1, HIGH]
}

/// `String.regionMatches([ignoreCase,] toffset, other, 0, len)`.
fn region_matches(s: &[u16], toffset: i32, other: &[u16], ignore_case: bool) -> bool {
    if toffset < 0 || toffset as usize + other.len() > s.len() {
        return false;
    }
    for (i, &o) in other.iter().enumerate() {
        let c = s[toffset as usize + i];
        if c == o {
            continue;
        }
        if ignore_case {
            use rucene_analysis_common::jdk::character::{to_lower_case_unit, to_upper_case_unit};
            let u1 = to_upper_case_unit(c);
            let u2 = to_upper_case_unit(o);
            if u1 == u2 || to_lower_case_unit(u1) == to_lower_case_unit(u2) {
                continue;
            }
        }
        return false;
    }
    true
}

const LAST0_START: i32 = 0;
const LAST1_RANGE: i32 = 1;
const LAST2_SET: i32 = 2;

const MODE0_NONE: i32 = 0;
const MODE1_INBRACKET: i32 = 1;
const MODE2_OUTBRACKET: i32 = 2;

const SETMODE0_NONE: i32 = 0;
const SETMODE1_UNICODESET: i32 = 1;
const SETMODE2_PROPERTYPAT: i32 = 2;
const SETMODE3_PREPARSED: i32 = 3;

const MAX_DEPTH: i32 = 100;

/// The filters of the property set API (`UnicodeSet.Filter` and its
/// private implementations).
enum Filter {
    /// `NumericValueFilter`.
    NumericValue(f64),
    /// `GeneralCategoryMaskFilter`.
    GeneralCategoryMask(i32),
    /// `IntPropertyFilter`.
    IntProperty(i32, i32),
    /// `ScriptExtensionsFilter`.
    ScriptExtensions(i32),
    /// `IdentifierTypeFilter`.
    IdentifierType(i32),
    /// `VersionFilter`.
    Version(VersionInfo),
}

/// `NO_VERSION`: VersionInfo for unassigned characters.
static NO_VERSION: LazyLock<VersionInfo> =
    LazyLock::new(|| VersionInfo::get_instance4(0, 0, 0, 0).expect("0.0.0.0 is a valid version"));

impl Filter {
    fn contains(&self, ch: i32) -> bool {
        match self {
            Filter::NumericValue(value) => UCharacter::get_unicode_numeric_value(ch) == *value,
            Filter::GeneralCategoryMask(mask) => ((1 << UCharacter::get_type(ch)) & mask) != 0,
            Filter::IntProperty(prop, value) => UCharacter::get_int_property_value(ch, *prop) == *value,
            Filter::ScriptExtensions(script) => UScript::has_script(ch, *script),
            Filter::IdentifierType(id_type) => UCharacterProperty::instance().has_id_type(ch, *id_type),
            Filter::Version(version) => {
                let v = UCharacter::get_age(ch).expect("inclusions hold code points");
                // Reference comparison ok; VersionInfo caches and reuses
                // unique objects.
                v != *NO_VERSION && v.compare_to(version) <= 0
            }
        }
    }
}

/// An extended symbol table: overrides property resolution.
///
/// Equivalent to the abstract class `UnicodeSet.XSymbolTable`, whose
/// methods all have defaults (`lookupMatcher`, `lookup` and
/// `parseReference` answer `null`, `applyPropertyAlias` `false`): an
/// implementation also implements [`SymbolTable`] (with those defaults
/// where it does not override them) and answers itself from
/// [`SymbolTable::as_x_symbol_table`].
pub trait XSymbolTable: SymbolTable {
    /// `applyPropertyAlias(String propertyName, String propertyValue,
    /// UnicodeSet result)`: fills `result` and answers true when the
    /// table handles the property.
    ///
    /// # Errors
    ///
    /// The errors of the implementation.
    fn apply_property_alias(&self, _property_name: &[u16], _property_value: &[u16], _result: &mut UnicodeSet) -> Result<bool> {
        Ok(false)
    }
}

/// `SpanCondition`: the condition of a span over a set.
///
/// Equivalent to the enum `UnicodeSet.SpanCondition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanCondition {
    /// `NOT_CONTAINED`: continues while the set does not contain the text.
    NotContained,
    /// `CONTAINED`: continues while the set contains the text.
    Contained,
    /// `SIMPLE`: longest match of strings.
    Simple,
    /// `CONDITION_COUNT`.
    ConditionCount,
}

impl SpanCondition {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }
}

/// `ComparisonStyle`: how `compareTo` orders sets.
///
/// Equivalent to the enum `UnicodeSet.ComparisonStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComparisonStyle {
    /// `SHORTER_FIRST`.
    ShorterFirst,
    /// `LEXICOGRAPHIC`.
    Lexicographic,
    /// `LONGER_FIRST`.
    LongerFirst,
}

/// A code point range of a set.
///
/// Equivalent to `UnicodeSet.EntryRange` (Java reuses one object per
/// iterator; here each step returns a value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntryRange {
    /// `codepoint`: the first code point.
    pub codepoint: i32,
    /// `codepointEnd`: the last code point.
    pub codepoint_end: i32,
}

impl EntryRange {
    /// `toString()`.
    pub fn to_units(&self) -> Vec<u16> {
        let mut b = Vec::new();
        if self.codepoint == self.codepoint_end {
            append_to_pat(&mut b, self.codepoint, false);
        } else {
            append_to_pat(&mut b, self.codepoint, false);
            b.push(b'-' as u16);
            append_to_pat(&mut b, self.codepoint_end, false);
        }
        b
    }
}

/// `UnicodeSet.EntryRangeIterator` (private).
#[derive(Debug)]
pub struct EntryRangeIterator<'a> {
    set: &'a UnicodeSet,
    pos: i32,
}

impl Iterator for EntryRangeIterator<'_> {
    type Item = EntryRange;

    fn next(&mut self) -> Option<EntryRange> {
        if self.pos < self.set.len - 1 {
            let r = EntryRange {
                codepoint: self.set.list[self.pos as usize],
                codepoint_end: self.set.list[self.pos as usize + 1] - 1,
            };
            self.pos += 2;
            Some(r)
        } else {
            None
        }
    }
}

/// Iterates over the elements of a set as strings: code points first, then
/// the strings.
///
/// Equivalent to `UnicodeSet.UnicodeSetIterator2` (private).
#[derive(Debug)]
pub struct UnicodeSetIterator2<'a> {
    source_list: Option<&'a [i32]>,
    len: i32,
    item: i32,
    current: i32,
    limit: i32,
    source_strings: &'a BTreeSet<Vec<u16>>,
    string_iterator: Option<std::collections::btree_set::Iter<'a, Vec<u16>>>,
}

impl<'a> UnicodeSetIterator2<'a> {
    fn new(source: &'a UnicodeSet) -> Self {
        // set according to invariants
        let len = source.len - 1;
        let mut it = Self {
            source_list: None,
            len,
            item: 0,
            current: 0,
            limit: 0,
            source_strings: &source.strings,
            string_iterator: None,
        };
        if len > 0 {
            it.source_list = Some(&source.list);
            it.current = source.list[0];
            it.limit = source.list[1];
            it.item = 2;
        } else {
            it.string_iterator = Some(source.strings.iter());
        }
        it
    }
}

impl Iterator for UnicodeSetIterator2<'_> {
    type Item = Vec<u16>;

    fn next(&mut self) -> Option<Vec<u16>> {
        let Some(source_list) = self.source_list else {
            return self.string_iterator.as_mut().and_then(|i| i.next()).cloned();
        };
        let codepoint = self.current;
        self.current += 1;
        // we have the codepoint we need, but we may need to adjust the state
        if self.current >= self.limit {
            if self.item >= self.len {
                self.string_iterator = Some(self.source_strings.iter());
                self.source_list = None;
            } else {
                self.current = source_list[self.item as usize];
                self.limit = source_list[self.item as usize + 1];
                self.item += 2;
            }
        }
        // Now return. Single code point is easy
        if codepoint <= 0xFFFF {
            return Some(vec![codepoint as u16]);
        }
        // But Java lacks a valueOfCodePoint method, so we have to work harder
        let offset = codepoint - 0x10000;
        Some(vec![(((offset as u32) >> 10) + 0xD800) as u16, ((offset & 0x3ff) + 0xDC00) as u16])
    }
}

/// `UnicodeSet.CodePointIteratorInt` (private): the code points of a set.
#[derive(Debug, Clone)]
pub struct CodePointIteratorInt<'a> {
    list: &'a [i32],
    last_range: i32,
    current_range: i32,
    range_start: i32,
    range_limit: i32,
}

impl<'a> CodePointIteratorInt<'a> {
    fn new(unicode_set: &'a UnicodeSet) -> Self {
        let list = &unicode_set.list[..];
        let last_range = unicode_set.len - 1;
        let mut current_range = 0;
        let range_start = list[current_range as usize];
        current_range += 1;
        let range_limit = if last_range > 0 {
            let l = list[current_range as usize];
            current_range += 1;
            l
        } else {
            range_start
        };
        Self { list, last_range, current_range, range_start, range_limit }
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        self.range_start < self.range_limit || self.current_range < self.last_range
    }
}

impl Iterator for CodePointIteratorInt<'_> {
    type Item = i32;

    fn next(&mut self) -> Option<i32> {
        if self.range_start >= self.range_limit {
            if self.current_range >= self.last_range {
                return None;
            }
            self.range_start = self.list[self.current_range as usize];
            self.current_range += 1;
            self.range_limit = self.list[self.current_range as usize];
            self.current_range += 1;
        }
        let r = self.range_start;
        self.range_start += 1;
        Some(r)
    }
}

/// `UnicodeSet.CodePointSpliterator` (private): the `Spliterator.OfInt`
/// behind `codePointStream()`.
#[derive(Debug, Clone)]
pub struct CodePointSpliterator<'a> {
    unicode_set: &'a UnicodeSet,
    cpi: CodePointIteratorInt<'a>,
}

impl<'a> CodePointSpliterator<'a> {
    /// `Spliterator.SIZED | ORDERED | DISTINCT | NONNULL`.
    const CHARACTERISTICS: i32 = 0x40 | 0x10 | 0x1 | 0x100;

    fn new(unicode_set: &'a UnicodeSet) -> Self {
        Self { unicode_set, cpi: CodePointIteratorInt::new(unicode_set) }
    }

    /// `estimateSize()`.
    pub fn estimate_size(&self) -> i64 {
        (self.unicode_set.size() - self.unicode_set.strings.len() as i32) as i64
    }

    /// `characteristics()` (`IMMUTABLE` added for a frozen set).
    pub fn characteristics(&self) -> i32 {
        if self.unicode_set.is_frozen() {
            0x400 | Self::CHARACTERISTICS
        } else {
            Self::CHARACTERISTICS
        }
    }

    /// `trySplit()`: never splits.
    pub fn try_split(&mut self) -> Option<Self> {
        None
    }

    /// `tryAdvance(IntConsumer action)`.
    pub fn try_advance(&mut self, mut action: impl FnMut(i32)) -> bool {
        match self.cpi.next() {
            Some(cp) => {
                action(cp);
                true
            }
            None => false,
        }
    }
}

impl Iterator for CodePointSpliterator<'_> {
    type Item = i32;

    fn next(&mut self) -> Option<i32> {
        self.cpi.next()
    }
}

impl PartialEq for UnicodeSet {
    /// `equals(Object o)`.
    fn eq(&self, that: &Self) -> bool {
        if std::ptr::eq(self, that) {
            return true;
        }
        if self.len != that.len {
            return false;
        }
        for i in 0..self.len as usize {
            if self.list[i] != that.list[i] {
                return false;
            }
        }
        self.strings == that.strings
    }
}

impl Eq for UnicodeSet {}

impl std::hash::Hash for UnicodeSet {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_i32(self.hash_code());
    }
}

impl fmt::Display for UnicodeSet {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf16_lossy(&self.to_units()))
    }
}

impl<'a> IntoIterator for &'a UnicodeSet {
    type Item = Vec<u16>;
    type IntoIter = UnicodeSetIterator2<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl UnicodeMatcher for UnicodeSet {
    /// `matches(Replaceable text, int[] offset, int limit, boolean
    /// incremental)`.
    fn matches(&self, text: &dyn Replaceable, offset: &mut [i32], limit: i32, incremental: bool) -> Result<i32> {
        if offset[0] == limit {
            // Strings, if any, have length != 0, so we don't worry
            // about them here.  If we ever allow zero-length strings
            // we much check for them here.
            if self.contains_valid(ETHER as i32) {
                Ok(if incremental { U_PARTIAL_MATCH } else { U_MATCH })
            } else {
                Ok(U_MISMATCH)
            }
        } else {
            if self.has_strings() {
                // try strings first

                // might separate forward and backward loops later
                // for now they are combined

                // TODO Improve efficiency of this, at least in the forward
                // direction, if not in both.  In the forward direction we
                // can assume the strings are sorted.

                let forward = offset[0] < limit;

                // firstChar is the leftmost char to match in the
                // forward direction or the rightmost char to match in
                // the reverse direction.
                let first_char = text.char_at(offset[0]);

                // If there are multiple strings that can match we
                // return the longest match.
                let mut high_water_length = 0;

                for trial in &self.strings {
                    if trial.is_empty() {
                        continue; // skip the empty string
                    }

                    let c = trial[if forward { 0 } else { trial.len() - 1 }];

                    // Strings are sorted, so we can optimize in the
                    // forward direction.
                    if forward && c > first_char {
                        break;
                    }
                    if c != first_char {
                        continue;
                    }

                    let length = Self::match_rest(text, offset[0], limit, trial);

                    if incremental {
                        let max_len = if forward { limit - offset[0] } else { offset[0] - limit };
                        if length == max_len {
                            // We have successfully matched but only up to limit.
                            return Ok(U_PARTIAL_MATCH);
                        }
                    }

                    if length == trial.len() as i32 {
                        // We have successfully matched the whole string.
                        if length > high_water_length {
                            high_water_length = length;
                        }
                        // In the forward direction we know strings
                        // are sorted so we can bail early.
                        if forward && length < high_water_length {
                            break;
                        }
                        continue;
                    }
                }

                // We've checked all strings without a partial match.
                // If we have full matches, return the longest one.
                if high_water_length != 0 {
                    offset[0] += if forward { high_water_length } else { -high_water_length };
                    return Ok(U_MATCH);
                }
            }
            self.filter_matches(text, offset, limit, incremental)
        }
    }

    fn to_pattern(&self, escape_unprintable: bool) -> Vec<u16> {
        UnicodeSet::to_pattern(self, escape_unprintable)
    }

    /// `matchesIndexValue(int v)`: implementation of UnicodeMatcher API.
    /// Returns true if this set contains any character whose low byte is
    /// the given value.
    fn matches_index_value(&self, v: i32) -> bool {
        /* The index value v, in the range [0,255], is contained in this set if
         * it is contained in any pair of this set.  Pairs either have the high
         * bytes equal, or unequal.  If the high bytes are equal, then we have
         * aaxx..aayy, where aa is the high byte.  Then v is contained if xx <=
         * v <= yy.  If the high bytes are unequal we have aaxx..bbyy, bb>aa.
         * Then v is contained if xx <= v || v <= yy.  (This is identical to the
         * time zone month containment logic.)
         */
        for i in 0..self.get_range_count() {
            let low = self.get_range_start(i);
            let high = self.get_range_end(i);
            if (low & !0xFF) == (high & !0xFF) {
                if (low & 0xFF) <= v && v <= (high & 0xFF) {
                    return true;
                }
            } else if (low & 0xFF) <= v || v <= (high & 0xFF) {
                return true;
            }
        }
        if self.has_strings() {
            for s in &self.strings {
                if s.is_empty() {
                    continue; // skip the empty string
                }
                let c = UTF16::char_at(s, 0);
                if (c & 0xFF) == v {
                    return true;
                }
            }
        }
        false
    }

    /// `addMatchSetTo(UnicodeSet toUnionTo)`.
    fn add_match_set_to(&self, to_union_to: &mut UnicodeSet) -> Result<()> {
        to_union_to.add_all(self)?;
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl UnicodeFilter for UnicodeSet {
    fn contains(&self, c: i32) -> Result<bool> {
        UnicodeSet::contains(self, c)
    }
}

impl Freezable<UnicodeSet> for UnicodeSet {
    fn is_frozen(&self) -> bool {
        UnicodeSet::is_frozen(self)
    }

    fn freeze(&mut self) -> &mut UnicodeSet {
        UnicodeSet::freeze(self)
    }

    fn clone_as_thawed(&self) -> UnicodeSet {
        UnicodeSet::clone_as_thawed(self)
    }
}
