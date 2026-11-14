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

//! A `TokenFilter` that stems German words, ported from
//! `org.apache.lucene.analysis.de.GermanStemFilter`.

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::german_stemmer::GermanStemmer;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`GermanStemFilter`]: what Lucene's `GermanStemFilter` adds
/// to `TokenFilter`.
#[derive(Debug)]
pub struct GermanStemFilterLogic {
    /// The actual token in the input stream.
    stemmer: GermanStemmer,
}

impl GermanStemFilterLogic {
    /// Set a alternative/custom `GermanStemmer` for this filter.
    ///
    /// Equivalent to `GermanStemFilter.setStemmer(GermanStemmer)`: `None`,
    /// Java's `null`, keeps the current stemmer. Java's method is public but
    /// names a package-private type, so only its package can call it.
    pub(crate) fn set_stemmer(&mut self, stemmer: Option<GermanStemmer>) {
        if let Some(stemmer) = stemmer {
            self.stemmer = stemmer;
        }
    }
}

impl TokenFilterLogic for GermanStemFilterLogic {
    /// Returns true for next token in the stream, or false at EOS.
    ///
    /// Equivalent to `GermanStemFilter.incrementToken()`; the term's
    /// `String`, `termAtt.toString()`, is its UTF-16 units.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let term = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
            drop(term_att);

            let is_keyword = source
                .get_attribute_interface::<dyn KeywordAttribute>()
                .expect("INVARIANT: the constructor added the KeywordAttribute")
                .is_keyword();
            if !is_keyword {
                let s = self.stemmer.stem(&term);
                // If not stemmed, don't waste the time adjusting the token.
                if s != term {
                    let mut term_att = source
                        .get_attribute_interface_mut::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute");
                    let chars = utf16_to_term_chars(&s);
                    term_att.copy_buffer(&chars, 0, chars.len());
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A `TokenFilter` that stems German words.
///
/// It supports a table of words that should not be stemmed at all. The
/// stemmer used can be changed at runtime after the filter object is
/// created (as long as it is a `GermanStemmer`).
///
/// To prevent terms from being stemmed use an instance of
/// `SetKeywordMarkerFilter` or a custom `TokenFilter` that sets the
/// `KeywordAttribute` before this `TokenStream`.
///
/// Equivalent to `org.apache.lucene.analysis.de.GermanStemFilter`.
pub type GermanStemFilter = TokenFilter<GermanStemFilterLogic>;

/// Creates a `GermanStemFilter` instance.
///
/// Equivalent to `GermanStemFilter(TokenStream)`: `input` is the source
/// `TokenStream`; the fields add the `CharTermAttribute` and the
/// `KeywordAttribute`, in that order.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_german_stem_filter(input: Box<dyn TokenStream>) -> Result<GermanStemFilter> {
    let mut filter = TokenFilter::new(
        input,
        GermanStemFilterLogic {
            stemmer: GermanStemmer::new(),
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}
