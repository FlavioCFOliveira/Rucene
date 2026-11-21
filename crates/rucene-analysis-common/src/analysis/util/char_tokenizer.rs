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

//! An abstract base class for simple, character-oriented tokenizers, ported
//! from `org.apache.lucene.analysis.util.CharTokenizer`.

use std::fmt::Debug;
use std::sync::Arc;

use rucene::analysis::character_utils::{fill_buffer, new_character_buffer, CharacterBuffer};
use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute, Tokenizer,
    TokenizerLogic, MAX_TOKEN_LENGTH_LIMIT,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};

use crate::jdk::character::{char_count, code_point_at, to_term_char};

/// `CharTokenizer.DEFAULT_MAX_WORD_LEN`.
pub const DEFAULT_MAX_WORD_LEN: i32 = 255;

/// `IO_BUFFER_SIZE`.
const IO_BUFFER_SIZE: usize = 4096;

/// The abstract member of `CharTokenizer`, which each subclass supplies.
pub trait CharTokenizerLogic: Debug + Send + Sync {
    /// Returns true iff a codepoint should be included in a token. This
    /// tokenizer generates as tokens adjacent sequences of codepoints which
    /// satisfy this predicate. Codepoints for which this is false are used to
    /// define token boundaries and are not included in tokens.
    ///
    /// Equivalent to the abstract `CharTokenizer.isTokenChar(int)`.
    fn is_token_char(&self, c: i32) -> bool;
}

/// The body of [`CharTokenizer`]: what Lucene's `CharTokenizer` adds to
/// `Tokenizer`, over the subclass's [`CharTokenizerLogic`].
///
/// Offsets, the token length limit and `maxTokenLen` count UTF-16 units, as
/// Java's `char`s do; the term buffer holds `char`s (see `rucene`'s
/// `CharTermAttribute`), one per code point, and a lone surrogate of the
/// input, which Java copies into the term as it is, is stored as U+FFFD
/// ([`to_term_char`]).
#[derive(Debug)]
pub struct CharTokenizerAdapter<T: CharTokenizerLogic> {
    logic: T,
    offset: i32,
    buffer_index: usize,
    data_len: usize,
    final_offset: i32,
    max_token_len: i32,
    io_buffer: CharacterBuffer,
}

impl<T: CharTokenizerLogic> CharTokenizerAdapter<T> {
    /// The fields of a `CharTokenizer` built by `CharTokenizer()` or
    /// `CharTokenizer(AttributeFactory)`: `maxTokenLen` is
    /// [`DEFAULT_MAX_WORD_LEN`].
    pub fn new(logic: T) -> Self {
        Self {
            logic,
            offset: 0,
            buffer_index: 0,
            data_len: 0,
            final_offset: 0,
            max_token_len: DEFAULT_MAX_WORD_LEN,
            io_buffer: new_character_buffer(IO_BUFFER_SIZE)
                .expect("INVARIANT: IO_BUFFER_SIZE is at least 2"),
        }
    }

    /// The fields of a `CharTokenizer` built by
    /// `CharTokenizer(AttributeFactory, int)`: `max_token_len` is the
    /// maximum token length the tokenizer will emit.
    ///
    /// # Errors
    ///
    /// Returns `IllegalArgumentException` with Java's message when
    /// `max_token_len` is not greater than 0 or exceeds
    /// `MAX_TOKEN_LENGTH_LIMIT` (1024*1024).
    pub fn with_max_token_len(logic: T, max_token_len: i32) -> Result<Self> {
        if max_token_len as i64 > MAX_TOKEN_LENGTH_LIMIT as i64 || max_token_len <= 0 {
            return Err(LuceneError::illegal_argument(format!(
                "maxTokenLen must be greater than 0 and less than {MAX_TOKEN_LENGTH_LIMIT} passed: {max_token_len}"
            )));
        }
        let mut adapter = Self::new(logic);
        adapter.max_token_len = max_token_len;
        Ok(adapter)
    }

    /// The subclass's logic.
    pub fn logic(&self) -> &T {
        &self.logic
    }
}

impl<T: CharTokenizerLogic> TokenizerLogic for CharTokenizerAdapter<T> {
    /// Equivalent to the final `CharTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        // `length` counts UTF-16 units, as Java's term length does;
        // `term_len` counts the `char`s written into the term buffer.
        let mut length: i32 = 0;
        let mut term_len: usize = 0;
        let mut start: i32 = -1; // this variable is always initialized
        let mut end: i32 = -1;
        {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let mut buffer_len = term_att.buffer().len();
            loop {
                if self.buffer_index >= self.data_len {
                    self.offset += self.data_len as i32;
                    // read supplementary char aware with CharacterUtils
                    fill_buffer(&mut self.io_buffer, input)?;
                    if self.io_buffer.length() == 0 {
                        self.data_len = 0; // so next offset += dataLen won't decrement offset
                        if length > 0 {
                            break;
                        } else {
                            drop(term_att);
                            self.final_offset =
                                Tokenizer::<Self>::correct_offset_of(Some(&*input), self.offset);
                            return Ok(false);
                        }
                    }
                    self.data_len = self.io_buffer.length();
                    self.buffer_index = 0;
                }
                // use CharacterUtils here to support < 3.1 UTF-16 code unit
                // behavior if the char based methods are gone
                let c = code_point_at(
                    self.io_buffer.buffer(),
                    self.buffer_index,
                    self.io_buffer.length(),
                );
                let char_count = char_count(c);
                self.buffer_index += char_count;

                if self.logic.is_token_char(c) {
                    // if it's a token char
                    if length == 0 {
                        // start of token
                        debug_assert!(start == -1);
                        start = self.offset + self.buffer_index as i32 - char_count as i32;
                        end = start;
                    } else if term_len >= buffer_len - 1 {
                        // supplementary could run out of bounds?
                        // make sure a supplementary fits in the buffer
                        buffer_len = term_att.resize_buffer(2 + term_len).len();
                    }
                    end += char_count as i32;
                    // buffer it, normalized
                    term_att.buffer_mut()[term_len] = to_term_char(c);
                    term_len += 1;
                    length += char_count as i32;
                    // buffer overflow! make sure to check for >= surrogate
                    // pair could break == test
                    if length >= self.max_token_len {
                        break;
                    }
                } else if length > 0 {
                    // at non-Letter w/ chars
                    break; // return 'em
                }
            }

            term_att.set_length(term_len);
        }
        debug_assert!(start != -1);
        let corrected_start = Tokenizer::<Self>::correct_offset_of(Some(&*input), start);
        self.final_offset = Tokenizer::<Self>::correct_offset_of(Some(&*input), end);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(corrected_start, self.final_offset);
        Ok(true)
    }

    /// Equivalent to the final `CharTokenizer.end()`; `Tokenizer::end` has
    /// run `super.end()`. It sets the final offset.
    fn end(&mut self, source: &mut AttributeSource, _input: Option<&dyn CharReader>) -> Result<()> {
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(self.final_offset, self.final_offset);
        Ok(())
    }

    /// Equivalent to `CharTokenizer.reset()`; `Tokenizer::reset` has run
    /// `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.buffer_index = 0;
        self.offset = 0;
        self.data_len = 0;
        self.final_offset = 0;
        self.io_buffer.reset(); // make sure to reset the IO buffer!!
        Ok(())
    }
}

/// An abstract base class for simple, character-oriented tokenizers.
///
/// The base class also provides factories to create instances of
/// `CharTokenizer` using Java 8 lambdas or method references. It is
/// possible to create an instance which behaves exactly like
/// `LetterTokenizer`: [`from_token_char_predicate`] over
/// `Character::isLetter`.
///
/// Equivalent to `org.apache.lucene.analysis.util.CharTokenizer`: a
/// [`Tokenizer`] whose body is a [`CharTokenizerAdapter`] over the
/// subclass's [`CharTokenizerLogic`]. The subclass's `isTokenChar` is that
/// logic, as `FilteringTokenFilter`'s `accept` is a
/// `FilteringTokenFilterLogic` in `rucene`.
pub type CharTokenizer<T> = Tokenizer<CharTokenizerAdapter<T>>;

/// The source of a `CharTokenizer` over `factory`, holding the attributes
/// its fields add, in their declaration order: the `CharTermAttribute`, then
/// the `OffsetAttribute`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws.
pub fn char_tokenizer_source(factory: Arc<dyn AttributeFactory>) -> Result<AttributeSource> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    Ok(source)
}

/// Creates a new [`CharTokenizer`] instance.
///
/// Equivalent to `CharTokenizer()`, whose attributes come from
/// `TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY`.
pub fn new_char_tokenizer<T: CharTokenizerLogic>(logic: T) -> CharTokenizer<T> {
    new_char_tokenizer_with_factory(default_token_attribute_factory(), logic)
        .expect("INVARIANT: the default token attribute factory implements the core attributes")
}

/// Creates a new [`CharTokenizer`] instance whose attributes come from
/// `factory`.
///
/// Equivalent to `CharTokenizer(AttributeFactory)`.
///
/// # Errors
///
/// As [`char_tokenizer_source`].
pub fn new_char_tokenizer_with_factory<T: CharTokenizerLogic>(
    factory: Arc<dyn AttributeFactory>,
    logic: T,
) -> Result<CharTokenizer<T>> {
    Ok(Tokenizer::with_source(
        char_tokenizer_source(factory)?,
        CharTokenizerAdapter::new(logic),
    ))
}

/// Creates a new [`CharTokenizer`] instance: `factory` is the attribute
/// factory to use, `max_token_len` the maximum token length the tokenizer
/// will emit. Must be greater than 0 and less than `MAX_TOKEN_LENGTH_LIMIT`
/// (1024*1024).
///
/// Equivalent to `CharTokenizer(AttributeFactory, int)`.
///
/// # Errors
///
/// As [`char_tokenizer_source`], and as
/// [`CharTokenizerAdapter::with_max_token_len`] for an invalid
/// `max_token_len`.
pub fn new_char_tokenizer_with_factory_and_max_token_len<T: CharTokenizerLogic>(
    factory: Arc<dyn AttributeFactory>,
    max_token_len: i32,
    logic: T,
) -> Result<CharTokenizer<T>> {
    let source = char_tokenizer_source(factory)?;
    Ok(Tokenizer::with_source(
        source,
        CharTokenizerAdapter::with_max_token_len(logic, max_token_len)?,
    ))
}

/// The anonymous subclass `fromTokenCharPredicate` creates, whose
/// `isTokenChar` tests the predicate.
pub struct PredicateCharTokenizerLogic {
    token_char_predicate: Box<dyn Fn(i32) -> bool + Send + Sync>,
}

impl Debug for PredicateCharTokenizerLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PredicateCharTokenizerLogic")
            .finish_non_exhaustive()
    }
}

impl CharTokenizerLogic for PredicateCharTokenizerLogic {
    fn is_token_char(&self, c: i32) -> bool {
        (self.token_char_predicate)(c)
    }
}

/// Creates a new instance of CharTokenizer using a custom predicate, supplied
/// as method reference or lambda expression. The predicate should return
/// `true` for all valid token characters.
///
/// Equivalent to `CharTokenizer.fromTokenCharPredicate(IntPredicate)`, over
/// `TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY`. Java's
/// `Objects.requireNonNull(tokenCharPredicate, "predicate must not be
/// null.")` has no counterpart: the predicate cannot be null.
pub fn from_token_char_predicate(
    token_char_predicate: impl Fn(i32) -> bool + Send + Sync + 'static,
) -> CharTokenizer<PredicateCharTokenizerLogic> {
    from_token_char_predicate_with_factory(default_token_attribute_factory(), token_char_predicate)
        .expect("INVARIANT: the default token attribute factory implements the core attributes")
}

/// Creates a new instance of CharTokenizer with the supplied attribute factory
/// using a custom predicate, supplied as method reference or lambda
/// expression. The predicate should return `true` for all valid token
/// characters.
///
/// Equivalent to `CharTokenizer.fromTokenCharPredicate(AttributeFactory,
/// IntPredicate)`.
///
/// # Errors
///
/// As [`char_tokenizer_source`].
pub fn from_token_char_predicate_with_factory(
    factory: Arc<dyn AttributeFactory>,
    token_char_predicate: impl Fn(i32) -> bool + Send + Sync + 'static,
) -> Result<CharTokenizer<PredicateCharTokenizerLogic>> {
    new_char_tokenizer_with_factory(
        factory,
        PredicateCharTokenizerLogic {
            token_char_predicate: Box::new(token_char_predicate),
        },
    )
}

/// Creates a new instance of CharTokenizer using a custom predicate, supplied
/// as method reference or lambda expression. The predicate should return
/// `true` for all valid token separator characters. This method is provided
/// for convenience to easily use predicates that are negated (they match the
/// separator characters, not the token characters).
///
/// Equivalent to `CharTokenizer.fromSeparatorCharPredicate(IntPredicate)`.
pub fn from_separator_char_predicate(
    separator_char_predicate: impl Fn(i32) -> bool + Send + Sync + 'static,
) -> CharTokenizer<PredicateCharTokenizerLogic> {
    from_separator_char_predicate_with_factory(
        default_token_attribute_factory(),
        separator_char_predicate,
    )
    .expect("INVARIANT: the default token attribute factory implements the core attributes")
}

/// Creates a new instance of CharTokenizer with the supplied attribute factory
/// using a custom predicate, supplied as method reference or lambda
/// expression. The predicate should return `true` for all valid token
/// separator characters.
///
/// Equivalent to `CharTokenizer.fromSeparatorCharPredicate(AttributeFactory,
/// IntPredicate)`, `fromTokenCharPredicate(factory,
/// separatorCharPredicate.negate())`.
///
/// # Errors
///
/// As [`char_tokenizer_source`].
pub fn from_separator_char_predicate_with_factory(
    factory: Arc<dyn AttributeFactory>,
    separator_char_predicate: impl Fn(i32) -> bool + Send + Sync + 'static,
) -> Result<CharTokenizer<PredicateCharTokenizerLogic>> {
    from_token_char_predicate_with_factory(factory, move |c| !separator_char_predicate(c))
}
