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

//! Normalizes Greek token text to lower case, ported from
//! `org.apache.lucene.analysis.el.GreekLowerCaseFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::case_folding;

use crate::jdk::character::{chars_to_utf16, code_point_at, utf16_to_term_chars};

/// `Character.toChars(int, char[], int)`: writes the UTF-16 units of
/// `code_point` at `index` of `dst` and returns their count.
fn to_chars(code_point: i32, dst: &mut [u16], index: usize) -> usize {
    let c = u32::try_from(code_point).ok().and_then(char::from_u32);
    match c {
        Some(c) => {
            let mut buf = [0u16; 2];
            let units = c.encode_utf16(&mut buf);
            dst[index..index + units.len()].copy_from_slice(units);
            units.len()
        }
        // A surrogate code point is its own single unit.
        None => {
            dst[index] = code_point as u16;
            1
        }
    }
}

/// The body of [`GreekLowerCaseFilter`]: what Lucene's
/// `GreekLowerCaseFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct GreekLowerCaseFilterLogic;

impl GreekLowerCaseFilterLogic {
    /// Equivalent to `lowerCase(int)`.
    fn lower_case(&self, codepoint: i32) -> i32 {
        match codepoint {
            /* There are two lowercase forms of sigma:
             *   U+03C2: small final sigma (end of word)
             *   U+03C3: small sigma (otherwise)
             *
             * Standardize both to U+03C3
             */
            0x03C2 /* small final sigma */ => 0x03C3, /* small sigma */

            /* Some greek characters contain diacritics.
             * This filter removes these, converting to the lowercase base form.
             */
            0x0386 /* capital alpha with tonos */
            | 0x03AC /* small alpha with tonos */ => 0x03B1, /* small alpha */

            0x0388 /* capital epsilon with tonos */
            | 0x03AD /* small epsilon with tonos */ => 0x03B5, /* small epsilon */

            0x0389 /* capital eta with tonos */
            | 0x03AE /* small eta with tonos */ => 0x03B7, /* small eta */

            0x038A /* capital iota with tonos */
            | 0x03AA /* capital iota with dialytika */
            | 0x03AF /* small iota with tonos */
            | 0x03CA /* small iota with dialytika */
            | 0x0390 /* small iota with dialytika and tonos */ => 0x03B9, /* small iota */

            0x038E /* capital upsilon with tonos */
            | 0x03AB /* capital upsilon with dialytika */
            | 0x03CD /* small upsilon with tonos */
            | 0x03CB /* small upsilon with dialytika */
            | 0x03B0 /* small upsilon with dialytika and tonos */ => 0x03C5, /* small upsilon */

            0x038C /* capital omicron with tonos */
            | 0x03CC /* small omicron with tonos */ => 0x03BF, /* small omicron */

            0x038F /* capital omega with tonos */
            | 0x03CE /* small omega with tonos */ => 0x03C9, /* small omega */

            /* The previous implementation did the conversion below.
             * Only implemented for backwards compatibility with old indexes.
             */
            0x03A2 /* reserved */ => 0x03C2, /* small final sigma */

            _ => case_folding::to_lower_case(codepoint),
        }
    }
}

impl TokenFilterLogic for GreekLowerCaseFilterLogic {
    /// Equivalent to `GreekLowerCaseFilter.incrementToken()`, over the
    /// term's UTF-16 units, Java's `termAtt.buffer()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let mut ch_array = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
            let ch_len = ch_array.len();
            let mut i = 0;
            while i < ch_len {
                let lower = self.lower_case(code_point_at(&ch_array, i, ch_len));
                i += to_chars(lower, &mut ch_array, i);
            }
            let chars = utf16_to_term_chars(&ch_array[..ch_len]);
            term_att.copy_buffer(&chars, 0, chars.len());
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Normalizes token text to lower case, removes some Greek diacritics, and
/// standardizes final sigma to sigma.
///
/// Equivalent to `org.apache.lucene.analysis.el.GreekLowerCaseFilter`.
pub type GreekLowerCaseFilter = TokenFilter<GreekLowerCaseFilterLogic>;

/// Create a GreekLowerCaseFilter that normalizes Greek token text.
///
/// * `input`: TokenStream to filter
///
/// Equivalent to `GreekLowerCaseFilter(TokenStream)`, whose field adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_greek_lower_case_filter(input: Box<dyn TokenStream>) -> Result<GreekLowerCaseFilter> {
    let mut filter = TokenFilter::new(input, GreekLowerCaseFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
