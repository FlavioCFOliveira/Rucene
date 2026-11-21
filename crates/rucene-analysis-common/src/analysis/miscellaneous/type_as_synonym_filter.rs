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
//! Adds the token type as a synonym, ported from
//! `org.apache.lucene.analysis.miscellaneous.TypeAsSynonymFilter`.

use std::collections::HashSet;

use rucene::analysis::{
    CharTermAttribute, FlagsAttribute, PositionIncrementAttribute, TokenFilter, TokenFilterLogic,
    TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};

/// The body of [`TypeAsSynonymFilter`]: what Lucene's `TypeAsSynonymFilter`
/// adds to `TokenFilter`.
#[derive(Debug)]
pub struct TypeAsSynonymFilterLogic {
    prefix: Option<String>,
    ignore: Option<HashSet<String>>,
    syn_flags_mask: i32,
    saved_token: Option<CapturedState>,
}

impl TokenFilterLogic for TypeAsSynonymFilterLogic {
    /// Equivalent to `TypeAsSynonymFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if let Some(saved_token) = self.saved_token.take() {
            // Emit last token's type at the same position
            source.restore_state(&saved_token)?;
            let token_type = source
                .get_attribute_interface::<dyn TypeAttribute>()
                .expect("INVARIANT: the constructor added the TypeAttribute")
                .type_value()
                .to_string();
            {
                let mut term_att = source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                term_att.set_empty();
                if let Some(prefix) = &self.prefix {
                    term_att.append_string(Some(prefix));
                }
                term_att.append_string(Some(&token_type));
            }
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(0);
            // control what flags transfer to synonym
            let mut flags_att = source
                .get_attribute_interface_mut::<dyn FlagsAttribute>()
                .expect("INVARIANT: the constructor added the FlagsAttribute");
            let flags = flags_att.get_flags();
            flags_att.set_flags(flags & self.syn_flags_mask);
            return Ok(true);
        } else if input.increment_token()? {
            // No pending token type to emit
            let ignored = {
                let type_att = source
                    .get_attribute_interface::<dyn TypeAttribute>()
                    .expect("INVARIANT: the constructor added the TypeAttribute");
                self.ignore
                    .as_ref()
                    .is_some_and(|ignore| ignore.contains(type_att.type_value()))
            };
            if !ignored {
                self.saved_token = source.capture_state();
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Equivalent to `TypeAsSynonymFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.saved_token = None;
        Ok(())
    }
}

/// Adds the `TypeAttribute.type()` as a synonym, i.e. another token at the
/// same position, optionally with a specified prefix prepended, optionally
/// transfering flags, and optionally ignoring some types.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.TypeAsSynonymFilter`.
pub type TypeAsSynonymFilter = TokenFilter<TypeAsSynonymFilterLogic>;

/// Equivalent to `TypeAsSynonymFilter(TokenStream)`, `this(input, null,
/// null, ~0)`.
///
/// # Errors
///
/// As [`new_type_as_synonym_filter_with`].
pub fn new_type_as_synonym_filter(input: Box<dyn TokenStream>) -> Result<TypeAsSynonymFilter> {
    new_type_as_synonym_filter_with(input, None, None, !0)
}

/// `prefix` is prepended to the type to form the synonym.
///
/// Equivalent to `TypeAsSynonymFilter(TokenStream, String)`, `this(input,
/// prefix, null, ~0)`.
///
/// # Errors
///
/// As [`new_type_as_synonym_filter_with`].
pub fn new_type_as_synonym_filter_with_prefix(
    input: Box<dyn TokenStream>,
    prefix: Option<String>,
) -> Result<TypeAsSynonymFilter> {
    new_type_as_synonym_filter_with(input, prefix, None, !0)
}

/// `input` is the input stream, `prefix` the prepended to the type to form
/// the synonym, `ignore` the types that should not be emitted as synonyms,
/// and `syn_flags_mask` a mask to control what flags are propagated to the
/// synonym.
///
/// Equivalent to `TypeAsSynonymFilter(TokenStream, String, Set<String>,
/// int)`, which adds the `CharTermAttribute`, `TypeAttribute`,
/// `PositionIncrementAttribute` and `FlagsAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_type_as_synonym_filter_with(
    input: Box<dyn TokenStream>,
    prefix: Option<String>,
    ignore: Option<HashSet<String>>,
    syn_flags_mask: i32,
) -> Result<TypeAsSynonymFilter> {
    let mut filter = TokenFilter::new(
        input,
        TypeAsSynonymFilterLogic {
            prefix,
            ignore,
            syn_flags_mask,
            saved_token: None,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn FlagsAttribute>()?;
    Ok(filter)
}
