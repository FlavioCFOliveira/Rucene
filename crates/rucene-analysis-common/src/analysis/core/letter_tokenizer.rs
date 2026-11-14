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

//! A tokenizer that divides text at non-letters, ported from
//! `org.apache.lucene.analysis.core.LetterTokenizer`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use crate::analysis::util::char_tokenizer::{
    new_char_tokenizer, new_char_tokenizer_with_factory,
    new_char_tokenizer_with_factory_and_max_token_len, CharTokenizer, CharTokenizerLogic,
};
use crate::jdk::character::is_letter;

/// The `isTokenChar` of [`LetterTokenizer`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LetterTokenizerLogic;

impl CharTokenizerLogic for LetterTokenizerLogic {
    /// Collects only characters which satisfy `Character.isLetter(int)`.
    ///
    /// Equivalent to `LetterTokenizer.isTokenChar(int)`.
    fn is_token_char(&self, c: i32) -> bool {
        is_letter(c)
    }
}

/// A LetterTokenizer is a tokenizer that divides text at non-letters. That's
/// to say, it defines tokens as maximal strings of adjacent letters, as
/// defined by java.lang.Character.isLetter() predicate.
///
/// Note: this does a decent job for most European languages, but does a
/// terrible job for some Asian languages, where words are not separated by
/// spaces.
///
/// Equivalent to `org.apache.lucene.analysis.core.LetterTokenizer`, a
/// `CharTokenizer` whose `isTokenChar` is [`LetterTokenizerLogic`].
pub type LetterTokenizer = CharTokenizer<LetterTokenizerLogic>;

/// Construct a new LetterTokenizer.
///
/// Equivalent to `LetterTokenizer()`.
pub fn new_letter_tokenizer() -> LetterTokenizer {
    new_char_tokenizer(LetterTokenizerLogic)
}

/// Construct a new LetterTokenizer using a given `AttributeFactory`.
///
/// Equivalent to `LetterTokenizer(AttributeFactory)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws.
pub fn new_letter_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
) -> Result<LetterTokenizer> {
    new_char_tokenizer_with_factory(factory, LetterTokenizerLogic)
}

/// Construct a new LetterTokenizer using a given `AttributeFactory`: `max_token_len` is
/// the maximum token length the tokenizer will emit. Must be greater than 0
/// and less than `MAX_TOKEN_LENGTH_LIMIT` (1024*1024).
///
/// Equivalent to `LetterTokenizer(AttributeFactory, int)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, and
/// `IllegalArgumentException` if `max_token_len` is invalid.
pub fn new_letter_tokenizer_with_factory_and_max_token_len(
    factory: Arc<dyn AttributeFactory>,
    max_token_len: i32,
) -> Result<LetterTokenizer> {
    new_char_tokenizer_with_factory_and_max_token_len(factory, max_token_len, LetterTokenizerLogic)
}
