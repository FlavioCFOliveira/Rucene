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

//! A `TokenFilter` that applies `NorwegianLightStemmer` to stem Norwegian
//! words, ported from `org.apache.lucene.analysis.no.NorwegianLightStemFilter`.

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::norwegian_light_stemmer::NorwegianLightStemmer;
use super::norwegian_light_stemmer::BOKMAAL;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`NorwegianLightStemFilter`]: what Lucene's
/// `NorwegianLightStemFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct NorwegianLightStemFilterLogic {
    stemmer: NorwegianLightStemmer,
}

impl TokenFilterLogic for NorwegianLightStemFilterLogic {
    /// Equivalent to `NorwegianLightStemFilter.incrementToken()`. The stemmer
    /// reads the term's UTF-16 units, Java's `termAtt.buffer()`, and may
    /// change them in place; the first `newlen` units become the term, as
    /// Java's `termAtt.setLength(newlen)` keeps them.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let is_keyword = source
                .get_attribute_interface::<dyn KeywordAttribute>()
                .expect("INVARIANT: the constructor added the KeywordAttribute")
                .is_keyword();
            if !is_keyword {
                let mut term_att = source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                let mut buffer = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
                let length = buffer.len();
                let newlen = self.stemmer.stem(&mut buffer, length);
                let chars = utf16_to_term_chars(&buffer[..newlen]);
                term_att.copy_buffer(&chars, 0, chars.len());
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A `TokenFilter` that applies `NorwegianLightStemmer` to stem Norwegian
/// words.
///
/// To prevent terms from being stemmed use an instance of
/// `SetKeywordMarkerFilter` or a custom `TokenFilter` that sets the
/// `KeywordAttribute` before this `TokenStream`.
///
/// Equivalent to `org.apache.lucene.analysis.no.NorwegianLightStemFilter`.
pub type NorwegianLightStemFilter = TokenFilter<NorwegianLightStemFilterLogic>;

/// Calls [`NorwegianLightStemFilter(input, BOKMAAL)`](new_norwegian_light_stem_filter_with_flags).
///
/// Equivalent to `NorwegianLightStemFilter(TokenStream)`.
///
/// # Errors
///
/// What [`new_norwegian_light_stem_filter_with_flags`] returns.
pub fn new_norwegian_light_stem_filter(
    input: Box<dyn TokenStream>,
) -> Result<NorwegianLightStemFilter> {
    new_norwegian_light_stem_filter_with_flags(input, BOKMAAL)
}

/// Creates a new NorwegianLightStemFilter
///
/// * `flags`: set to `NorwegianLightStemmer.BOKMAAL`,
///   `NorwegianLightStemmer.NYNORSK`, or both.
///
/// Equivalent to `NorwegianLightStemFilter(TokenStream, int)`: the field
/// initializers add the `CharTermAttribute` and the `KeywordAttribute`, in
/// that order, before the constructor body builds the stemmer. The filter's
/// source shares the input's attributes, so they are added to the input's
/// source here, before the stemmer is built.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws, and
/// `IllegalArgumentException` "invalid flags" from the stemmer.
pub fn new_norwegian_light_stem_filter_with_flags(
    mut input: Box<dyn TokenStream>,
    flags: i32,
) -> Result<NorwegianLightStemFilter> {
    let source = input.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    let stemmer = NorwegianLightStemmer::new(flags)?;
    Ok(TokenFilter::new(
        input,
        NorwegianLightStemFilterLogic { stemmer },
    ))
}
