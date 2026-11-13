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

//! Normalizes tokens extracted with `ClassicTokenizer`, ported from
//! `org.apache.lucene.analysis.classic.ClassicFilter`.

use rucene::analysis::{
    CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::classic_tokenizer::{ACRONYM, APOSTROPHE, TOKEN_TYPES};

/// Equivalent to `ClassicFilter.APOSTROPHE_TYPE`,
/// `ClassicTokenizer.TOKEN_TYPES[ClassicTokenizer.APOSTROPHE]`.
fn apostrophe_type() -> &'static str {
    TOKEN_TYPES[APOSTROPHE as usize]
}

/// Equivalent to `ClassicFilter.ACRONYM_TYPE`,
/// `ClassicTokenizer.TOKEN_TYPES[ClassicTokenizer.ACRONYM]`.
fn acronym_type() -> &'static str {
    TOKEN_TYPES[ACRONYM as usize]
}

/// Java's `type == CONSTANT`: whether `type_value` is the very string
/// `constant`, compared by reference as Java compares `String`s with `==`.
///
/// **Divergence from Lucene 10.5.0.** Java interns every string literal, so a
/// type set from any `"<APOSTROPHE>"` literal of any class is the same object
/// as `ClassicTokenizer.TOKEN_TYPES[APOSTROPHE]`; Rust does not intern, so only
/// a type set from the element of [`TOKEN_TYPES`] itself (as `ClassicTokenizer`
/// sets it, and as attribute copies keep it) is that string here. A type built
/// at run time is a different object in both languages.
fn is_same_string(type_value: &str, constant: &'static str) -> bool {
    std::ptr::eq(type_value.as_ptr(), constant.as_ptr()) && type_value.len() == constant.len()
}

/// The body of [`ClassicFilter`]: what Lucene's `ClassicFilter` adds to
/// `TokenFilter`.
#[derive(Debug)]
pub struct ClassicFilterLogic;

impl TokenFilterLogic for ClassicFilterLogic {
    /// Returns the next token in the stream, or null at EOS.
    ///
    /// Removes `'s` from the end of words.
    ///
    /// Removes dots from acronyms.
    ///
    /// Equivalent to `ClassicFilter.incrementToken()`. Java tests the last two
    /// UTF-16 units of the term and removes `'.'` units; the characters it
    /// looks for are of the Basic Multilingual Plane and not surrogates, so the
    /// test and the removal read the term's `char`s with the same result.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }

        // The type is read first, and its attribute released, before the term
        // is borrowed: one attribute implementation may stand for both
        // interfaces (`PackedTokenAttributeImpl`).
        let (is_apostrophe_type, is_acronym_type) = {
            let type_att = source
                .get_attribute_interface_mut::<dyn TypeAttribute>()
                .expect("INVARIANT: the constructor added the TypeAttribute");
            let type_value = type_att.type_value();
            (
                is_same_string(type_value, apostrophe_type()),
                is_same_string(type_value, acronym_type()),
            )
        };
        let mut term_att = source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let buffer_length = term_att.length();

        // remove 's
        if is_apostrophe_type
            && buffer_length >= 2
            && term_att.buffer()[buffer_length - 2] == '\''
            && (term_att.buffer()[buffer_length - 1] == 's'
                || term_att.buffer()[buffer_length - 1] == 'S')
        {
            // Strip last 2 characters off
            term_att.set_length(buffer_length - 2);
        } else if is_acronym_type {
            // remove dots
            let buffer = term_att.buffer_mut();
            let mut upto = 0;
            for i in 0..buffer_length {
                let c = buffer[i];
                if c != '.' {
                    buffer[upto] = c;
                    upto += 1;
                }
            }
            term_att.set_length(upto);
        }

        Ok(true)
    }
}

/// Normalizes tokens extracted with `ClassicTokenizer`.
///
/// Equivalent to `org.apache.lucene.analysis.classic.ClassicFilter`, a
/// [`TokenFilter`] whose body is a [`ClassicFilterLogic`].
pub type ClassicFilter = TokenFilter<ClassicFilterLogic>;

/// Construct filtering `input`.
///
/// Equivalent to `ClassicFilter(TokenStream)`, whose fields add the
/// `TypeAttribute` and the `CharTermAttribute`, in that order.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_classic_filter(input: Box<dyn TokenStream>) -> Result<ClassicFilter> {
    let mut filter = TokenFilter::new(input, ClassicFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn TypeAttribute>()?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
