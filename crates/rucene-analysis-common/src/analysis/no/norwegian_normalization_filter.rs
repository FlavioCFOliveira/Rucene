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

//! Normalizes the interchangeable Scandinavian characters with the
//! Norwegian folding rules, ported from
//! `org.apache.lucene.analysis.no.NorwegianNormalizationFilter`.

use std::collections::BTreeSet;

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::analysis::miscellaneous::scandinavian_normalizer::{Foldings, ScandinavianNormalizer};

/// The body of [`NorwegianNormalizationFilter`]: what Lucene's
/// `NorwegianNormalizationFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct NorwegianNormalizationFilterLogic {
    normalizer: ScandinavianNormalizer,
}

impl TokenFilterLogic for NorwegianNormalizationFilterLogic {
    /// Equivalent to `NorwegianNormalizationFilter.incrementToken()`.
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
        let length = char_term_attribute.length();
        let new_length = self
            .normalizer
            .process_token(char_term_attribute.buffer_mut(), length);
        char_term_attribute.set_length(new_length);
        Ok(true)
    }
}

/// This filter normalize use of the interchangeable Scandinavian characters
/// æÆäÄöÖøØ and folded variants (ae, oe, aa) by transforming them to
/// åÅæÆøØ. This is similar to ScandinavianNormalizationFilter, except for
/// the folding rules customized for Norwegian.
///
/// blåbærsyltetøj == blåbärsyltetöj == blaabaersyltetoej
///
/// * see: `ScandinavianNormalizationFilter`
///
/// Equivalent to `org.apache.lucene.analysis.no.NorwegianNormalizationFilter`.
pub type NorwegianNormalizationFilter = TokenFilter<NorwegianNormalizationFilterLogic>;

/// Equivalent to `NorwegianNormalizationFilter(TokenStream)`: the
/// constructor builds `new ScandinavianNormalizer(EnumSet.of(Foldings.AE,
/// Foldings.OE, Foldings.AA))`, the field adds the `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_norwegian_normalization_filter(
    input: Box<dyn TokenStream>,
) -> Result<NorwegianNormalizationFilter> {
    let mut filter = TokenFilter::new(
        input,
        NorwegianNormalizationFilterLogic {
            normalizer: ScandinavianNormalizer::new(BTreeSet::from([
                Foldings::AE,
                Foldings::OE,
                Foldings::AA,
            ])),
        },
    );
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
