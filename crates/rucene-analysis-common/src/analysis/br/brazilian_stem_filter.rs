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

//! A `TokenFilter` that applies `BrazilianStemmer`, ported from
//! `org.apache.lucene.analysis.br.BrazilianStemFilter`.

use std::collections::HashSet;

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::brazilian_stemmer::BrazilianStemmer;
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The body of [`BrazilianStemFilter`]: what Lucene's
/// `BrazilianStemFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct BrazilianStemFilterLogic {
    /// `BrazilianStemmer` in use by this filter.
    stemmer: BrazilianStemmer,
    /// Java's private `Set<?> exclusions`, initialized to null and never
    /// assigned: always `None`.
    exclusions: Option<HashSet<Vec<u16>>>,
}

impl TokenFilterLogic for BrazilianStemFilterLogic {
    /// Equivalent to `BrazilianStemFilter.incrementToken()`; the term's
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
            // Check the exclusion table.
            if !is_keyword
                && self
                    .exclusions
                    .as_ref()
                    .is_none_or(|exclusions| !exclusions.contains(&term))
            {
                let s = self.stemmer.stem(&term);
                // If not stemmed, don't waste the time adjusting the token.
                if let Some(s) = s {
                    if s != term {
                        let mut term_att = source
                            .get_attribute_interface_mut::<dyn CharTermAttribute>()
                            .expect("INVARIANT: the constructor added the CharTermAttribute");
                        let chars = utf16_to_term_chars(&s);
                        term_att.copy_buffer(&chars, 0, chars.len());
                    }
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A `TokenFilter` that applies `BrazilianStemmer`.
///
/// To prevent terms from being stemmed use an instance of
/// `SetKeywordMarkerFilter` or a custom `TokenFilter` that sets the
/// `KeywordAttribute` before this `TokenStream`.
///
/// * see: `SetKeywordMarkerFilter`
///
/// Equivalent to `org.apache.lucene.analysis.br.BrazilianStemFilter`.
pub type BrazilianStemFilter = TokenFilter<BrazilianStemFilterLogic>;

/// Creates a new BrazilianStemFilter
///
/// * `input`: the source `TokenStream`
///
/// Equivalent to `BrazilianStemFilter(TokenStream)`, whose fields add the
/// `CharTermAttribute` and the `KeywordAttribute`, in that order.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_brazilian_stem_filter(input: Box<dyn TokenStream>) -> Result<BrazilianStemFilter> {
    let mut filter = TokenFilter::new(
        input,
        BrazilianStemFilterLogic {
            stemmer: BrazilianStemmer::new(),
            exclusions: None,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}
