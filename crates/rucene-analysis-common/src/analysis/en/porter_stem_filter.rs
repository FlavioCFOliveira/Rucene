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
    AttributeHandle, CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic,
    TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::porter_stemmer::PorterStemmer;
use crate::jdk::character::{chars_to_utf16_into, utf16_to_term_chars_into};

/// The body of [`PorterStemFilter`]: what Lucene's `PorterStemFilter` adds
/// to `TokenFilter`.
#[derive(Debug)]
pub struct PorterStemFilterLogic {
    stemmer: PorterStemmer,
    /// `private final CharTermAttribute termAtt = addAttribute(CharTermAttribute.class)`.
    term_att: AttributeHandle<dyn CharTermAttribute>,
    /// `private final KeywordAttribute keywordAttr = addAttribute(KeywordAttribute.class)`.
    keyword_attr: AttributeHandle<dyn KeywordAttribute>,
    /// PERF: the term's UTF-16 units, Java's `termAtt.buffer()`, and the
    /// stem's term `char`s: buffers reused across tokens for the port-only
    /// conversions between the term's `char`s and UTF-16 (rmp #679).
    units: Vec<u16>,
    chars: Vec<char>,
}

impl TokenFilterLogic for PorterStemFilterLogic {
    /// Equivalent to `PorterStemFilter.incrementToken()`. The stemmer reads
    /// the term's UTF-16 units, Java's `termAtt.buffer()`; its result buffer
    /// replaces the term when it reports a change.
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }

        // PERF: through the fields' handles, as Java's fields, with no
        // attribute lookup by type per token (rmp #679).
        let is_keyword = self.keyword_attr.borrow().is_keyword();
        if !is_keyword {
            let mut term_att = self.term_att.borrow_mut();
            chars_to_utf16_into(&term_att.buffer()[..term_att.length()], &mut self.units);
            let units = &self.units;
            if self.stemmer.stem_range(units, 0, units.len()) {
                utf16_to_term_chars_into(
                    &self.stemmer.get_result_buffer()[..self.stemmer.get_result_length()],
                    &mut self.chars,
                );
                term_att.copy_buffer(&self.chars, 0, self.chars.len());
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
    TokenFilter::with_logic(input, |source| {
        let term_att = AttributeHandle::add(source)?;
        let keyword_attr = AttributeHandle::add(source)?;
        Ok(PorterStemFilterLogic {
            stemmer: PorterStemmer::new(),
            term_att,
            keyword_attr,
            units: Vec::new(),
            chars: Vec::new(),
        })
    })
}
