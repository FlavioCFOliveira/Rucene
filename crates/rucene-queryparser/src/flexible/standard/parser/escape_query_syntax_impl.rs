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

//! Port of `EscapeQuerySyntaxImpl.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.parser`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use super::parse_exception::ParseException;
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::parser::escape_query_syntax::{EscapeQuerySyntax, Type};
use crate::flexible::core::util::unescaped_char_sequence::UnescapedCharSequence;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::locale::Locale;
use crate::jdk::object::JavaObject;
use crate::jdk::string::to_lower_case;

/// Implementation of [`EscapeQuerySyntax`] for the standard lucene syntax.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.parser.EscapeQuerySyntaxImpl`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EscapeQuerySyntaxImpl;

/// `wildcardChars`.
const WILDCARD_CHARS: [u16; 2] = [b'*' as u16, b'?' as u16];

/// `escapableTermExtraFirstChars`.
const ESCAPABLE_TERM_EXTRA_FIRST_CHARS: [&str; 3] = ["+", "-", "@"];

/// `escapableTermChars`.
const ESCAPABLE_TERM_CHARS: [&str; 15] = [
    "\"", "<", ">", "=", "!", "(", ")", "^", "[", "{", ":", "]", "}", "~", "/",
];

// TODO: check what to do with these "*", "?", "\\"
/// `escapableQuotedChars`.
const ESCAPABLE_QUOTED_CHARS: [&str; 1] = ["\""];

/// `escapableWhiteChars`.
const ESCAPABLE_WHITE_CHARS: [&str; 7] = [" ", "\t", "\n", "\r", "\u{c}", "\u{8}", "\u{3000}"];

/// `escapableWordTokens`.
const ESCAPABLE_WORD_TOKENS: [&str; 8] = [
    "AND",
    "OR",
    "NOT",
    "TO",
    "WITHIN",
    "SENTENCE",
    "PARAGRAPH",
    "INORDER",
];

/// The UTF-16 units of a string.
fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `String.indexOf(int ch, int fromIndex)` over UTF-16 units, `-1` when
/// absent.
fn index_of(s: &[u16], ch: u16, from_index: usize) -> Option<usize> {
    s.iter()
        .skip(from_index)
        .position(|&c| c == ch)
        .map(|i| i + from_index)
}

/// `StringBuilder.append(CharSequence s, int start, int end)`.
fn append_range(result: &mut Vec<u16>, s: &[u16], start: usize, end: usize) -> Result<()> {
    if start > end || end > s.len() {
        return Err(LuceneError::unchecked(
            "java.lang.IndexOutOfBoundsException",
            format!("start {start}, end {end}, length {}", s.len()),
        ));
    }
    result.extend_from_slice(&s[start..end]);
    Ok(())
}

impl EscapeQuerySyntaxImpl {
    /// Equivalent to `new EscapeQuerySyntaxImpl()`.
    pub fn new() -> Self {
        Self
    }

    /// Equivalent to the private static `escapeChar(CharSequence str, Locale
    /// locale)`.
    fn escape_char(str: Vec<u16>, locale: &Locale) -> Result<Vec<u16>> {
        if str.is_empty() {
            return Ok(str);
        }
        let mut buffer = str;
        // regular escapable char for terms
        for escapable_term_char in ESCAPABLE_TERM_CHARS {
            buffer = Self::escape_ignoring_case(
                buffer,
                &to_lower_case(&units(escapable_term_char), locale),
                &units("\\"),
                locale,
            )?;
        }
        // first char of a term as more escaping chars
        for escapable_term_extra_first_char in ESCAPABLE_TERM_EXTRA_FIRST_CHARS {
            if buffer.first() == units(escapable_term_extra_first_char).first() {
                let mut escaped = units("\\");
                escaped.extend_from_slice(&buffer);
                buffer = escaped;
                break;
            }
        }
        Ok(buffer)
    }

    /// Equivalent to the private static `escapeQuoted(CharSequence str,
    /// Locale locale)`.
    fn escape_quoted(str: Vec<u16>, locale: &Locale) -> Result<Vec<u16>> {
        if str.is_empty() {
            return Ok(str);
        }
        let mut buffer = str;
        for escapable_quoted_char in ESCAPABLE_QUOTED_CHARS {
            buffer = Self::escape_ignoring_case(
                buffer,
                &to_lower_case(&units(escapable_quoted_char), locale),
                &units("\\"),
                locale,
            )?;
        }
        Ok(buffer)
    }

    /// Equivalent to the private static `escapeTerm(CharSequence term,
    /// Locale locale)`.
    fn escape_term(term: Vec<u16>, locale: &Locale) -> Result<Vec<u16>> {
        if term.is_empty() {
            return Ok(term);
        }
        // escape single chars
        let term = Self::escape_char(term, locale)?;
        let term = Self::escape_white_char(term, locale)?;
        // escape parser words
        for escapable_word_token in ESCAPABLE_WORD_TOKENS {
            if crate::jdk::string::equals_ignore_case(
                escapable_word_token,
                &String::from_utf16_lossy(&term),
            ) {
                let mut escaped = units("\\");
                escaped.extend_from_slice(&term);
                return Ok(escaped);
            }
        }
        Ok(term)
    }

    /// prepend every case-insensitive occurrence of the `sequence1` in the
    /// `string` with the `escapeChar`. When the `sequence1` is empty, every
    /// character in the `string` is escaped.
    ///
    /// Equivalent to the private static `escapeIgnoringCase(CharSequence
    /// string, CharSequence sequence1, CharSequence escapeChar, Locale
    /// locale)`, over UTF-16 units: the occurrences are searched in the
    /// lower-cased string and copied from the original one at the same
    /// indexes.
    fn escape_ignoring_case(
        string: Vec<u16>,
        sequence1: &[u16],
        escape_char: &[u16],
        locale: &Locale,
    ) -> Result<Vec<u16>> {
        let count = string.len();
        let sequence1_length = sequence1.len();
        // empty search string - escape every character
        if sequence1_length == 0 {
            let mut result = Vec::with_capacity(count * (1 + escape_char.len()));
            for &c in &string {
                result.extend_from_slice(escape_char);
                result.push(c);
            }
            return Ok(result);
        }
        // normal case
        let lowercase = to_lower_case(&string, locale);
        let mut result: Vec<u16> = Vec::new();
        let first = sequence1[0];
        let mut start = 0usize;
        let mut copy_start = 0usize;
        while start < count {
            let Some(first_index) = index_of(&lowercase, first, start) else {
                break;
            };
            let mut found = true;
            if sequence1.len() > 1 {
                if first_index + sequence1_length > count {
                    break;
                }
                for (i, &expected) in sequence1.iter().enumerate().skip(1) {
                    let c = *lowercase.get(first_index + i).ok_or_else(|| {
                        LuceneError::from(rucene::error::LuceneErrorKind::StringIndexOutOfBounds(
                            format!(
                                "Index {} out of bounds for length {}",
                                first_index + i,
                                lowercase.len()
                            ),
                        ))
                    })?;
                    if c != expected {
                        found = false;
                        break;
                    }
                }
            }
            if found {
                append_range(&mut result, &string, copy_start, first_index)?;
                result.extend_from_slice(escape_char);
                append_range(
                    &mut result,
                    &string,
                    first_index,
                    first_index + sequence1_length,
                )?;
                copy_start = first_index + sequence1_length;
                start = copy_start;
            } else {
                start = first_index + 1;
            }
        }
        if result.is_empty() && copy_start == 0 {
            return Ok(string);
        }
        append_range(&mut result, &string, copy_start, string.len())?;
        Ok(result)
    }

    /// escape all tokens that are part of the parser syntax on a given
    /// string
    ///
    /// Equivalent to the private static `escapeWhiteChar(CharSequence str,
    /// Locale locale)`.
    fn escape_white_char(str: Vec<u16>, locale: &Locale) -> Result<Vec<u16>> {
        if str.is_empty() {
            return Ok(str);
        }
        let mut buffer = str;
        for escapable_white_char in ESCAPABLE_WHITE_CHARS {
            buffer = Self::escape_ignoring_case(
                buffer,
                &to_lower_case(&units(escapable_white_char), locale),
                &units("\\"),
                locale,
            )?;
        }
        Ok(buffer)
    }
}

impl EscapeQuerySyntaxImpl {
    /// Returns a String where the escape char has been removed, or kept only
    /// once if there was a double escape.
    ///
    /// Supports escaped Unicode characters, e.g. translates `\\u0041` to
    /// `A`.
    ///
    /// Equivalent to the static `discardEscapeChar(CharSequence input)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` `INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION` of a
    /// truncated `\\u` escape, `INVALID_SYNTAX_ESCAPE_CHARACTER` of a
    /// trailing escape character, and the one of
    /// `hexToInt`.
    pub fn discard_escape_char(input: &CharSequence) -> Result<UnescapedCharSequence> {
        let input_length = input.length();
        // Create char array to hold unescaped char sequence
        let mut output = vec![0u16; input_length];
        let mut was_escaped = vec![false; input_length];

        // The length of the output can be less than the input
        // due to discarded escape chars. This variable holds
        // the actual length of the output
        let mut length = 0usize;

        // We remember whether the last processed character was
        // an escape character
        let mut last_char_was_escape_char = false;

        // The multiplier the current unicode digit must be multiplied with.
        // E.g. the first digit must be multiplied with 16^3, the second with 16^2...
        let mut code_point_multiplier: i32 = 0;

        // Used to calculate the codepoint of the escaped unicode character
        let mut code_point: i32 = 0;

        for i in 0..input_length {
            let cur_char = input.char_at(i)?;
            if code_point_multiplier > 0 {
                code_point = code_point
                    .wrapping_add(Self::hex_to_int(cur_char)?.wrapping_mul(code_point_multiplier));
                code_point_multiplier = ((code_point_multiplier as u32) >> 4) as i32;
                if code_point_multiplier == 0 {
                    output[length] = code_point as u16;
                    length += 1;
                    code_point = 0;
                }
            } else if last_char_was_escape_char {
                if cur_char == u16::from(b'u') {
                    // found an escaped unicode character
                    code_point_multiplier = 16 * 16 * 16;
                } else {
                    // this character was escaped
                    output[length] = cur_char;
                    was_escaped[length] = true;
                    length += 1;
                }
                last_char_was_escape_char = false;
            } else if cur_char == u16::from(b'\\') {
                last_char_was_escape_char = true;
            } else {
                output[length] = cur_char;
                length += 1;
            }
        }

        if code_point_multiplier > 0 {
            return Err(ParseException::with_message(Arc::new(MessageImpl::new(
                QueryParserMessages::INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION.get(),
            )))
            .error());
        }

        if last_char_was_escape_char {
            return Err(ParseException::with_message(Arc::new(MessageImpl::new(
                QueryParserMessages::INVALID_SYNTAX_ESCAPE_CHARACTER.get(),
            )))
            .error());
        }

        UnescapedCharSequence::new(&output, &was_escaped, 0, length)
    }

    /// Returns the numeric value of the hexadecimal character
    ///
    /// Equivalent to the private static `hexToInt(char c)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` `INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE` (with
    /// the character) of a character that is not a hexadecimal digit.
    fn hex_to_int(c: u16) -> Result<i32> {
        if (u16::from(b'0')..=u16::from(b'9')).contains(&c) {
            Ok(i32::from(c - u16::from(b'0')))
        } else if (u16::from(b'a')..=u16::from(b'f')).contains(&c) {
            Ok(i32::from(c - u16::from(b'a')) + 10)
        } else if (u16::from(b'A')..=u16::from(b'F')).contains(&c) {
            Ok(i32::from(c - u16::from(b'A')) + 10)
        } else {
            Err(
                ParseException::with_message(Arc::new(MessageImpl::with_args(
                    QueryParserMessages::INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE.get(),
                    vec![JavaObject::character(c)],
                )))
                .error(),
            )
        }
    }
}

impl EscapeQuerySyntax for EscapeQuerySyntaxImpl {
    /// Equivalent to `escape(CharSequence text, Locale locale, Type type)`:
    /// the wildcards that were escaped and the backslashes escaped again,
    /// then the text escaped as a quoted string (`STRING`) or as a term.
    fn escape(
        &self,
        text: Option<&CharSequence>,
        locale: &Locale,
        type_: Type,
    ) -> Result<Option<CharSequence>> {
        let Some(text) = text else {
            return Ok(None);
        };
        if text.is_empty() {
            return Ok(Some(text.clone()));
        }
        // escape wildcards and the escape char (this has to be performed
        // before anything else) since we need to preserve the
        // UnescapedCharSequence and escape the original escape chars
        let escaped = match text
            .as_any()
            .and_then(|a| a.downcast_ref::<UnescapedCharSequence>())
        {
            Some(u) => u.to_string_escaped_with(&WILDCARD_CHARS),
            None => UnescapedCharSequence::from_text(text).to_string_escaped_with(&WILDCARD_CHARS),
        };
        let result = if type_ == Type::String {
            Self::escape_quoted(units(&escaped), locale)?
        } else {
            Self::escape_term(units(&escaped), locale)?
        };
        Ok(Some(CharSequence::from(String::from_utf16_lossy(&result))))
    }
}
