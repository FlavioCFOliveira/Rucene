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

//! A high-performance kstem filter for english, ported from
//! `org.apache.lucene.analysis.en.KStemFilter`.

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::kstemmer::{KStemmer, StemResult};
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`KStemFilter`]: what Lucene's `KStemFilter` adds to
/// `TokenFilter`.
#[derive(Debug)]
pub struct KStemFilterLogic {
    stemmer: KStemmer,
}

impl TokenFilterLogic for KStemFilterLogic {
    /// Returns the next, stemmed, input Token.
    ///
    /// Equivalent to `KStemFilter.incrementToken()`. The stemmer reads the
    /// term's UTF-16 units, Java's `termAttribute.buffer()`; the stem
    /// (`stemmer.asCharSequence()`) replaces the term.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }

        let is_keyword = source
            .get_attribute_interface::<dyn KeywordAttribute>()
            .expect("INVARIANT: the constructor added the KeywordAttribute")
            .is_keyword();
        if !is_keyword {
            let mut term_attribute = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let term = chars_to_utf16(&term_attribute.buffer()[..term_attribute.length()]);
            let len = term.len();
            if self.stemmer.stem(&term, len) {
                term_attribute.set_empty();
                match self.stemmer.as_char_sequence() {
                    StemResult::Root(root) => term_attribute.append_string(Some(root)),
                    StemResult::Word(word) => {
                        let chars = utf16_to_term_chars(word);
                        term_attribute.copy_buffer(&chars, 0, chars.len());
                    }
                }
            }
        }

        Ok(true)
    }
}

/// A high-performance kstem filter for english.
///
/// See ["Viewing Morphology as an Inference
/// Process"](http://ciir.cs.umass.edu/pubfiles/ir-35.pdf) (Krovetz, R.,
/// Proceedings of the Sixteenth Annual International ACM SIGIR Conference on
/// Research and Development in Information Retrieval, 191-203, 1993).
///
/// All terms must already be lowercased for this filter to work correctly.
///
/// Note: This filter is aware of the `KeywordAttribute`. To prevent certain
/// terms from being passed to the stemmer `KeywordAttribute.isKeyword()`
/// should be set to `true` in a previous `TokenStream`.
///
/// Note: For including the original term as well as the stemmed version,
/// see `org.apache.lucene.analysis.miscellaneous.KeywordRepeatFilterFactory`.
///
/// Equivalent to `org.apache.lucene.analysis.en.KStemFilter`.
pub type KStemFilter = TokenFilter<KStemFilterLogic>;

/// Equivalent to `KStemFilter(TokenStream)`, whose fields add the
/// `CharTermAttribute` and the `KeywordAttribute`, in that order.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_kstem_filter(input: Box<dyn TokenStream>) -> Result<KStemFilter> {
    let mut filter = TokenFilter::new(
        input,
        KStemFilterLogic {
            stemmer: KStemmer::new(),
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}
