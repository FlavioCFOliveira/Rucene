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

//! Normalize token text with ICU's `Normalizer2`, ported from
//! `org.apache.lucene.analysis.icu.ICUNormalizer2Filter`.

use std::sync::Arc;

use icu4j::text::normalizer::Normalizer;
use icu4j::text::normalizer2::{self, Mode, Normalizer2};
use rucene::analysis::character_utils::{set_term_utf16, term_to_utf16};
use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// Normalize token text with ICU's `com.ibm.icu.text.Normalizer2`.
///
/// With this filter, you can normalize text in the following ways:
///
/// * NFKC Normalization, Case Folding, and removing Ignorables (the default)
/// * Using a standard Normalization mode (NFC, NFD, NFKC, NFKD)
/// * Based on rules from a custom normalization mapping.
///
/// If you use the defaults, this filter is a simple way to standardize
/// Unicode text in a language-independent way for search:
///
/// * The case folding that it does can be seen as a replacement for
///   LowerCaseFilter: For example, it handles cases such as the Greek sigma,
///   so that "Μάϊος" and "ΜΆΪΟΣ" will match correctly.
/// * The normalization will standardizes different forms of the same
///   character in Unicode. For example, CJK full-width numbers will be
///   standardized to their ASCII forms.
/// * Ignorables such as Zero-Width Joiner and Variation Selectors are
///   removed. These are typically modifier characters that affect display.
///
/// See `com.ibm.icu.text.Normalizer2` and `com.ibm.icu.text.FilteredNormalizer2`.
///
/// Equivalent to the body of `org.apache.lucene.analysis.icu.ICUNormalizer2Filter`
/// (a `TokenFilter`): its fields and its final `incrementToken()`. The
/// filter is [`ICUNormalizer2Filter`].
///
/// **Divergence from Lucene 10.5.0 (inherited).** Java reads the term
/// buffer's `char`s (UTF-16 units) and writes the result back with
/// `termAtt.setEmpty().append(buffer)`; `rucene`'s term buffer holds Rust
/// `char`s, so the term is converted to UTF-16 units for the normalizer
/// ([`term_to_utf16`]) and the result decoded back ([`set_term_utf16`],
/// a lone surrogate becoming U+FFFD), the bridge every port over the term
/// buffer uses.
#[derive(Debug)]
pub struct ICUNormalizer2FilterLogic {
    normalizer: Arc<dyn Normalizer2>,
    /// `buffer`: the normalized text.
    buffer: Vec<u16>,
    /// The term's UTF-16 units, Java's `termAtt` as a `CharSequence`.
    units: Vec<u16>,
}

impl ICUNormalizer2FilterLogic {
    /// The normalizer this filter applies.
    pub fn normalizer(&self) -> &Arc<dyn Normalizer2> {
        &self.normalizer
    }
}

impl TokenFilterLogic for ICUNormalizer2FilterLogic {
    /// Equivalent to `ICUNormalizer2Filter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            term_to_utf16(&term_att.buffer()[..term_att.length()], &mut self.units);
            if self.normalizer.quick_check(&self.units)? != Normalizer::YES {
                self.buffer.clear();
                self.normalizer.normalize(&self.units, &mut self.buffer)?;
                set_term_utf16(&mut *term_att, &self.buffer);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Normalize token text with ICU's `Normalizer2`.
///
/// Equivalent to `org.apache.lucene.analysis.icu.ICUNormalizer2Filter`; see
/// [`ICUNormalizer2FilterLogic`].
pub type ICUNormalizer2Filter = TokenFilter<ICUNormalizer2FilterLogic>;

/// Create a new Normalizer2Filter that combines NFKC normalization, Case
/// Folding, and removes Default Ignorables (NFKC_Casefold).
///
/// Equivalent to `ICUNormalizer2Filter(TokenStream)`, over
/// `Normalizer2.getInstance(null, "nfkc_cf", Normalizer2.Mode.COMPOSE)`.
///
/// # Errors
///
/// The errors of loading the `nfkc_cf` data, and of adding the
/// `CharTermAttribute`.
pub fn new_icu_normalizer2_filter(input: Box<dyn TokenStream>) -> Result<ICUNormalizer2Filter> {
    let normalizer: Arc<dyn Normalizer2> =
        normalizer2::get_instance(None, "nfkc_cf", Mode::Compose)?;
    new_icu_normalizer2_filter_with(input, normalizer)
}

/// Create a new Normalizer2Filter with the specified Normalizer2.
///
/// * `input`: stream
/// * `normalizer`: normalizer to use
///
/// Equivalent to `ICUNormalizer2Filter(TokenStream, Normalizer2)`; Java's
/// `Objects.requireNonNull(normalizer, "normalizer")` has no counterpart,
/// an `Arc` being never null.
///
/// # Errors
///
/// What adding the `CharTermAttribute` raises
/// (`addAttribute(CharTermAttribute.class)`).
pub fn new_icu_normalizer2_filter_with(
    input: Box<dyn TokenStream>,
    normalizer: Arc<dyn Normalizer2>,
) -> Result<ICUNormalizer2Filter> {
    let mut filter = TokenFilter::new(
        input,
        ICUNormalizer2FilterLogic {
            normalizer,
            buffer: Vec::new(),
            units: Vec::new(),
        },
    );
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
