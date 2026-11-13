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
//! Skips the wrapped filters for protected terms, ported from
//! `org.apache.lucene.analysis.miscellaneous.ProtectedTermFilter`.

use rucene::analysis::{CharArraySet, CharTermAttribute, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::conditional_token_filter::{
    new_conditional_token_filter, ConditionalTokenFilter, ConditionalTokenFilterLogic,
};
use crate::jdk::character::chars_to_utf16;

/// The `shouldFilter()` of [`ProtectedTermFilter`].
#[derive(Debug)]
pub struct ProtectedTermFilterLogic {
    /// `None` is Java's `null`, which the constructor accepts.
    protected_terms: Option<CharArraySet>,
}

impl ConditionalTokenFilterLogic for ProtectedTermFilterLogic {
    /// Equivalent to `ProtectedTermFilter.shouldFilter()`: the term is
    /// filtered unless `protectedTerms` contains it (over its UTF-16 units,
    /// the set's keys).
    ///
    /// # Panics
    ///
    /// With the `NullPointerException` Java throws when the set is `null`.
    fn should_filter(&mut self, source: &AttributeSource) -> Result<bool> {
        let Some(protected_terms) = &self.protected_terms else {
            rucene::store::java_nio_file::throw_unchecked("java.lang.NullPointerException", None)
        };
        let term_att = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let units = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
        let b = protected_terms.contains_chars(Some(&units), 0, units.len());
        Ok(!b)
    }
}

/// A `ConditionalTokenFilter` that only applies its wrapped filters to
/// tokens that are not contained in a protected set.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ProtectedTermFilter`.
pub type ProtectedTermFilter = ConditionalTokenFilter<ProtectedTermFilterLogic>;

/// Creates a new ProtectedTermFilter: `protected_terms` the set of terms to
/// skip the wrapped filters for (`None` for Java's `null`), `input` the input TokenStream, and
/// `input_factory` a factory function to create the wrapped filter(s).
///
/// Equivalent to `ProtectedTermFilter(CharArraySet, TokenStream,
/// Function<TokenStream, TokenStream>)`, whose field initializer adds the
/// `CharTermAttribute` after `super(input, inputFactory)`.
///
/// # Errors
///
/// What [`new_conditional_token_filter`] raises, and what the attribute
/// factory raises.
pub fn new_protected_term_filter<F>(
    protected_terms: Option<CharArraySet>,
    input: Box<dyn TokenStream>,
    input_factory: F,
) -> Result<ProtectedTermFilter>
where
    F: FnOnce(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>,
{
    let mut filter = new_conditional_token_filter(
        input,
        input_factory,
        ProtectedTermFilterLogic { protected_terms },
    )?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
