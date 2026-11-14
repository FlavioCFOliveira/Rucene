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

//! Tokenizer for URLs, email addresses and the Word Break rules of UAX#29,
//! ported from `org.apache.lucene.analysis.email.UAX29URLEmailTokenizer`.

use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute,
    PositionIncrementAttribute, Tokenizer, TokenizerLogic, TypeAttribute, DEFAULT_MAX_TOKEN_LENGTH,
};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefMut;
use rucene::util::attribute::{AttributeFactory, AttributeInterface, AttributeSource};

use super::uax29_url_email_tokenizer_impl::{UAX29URLEmailTokenizerImpl, YYEOF};

/// Alpha/numeric token type
///
/// Equivalent to `UAX29URLEmailTokenizer.ALPHANUM`.
pub const ALPHANUM: i32 = 0;
/// Numeric token type
///
/// Equivalent to `UAX29URLEmailTokenizer.NUM`.
pub const NUM: i32 = 1;
/// Southeast Asian token type
///
/// Equivalent to `UAX29URLEmailTokenizer.SOUTHEAST_ASIAN`.
pub const SOUTHEAST_ASIAN: i32 = 2;
/// Ideographic token type
///
/// Equivalent to `UAX29URLEmailTokenizer.IDEOGRAPHIC`.
pub const IDEOGRAPHIC: i32 = 3;
/// Hiragana token type
///
/// Equivalent to `UAX29URLEmailTokenizer.HIRAGANA`.
pub const HIRAGANA: i32 = 4;
/// Katakana token type
///
/// Equivalent to `UAX29URLEmailTokenizer.KATAKANA`.
pub const KATAKANA: i32 = 5;
/// Hangul token type
///
/// Equivalent to `UAX29URLEmailTokenizer.HANGUL`.
pub const HANGUL: i32 = 6;
/// URL token type
///
/// Equivalent to `UAX29URLEmailTokenizer.URL`.
pub const URL: i32 = 7;
/// Email token type
///
/// Equivalent to `UAX29URLEmailTokenizer.EMAIL`.
pub const EMAIL: i32 = 8;
/// Emoji token type.
///
/// Equivalent to `UAX29URLEmailTokenizer.EMOJI`.
pub const EMOJI: i32 = 9;

/// String token types that correspond to token type int constants
///
/// Equivalent to `UAX29URLEmailTokenizer.TOKEN_TYPES`, whose elements but
/// `<URL>` and `<EMAIL>` are those of `StandardTokenizer.TOKEN_TYPES`.
pub static TOKEN_TYPES: [&str; 10] = [
    rucene::analysis::TOKEN_TYPES[rucene::analysis::ALPHANUM],
    rucene::analysis::TOKEN_TYPES[rucene::analysis::NUM],
    rucene::analysis::TOKEN_TYPES[rucene::analysis::SOUTHEAST_ASIAN],
    rucene::analysis::TOKEN_TYPES[rucene::analysis::IDEOGRAPHIC],
    rucene::analysis::TOKEN_TYPES[rucene::analysis::HIRAGANA],
    rucene::analysis::TOKEN_TYPES[rucene::analysis::KATAKANA],
    rucene::analysis::TOKEN_TYPES[rucene::analysis::HANGUL],
    "<URL>",
    "<EMAIL>",
    rucene::analysis::TOKEN_TYPES[rucene::analysis::EMOJI],
];

/// Absolute maximum sized token
///
/// Equivalent to `UAX29URLEmailTokenizer.MAX_TOKEN_LENGTH_LIMIT`.
pub const MAX_TOKEN_LENGTH_LIMIT: i32 = 1024 * 1024;

/// The body of [`UAX29URLEmailTokenizer`]: what Lucene's
/// `UAX29URLEmailTokenizer` adds to `Tokenizer`.
///
/// Equivalent to the members of
/// `org.apache.lucene.analysis.email.UAX29URLEmailTokenizer`; the `Tokenizer`
/// half is [`Tokenizer`].
#[derive(Debug)]
pub struct UAX29URLEmailTokenizerLogic {
    /// A private instance of the JFlex-constructed scanner
    scanner: UAX29URLEmailTokenizerImpl,
    skipped_positions: i32,
    max_token_length: i32,
}

impl Default for UAX29URLEmailTokenizerLogic {
    fn default() -> Self {
        Self::new()
    }
}

impl UAX29URLEmailTokenizerLogic {
    /// Creates the tokenizer body: a newly created JFlex scanner and the
    /// default maximum token length, `StandardAnalyzer.DEFAULT_MAX_TOKEN_LENGTH`.
    ///
    /// Equivalent to the field initializers and `getScanner()`, `new
    /// UAX29URLEmailTokenizerImpl(input)`.
    pub fn new() -> Self {
        Self {
            scanner: UAX29URLEmailTokenizerImpl::new(),
            skipped_positions: 0,
            max_token_length: DEFAULT_MAX_TOKEN_LENGTH as i32,
        }
    }

    /// Set the max allowed token length. Tokens larger than this will be
    /// chopped up at this token length and emitted as multiple tokens. If you
    /// need to skip such large tokens, you could increase this max length, and
    /// then use `LengthFilter` to remove long tokens. The default is
    /// `UAX29URLEmailAnalyzer.DEFAULT_MAX_TOKEN_LENGTH`.
    ///
    /// Equivalent to `UAX29URLEmailTokenizer.setMaxTokenLength(int)`: a new
    /// length resizes the scanner's buffer.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if the given length is outside of the range
    /// [1, [`MAX_TOKEN_LENGTH_LIMIT`]], with Java's messages.
    pub fn set_max_token_length(&mut self, length: i32) -> Result<()> {
        if length < 1 {
            return Err(LuceneError::illegal_argument(
                "maxTokenLength must be greater than zero".to_string(),
            ));
        } else if length > MAX_TOKEN_LENGTH_LIMIT {
            return Err(LuceneError::illegal_argument(format!(
                "maxTokenLength may not exceed {MAX_TOKEN_LENGTH_LIMIT}"
            )));
        }
        if length != self.max_token_length {
            self.max_token_length = length;
            self.scanner.set_buffer_size(length as usize);
        }
        Ok(())
    }

    /// Equivalent to `UAX29URLEmailTokenizer.getMaxTokenLength()`.
    pub fn get_max_token_length(&self) -> i32 {
        self.max_token_length
    }
}

/// The attribute interface `I` of a `UAX29URLEmailTokenizer`'s source, which
/// [`uax29_url_email_tokenizer_source`] added.
fn interface_mut<I: AttributeInterface + ?Sized>(source: &AttributeSource) -> AtomicRefMut<'_, I> {
    source
        .get_attribute_interface_mut::<I>()
        .expect("INVARIANT: uax29_url_email_tokenizer_source added the attribute")
}

impl TokenizerLogic for UAX29URLEmailTokenizerLogic {
    /// Equivalent to `UAX29URLEmailTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        self.skipped_positions = 0;

        loop {
            let token_type = self.scanner.get_next_token(reader)?;

            if token_type == YYEOF {
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
                interface_mut::<dyn TypeAttribute>(source)
                    .set_type_static(TOKEN_TYPES[token_type as usize]);
                return Ok(true);
            } else {
                // When we skip a too-long term, we still increment the
                // position increment
                self.skipped_positions += 1;
            }
        }
    }

    /// Equivalent to `UAX29URLEmailTokenizer.end()`; `Tokenizer::end` has run
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

    /// Equivalent to `UAX29URLEmailTokenizer.close()`; `Tokenizer::close` has
    /// run `super.close()`.
    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.yyreset();
        Ok(())
    }

    /// Equivalent to `UAX29URLEmailTokenizer.reset()`; `Tokenizer::reset` has
    /// run `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.yyreset();
        self.skipped_positions = 0;
        Ok(())
    }
}

/// This class implements Word Break rules from the Unicode Text Segmentation
/// algorithm, as specified in
/// [Unicode Standard Annex #29](http://unicode.org/reports/tr29/) URLs and
/// email addresses are also tokenized according to the relevant RFCs.
///
/// Equivalent to `org.apache.lucene.analysis.email.UAX29URLEmailTokenizer`,
/// a [`Tokenizer`] whose body is a [`UAX29URLEmailTokenizerLogic`].
pub type UAX29URLEmailTokenizer = Tokenizer<UAX29URLEmailTokenizerLogic>;

/// Creates a new instance of the UAX29URLEmailTokenizer. Attaches the
/// `input` to the newly created JFlex scanner.
///
/// Equivalent to `UAX29URLEmailTokenizer()`.
pub fn new_uax29_url_email_tokenizer() -> UAX29URLEmailTokenizer {
    new_uax29_url_email_tokenizer_with_factory(default_token_attribute_factory())
        .expect("INVARIANT: the default token attribute factory implements the core attributes")
}

/// Creates a new UAX29URLEmailTokenizer with a given `AttributeFactory`
///
/// Equivalent to `UAX29URLEmailTokenizer(AttributeFactory)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws.
pub fn new_uax29_url_email_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
) -> Result<UAX29URLEmailTokenizer> {
    Ok(Tokenizer::with_source(
        uax29_url_email_tokenizer_source(factory)?,
        UAX29URLEmailTokenizerLogic::new(),
    ))
}

/// The source of a `UAX29URLEmailTokenizer` over `factory`, holding the
/// attributes its fields add, in their declaration order: `CharTermAttribute`,
/// `OffsetAttribute`, `PositionIncrementAttribute`, `TypeAttribute`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement.
pub(crate) fn uax29_url_email_tokenizer_source(
    factory: Arc<dyn AttributeFactory>,
) -> Result<AttributeSource> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(source)
}
