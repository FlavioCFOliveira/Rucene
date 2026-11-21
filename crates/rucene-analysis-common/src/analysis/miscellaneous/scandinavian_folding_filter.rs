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
//! Folds the Scandinavian characters, ported from
//! `org.apache.lucene.analysis.miscellaneous.ScandinavianFoldingFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::analysis::util::stemmer_util::StemmerUtil;

const AA: char = '\u{00C5}'; // Å
const AA_LOWER: char = '\u{00E5}'; // å
const AE: char = '\u{00C6}'; // Æ
const AE_LOWER: char = '\u{00E6}'; // æ
const AE_SE: char = '\u{00C4}'; // Ä
const AE_SE_LOWER: char = '\u{00E4}'; // ä
const OE: char = '\u{00D8}'; // Ø
const OE_LOWER: char = '\u{00F8}'; // ø
const OE_SE: char = '\u{00D6}'; // Ö
const OE_SE_LOWER: char = '\u{00F6}'; // ö

/// The body of [`ScandinavianFoldingFilter`]: what Lucene's
/// `ScandinavianFoldingFilter` adds to `TokenFilter`.
#[derive(Debug, Default)]
pub struct ScandinavianFoldingFilterLogic;

impl TokenFilterLogic for ScandinavianFoldingFilterLogic {
    /// Equivalent to `ScandinavianFoldingFilter.incrementToken()`; every
    /// character compared is in the BMP, so the `char` buffer folds as
    /// Java's `char[]`. Java's two delete branches are kept apart, as in the
    /// source.
    #[allow(clippy::if_same_then_else)]
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }

        let mut char_term_attribute = source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let mut length = char_term_attribute.length();
        let buffer = char_term_attribute.buffer_mut();
        let mut i = 0;
        while i < length {
            if buffer[i] == AA_LOWER || buffer[i] == AE_SE_LOWER || buffer[i] == AE_LOWER {
                buffer[i] = 'a';
            } else if buffer[i] == AA || buffer[i] == AE_SE || buffer[i] == AE {
                buffer[i] = 'A';
            } else if buffer[i] == OE_LOWER || buffer[i] == OE_SE_LOWER {
                buffer[i] = 'o';
            } else if buffer[i] == OE || buffer[i] == OE_SE {
                buffer[i] = 'O';
            } else if length - 1 > i {
                let next = buffer[i + 1];
                if (buffer[i] == 'a' || buffer[i] == 'A')
                    && (next == 'a'
                        || next == 'A'
                        || next == 'e'
                        || next == 'E'
                        || next == 'o'
                        || next == 'O')
                {
                    length = StemmerUtil::delete(buffer, i + 1, length);
                } else if (buffer[i] == 'o' || buffer[i] == 'O')
                    && (next == 'e' || next == 'E' || next == 'o' || next == 'O')
                {
                    length = StemmerUtil::delete(buffer, i + 1, length);
                }
            }
            i += 1;
        }

        char_term_attribute.set_length(length);
        Ok(true)
    }
}

/// This filter folds Scandinavian characters åÅäæÄÆ->a and öÖøØ->o. It also
/// discriminate against use of double vowels aa, ae, ao, oe and oo, leaving
/// just the first one.
///
/// It's a semantically more destructive solution than
/// `ScandinavianNormalizationFilter` but can in addition help with matching
/// raksmorgas as räksmörgås.
///
/// blåbærsyltetøj == blåbärsyltetöj == blaabaarsyltetoej == blabarsyltetoj;
/// räksmörgås == ræksmørgås == ræksmörgaos == raeksmoergaas == raksmorgas.
///
/// Background: Swedish åäö are in fact the same letters as Norwegian and
/// Danish åæø and thus interchangeable when used between these languages.
/// They are however folded differently when people type them on a keyboard
/// lacking these characters. In that situation almost all Swedish people use
/// a, a, o instead of å, ä, ö. Norwegians and Danes on the other hand
/// usually type aa, ae and oe instead of å, æ and ø. Some do however use a,
/// a, o, oo, ao and sometimes permutations of everything above.
///
/// This filter solves that mismatch problem, but might also cause new.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ScandinavianFoldingFilter`.
pub type ScandinavianFoldingFilter = TokenFilter<ScandinavianFoldingFilterLogic>;

/// Equivalent to `ScandinavianFoldingFilter(TokenStream)`, which adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_scandinavian_folding_filter(
    input: Box<dyn TokenStream>,
) -> Result<ScandinavianFoldingFilter> {
    let mut filter = TokenFilter::new(input, ScandinavianFoldingFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
