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

//! A tokenizer that divides text at Unicode whitespace, ported from
//! `org.apache.lucene.analysis.core.UnicodeWhitespaceTokenizer`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use crate::analysis::util::char_tokenizer::{
    new_char_tokenizer, new_char_tokenizer_with_factory,
    new_char_tokenizer_with_factory_and_max_token_len, CharTokenizer, CharTokenizerLogic,
};
use crate::analysis::util::unicode_props::UnicodeProps;

/// The `isTokenChar` of [`UnicodeWhitespaceTokenizer`].
#[derive(Clone, Copy, Debug, Default)]
pub struct UnicodeWhitespaceTokenizerLogic;

impl CharTokenizerLogic for UnicodeWhitespaceTokenizerLogic {
    /// Collects only characters which do not satisfy Unicode's WHITESPACE property.
    ///
    /// Equivalent to `UnicodeWhitespaceTokenizer.isTokenChar(int)`.
    fn is_token_char(&self, c: i32) -> bool {
        !UnicodeProps::whitespace().get(c as usize)
    }
}

/// A UnicodeWhitespaceTokenizer is a tokenizer that divides text at
/// whitespace. Adjacent sequences of non-Whitespace characters form tokens
/// (according to Unicode's WHITESPACE property).
///
/// *For Unicode version see: `UnicodeProps`*
///
/// Equivalent to `org.apache.lucene.analysis.core.UnicodeWhitespaceTokenizer`,
/// a `CharTokenizer` whose `isTokenChar` is
/// [`UnicodeWhitespaceTokenizerLogic`].
pub type UnicodeWhitespaceTokenizer = CharTokenizer<UnicodeWhitespaceTokenizerLogic>;

/// Construct a new UnicodeWhitespaceTokenizer.
///
/// Equivalent to `UnicodeWhitespaceTokenizer()`.
pub fn new_unicode_whitespace_tokenizer() -> UnicodeWhitespaceTokenizer {
    new_char_tokenizer(UnicodeWhitespaceTokenizerLogic)
}

/// Construct a new UnicodeWhitespaceTokenizer using a given `AttributeFactory`.
///
/// Equivalent to `UnicodeWhitespaceTokenizer(AttributeFactory)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws.
pub fn new_unicode_whitespace_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
) -> Result<UnicodeWhitespaceTokenizer> {
    new_char_tokenizer_with_factory(factory, UnicodeWhitespaceTokenizerLogic)
}

/// Construct a new UnicodeWhitespaceTokenizer using a given `AttributeFactory`: `max_token_len` is
/// the maximum token length the tokenizer will emit. Must be greater than 0
/// and less than `MAX_TOKEN_LENGTH_LIMIT` (1024*1024).
///
/// Equivalent to `UnicodeWhitespaceTokenizer(AttributeFactory, int)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, and
/// `IllegalArgumentException` if `max_token_len` is invalid.
pub fn new_unicode_whitespace_tokenizer_with_factory_and_max_token_len(
    factory: Arc<dyn AttributeFactory>,
    max_token_len: i32,
) -> Result<UnicodeWhitespaceTokenizer> {
    new_char_tokenizer_with_factory_and_max_token_len(
        factory,
        max_token_len,
        UnicodeWhitespaceTokenizerLogic,
    )
}
