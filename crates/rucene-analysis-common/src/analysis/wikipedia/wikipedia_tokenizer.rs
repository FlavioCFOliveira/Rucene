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

//! Extension of StandardTokenizer that is aware of Wikipedia syntax, ported
//! from `org.apache.lucene.analysis.wikipedia.WikipediaTokenizer`.

use std::collections::HashSet;
use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, FlagsAttribute,
    OffsetAttribute, PositionIncrementAttribute, Tokenizer, TokenizerLogic, TypeAttribute,
};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefMut;
use rucene::util::attribute::{
    AttributeFactory, AttributeInterface, AttributeSource, CapturedState,
};

use super::wikipedia_tokenizer_impl::{WikipediaTokenizerImpl, YYEOF};

/// Equivalent to `WikipediaTokenizer.INTERNAL_LINK`.
pub const INTERNAL_LINK: &str = "il";
/// Equivalent to `WikipediaTokenizer.EXTERNAL_LINK`.
pub const EXTERNAL_LINK: &str = "el";
/// The URL part of the link, i.e. the first token
///
/// Equivalent to `WikipediaTokenizer.EXTERNAL_LINK_URL`.
pub const EXTERNAL_LINK_URL: &str = "elu";
/// Equivalent to `WikipediaTokenizer.CITATION`.
pub const CITATION: &str = "ci";
/// Equivalent to `WikipediaTokenizer.CATEGORY`.
pub const CATEGORY: &str = "c";
/// Equivalent to `WikipediaTokenizer.BOLD`.
pub const BOLD: &str = "b";
/// Equivalent to `WikipediaTokenizer.ITALICS`.
pub const ITALICS: &str = "i";
/// Equivalent to `WikipediaTokenizer.BOLD_ITALICS`.
pub const BOLD_ITALICS: &str = "bi";
/// Equivalent to `WikipediaTokenizer.HEADING`.
pub const HEADING: &str = "h";
/// Equivalent to `WikipediaTokenizer.SUB_HEADING`.
pub const SUB_HEADING: &str = "sh";

/// Equivalent to `WikipediaTokenizer.ALPHANUM_ID`.
pub const ALPHANUM_ID: i32 = 0;
/// Equivalent to `WikipediaTokenizer.APOSTROPHE_ID`.
pub const APOSTROPHE_ID: i32 = 1;
/// Equivalent to `WikipediaTokenizer.ACRONYM_ID`.
pub const ACRONYM_ID: i32 = 2;
/// Equivalent to `WikipediaTokenizer.COMPANY_ID`.
pub const COMPANY_ID: i32 = 3;
/// Equivalent to `WikipediaTokenizer.EMAIL_ID`.
pub const EMAIL_ID: i32 = 4;
/// Equivalent to `WikipediaTokenizer.HOST_ID`.
pub const HOST_ID: i32 = 5;
/// Equivalent to `WikipediaTokenizer.NUM_ID`.
pub const NUM_ID: i32 = 6;
/// Equivalent to `WikipediaTokenizer.CJ_ID`.
pub const CJ_ID: i32 = 7;
/// Equivalent to `WikipediaTokenizer.INTERNAL_LINK_ID`.
pub const INTERNAL_LINK_ID: i32 = 8;
/// Equivalent to `WikipediaTokenizer.EXTERNAL_LINK_ID`.
pub const EXTERNAL_LINK_ID: i32 = 9;
/// Equivalent to `WikipediaTokenizer.CITATION_ID`.
pub const CITATION_ID: i32 = 10;
/// Equivalent to `WikipediaTokenizer.CATEGORY_ID`.
pub const CATEGORY_ID: i32 = 11;
/// Equivalent to `WikipediaTokenizer.BOLD_ID`.
pub const BOLD_ID: i32 = 12;
/// Equivalent to `WikipediaTokenizer.ITALICS_ID`.
pub const ITALICS_ID: i32 = 13;
/// Equivalent to `WikipediaTokenizer.BOLD_ITALICS_ID`.
pub const BOLD_ITALICS_ID: i32 = 14;
/// Equivalent to `WikipediaTokenizer.HEADING_ID`.
pub const HEADING_ID: i32 = 15;
/// Equivalent to `WikipediaTokenizer.SUB_HEADING_ID`.
pub const SUB_HEADING_ID: i32 = 16;
/// Equivalent to `WikipediaTokenizer.EXTERNAL_LINK_URL_ID`.
pub const EXTERNAL_LINK_URL_ID: i32 = 17;

/// String token types that correspond to token type int constants
///
/// Equivalent to `WikipediaTokenizer.TOKEN_TYPES`.
pub static TOKEN_TYPES: [&str; 18] = [
    "<ALPHANUM>",
    "<APOSTROPHE>",
    "<ACRONYM>",
    "<COMPANY>",
    "<EMAIL>",
    "<HOST>",
    "<NUM>",
    "<CJ>",
    INTERNAL_LINK,
    EXTERNAL_LINK,
    CITATION,
    CATEGORY,
    BOLD,
    ITALICS,
    BOLD_ITALICS,
    HEADING,
    SUB_HEADING,
    EXTERNAL_LINK_URL,
];

/// Only output tokens
///
/// Equivalent to `WikipediaTokenizer.TOKENS_ONLY`.
pub const TOKENS_ONLY: i32 = 0;

/// Only output untokenized tokens, which are tokens that would normally be
/// split into several tokens
///
/// Equivalent to `WikipediaTokenizer.UNTOKENIZED_ONLY`.
pub const UNTOKENIZED_ONLY: i32 = 1;

/// Output the both the untokenized token and the splits
///
/// Equivalent to `WikipediaTokenizer.BOTH`.
pub const BOTH: i32 = 2;

/// This flag is used to indicate that the produced "Token" would, if
/// [`TOKENS_ONLY`] was used, produce multiple tokens.
///
/// Equivalent to `WikipediaTokenizer.UNTOKENIZED_TOKEN_FLAG`.
pub const UNTOKENIZED_TOKEN_FLAG: i32 = 1;

/// The body of [`WikipediaTokenizer`]: what Lucene's `WikipediaTokenizer` adds
/// to `Tokenizer`.
///
/// Equivalent to the members of
/// `org.apache.lucene.analysis.wikipedia.WikipediaTokenizer`; the `Tokenizer`
/// half is [`Tokenizer`].
#[derive(Debug)]
pub struct WikipediaTokenizerLogic {
    /// A private instance of the JFlex-constructed scanner
    scanner: WikipediaTokenizerImpl,
    token_output: i32,
    untokenized_types: HashSet<String>,
    /// Java's `Iterator<AttributeSource.State> tokens`, `null` as `None`.
    tokens: Option<std::vec::IntoIter<CapturedState>>,
    first: bool,
}

impl Default for WikipediaTokenizerLogic {
    fn default() -> Self {
        Self::new(TOKENS_ONLY, HashSet::new())
            .expect("INVARIANT: TOKENS_ONLY is a valid token output")
    }
}

/// The attribute interface `I` of a `WikipediaTokenizer`'s source, which
/// [`wikipedia_tokenizer_source`] added.
fn interface_mut<I: AttributeInterface + ?Sized>(source: &AttributeSource) -> AtomicRefMut<'_, I> {
    source
        .get_attribute_interface_mut::<I>()
        .expect("INVARIANT: wikipedia_tokenizer_source added the attribute")
}

/// `String.trim()` over UTF-16 code units: the units up to U+0020 removed
/// from both ends.
fn java_trim(units: &[u16]) -> &[u16] {
    let mut start = 0;
    let mut end = units.len();
    while start < end && units[start] <= 0x20 {
        start += 1;
    }
    while start < end && units[end - 1] <= 0x20 {
        end -= 1;
    }
    &units[start..end]
}

impl WikipediaTokenizerLogic {
    /// Creates the tokenizer body: a newly created JFlex scanner, with the
    /// given token output.
    ///
    /// Equivalent to `WikipediaTokenizer(int, Set<String>)` and its private
    /// `init(int, Set<String>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException`, `tokenOutput must be TOKENS_ONLY,
    /// UNTOKENIZED_ONLY or BOTH`, for any other `token_output`.
    pub fn new(token_output: i32, untokenized_types: HashSet<String>) -> Result<Self> {
        // TODO: cutover to enum
        if token_output != TOKENS_ONLY && token_output != UNTOKENIZED_ONLY && token_output != BOTH {
            return Err(LuceneError::illegal_argument(
                "tokenOutput must be TOKENS_ONLY, UNTOKENIZED_ONLY or BOTH".to_string(),
            ));
        }
        Ok(Self {
            scanner: WikipediaTokenizerImpl::new(),
            token_output,
            untokenized_types,
            tokens: None,
            first: false,
        })
    }

    /// Equivalent to `WikipediaTokenizer.collapseAndSaveTokens(int, String)`.
    fn collapse_and_save_tokens(
        &mut self,
        token_type: i32,
        type_value: &'static str,
        source: &AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<()> {
        // collapse
        let mut buffer: Vec<u16> = Vec::with_capacity(32);
        let mut num_added = self.scanner.set_text(&mut buffer);
        // TODO: how to know how much whitespace to add
        let the_start = self.scanner.yychar();
        let mut last_pos = the_start + num_added;
        let mut tmp_tok_type;
        let mut num_seen = 0;
        let mut tmp: Vec<CapturedState> = Vec::new();
        self.setup_saved_token(0, type_value, source, reader);
        tmp.push(capture_state(source));
        // while we can get a token and that token is the same type and we have
        // not transitioned to a new wiki-item of the same type
        loop {
            tmp_tok_type = self.scanner.get_next_token(reader)?;
            if !(tmp_tok_type != YYEOF
                && tmp_tok_type == token_type
                && self.scanner.get_num_wiki_tokens_seen() > num_seen)
            {
                break;
            }
            let curr_pos = self.scanner.yychar();
            // append whitespace
            for _ in 0..(curr_pos - last_pos) {
                buffer.push(u16::from(b' '));
            }
            num_added = self.scanner.set_text(&mut buffer);
            let position_increment = self.scanner.get_position_increment();
            self.setup_saved_token(position_increment, type_value, source, reader);
            tmp.push(capture_state(source));
            num_seen += 1;
            last_pos = curr_pos + num_added;
        }
        // trim the buffer
        // TODO: this is inefficient
        let s = java_trim(&buffer);
        let s_length = s.len() as i32;
        {
            let mut term_att = interface_mut::<dyn CharTermAttribute>(source);
            term_att.set_empty();
            for c in char::decode_utf16(s.iter().copied()) {
                term_att.append_char(c.unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
        let input: &dyn CharReader = reader;
        interface_mut::<dyn OffsetAttribute>(source).set_offset(
            Tokenizer::<Self>::correct_offset_of(Some(input), the_start),
            Tokenizer::<Self>::correct_offset_of(Some(input), the_start + s_length),
        );
        interface_mut::<dyn FlagsAttribute>(source).set_flags(UNTOKENIZED_TOKEN_FLAG);
        // The way the loop is written, we will have proceeded to the next
        // token.  We need to pushback the scanner to lastPos
        if tmp_tok_type != YYEOF {
            let length = self.scanner.yylength();
            self.scanner.yypushback(length);
        }
        self.tokens = Some(tmp.into_iter());
        Ok(())
    }

    /// Equivalent to `WikipediaTokenizer.setupSavedToken(int, String)`.
    fn setup_saved_token(
        &mut self,
        position_inc: i32,
        type_value: &'static str,
        source: &AttributeSource,
        reader: &dyn CharReader,
    ) {
        self.setup_token(source, reader);
        interface_mut::<dyn PositionIncrementAttribute>(source)
            .set_position_increment(position_inc);
        interface_mut::<dyn TypeAttribute>(source).set_type_static(type_value);
    }

    /// Equivalent to `WikipediaTokenizer.collapseTokens(int)`.
    fn collapse_tokens(
        &mut self,
        token_type: i32,
        source: &AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<()> {
        // collapse
        let mut buffer: Vec<u16> = Vec::with_capacity(32);
        let mut num_added = self.scanner.set_text(&mut buffer);
        // TODO: how to know how much whitespace to add
        let the_start = self.scanner.yychar();
        let mut last_pos = the_start + num_added;
        let mut tmp_tok_type;
        let mut num_seen = 0;
        // while we can get a token and that token is the same type and we have
        // not transitioned to a new wiki-item of the same type
        loop {
            tmp_tok_type = self.scanner.get_next_token(reader)?;
            if !(tmp_tok_type != YYEOF
                && tmp_tok_type == token_type
                && self.scanner.get_num_wiki_tokens_seen() > num_seen)
            {
                break;
            }
            let curr_pos = self.scanner.yychar();
            // append whitespace
            for _ in 0..(curr_pos - last_pos) {
                buffer.push(u16::from(b' '));
            }
            num_added = self.scanner.set_text(&mut buffer);
            num_seen += 1;
            last_pos = curr_pos + num_added;
        }
        // trim the buffer
        // TODO: this is inefficient
        let s = java_trim(&buffer);
        let s_length = s.len() as i32;
        {
            let mut term_att = interface_mut::<dyn CharTermAttribute>(source);
            term_att.set_empty();
            for c in char::decode_utf16(s.iter().copied()) {
                term_att.append_char(c.unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
        let input: &dyn CharReader = reader;
        interface_mut::<dyn OffsetAttribute>(source).set_offset(
            Tokenizer::<Self>::correct_offset_of(Some(input), the_start),
            Tokenizer::<Self>::correct_offset_of(Some(input), the_start + s_length),
        );
        interface_mut::<dyn FlagsAttribute>(source).set_flags(UNTOKENIZED_TOKEN_FLAG);
        // The way the loop is written, we will have proceeded to the next
        // token.  We need to pushback the scanner to lastPos
        if tmp_tok_type != YYEOF {
            let length = self.scanner.yylength();
            self.scanner.yypushback(length);
        } else {
            self.tokens = None;
        }
        Ok(())
    }

    /// Equivalent to `WikipediaTokenizer.setupToken()`: the offsets end at
    /// `start + termAtt.length()`, the token's length in UTF-16 code units,
    /// `yylength()`, as Java's `char` length is (the attribute here counts
    /// characters).
    fn setup_token(&mut self, source: &AttributeSource, reader: &dyn CharReader) {
        self.scanner
            .get_text(&mut *interface_mut::<dyn CharTermAttribute>(source));
        let start = self.scanner.yychar();
        interface_mut::<dyn OffsetAttribute>(source).set_offset(
            Tokenizer::<Self>::correct_offset_of(Some(reader), start),
            Tokenizer::<Self>::correct_offset_of(Some(reader), start + self.scanner.yylength()),
        );
    }
}

/// `captureState()`: the source holds attributes, so there is a state.
fn capture_state(source: &AttributeSource) -> CapturedState {
    source
        .capture_state()
        .expect("INVARIANT: the tokenizer's source holds attributes")
}

impl TokenizerLogic for WikipediaTokenizerLogic {
    /// Equivalent to `WikipediaTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        if let Some(tokens) = self.tokens.as_mut() {
            if let Some(state) = tokens.next() {
                source.restore_state(&state)?;
                return Ok(true);
            }
        }
        source.clear_attributes();
        let token_type = self.scanner.get_next_token(reader)?;

        if token_type == YYEOF {
            return Ok(false);
        }
        let type_value = TOKEN_TYPES[token_type as usize];
        if self.token_output == TOKENS_ONLY || !self.untokenized_types.contains(type_value) {
            self.setup_token(source, reader);
        } else if self.token_output == UNTOKENIZED_ONLY
            && self.untokenized_types.contains(type_value)
        {
            self.collapse_tokens(token_type, source, reader)?;
        } else if self.token_output == BOTH {
            // collapse into a single token, add it to tokens AND output the
            // individual tokens output the untokenized Token first
            self.collapse_and_save_tokens(token_type, type_value, source, reader)?;
        }
        let mut posinc = self.scanner.get_position_increment();
        if self.first && posinc == 0 {
            posinc = 1; // don't emit posinc=0 for the first token!
        }
        interface_mut::<dyn PositionIncrementAttribute>(source).set_position_increment(posinc);
        interface_mut::<dyn TypeAttribute>(source).set_type_static(type_value);
        self.first = false;
        Ok(true)
    }

    /// Equivalent to `WikipediaTokenizer.end()`; `Tokenizer::end` has run
    /// `super.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        // set final offset
        let final_offset = Tokenizer::<Self>::correct_offset_of(
            input,
            self.scanner.yychar() + self.scanner.yylength(),
        );
        interface_mut::<dyn OffsetAttribute>(source).set_offset(final_offset, final_offset);
        Ok(())
    }

    /// Equivalent to `WikipediaTokenizer.close()`; `Tokenizer::close` has run
    /// `super.close()`.
    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.yyreset();
        Ok(())
    }

    /// Equivalent to `WikipediaTokenizer.reset()`; `Tokenizer::reset` has run
    /// `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.yyreset();
        self.tokens = None;
        self.scanner.reset();
        self.first = true;
        Ok(())
    }
}

/// Extension of StandardTokenizer that is aware of Wikipedia syntax. It is
/// based off of the Wikipedia tutorial available at
/// <http://en.wikipedia.org/wiki/Wikipedia:Tutorial>, but it may not be
/// complete.
///
/// Equivalent to `org.apache.lucene.analysis.wikipedia.WikipediaTokenizer`
/// (`@lucene.experimental`), a [`Tokenizer`] whose body is a
/// [`WikipediaTokenizerLogic`]. Java's `@IgnoreRandomChains` annotation
/// ("TODO: it seems to mess up offsets!?") is test-framework metadata, read
/// by `TestRandomChains`; it has no runtime effect to port.
pub type WikipediaTokenizer = Tokenizer<WikipediaTokenizerLogic>;

/// Creates a new instance of the `WikipediaTokenizer`. Attaches the `input`
/// to a newly created JFlex scanner.
///
/// Equivalent to `WikipediaTokenizer()`, `this(TOKENS_ONLY,
/// Collections.emptySet())`.
pub fn new_wikipedia_tokenizer() -> WikipediaTokenizer {
    new_wikipedia_tokenizer_with_factory(
        default_token_attribute_factory(),
        TOKENS_ONLY,
        HashSet::new(),
    )
    .expect("INVARIANT: the default factory implements the core attributes; TOKENS_ONLY is valid")
}

/// Creates a new instance of the `WikipediaTokenizer`. Attaches the `input`
/// to the newly created JFlex scanner.
///
/// Equivalent to `WikipediaTokenizer(int, Set<String>)`.
///
/// # Errors
///
/// What [`WikipediaTokenizerLogic::new`] returns.
pub fn new_wikipedia_tokenizer_with_output(
    token_output: i32,
    untokenized_types: HashSet<String>,
) -> Result<WikipediaTokenizer> {
    new_wikipedia_tokenizer_with_factory(
        default_token_attribute_factory(),
        token_output,
        untokenized_types,
    )
}

/// Creates a new instance of the `WikipediaTokenizer`. Attaches the `input`
/// to the newly created JFlex scanner. Uses the given `AttributeFactory`.
///
/// Equivalent to `WikipediaTokenizer(AttributeFactory, int, Set<String>)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, and what
/// [`WikipediaTokenizerLogic::new`] returns.
pub fn new_wikipedia_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
    token_output: i32,
    untokenized_types: HashSet<String>,
) -> Result<WikipediaTokenizer> {
    let source = wikipedia_tokenizer_source(factory)?;
    Ok(Tokenizer::with_source(
        source,
        WikipediaTokenizerLogic::new(token_output, untokenized_types)?,
    ))
}

/// The source of a `WikipediaTokenizer` over `factory`, holding the
/// attributes its fields add, in their declaration order: `OffsetAttribute`,
/// `TypeAttribute`, `PositionIncrementAttribute`, `CharTermAttribute`,
/// `FlagsAttribute`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement.
pub(crate) fn wikipedia_tokenizer_source(
    factory: Arc<dyn AttributeFactory>,
) -> Result<AttributeSource> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn FlagsAttribute>()?;
    Ok(source)
}
