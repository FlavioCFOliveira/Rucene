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

//! Transforms the token stream as per the Porter stemming algorithm, ported
//! from `org.apache.lucene.analysis.en.PorterStemFilter`.

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::porter_stemmer::PorterStemmer;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`PorterStemFilter`]: what Lucene's `PorterStemFilter` adds
/// to `TokenFilter`.
#[derive(Debug)]
pub struct PorterStemFilterLogic {
    stemmer: PorterStemmer,
}

impl TokenFilterLogic for PorterStemFilterLogic {
    /// Equivalent to `PorterStemFilter.incrementToken()`. The stemmer reads
    /// the term's UTF-16 units, Java's `termAtt.buffer()`; its result buffer
    /// replaces the term when it reports a change.
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
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let units = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
            if self.stemmer.stem_range(&units, 0, units.len()) {
                let chars = utf16_to_term_chars(
                    &self.stemmer.get_result_buffer()[..self.stemmer.get_result_length()],
                );
                term_att.copy_buffer(&chars, 0, chars.len());
            }
        }
        Ok(true)
    }
}

/// Transforms the token stream as per the Porter stemming algorithm. Note:
/// the input to the stemming filter must already be in lower case, so you
/// will need to use LowerCaseFilter or LowerCaseTokenizer farther down the
/// Tokenizer chain in order for this to work properly!
///
/// To use this filter with other analyzers, you'll want to write an
/// Analyzer class that sets up the TokenStream chain as you want it. To use
/// this with LowerCaseTokenizer, for example, you'd write an analyzer like
/// this:
///
/// ```text
/// class MyAnalyzer extends Analyzer {
///   @Override
///   protected TokenStreamComponents createComponents(String fieldName) {
///     Tokenizer source = new LowerCaseTokenizer(version, reader);
///     return new TokenStreamComponents(source, new PorterStemFilter(source));
///   }
/// }
/// ```
///
/// Note: This filter is aware of the `KeywordAttribute`. To prevent certain
/// terms from being passed to the stemmer `KeywordAttribute.isKeyword()`
/// should be set to `true` in a previous `TokenStream`.
///
/// Note: For including the original term as well as the stemmed version,
/// see `org.apache.lucene.analysis.miscellaneous.KeywordRepeatFilterFactory`.
///
/// Equivalent to `org.apache.lucene.analysis.en.PorterStemFilter`.
pub type PorterStemFilter = TokenFilter<PorterStemFilterLogic>;

/// Equivalent to `PorterStemFilter(TokenStream)`, whose fields add the
/// `CharTermAttribute` and the `KeywordAttribute`, in that order.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_porter_stem_filter(input: Box<dyn TokenStream>) -> Result<PorterStemFilter> {
    let mut filter = TokenFilter::new(
        input,
        PorterStemFilterLogic {
            stemmer: PorterStemmer::new(),
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}
