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

//! Normalises token text to lower case for Irish, ported from
//! `org.apache.lucene.analysis.ga.IrishLowerCaseFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::{chars_to_utf16, to_lower_case_unit, utf16_to_term_chars};

/// The body of [`IrishLowerCaseFilter`]: what Lucene's
/// `IrishLowerCaseFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct IrishLowerCaseFilterLogic;

impl IrishLowerCaseFilterLogic {
    /// Equivalent to `isUpperVowel(int)`.
    fn is_upper_vowel(&self, v: u16) -> bool {
        matches!(
            char::from_u32(u32::from(v)),
            Some('A' | 'E' | 'I' | 'O' | 'U')
                // vowels with acute accent (fada)
                | Some('\u{00c1}' | '\u{00c9}' | '\u{00cd}' | '\u{00d3}' | '\u{00da}')
        )
    }
}

impl TokenFilterLogic for IrishLowerCaseFilterLogic {
    /// Equivalent to `IrishLowerCaseFilter.incrementToken()`, over the
    /// term's UTF-16 units, Java's `termAtt.buffer()`: each unit is
    /// lower-cased by `Character.toLowerCase(char)`, so a surrogate keeps its
    /// value.
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
            let mut ch_len = ch_array.len();
            let mut idx = 0;

            if ch_len > 1
                && (ch_array[0] == 'n' as u16 || ch_array[0] == 't' as u16)
                && self.is_upper_vowel(ch_array[1])
            {
                // `termAtt.resizeBuffer(chLen + 1)`, the shift right of every
                // unit from index 1 and the hyphen at index 1.
                ch_array.insert(1, '-' as u16);
                idx = 2;
                ch_len += 1;
            }

            // `Character.toChars(Character.toLowerCase(chArray[i]), chArray, i)`
            // writes one unit: a `char` lower-cases to a `char`.
            for c in &mut ch_array[idx..ch_len] {
                *c = to_lower_case_unit(*c);
            }
            let chars = utf16_to_term_chars(&ch_array[..ch_len]);
            term_att.copy_buffer(&chars, 0, chars.len());
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Normalises token text to lower case, handling t-prothesis and
/// n-eclipsis (i.e., that 'nAthair' should become 'n-athair')
///
/// Equivalent to `org.apache.lucene.analysis.ga.IrishLowerCaseFilter`.
pub type IrishLowerCaseFilter = TokenFilter<IrishLowerCaseFilterLogic>;

/// Create an IrishLowerCaseFilter that normalises Irish token text.
///
/// Equivalent to `IrishLowerCaseFilter(TokenStream)`, whose field adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_irish_lower_case_filter(input: Box<dyn TokenStream>) -> Result<IrishLowerCaseFilter> {
    let mut filter = TokenFilter::new(input, IrishLowerCaseFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
