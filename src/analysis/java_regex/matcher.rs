//! `java.util.regex.Matcher` over a [`JavaPattern`](super::JavaPattern).
//!
//! The analysis classes that run a `Pattern` over text — `PatternTokenizer`,
//! `PatternReplaceFilter`, `PatternReplaceCharFilter`,
//! `PatternCaptureGroupTokenFilter`, `PatternTypingFilter` of
//! `lucene/analysis/common` — drive a `Matcher` step by step: `find()`,
//! `start(int)`/`end(int)` in UTF-16 units, `appendReplacement` and
//! `appendTail` with Java's replacement syntax, `replaceAll` and
//! `replaceFirst`. This module reproduces those methods from the JDK 25
//! `Matcher` specification (`java/util/regex/Matcher.java`, read as
//! specification, not copied): the matcher's state is Java's `first`, `last`,
//! `groups` and `lastAppendPosition`, and each method updates it as the JDK's
//! does. The region is always the whole input, with anchoring and opaque
//! bounds, the defaults a matcher has after `reset`.

use std::fmt;

use crate::error::{LuceneError, Result};

use super::JavaPattern;

/// The class of the exceptions `start`, `end` and `group` throw for a group
/// the pattern does not have.
const INDEX_OUT_OF_BOUNDS: &str = "java.lang.IndexOutOfBoundsException";

/// A matcher of a [`JavaPattern`] over a character sequence.
///
/// Equivalent to `java.util.regex.Matcher`, for the methods the analysis
/// classes call. Indices are UTF-16 indices into the input, as Java's are.
///
/// **Divergences from JDK 25**, beyond those of the compiled pattern
/// (declared on
/// [`AbstractAnalysisFactory::get_pattern`](crate::analysis::factories::AbstractAnalysisFactory::get_pattern)):
///
/// * **Unpaired surrogates.** The engine runs over a Rust string, which
///   cannot hold an unpaired surrogate: each one is searched as U+FFFD, one
///   UTF-16 unit as the surrogate is, so indices are unchanged. A pattern
///   that matches an unpaired surrogate in Java (`.`, `[^a]`, `\uD800`) sees
///   U+FFFD here instead; the text a method copies out of the input
///   (`appendReplacement`, `appendTail`, `group`) is the input's own units.
/// * **A search that starts inside a surrogate pair.** Java's `find()`
///   resumes one unit after an empty match, which is the low half of a pair
///   when the match was just before a supplementary character, and can
///   match there. A Rust string has no such position: the search starts at
///   the next character, as Java does for a pattern holding a supplementary
///   character (its `StartS` node skips the low half).
pub struct JavaMatcher {
    pattern: JavaPattern,
    /// The input, Java's `text`.
    text: Vec<u16>,
    /// The input as the engine sees it, unpaired surrogates as U+FFFD.
    haystack: String,
    /// For each UTF-16 index of the input, and its length, the byte offset
    /// in `haystack` that starts it; the low half of a surrogate pair maps
    /// one byte into its character, so the offsets are strictly increasing
    /// and a character boundary is never such a position.
    byte_of_unit: Vec<usize>,
    /// The bounds of the groups of the last match, `-1` when unset, Java's
    /// `groups`.
    groups: Vec<i32>,
    /// The start of the last match, `-1` when there is none, Java's `first`.
    first: i32,
    /// The end of the last match, Java's `last`.
    last: i32,
    /// Java's `lastAppendPosition`.
    last_append_position: i32,
}

impl fmt::Debug for JavaMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `Matcher.toString()`.
        write!(
            f,
            "java.util.regex.Matcher[pattern={} region=0,{} lastmatch=]",
            self.pattern.pattern(),
            self.text.len()
        )
    }
}

impl JavaMatcher {
    /// Equivalent to `Matcher(Pattern, CharSequence)`, which
    /// `Pattern.matcher(CharSequence)` calls: a matcher over `input`, reset.
    pub(crate) fn new(pattern: JavaPattern, input: &[u16]) -> Self {
        let group_count = pattern.group_count();
        let mut matcher = Self {
            pattern,
            text: Vec::new(),
            haystack: String::new(),
            byte_of_unit: Vec::new(),
            groups: vec![-1; (group_count + 1) * 2],
            first: -1,
            last: 0,
            last_append_position: 0,
        };
        matcher.set_text(input);
        matcher.reset();
        matcher
    }

    /// Installs `input` as the text, with its engine view.
    fn set_text(&mut self, input: &[u16]) {
        self.text.clear();
        self.text.extend_from_slice(input);
        self.haystack.clear();
        self.byte_of_unit.clear();
        let mut i = 0;
        while i < input.len() {
            let unit = input[i];
            let byte = self.haystack.len();
            if (0xD800..0xDC00).contains(&unit)
                && i + 1 < input.len()
                && (0xDC00..0xE000).contains(&input[i + 1])
            {
                let code_point = 0x10000
                    + ((u32::from(unit) - 0xD800) << 10)
                    + (u32::from(input[i + 1]) - 0xDC00);
                self.haystack
                    .push(char::from_u32(code_point).expect("INVARIANT: a surrogate pair decodes"));
                self.byte_of_unit.push(byte);
                self.byte_of_unit.push(byte + 1);
                i += 2;
            } else {
                self.haystack
                    .push(char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER));
                self.byte_of_unit.push(byte);
                i += 1;
            }
        }
        self.byte_of_unit.push(self.haystack.len());
    }

    /// The UTF-16 index whose character starts at byte `byte` of the
    /// haystack (a match bound is always one).
    fn unit_of_byte(&self, byte: usize) -> i32 {
        self.byte_of_unit
            .binary_search(&byte)
            .expect("INVARIANT: a match bound is a character boundary") as i32
    }

    /// The pattern this matcher interprets, `Matcher.pattern()`.
    pub fn pattern(&self) -> &JavaPattern {
        &self.pattern
    }

    /// The input, as UTF-16 units.
    pub fn text(&self) -> &[u16] {
        &self.text
    }

    /// Resets this matcher: no match, the search starting at the beginning
    /// of the input.
    ///
    /// Equivalent to `Matcher.reset()`.
    pub fn reset(&mut self) -> &mut Self {
        self.first = -1;
        self.last = 0;
        self.groups.fill(-1);
        self.last_append_position = 0;
        self
    }

    /// Resets this matcher with a new input sequence.
    ///
    /// Equivalent to `Matcher.reset(CharSequence)`.
    pub fn reset_input(&mut self, input: &[u16]) -> &mut Self {
        self.set_text(input);
        self.reset()
    }

    /// The number of capturing groups in this matcher's pattern.
    ///
    /// Equivalent to `Matcher.groupCount()`.
    pub fn group_count(&self) -> usize {
        self.pattern.group_count()
    }

    /// Attempts to find the next subsequence of the input that matches the
    /// pattern: from the beginning of the input after a reset, else from the
    /// end of the previous match, one character further when that match was
    /// empty.
    ///
    /// Equivalent to `Matcher.find()`.
    pub fn find(&mut self) -> bool {
        let mut next_search_index = self.last;
        if next_search_index == self.first {
            next_search_index += 1;
        }
        // If next search starts beyond region then it fails
        if next_search_index > self.text.len() as i32 {
            self.groups.fill(-1);
            return false;
        }
        self.search(next_search_index)
    }

    /// Equivalent to the package-private `Matcher.search(int)`: an unanchored
    /// search from `from`.
    fn search(&mut self, from: i32) -> bool {
        let mut from = from.max(0) as usize;
        // See the type's documentation: a search does not start inside a
        // surrogate pair.
        while from < self.text.len() && !self.haystack.is_char_boundary(self.byte_of_unit[from]) {
            from += 1;
        }
        self.first = from as i32;
        self.groups.fill(-1);
        let captures = self
            .pattern
            .find_regex()
            .captures_from_pos(self.haystack.as_str(), self.byte_of_unit[from])
            .expect("INVARIANT: the engine has no backtracking limit");
        let Some(captures) = captures else {
            self.first = -1;
            return false;
        };
        let bounds: Vec<Option<(usize, usize)>> = (0..=self.group_count())
            .map(|group| captures.get(group).map(|m| (m.start(), m.end())))
            .collect();
        for (group, bound) in bounds.into_iter().enumerate() {
            if let Some((start, end)) = bound {
                self.groups[group * 2] = self.unit_of_byte(start);
                self.groups[group * 2 + 1] = self.unit_of_byte(end);
            }
        }
        self.first = self.groups[0];
        self.last = self.groups[1];
        true
    }

    /// `Matcher.hasMatch()`: whether the last match operation succeeded.
    pub fn has_match(&self) -> bool {
        self.first >= 0
    }

    /// The private `checkMatch()`.
    fn check_match(&self) -> Result<()> {
        if self.has_match() {
            Ok(())
        } else {
            Err(LuceneError::illegal_state("No match found"))
        }
    }

    /// The private `checkGroup(int)`.
    fn check_group(&self, group: i32) -> Result<()> {
        if group < 0 || group > self.group_count() as i32 {
            return Err(LuceneError::unchecked(
                INDEX_OUT_OF_BOUNDS,
                format!("No group {group}"),
            ));
        }
        Ok(())
    }

    /// The start index of the previous match, `Matcher.start()`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` "No match found" when there is no match.
    pub fn start(&self) -> Result<i32> {
        self.check_match()?;
        Ok(self.first)
    }

    /// The start index of the subsequence captured by `group` during the
    /// previous match, `-1` if the group did not participate.
    ///
    /// Equivalent to `Matcher.start(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` "No match found" when there is no match, and
    /// `IndexOutOfBoundsException` "No group N" for a group the pattern does
    /// not have.
    pub fn start_group(&self, group: i32) -> Result<i32> {
        self.check_match()?;
        self.check_group(group)?;
        Ok(self.groups[group as usize * 2])
    }

    /// The offset after the last character matched, `Matcher.end()`.
    ///
    /// # Errors
    ///
    /// As [`start`](Self::start).
    pub fn end(&self) -> Result<i32> {
        self.check_match()?;
        Ok(self.last)
    }

    /// The offset after the last character of the subsequence captured by
    /// `group` during the previous match, `-1` if the group did not
    /// participate.
    ///
    /// Equivalent to `Matcher.end(int)`.
    ///
    /// # Errors
    ///
    /// As [`start_group`](Self::start_group).
    pub fn end_group(&self, group: i32) -> Result<i32> {
        self.check_match()?;
        self.check_group(group)?;
        Ok(self.groups[group as usize * 2 + 1])
    }

    /// The input subsequence captured by `group` during the previous match,
    /// `None` if the group did not participate.
    ///
    /// Equivalent to `Matcher.group(int)`.
    ///
    /// # Errors
    ///
    /// As [`start_group`](Self::start_group).
    pub fn group(&self, group: i32) -> Result<Option<Vec<u16>>> {
        self.check_match()?;
        self.check_group(group)?;
        let (start, end) = (
            self.groups[group as usize * 2],
            self.groups[group as usize * 2 + 1],
        );
        if start == -1 || end == -1 {
            return Ok(None);
        }
        Ok(Some(self.text[start as usize..end as usize].to_vec()))
    }

    /// The start of the last match, for a caller that knows there is one.
    pub(crate) fn match_start(&self) -> usize {
        self.first as usize
    }

    /// The end of the last match, for a caller that knows there is one.
    pub(crate) fn match_end(&self) -> usize {
        self.last as usize
    }

    /// Implements a non-terminal append-and-replace step: appends the input
    /// from the append position up to the match, then `replacement` with its
    /// group references expanded, and moves the append position to the end
    /// of the match.
    ///
    /// Equivalent to `Matcher.appendReplacement(StringBuffer, String)` (and
    /// its `StringBuilder` twin): in `replacement`, `$g` names a group by
    /// number (the longest run of digits that is a group of the pattern) and
    /// `${name}` by name, and `\` escapes the next character.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` "No match found" when there is no match;
    /// `IllegalArgumentException` with Java's messages for a malformed
    /// replacement, after which `sb` is as it was; and
    /// `IndexOutOfBoundsException` "No group N" for a number the pattern has
    /// no group for, after which `sb` keeps what was appended, as Java's
    /// does.
    pub fn append_replacement(&mut self, sb: &mut Vec<u16>, replacement: &str) -> Result<()> {
        self.check_match()?;
        let cur_len = sb.len();
        // Append the intervening text
        sb.extend_from_slice(&self.text[self.last_append_position as usize..self.first as usize]);
        // Append the match substitution
        if let Err(error) = self.append_expanded_replacement(sb, replacement) {
            if error.is_illegal_argument_exception() {
                sb.truncate(cur_len);
            }
            return Err(error);
        }
        self.last_append_position = self.last;
        Ok(())
    }

    /// Equivalent to the private `Matcher.appendExpandedReplacement(Appendable,
    /// String)`. Java walks the replacement by UTF-16 unit and appends each
    /// unit it copies; walking it by character appends the same units, since
    /// no unit of a supplementary character is `\`, `$`, `{`, `}` or a
    /// digit.
    fn append_expanded_replacement(&self, app: &mut Vec<u16>, replacement: &str) -> Result<()> {
        let chars: Vec<char> = replacement.chars().collect();
        let mut buf = [0u16; 2];
        let mut cursor = 0;
        while cursor < chars.len() {
            let mut next_char = chars[cursor];
            if next_char == '\\' {
                cursor += 1;
                if cursor == chars.len() {
                    return Err(LuceneError::illegal_argument(
                        "character to be escaped is missing",
                    ));
                }
                next_char = chars[cursor];
                app.extend_from_slice(next_char.encode_utf16(&mut buf));
                cursor += 1;
            } else if next_char == '$' {
                // Skip past $
                cursor += 1;
                // Throw IAE if this "$" is the last character in replacement
                if cursor == chars.len() {
                    return Err(LuceneError::illegal_argument(
                        "Illegal group reference: group index is missing",
                    ));
                }
                next_char = chars[cursor];
                let ref_num: i32;
                if next_char == '{' {
                    cursor += 1;
                    let begin = cursor;
                    while cursor < chars.len() {
                        next_char = chars[cursor];
                        if next_char.is_ascii_lowercase()
                            || next_char.is_ascii_uppercase()
                            || next_char.is_ascii_digit()
                        {
                            cursor += 1;
                        } else {
                            break;
                        }
                    }
                    if begin == cursor {
                        return Err(LuceneError::illegal_argument(
                            "named capturing group has 0 length name",
                        ));
                    }
                    if next_char != '}' {
                        return Err(LuceneError::illegal_argument(
                            "named capturing group is missing trailing '}'",
                        ));
                    }
                    let gname: String = chars[begin..cursor].iter().collect();
                    if chars[begin].is_ascii_digit() {
                        return Err(LuceneError::illegal_argument(format!(
                            "capturing group name {{{gname}}} starts with digit character"
                        )));
                    }
                    let Some(&number) = self.pattern.named_groups().get(&gname) else {
                        return Err(LuceneError::illegal_argument(format!(
                            "No group with name {{{gname}}}"
                        )));
                    };
                    ref_num = number as i32;
                    cursor += 1;
                } else {
                    // The first number is always a group
                    let first_digit = next_char as i32 - '0' as i32;
                    if !(0..=9).contains(&first_digit) {
                        return Err(LuceneError::illegal_argument("Illegal group reference"));
                    }
                    let mut number = first_digit;
                    cursor += 1;
                    // Capture the largest legal group string
                    while cursor < chars.len() {
                        let next_digit = chars[cursor] as i32 - '0' as i32;
                        if !(0..=9).contains(&next_digit) {
                            // not a number
                            break;
                        }
                        let new_ref_num = number.wrapping_mul(10).wrapping_add(next_digit);
                        if (self.group_count() as i32) < new_ref_num {
                            break;
                        }
                        number = new_ref_num;
                        cursor += 1;
                    }
                    ref_num = number;
                }
                // Append group
                let (start, end) = (self.start_group(ref_num)?, self.end_group(ref_num)?);
                if start != -1 && end != -1 {
                    app.extend_from_slice(&self.text[start as usize..end as usize]);
                }
            } else {
                app.extend_from_slice(next_char.encode_utf16(&mut buf));
                cursor += 1;
            }
        }
        Ok(())
    }

    /// Implements a terminal append-and-replace step: appends the input from
    /// the append position to its end.
    ///
    /// Equivalent to `Matcher.appendTail(StringBuffer)`.
    pub fn append_tail(&self, sb: &mut Vec<u16>) {
        sb.extend_from_slice(&self.text[self.last_append_position as usize..]);
    }

    /// Replaces every subsequence of the input that matches the pattern with
    /// `replacement`, its group references expanded as
    /// [`append_replacement`](Self::append_replacement) does.
    ///
    /// Equivalent to `Matcher.replaceAll(String)`, which first resets this
    /// matcher.
    ///
    /// # Errors
    ///
    /// As [`append_replacement`](Self::append_replacement).
    pub fn replace_all(&mut self, replacement: &str) -> Result<Vec<u16>> {
        self.reset();
        let mut result = self.find();
        if result {
            let mut sb = Vec::new();
            loop {
                self.append_replacement(&mut sb, replacement)?;
                result = self.find();
                if !result {
                    break;
                }
            }
            self.append_tail(&mut sb);
            return Ok(sb);
        }
        Ok(self.text.clone())
    }

    /// Replaces the first subsequence of the input that matches the pattern
    /// with `replacement`, its group references expanded as
    /// [`append_replacement`](Self::append_replacement) does.
    ///
    /// Equivalent to `Matcher.replaceFirst(String)`, which first resets this
    /// matcher.
    ///
    /// # Errors
    ///
    /// As [`append_replacement`](Self::append_replacement).
    pub fn replace_first(&mut self, replacement: &str) -> Result<Vec<u16>> {
        self.reset();
        if !self.find() {
            return Ok(self.text.clone());
        }
        let mut sb = Vec::new();
        self.append_replacement(&mut sb, replacement)?;
        self.append_tail(&mut sb);
        Ok(sb)
    }
}
