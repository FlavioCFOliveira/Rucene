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

//! A tokenizer that matches tokens with a Lucene `RegExp` automaton, ported
//! from `org.apache.lucene.analysis.pattern.SimplePatternTokenizer`.

use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute, Tokenizer,
    TokenizerLogic,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};
use rucene::util::automaton::{
    Automaton, CharacterRunAutomaton, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene::util::ArrayUtil;

use crate::jdk::character::utf16_to_term_chars;

/// `Character.toCodePoint(char, char)`, with Java's `int` arithmetic, which
/// the tokenizer also applies to a high surrogate followed by something
/// other than a low one (`(char) -1` at the end of the input).
pub(crate) fn to_code_point(high: i32, low: i32) -> i32 {
    ((high << 10).wrapping_add(low)).wrapping_add(0x10000 - (0xD800 << 10) - 0xDC00)
}

/// The body of [`SimplePatternTokenizer`]: what Lucene's
/// `SimplePatternTokenizer` adds to `Tokenizer`.
///
/// **Divergence from Lucene 10.5.0.** Java scans the units straight into the
/// term's `char[]` and pushes back from it; `rucene`'s term buffer holds
/// `char`s, not UTF-16 units, so the units of the token being scanned are
/// held by this logic (`token`) and decoded into the term when a token is
/// emitted, an unpaired surrogate becoming U+FFFD
/// (`jdk::character::utf16_to_term_chars`).
#[derive(Debug)]
pub struct SimplePatternTokenizerLogic {
    run_dfa: CharacterRunAutomaton,
    // TODO: we could likely use a single rolling buffer instead of two separate char buffers here.
    // We could also use PushBackReader but I
    // suspect it's slowish:
    pending_chars: Vec<u16>,
    pending_limit: i32,
    pending_upto: i32,
    offset: i32,
    token_upto: i32,
    buffer: Vec<u16>,
    buffer_limit: i32,
    buffer_next_read: i32,
    /// The units of Java's term buffer.
    token: Vec<u16>,
}

impl SimplePatternTokenizerLogic {
    /// Pushes back the last `count` characters in current token's buffer.
    ///
    /// Equivalent to the private `pushBack(int)`.
    fn push_back(&mut self, count: i32) {
        if self.pending_limit == 0 {
            if self.buffer_limit != -1 && self.buffer_next_read >= count {
                // optimize common case when the chars we are pushing back are still in the buffer
                self.buffer_next_read -= count;
            } else {
                if count as usize > self.pending_chars.len() {
                    let new_length = ArrayUtil::oversize(count as usize, 2);
                    self.pending_chars.resize(new_length, 0);
                }
                let from = (self.token_upto - count) as usize;
                self.pending_chars[..count as usize]
                    .copy_from_slice(&self.token[from..from + count as usize]);
                self.pending_limit = count;
            }
        } else {
            // we are pushing back what is already in our pending buffer
            self.pending_upto -= count;
            debug_assert!(self.pending_upto >= 0);
        }
        self.offset -= count;
    }

    /// Equivalent to the private `appendToToken(char)`.
    fn append_to_token(&mut self, ch: u16) {
        if self.token_upto as usize == self.token.len() {
            // `termAtt.resizeBuffer(tokenUpto + 1)`
            let new_length = ArrayUtil::oversize(self.token_upto as usize + 1, 2);
            self.token.resize(new_length, 0);
        }
        self.token[self.token_upto as usize] = ch;
        self.token_upto += 1;
    }

    /// Equivalent to the private `nextCodeUnit()`: `-1` at the end of the
    /// input.
    fn next_code_unit(&mut self, input: &mut dyn CharReader) -> Result<i32> {
        let result;
        if self.pending_upto < self.pending_limit {
            result = self.pending_chars[self.pending_upto as usize];
            self.pending_upto += 1;
            if self.pending_upto == self.pending_limit {
                // We used up the pending buffer
                self.pending_upto = 0;
                self.pending_limit = 0;
            }
            self.append_to_token(result);
            self.offset += 1;
        } else if self.buffer_limit == -1 {
            return Ok(-1);
        } else {
            debug_assert!(
                self.buffer_next_read <= self.buffer_limit,
                "bufferNextRead={} bufferLimit={}",
                self.buffer_next_read,
                self.buffer_limit
            );
            if self.buffer_next_read == self.buffer_limit {
                // `input.read` answers -1 at the end of the input, a
                // `CharReader` 0.
                let read = input.read(&mut self.buffer)?;
                self.buffer_limit = if read == 0 { -1 } else { read as i32 };
                if self.buffer_limit == -1 {
                    return Ok(-1);
                }
                self.buffer_next_read = 0;
            }
            result = self.buffer[self.buffer_next_read as usize];
            self.buffer_next_read += 1;
            self.offset += 1;
            self.append_to_token(result);
        }
        Ok(i32::from(result))
    }

    /// Equivalent to the private `nextCodePoint()`.
    fn next_code_point(&mut self, input: &mut dyn CharReader) -> Result<i32> {
        let ch = self.next_code_unit(input)?;
        if ch == -1 {
            return Ok(ch);
        }
        if (0xD800..0xDC00).contains(&ch) {
            // `(char) nextCodeUnit()`: `-1` becomes U+FFFF.
            let low = self.next_code_unit(input)? & 0xFFFF;
            Ok(to_code_point(ch, low))
        } else {
            Ok(ch)
        }
    }
}

impl TokenizerLogic for SimplePatternTokenizerLogic {
    /// Equivalent to `SimplePatternTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        self.token_upto = 0;

        loop {
            let offset_start = self.offset;

            // The runDFA operates in Unicode space, not UTF16 (java's char):

            let mut ch = self.next_code_point(input)?;
            if ch == -1 {
                return Ok(false);
            }

            let mut state = self.run_dfa.step(0, ch);

            if state != -1 {
                // a token just possibly started; keep scanning to see if the token is accepted:
                let mut last_accept_length = -1;
                loop {
                    if self.run_dfa.is_accept(state) {
                        // record that the token matches here, but keep scanning in case a longer match also
                        // works (greedy):
                        last_accept_length = self.token_upto;
                    }
                    ch = self.next_code_point(input)?;
                    if ch == -1 {
                        break;
                    }
                    state = self.run_dfa.step(state, ch);
                    if state == -1 {
                        break;
                    }
                }

                if last_accept_length != -1 {
                    // we found a token
                    let extra = self.token_upto - last_accept_length;
                    if extra != 0 {
                        self.push_back(extra);
                    }
                    let chars = utf16_to_term_chars(&self.token[..last_accept_length as usize]);
                    source
                        .get_attribute_interface_mut::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute")
                        .copy_buffer(&chars, 0, chars.len());
                    let start = Tokenizer::<Self>::correct_offset_of(Some(&*input), offset_start);
                    let end = Tokenizer::<Self>::correct_offset_of(
                        Some(&*input),
                        offset_start + last_accept_length,
                    );
                    source
                        .get_attribute_interface_mut::<dyn OffsetAttribute>()
                        .expect("INVARIANT: the constructor added the OffsetAttribute")
                        .set_offset(start, end);
                    return Ok(true);
                } else if ch == -1 {
                    return Ok(false);
                } else {
                    // false alarm: there was no token here; push back all but the first character we scanned
                    self.push_back(self.token_upto - 1);
                    self.token_upto = 0;
                }
            } else {
                self.token_upto = 0;
            }
        }
    }

    /// Equivalent to `SimplePatternTokenizer.end()`; `Tokenizer::end` has
    /// run `super.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        let ofs = Tokenizer::<Self>::correct_offset_of(
            input,
            self.offset + self.pending_limit - self.pending_upto,
        );
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(ofs, ofs);
        Ok(())
    }

    /// Equivalent to `SimplePatternTokenizer.reset()`; `Tokenizer::reset`
    /// has run `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.offset = 0;
        self.pending_upto = 0;
        self.pending_limit = 0;
        self.token_upto = 0;
        self.buffer_next_read = 0;
        self.buffer_limit = 0;
        Ok(())
    }
}

/// This tokenizer uses a Lucene `RegExp` or (expert usage) a pre-built
/// determinized `Automaton`, to locate tokens. The regexp syntax is more
/// limited than `PatternTokenizer`, but the tokenization is quite a bit
/// faster. The provided regex should match valid token characters (not token
/// separator characters, like `String.split`). The matching is greedy: the
/// longest match at a given start point will be the next token. Empty string
/// tokens are never produced.
///
/// Equivalent to `org.apache.lucene.analysis.pattern.SimplePatternTokenizer`
/// (`@lucene.experimental`).
pub type SimplePatternTokenizer = Tokenizer<SimplePatternTokenizerLogic>;

/// See `RegExp` for the accepted syntax.
///
/// Equivalent to `SimplePatternTokenizer(String)`, `this(DEFAULT_TOKEN_ATTRIBUTE_FACTORY,
/// regexp, Operations.DEFAULT_DETERMINIZE_WORK_LIMIT)`.
///
/// # Errors
///
/// As [`new_simple_pattern_tokenizer_with_factory`].
pub fn new_simple_pattern_tokenizer(regexp: &str) -> Result<SimplePatternTokenizer> {
    new_simple_pattern_tokenizer_with_factory(
        default_token_attribute_factory(),
        regexp,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    )
}

/// Runs a pre-built automaton.
///
/// Equivalent to `SimplePatternTokenizer(Automaton)`.
///
/// # Errors
///
/// As [`new_simple_pattern_tokenizer_with_factory_from_automaton`].
pub fn new_simple_pattern_tokenizer_from_automaton(
    dfa: Automaton,
) -> Result<SimplePatternTokenizer> {
    new_simple_pattern_tokenizer_with_factory_from_automaton(default_token_attribute_factory(), dfa)
}

/// See `RegExp` for the accepted syntax.
///
/// Equivalent to `SimplePatternTokenizer(AttributeFactory, String, int)`,
/// `this(factory, new RegExp(regexp).toAutomaton())`: as Java's, the
/// automaton is not determinized, and `determinize_work_limit` is unused.
///
/// # Errors
///
/// What `RegExp` returns for an invalid regular expression, and as
/// [`new_simple_pattern_tokenizer_with_factory_from_automaton`], whose
/// `IllegalArgumentException` a regular expression whose automaton is not
/// deterministic meets.
pub fn new_simple_pattern_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
    regexp: &str,
    determinize_work_limit: i32,
) -> Result<SimplePatternTokenizer> {
    let _ = determinize_work_limit;
    new_simple_pattern_tokenizer_with_factory_from_automaton(
        factory,
        RegExp::new(regexp)?.to_automaton()?,
    )
}

/// Runs a pre-built automaton.
///
/// Equivalent to `SimplePatternTokenizer(AttributeFactory, Automaton)`.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws, and `IllegalArgumentException` "please determinize
/// the incoming automaton first" for an automaton that is not deterministic.
pub fn new_simple_pattern_tokenizer_with_factory_from_automaton(
    factory: Arc<dyn AttributeFactory>,
    dfa: Automaton,
) -> Result<SimplePatternTokenizer> {
    let (source, logic) = simple_pattern_tokenizer_parts(factory, dfa)?;
    Ok(Tokenizer::with_source(source, logic))
}

/// The source and the body of the `SimplePatternTokenizer` that
/// [`new_simple_pattern_tokenizer_with_factory_from_automaton`] builds, for a
/// caller that holds the body behind a box (`SimplePatternTokenizerFactory`).
///
/// # Errors
///
/// As [`new_simple_pattern_tokenizer_with_factory_from_automaton`].
pub(crate) fn simple_pattern_tokenizer_parts(
    factory: Arc<dyn AttributeFactory>,
    dfa: Automaton,
) -> Result<(AttributeSource, SimplePatternTokenizerLogic)> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;

    // we require user to do this up front because it is a possibly very costly operation, and user
    // may be creating us frequently, not
    // realizing this ctor is otherwise trappy
    if !dfa.is_deterministic() {
        return Err(LuceneError::illegal_argument(
            "please determinize the incoming automaton first",
        ));
    }

    let run_dfa = CharacterRunAutomaton::new(dfa)?;
    Ok((
        source,
        SimplePatternTokenizerLogic {
            run_dfa,
            pending_chars: vec![0; 8],
            pending_limit: 0,
            pending_upto: 0,
            offset: 0,
            token_upto: 0,
            buffer: vec![0; 1024],
            buffer_limit: 0,
            buffer_next_read: 0,
            token: Vec::new(),
        },
    ))
}
