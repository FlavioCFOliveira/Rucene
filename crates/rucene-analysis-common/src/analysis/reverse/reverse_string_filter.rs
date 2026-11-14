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

//! Reverses each token, ported from
//! `org.apache.lucene.analysis.reverse.ReverseStringFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// Equivalent to the private `ReverseStringFilter.NOMARKER`.
const NOMARKER: u16 = 0xFFFF;

/// Example marker character: U+0001 (START OF HEADING)
///
/// Equivalent to `ReverseStringFilter.START_OF_HEADING_MARKER`.
pub const START_OF_HEADING_MARKER: u16 = 0x0001;

/// Example marker character: U+001F (INFORMATION SEPARATOR ONE)
///
/// Equivalent to `ReverseStringFilter.INFORMATION_SEPARATOR_MARKER`.
pub const INFORMATION_SEPARATOR_MARKER: u16 = 0x001F;

/// Example marker character: U+EC00 (PRIVATE USE AREA: EC00)
///
/// Equivalent to `ReverseStringFilter.PUA_EC00_MARKER`.
pub const PUA_EC00_MARKER: u16 = 0xEC00;

/// Example marker character: U+200F (RIGHT-TO-LEFT MARK)
///
/// Equivalent to `ReverseStringFilter.RTL_DIRECTION_MARKER`.
pub const RTL_DIRECTION_MARKER: u16 = 0x200F;

/// The body of [`ReverseStringFilter`]: what Lucene's `ReverseStringFilter`
/// adds to `TokenFilter`.
///
/// The term is reversed as UTF-16 units, as Java reverses its `char[]`, and
/// decoded back into the term; an unpaired surrogate the reversal leaves (a
/// surrogate marker) becomes U+FFFD (`jdk::character::utf16_to_term_chars`).
#[derive(Debug)]
pub struct ReverseStringFilterLogic {
    marker: u16,
}

impl TokenFilterLogic for ReverseStringFilterLogic {
    /// Equivalent to `ReverseStringFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut buffer = {
                let term = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                chars_to_utf16(&term.buffer()[..term.length()])
            };
            let mut len = buffer.len();
            if self.marker != NOMARKER {
                len += 1;
                buffer.push(self.marker);
            }
            reverse_range(&mut buffer, 0, len);
            let chars = utf16_to_term_chars(&buffer[..len]);
            source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute")
                .copy_buffer(&chars, 0, chars.len());
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Reverse token string, for example "country" => "yrtnuoc".
///
/// If `marker` is supplied, then tokens will be also prepended by that
/// character. For example, with a marker of \u0001, "country" =>
/// "\u0001yrtnuoc". This is useful when implementing efficient leading
/// wildcards search.
///
/// Equivalent to `org.apache.lucene.analysis.reverse.ReverseStringFilter`.
pub type ReverseStringFilter = TokenFilter<ReverseStringFilterLogic>;

/// Create a new ReverseStringFilter that reverses all tokens in the supplied
/// `TokenStream`.
///
/// The reversed tokens will not be marked.
///
/// Equivalent to `ReverseStringFilter(TokenStream)`.
///
/// # Errors
///
/// As [`new_reverse_string_filter_with_marker`].
pub fn new_reverse_string_filter(input: Box<dyn TokenStream>) -> Result<ReverseStringFilter> {
    new_reverse_string_filter_with_marker(input, NOMARKER)
}

/// Create a new ReverseStringFilter that reverses and marks all tokens in the
/// supplied `TokenStream`.
///
/// The reversed tokens will be prepended (marked) by the `marker` character.
///
/// Equivalent to `ReverseStringFilter(TokenStream, char)`, which adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_reverse_string_filter_with_marker(
    input: Box<dyn TokenStream>,
    marker: u16,
) -> Result<ReverseStringFilter> {
    let mut filter = TokenFilter::new(input, ReverseStringFilterLogic { marker });
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}

/// Reverses the given input string
///
/// Returns the given input string in reversed order.
///
/// Equivalent to the static `ReverseStringFilter.reverse(String)`.
pub fn reverse(input: &str) -> String {
    let mut char_input: Vec<u16> = input.encode_utf16().collect();
    let len = char_input.len();
    reverse_range(&mut char_input, 0, len);
    String::from_utf16_lossy(&char_input)
}

/// Reverses the given input buffer in-place
///
/// Equivalent to the static `ReverseStringFilter.reverse(char[])`.
pub fn reverse_buffer(buffer: &mut [u16]) {
    let len = buffer.len();
    reverse_range(buffer, 0, len);
}

/// Partially reverses the given input buffer in-place from offset 0 up to
/// the given length.
///
/// Equivalent to the static `ReverseStringFilter.reverse(char[], int)`.
pub fn reverse_len(buffer: &mut [u16], len: usize) {
    reverse_range(buffer, 0, len);
}

/// `Character.isSurrogatePair(char, char)`.
fn is_surrogate_pair(high: u16, low: u16) -> bool {
    (0xD800..0xDC00).contains(&high) && (0xDC00..0xE000).contains(&low)
}

/// Partially reverses the given input buffer in-place from the given offset
/// up to the given length.
///
/// * `buffer` the input char array to reverse
/// * `start` the offset from where to reverse the buffer
/// * `len` the length in the buffer up to where the buffer should be
///   reversed
///
/// Equivalent to the static `ReverseStringFilter.reverse(char[], int, int)`.
///
/// # Panics
///
/// Where Java throws `ArrayIndexOutOfBoundsException`: a range past the end
/// of `buffer`.
pub fn reverse_range(buffer: &mut [u16], start: usize, len: usize) {
    /* modified version of Apache Harmony AbstractStringBuilder reverse0() */
    if len < 2 {
        return;
    }
    let mut end = (start + len) - 1;
    let mut front_high = buffer[start];
    let mut end_low = buffer[end];
    let mut allow_front_sur = true;
    let mut allow_end_sur = true;
    let mid = start + (len >> 1);
    let mut i = start;
    while i < mid {
        let front_low = buffer[i + 1];
        let end_high = buffer[end - 1];
        let sur_at_front = allow_front_sur && is_surrogate_pair(front_high, front_low);
        if sur_at_front && (len < 3) {
            // nothing to do since surAtFront is allowed and 1 char left
            return;
        }
        let sur_at_end = allow_end_sur && is_surrogate_pair(end_high, end_low);
        allow_front_sur = true;
        allow_end_sur = true;
        if sur_at_front == sur_at_end {
            if sur_at_front {
                // both surrogates
                buffer[end] = front_low;
                end -= 1;
                buffer[end] = front_high;
                buffer[i] = end_high;
                i += 1;
                buffer[i] = end_low;
                front_high = buffer[i + 1];
                end_low = buffer[end - 1];
            } else {
                // neither surrogates
                buffer[end] = front_high;
                buffer[i] = end_low;
                front_high = front_low;
                end_low = end_high;
            }
        } else if sur_at_front {
            // surrogate only at the front
            buffer[end] = front_low;
            buffer[i] = end_low;
            end_low = end_high;
            allow_front_sur = false;
        } else {
            // surrogate only at the end
            buffer[end] = front_high;
            buffer[i] = end_high;
            front_high = front_low;
            allow_end_sur = false;
        }
        i += 1;
        end -= 1;
    }
    if (len & 0x01) == 1 && !(allow_front_sur && allow_end_sur) {
        // only if odd length
        buffer[end] = if allow_front_sur { end_low } else { front_high };
    }
}
