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

//! A `TokenFilter` that applies `EnglishMinimalStemmer` to stem English
//! words, ported from `org.apache.lucene.analysis.en.EnglishMinimalStemFilter`.

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::english_minimal_stemmer::EnglishMinimalStemmer;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`EnglishMinimalStemFilter`]: what Lucene's
/// `EnglishMinimalStemFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct EnglishMinimalStemFilterLogic {
    stemmer: EnglishMinimalStemmer,
}

impl TokenFilterLogic for EnglishMinimalStemFilterLogic {
    /// Equivalent to `EnglishMinimalStemFilter.incrementToken()`. The
    /// stemmer reads the term's UTF-16 units, Java's `termAtt.buffer()`; the
    /// stemmed units are written back to the term.
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
                let mut units = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
                let len = units.len();
                let newlen = self.stemmer.stem(&mut units, len);
                if newlen != len {
                    let chars = utf16_to_term_chars(&units[..newlen]);
                    term_att.copy_buffer(&chars, 0, chars.len());
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A `TokenFilter` that applies `EnglishMinimalStemmer` to stem English
/// words.
///
/// To prevent terms from being stemmed use an instance of
/// `SetKeywordMarkerFilter` or a custom `TokenFilter` that sets the
/// `KeywordAttribute` before this `TokenStream`.
///
/// Equivalent to `org.apache.lucene.analysis.en.EnglishMinimalStemFilter`.
pub type EnglishMinimalStemFilter = TokenFilter<EnglishMinimalStemFilterLogic>;

/// Equivalent to `EnglishMinimalStemFilter(TokenStream)`, whose fields add
/// the `CharTermAttribute` and the `KeywordAttribute`, in that order.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_english_minimal_stem_filter(
    input: Box<dyn TokenStream>,
) -> Result<EnglishMinimalStemFilter> {
    let mut filter = TokenFilter::new(
        input,
        EnglishMinimalStemFilterLogic {
            stemmer: EnglishMinimalStemmer::new(),
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}
