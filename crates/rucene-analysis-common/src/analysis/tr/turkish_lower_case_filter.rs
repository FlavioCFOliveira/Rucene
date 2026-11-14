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

//! Normalizes Turkish token text to lower case, ported from
//! `org.apache.lucene.analysis.tr.TurkishLowerCaseFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::case_folding;

use crate::jdk::character::{
    char_count, chars_to_utf16, code_point_at, get_type, utf16_to_term_chars, NON_SPACING_MARK,
};

const LATIN_CAPITAL_LETTER_I: i32 = 0x0049;
const LATIN_SMALL_LETTER_I: u16 = 0x0069;
const LATIN_SMALL_LETTER_DOTLESS_I: u16 = 0x0131;
const COMBINING_DOT_ABOVE: i32 = 0x0307;

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

/// The body of [`TurkishLowerCaseFilter`]: what Lucene's
/// `TurkishLowerCaseFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct TurkishLowerCaseFilterLogic;

impl TurkishLowerCaseFilterLogic {
    /// lookahead for a combining dot above. other NSMs may be in between.
    ///
    /// Equivalent to `isBeforeDot(char[], int, int)`.
    fn is_before_dot(&self, s: &[u16], pos: usize, len: usize) -> bool {
        let mut i = pos;
        while i < len {
            let ch = code_point_at(s, i, len);
            if get_type(ch) != NON_SPACING_MARK {
                return false;
            }
            if ch == COMBINING_DOT_ABOVE {
                return true;
            }
            i += char_count(ch);
        }

        false
    }

    /// delete a character in-place. rarely happens, only if
    /// COMBINING_DOT_ABOVE is found after an i
    ///
    /// Equivalent to `delete(char[], int, int)`.
    fn delete(&self, s: &mut [u16], pos: usize, len: usize) -> usize {
        if pos < len {
            s.copy_within(pos + 1..len, pos);
        }

        len - 1
    }
}

impl TokenFilterLogic for TurkishLowerCaseFilterLogic {
    /// Equivalent to `TurkishLowerCaseFilter.incrementToken()`, over the
    /// term's UTF-16 units, Java's `termAtt.buffer()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let mut i_or_after = false;

        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let mut buffer = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
            let mut length = buffer.len();
            let mut i = 0;
            while i < length {
                let ch = code_point_at(&buffer, i, length);

                i_or_after = ch == LATIN_CAPITAL_LETTER_I
                    || (i_or_after && get_type(ch) == NON_SPACING_MARK);

                if i_or_after {
                    // all the special I turkish handling happens here.
                    match ch {
                        // remove COMBINING_DOT_ABOVE to mimic composed lowercase
                        COMBINING_DOT_ABOVE => {
                            length = self.delete(&mut buffer, i, length);
                            continue;
                        }
                        // i itself, it depends if it is followed by COMBINING_DOT_ABOVE
                        // if it is, we will make it small i and later remove the dot
                        LATIN_CAPITAL_LETTER_I => {
                            if self.is_before_dot(&buffer, i + 1, length) {
                                buffer[i] = LATIN_SMALL_LETTER_I;
                            } else {
                                buffer[i] = LATIN_SMALL_LETTER_DOTLESS_I;
                                // below is an optimization. no COMBINING_DOT_ABOVE follows,
                                // so don't waste time calculating Character.getType(), etc
                                i_or_after = false;
                            }
                            i += 1;
                            continue;
                        }
                        _ => {}
                    }
                }

                i += to_chars(case_folding::to_lower_case(ch), &mut buffer, i);
            }

            let chars = utf16_to_term_chars(&buffer[..length]);
            term_att.copy_buffer(&chars, 0, chars.len());
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Normalizes Turkish token text to lower case.
///
/// Turkish and Azeri have unique casing behavior for some characters. This
/// filter applies Turkish lowercase rules. For more information, see
/// [http://en.wikipedia.org/wiki/Turkish_dotted_and_dotless_I](http://en.wikipedia.org/wiki/Turkish_dotted_and_dotless_I)
///
/// Equivalent to `org.apache.lucene.analysis.tr.TurkishLowerCaseFilter`.
pub type TurkishLowerCaseFilter = TokenFilter<TurkishLowerCaseFilterLogic>;

/// Create a new TurkishLowerCaseFilter, that normalizes Turkish token text
/// to lower case.
///
/// * `input`: TokenStream to filter
///
/// Equivalent to `TurkishLowerCaseFilter(TokenStream)`, whose field adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_turkish_lower_case_filter(
    input: Box<dyn TokenStream>,
) -> Result<TurkishLowerCaseFilter> {
    let mut filter = TokenFilter::new(input, TurkishLowerCaseFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
