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

//! Normalizes German characters, ported from
//! `org.apache.lucene.analysis.de.GermanNormalizationFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::analysis::util::stemmer_util::StemmerUtil;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

// FSM with 3 states:
/// ordinary state
const N: i32 = 0;
/// stops 'u' from entering umlaut state
const V: i32 = 1;
/// umlaut state, allows e-deletion
const U: i32 = 2;

/// The body of [`GermanNormalizationFilter`]: what Lucene's
/// `GermanNormalizationFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct GermanNormalizationFilterLogic;

impl TokenFilterLogic for GermanNormalizationFilterLogic {
    /// Equivalent to `GermanNormalizationFilter.incrementToken()`, over the
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
            let mut state = N;
            let mut buffer = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
            let mut length = buffer.len();
            let mut i = 0;
            while i < length {
                let c = buffer[i];
                match char::from_u32(u32::from(c)) {
                    Some('a' | 'o') => state = U,
                    Some('u') => state = if state == N { U } else { V },
                    Some('e') => {
                        if state == U {
                            // Java's `delete(buffer, i--, length)`: the next
                            // unit moves to `i`, which the loop visits again.
                            length = StemmerUtil::delete(&mut buffer, i, length);
                            state = V;
                            continue;
                        }
                        state = V;
                    }
                    Some('i' | 'q' | 'y') => state = V,
                    Some('ä') => {
                        buffer[i] = 'a' as u16;
                        state = V;
                    }
                    Some('ö') => {
                        buffer[i] = 'o' as u16;
                        state = V;
                    }
                    Some('ü') => {
                        buffer[i] = 'u' as u16;
                        state = V;
                    }
                    Some('ß') => {
                        buffer[i] = 's' as u16;
                        i += 1;
                        // `resizeBuffer(1 + length)` and the copy of the tail
                        // one unit right.
                        buffer.insert(i, 's' as u16);
                        length += 1;
                        state = N;
                    }
                    _ => state = N,
                }
                i += 1;
            }
            let chars = utf16_to_term_chars(&buffer[..length]);
            term_att.copy_buffer(&chars, 0, chars.len());
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Normalizes German characters according to the heuristics of the
/// [German snowball algorithm](https://snowballstem.org/algorithms/german/stemmer.html).
/// It allows for the fact that ä, ö and ü are sometimes written as ae, oe
/// and ue.
///
/// * 'ß' is replaced by 'ss'
/// * 'ä', 'ö', 'ü' are replaced by 'a', 'o', 'u', respectively.
/// * 'ae' and 'oe' are replaced by 'a', and 'o', respectively.
/// * 'ue' is replaced by 'u', when not following a vowel or q.
///
/// This is useful if you want this normalization without using the German
/// stemmer, or perhaps no stemming at all.
///
/// Equivalent to `org.apache.lucene.analysis.de.GermanNormalizationFilter`.
pub type GermanNormalizationFilter = TokenFilter<GermanNormalizationFilterLogic>;

/// Equivalent to `GermanNormalizationFilter(TokenStream)`, whose field adds
/// the `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_german_normalization_filter(
    input: Box<dyn TokenStream>,
) -> Result<GermanNormalizationFilter> {
    let mut filter = TokenFilter::new(input, GermanNormalizationFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
