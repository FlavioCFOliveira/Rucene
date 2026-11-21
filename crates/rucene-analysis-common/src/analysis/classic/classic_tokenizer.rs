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

//! A grammar-based tokenizer constructed with JFlex, ported from
//! `org.apache.lucene.analysis.classic.ClassicTokenizer`.

use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute,
    PositionIncrementAttribute, Tokenizer, TokenizerLogic, TypeAttribute, DEFAULT_MAX_TOKEN_LENGTH,
};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefMut;
use rucene::util::attribute::{AttributeFactory, AttributeInterface, AttributeSource};

use super::classic_tokenizer_impl::{self, ClassicTokenizerImpl};

/// Equivalent to `ClassicTokenizer.ALPHANUM`.
pub const ALPHANUM: i32 = 0;
/// Equivalent to `ClassicTokenizer.APOSTROPHE`.
pub const APOSTROPHE: i32 = 1;
/// Equivalent to `ClassicTokenizer.ACRONYM`.
pub const ACRONYM: i32 = 2;
/// Equivalent to `ClassicTokenizer.COMPANY`.
pub const COMPANY: i32 = 3;
/// Equivalent to `ClassicTokenizer.EMAIL`.
pub const EMAIL: i32 = 4;
/// Equivalent to `ClassicTokenizer.HOST`.
pub const HOST: i32 = 5;
/// Equivalent to `ClassicTokenizer.NUM`.
pub const NUM: i32 = 6;
/// Equivalent to `ClassicTokenizer.CJ`.
pub const CJ: i32 = 7;

/// Equivalent to `ClassicTokenizer.ACRONYM_DEP`.
pub const ACRONYM_DEP: i32 = 8;

/// String token types that correspond to token type int constants
///
/// Equivalent to `ClassicTokenizer.TOKEN_TYPES`. A `static`, so that each
/// element is one string at one address: `ClassicFilter` compares a token's
/// type with these elements by reference, as Java's `==` does.
pub static TOKEN_TYPES: [&str; 9] = [
    "<ALPHANUM>",
    "<APOSTROPHE>",
    "<ACRONYM>",
    "<COMPANY>",
    "<EMAIL>",
    "<HOST>",
    "<NUM>",
    "<CJ>",
    "<ACRONYM_DEP>",
];

/// The body of [`ClassicTokenizer`]: what Lucene's `ClassicTokenizer` adds to
/// `Tokenizer`.
///
/// Equivalent to the members of `org.apache.lucene.analysis.classic.ClassicTokenizer`;
/// the `Tokenizer` half is [`Tokenizer`].
#[derive(Debug)]
pub struct ClassicTokenizerLogic {
    /// A private instance of the JFlex-constructed scanner
    scanner: ClassicTokenizerImpl,
    skipped_positions: i32,
    max_token_length: i32,
}

impl Default for ClassicTokenizerLogic {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassicTokenizerLogic {
    /// Creates the tokenizer body with the default maximum token length,
    /// `StandardAnalyzer.DEFAULT_MAX_TOKEN_LENGTH`.
    ///
    /// Equivalent to the field initializers and `ClassicTokenizer.init()`,
    /// `this.scanner = new ClassicTokenizerImpl(input)`.
    pub fn new() -> Self {
        Self {
            scanner: ClassicTokenizerImpl::new(),
            skipped_positions: 0,
            max_token_length: DEFAULT_MAX_TOKEN_LENGTH as i32,
        }
    }

    /// Set the max allowed token length. Any token longer than this is skipped.
    ///
    /// Equivalent to `ClassicTokenizer.setMaxTokenLength(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException`, `maxTokenLength must be greater than zero`,
    /// for a length below 1.
    pub fn set_max_token_length(&mut self, length: i32) -> Result<()> {
        if length < 1 {
            return Err(LuceneError::illegal_argument(
                "maxTokenLength must be greater than zero".to_string(),
            ));
        }
        self.max_token_length = length;
        Ok(())
    }

    /// Equivalent to `ClassicTokenizer.getMaxTokenLength()`.
    pub fn get_max_token_length(&self) -> i32 {
        self.max_token_length
    }
}

/// The attribute interface `I` of a `ClassicTokenizer`'s source, which
/// [`classic_tokenizer_source`] added.
fn interface_mut<I: AttributeInterface + ?Sized>(source: &AttributeSource) -> AtomicRefMut<'_, I> {
    source
        .get_attribute_interface_mut::<I>()
        .expect("INVARIANT: classic_tokenizer_source added the attribute")
}

impl TokenizerLogic for ClassicTokenizerLogic {
    /// Equivalent to `ClassicTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        self.skipped_positions = 0;

        loop {
            let token_type = self.scanner.get_next_token(reader)?;

            if token_type == classic_tokenizer_impl::YYEOF {
                return Ok(false);
            }

            if self.scanner.yylength() <= self.max_token_length {
                interface_mut::<dyn PositionIncrementAttribute>(source)
                    .set_position_increment(self.skipped_positions + 1);
                self.scanner
                    .get_text(&mut *interface_mut::<dyn CharTermAttribute>(source));
                let start = self.scanner.yychar();
                // `correctOffset(start + termAtt.length())`: the term's length
                // in UTF-16 code units, `yylength()`, as Java's `char` length
                // is; the attribute here counts characters.
                let input: &dyn CharReader = reader;
                let correct =
                    |offset: i32| Tokenizer::<Self>::correct_offset_of(Some(input), offset);
                interface_mut::<dyn OffsetAttribute>(source)
                    .set_offset(correct(start), correct(start + self.scanner.yylength()));

                if token_type == ACRONYM_DEP {
                    interface_mut::<dyn TypeAttribute>(source)
                        .set_type_static(TOKEN_TYPES[HOST as usize]);
                    let mut term_att = interface_mut::<dyn CharTermAttribute>(source);
                    let length = term_att.length();
                    term_att.set_length(length - 1); // remove extra '.'
                } else {
                    interface_mut::<dyn TypeAttribute>(source)
                        .set_type_static(TOKEN_TYPES[token_type as usize]);
                }
                return Ok(true);
            } else {
                // When we skip a too-long term, we still increment the
                // position increment
                self.skipped_positions += 1;
            }
        }
    }

    /// Equivalent to `ClassicTokenizer.end()`; `Tokenizer::end` has run
    /// `super.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        // set final offset
        let final_offset = Tokenizer::<Self>::correct_offset_of(
            input,
            self.scanner.yychar() + self.scanner.yylength(),
        );
        interface_mut::<dyn OffsetAttribute>(source).set_offset(final_offset, final_offset);
        // adjust any skipped tokens
        let mut pos_incr_att = interface_mut::<dyn PositionIncrementAttribute>(source);
        let current = pos_incr_att.get_position_increment();
        pos_incr_att.set_position_increment(current + self.skipped_positions);
        Ok(())
    }

    /// Equivalent to `ClassicTokenizer.close()`; `Tokenizer::close` has run
    /// `super.close()`, and `scanner.yyreset(input)` takes the reader at the
    /// next reading call.
    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.yyreset();
        Ok(())
    }

    /// Equivalent to `ClassicTokenizer.reset()`; `Tokenizer::reset` has run
    /// `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.yyreset();
        self.skipped_positions = 0;
        Ok(())
    }
}

/// A grammar based tokenizer constructed with JFlex
///
/// This should be a good tokenizer for most European-language documents:
///
/// * Splits words at punctuation characters, removing punctuation. However, a
///   dot that's not followed by whitespace is considered part of a token.
/// * Splits words at hyphens, unless there's a number in the token, in which
///   case the whole token is interpreted as a product number and is not
///   split.
/// * Recognizes email addresses and internet hostnames as one token.
///
/// Many applications have specific tokenizer needs. If this tokenizer does
/// not suit your application, please consider copying this source code
/// directory to your project and maintaining your own grammar-based
/// tokenizer.
///
/// ClassicTokenizer was named StandardTokenizer in Lucene versions prior to
/// 3.1. As of 3.1, `StandardTokenizer` implements Unicode text segmentation,
/// as specified by UAX#29.
///
/// Equivalent to `org.apache.lucene.analysis.classic.ClassicTokenizer`, a
/// [`Tokenizer`] whose body is a [`ClassicTokenizerLogic`].
pub type ClassicTokenizer = Tokenizer<ClassicTokenizerLogic>;

/// Creates a new instance of the `ClassicTokenizer`. Attaches the `input` to
/// the newly created JFlex scanner.
///
/// See <http://issues.apache.org/jira/browse/LUCENE-1068>
///
/// Equivalent to `ClassicTokenizer()`, whose attributes come from
/// `TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY`.
pub fn new_classic_tokenizer() -> ClassicTokenizer {
    new_classic_tokenizer_with_factory(default_token_attribute_factory())
        .expect("INVARIANT: the default token attribute factory implements the core attributes")
}

/// Creates a new ClassicTokenizer with a given `AttributeFactory`.
///
/// Equivalent to `ClassicTokenizer(AttributeFactory)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws.
pub fn new_classic_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
) -> Result<ClassicTokenizer> {
    Ok(Tokenizer::with_source(
        classic_tokenizer_source(factory)?,
        ClassicTokenizerLogic::new(),
    ))
}

/// The source of a `ClassicTokenizer` over `factory`, holding the attributes
/// its fields add, in their declaration order: `CharTermAttribute`,
/// `OffsetAttribute`, `PositionIncrementAttribute`, `TypeAttribute`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement.
pub(crate) fn classic_tokenizer_source(
    factory: Arc<dyn AttributeFactory>,
) -> Result<AttributeSource> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(source)
}
