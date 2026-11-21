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

//! Normalizes Serbian Cyrillic to Latin, ported from
//! `org.apache.lucene.analysis.sr.SerbianNormalizationRegularFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`SerbianNormalizationRegularFilter`]: what Lucene's
/// `SerbianNormalizationRegularFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct SerbianNormalizationRegularFilterLogic;

impl TokenFilterLogic for SerbianNormalizationRegularFilterLogic {
    /// Equivalent to `SerbianNormalizationRegularFilter.incrementToken()`,
    /// on the term's UTF-16 units, Java's `termAtt.buffer()`. Where Java
    /// grows the buffer (`resizeBuffer(1 + length)`) and shifts the tail one
    /// unit right to replace a letter by two, the units here take the second
    /// letter by an insertion.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let mut buffer = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
            let mut length = buffer.len();
            let mut i = 0;
            while i < length {
                let c = buffer[i];
                match char::from_u32(u32::from(c)) {
                    Some('а') => buffer[i] = 'a' as u16,
                    Some('б') => buffer[i] = 'b' as u16,
                    Some('в') => buffer[i] = 'v' as u16,
                    Some('г') => buffer[i] = 'g' as u16,
                    Some('д') => buffer[i] = 'd' as u16,
                    Some('ђ') => buffer[i] = 'đ' as u16,
                    Some('е') => buffer[i] = 'e' as u16,
                    Some('ж') => buffer[i] = 'ž' as u16,
                    Some('з') => buffer[i] = 'z' as u16,
                    Some('и') => buffer[i] = 'i' as u16,
                    Some('ј') => buffer[i] = 'j' as u16,
                    Some('к') => buffer[i] = 'k' as u16,
                    Some('л') => buffer[i] = 'l' as u16,
                    Some('љ') => expand(&mut buffer, &mut i, &mut length, 'l', 'j'),
                    Some('м') => buffer[i] = 'm' as u16,
                    Some('н') => buffer[i] = 'n' as u16,
                    Some('њ') => expand(&mut buffer, &mut i, &mut length, 'n', 'j'),
                    Some('о') => buffer[i] = 'o' as u16,
                    Some('п') => buffer[i] = 'p' as u16,
                    Some('р') => buffer[i] = 'r' as u16,
                    Some('с') => buffer[i] = 's' as u16,
                    Some('т') => buffer[i] = 't' as u16,
                    Some('ћ') => buffer[i] = 'ć' as u16,
                    Some('у') => buffer[i] = 'u' as u16,
                    Some('ф') => buffer[i] = 'f' as u16,
                    Some('х') => buffer[i] = 'h' as u16,
                    Some('ц') => buffer[i] = 'c' as u16,
                    Some('ч') => buffer[i] = 'č' as u16,
                    Some('џ') => expand(&mut buffer, &mut i, &mut length, 'd', 'ž'),
                    Some('ш') => buffer[i] = 'š' as u16,
                    _ => {}
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

/// Replaces the letter at `*i` by `first` and `second`: Java's
/// `buffer = termAtt.resizeBuffer(1 + length)`, the copy of the tail one unit
/// right, then `buffer[i] = first; buffer[++i] = second; length++;`.
fn expand(buffer: &mut Vec<u16>, i: &mut usize, length: &mut usize, first: char, second: char) {
    buffer[*i] = first as u16;
    buffer.insert(*i + 1, second as u16);
    *i += 1;
    *length += 1;
}

/// Normalizes Serbian Cyrillic to Latin.
///
/// Note that it expects lowercased input.
///
/// Equivalent to `org.apache.lucene.analysis.sr.SerbianNormalizationRegularFilter`.
pub type SerbianNormalizationRegularFilter = TokenFilter<SerbianNormalizationRegularFilterLogic>;

/// Equivalent to `SerbianNormalizationRegularFilter(TokenStream)`, whose
/// field adds the `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_serbian_normalization_regular_filter(
    input: Box<dyn TokenStream>,
) -> Result<SerbianNormalizationRegularFilter> {
    let mut filter = TokenFilter::new(input, SerbianNormalizationRegularFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
