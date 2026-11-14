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
//! Normalizes the interchangeable Scandinavian characters, ported from
//! `org.apache.lucene.analysis.miscellaneous.ScandinavianNormalizationFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::scandinavian_normalizer::{ScandinavianNormalizer, ALL_FOLDINGS};

/// The body of [`ScandinavianNormalizationFilter`]: what Lucene's
/// `ScandinavianNormalizationFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct ScandinavianNormalizationFilterLogic {
    normalizer: ScandinavianNormalizer,
}

impl TokenFilterLogic for ScandinavianNormalizationFilterLogic {
    /// Equivalent to `ScandinavianNormalizationFilter.incrementToken()`.
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
/// æÆäÄöÖøØ and folded variants (aa, ao, ae, oe and oo) by transforming them
/// to åÅæÆøØ.
///
/// It's a semantically less destructive solution than
/// `ScandinavianFoldingFilter`, most useful when a person with a Norwegian
/// or Danish keyboard queries a Swedish index and vice versa. This filter
/// does **not** the common Swedish folds of å and ä to a nor ö to o.
///
/// blåbærsyltetøj == blåbärsyltetöj == blaabaarsyltetoej but not ==
/// blabarsyltetoj; räksmörgås == ræksmørgås == ræksmörgaos == raeksmoergaas
/// but not == raksmorgas.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ScandinavianNormalizationFilter`.
pub type ScandinavianNormalizationFilter = TokenFilter<ScandinavianNormalizationFilterLogic>;

/// Equivalent to `ScandinavianNormalizationFilter(TokenStream)`: a
/// `ScandinavianNormalizer` with `ALL_FOLDINGS`; it adds the
/// `CharTermAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_scandinavian_normalization_filter(
    input: Box<dyn TokenStream>,
) -> Result<ScandinavianNormalizationFilter> {
    let mut filter = TokenFilter::new(
        input,
        ScandinavianNormalizationFilterLogic {
            normalizer: ScandinavianNormalizer::new(ALL_FOLDINGS.clone()),
        },
    );
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
