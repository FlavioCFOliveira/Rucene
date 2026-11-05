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

//! Port of `org.apache.lucene.tests.analysis.MockTokenizer`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::RandomizedContext;
use rucene::analysis::{CharReader, TokenStream, Tokenizer, TokenizerLogic};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};
use rucene::util::automaton::case_folding;
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;
use rucene::util::automaton::{CharacterRunAutomaton, Operations, RegExp};

use super::attributes::{char_term_id, offset_id, Att};
use super::base_token_stream_test_case::BaseTokenStreamTestCase;

fn determinized(regexp: &str) -> Arc<CharacterRunAutomaton> {
    let a = RegExp::new(regexp)
        .and_then(|r| r.to_automaton())
        .expect("INVARIANT: a valid regular expression");
    let a = Operations::determinize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT)
        .expect("INVARIANT: a small automaton determinizes");
    Arc::new(CharacterRunAutomaton::new(a).expect("INVARIANT: a deterministic automaton"))
}

/// Acts Similar to WhitespaceTokenizer.
///
/// Equivalent to `MockTokenizer.WHITESPACE`.
pub static WHITESPACE: LazyLock<Arc<CharacterRunAutomaton>> =
    LazyLock::new(|| determinized("[^ \t\r\n]+"));

/// Acts Similar to KeywordTokenizer.
///
/// Equivalent to `MockTokenizer.KEYWORD`.
pub static KEYWORD: LazyLock<Arc<CharacterRunAutomaton>> = LazyLock::new(|| determinized(".*"));

/// Acts like LetterTokenizer; the regex is an incomplete Unicode 5.2
/// `[:Letter:]`.
///
/// Equivalent to `MockTokenizer.SIMPLE`.
pub static SIMPLE: LazyLock<Arc<CharacterRunAutomaton>> = LazyLock::new(|| {
    determinized(
        "[A-Za-z\u{aa}\u{b5}\u{ba}\u{c0}-\u{d6}\u{d8}-\u{f6}\u{f8}-\u{2c1}\u{4e00}-\u{9fcc}]+",
    )
});

/// Limit the default token length to a size that doesn't cause random
/// analyzer failures on unpredictable data like the enwiki data set.
///
/// Equivalent to `MockTokenizer.DEFAULT_MAX_TOKEN_LENGTH`.
pub const DEFAULT_MAX_TOKEN_LENGTH: i32 = 255;

/// The consumer workflow states of `MockTokenizer.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// consumer set a reader input either via ctor or via reset(Reader)
    SetReader,
    /// consumer has called reset()
    Reset,
    /// consumer is consuming, has called incrementToken() == true
    Increment,
    /// consumer has called incrementToken() which returned false
    IncrementFalse,
    /// consumer has called end() to perform end of stream operations
    End,
    /// consumer has called close() to release any resources
    Close,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            State::SetReader => "SETREADER",
            State::Reset => "RESET",
            State::Increment => "INCREMENT",
            State::IncrementFalse => "INCREMENT_FALSE",
            State::End => "END",
            State::Close => "CLOSE",
        })
    }
}

/// The body of [`MockTokenizer`]: Java's subclass of `Tokenizer`.
///
/// Offsets and the token length limit count UTF-16 code units, as Java's
/// `char`s do; the reader yields Unicode scalar values, so a supplementary
/// character read is handed to `readCodePoint` as its two surrogates, one per
/// `readChar` call, and each call draws from the tokenizer's random exactly as
/// Java's does.
#[derive(Debug)]
pub struct MockTokenizerLogic {
    run_automaton: Arc<CharacterRunAutomaton>,
    lower_case: bool,
    max_token_length: i32,
    state: i32,
    term_att: Att,
    offset_att: Att,
    off: i32,
    // buffered state (previous codepoint and offset). we replay this once we
    // hit a reject state in case it's permissible as the start of a new term.
    buffered_code_point: i32,
    buffered_off: i32,
    stream_state: State,
    last_offset: i32,
    enable_checks: bool,
    // evil: but we don't change the behavior with this random, we only switch up how we read
    random: JdkRandom,
    /// The low surrogate of the supplementary character the last read
    /// returned the high surrogate of (Rust-only: the reader holds whole
    /// scalar values).
    pending_low_surrogate: Option<u16>,
    /// The UTF-16 length of the term being built, Java's `termAtt.length()`.
    term_length16: i32,
}

impl MockTokenizerLogic {
    // we allow some checks (e.g. state machine) to be turned off.
    // turning off checks just means we suppress exceptions from them
    fn fail(&self, message: String) -> Result<()> {
        if self.enable_checks {
            return Err(LuceneError::illegal_state(message));
        }
        Ok(())
    }

    fn fail_always(message: String) -> Result<()> {
        Err(LuceneError::illegal_state(message))
    }

    /// `readCodePoint()`.
    fn read_code_point(&mut self, input: &mut dyn CharReader) -> Result<i32> {
        let ch = self.read_char(input)?;
        if ch < 0 {
            return Ok(ch);
        }
        if is_low_surrogate(ch) {
            Self::fail_always(format!("unpaired low surrogate: {:x}", ch))?;
        }
        self.off += 1;
        if is_high_surrogate(ch) {
            let ch2 = self.read_char(input)?;
            if ch2 >= 0 {
                self.off += 1;
                if !is_low_surrogate(ch2) {
                    Self::fail_always(format!(
                        "unpaired high surrogate: {:x}, followed by: {:x}",
                        ch, ch2
                    ))?;
                }
                return Ok(((ch - 0xd800) << 10) + (ch2 - 0xdc00) + 0x10000);
            } else {
                Self::fail_always(format!(
                    "stream ends with unpaired high surrogate: {:x}",
                    ch
                ))?;
            }
        }
        Ok(ch)
    }

    /// `readChar()`: one UTF-16 code unit, read through one of four `Reader`
    /// methods picked by `random.nextInt(10)`; each reads a single `char`.
    fn read_char(&mut self, input: &mut dyn CharReader) -> Result<i32> {
        // read(char[]), read(char[], int, int), read(CharBuffer) or read():
        // the same single-char read here.
        let _ = self.random.next_int_bound(10);
        if let Some(low) = self.pending_low_surrogate.take() {
            return Ok(i32::from(low));
        }
        let mut c = ['\0'; 1];
        let ret = input.read(&mut c)?;
        if ret == 0 {
            return Ok(-1);
        }
        let mut units = [0u16; 2];
        let encoded = c[0].encode_utf16(&mut units);
        if encoded.len() == 2 {
            self.pending_low_surrogate = Some(units[1]);
        }
        Ok(i32::from(units[0]))
    }

    /// `isTokenChar(int)`.
    fn is_token_char(&mut self, c: i32) -> bool {
        if self.state < 0 {
            self.state = 0;
        }
        self.state = self.run_automaton.step(self.state, c);
        self.state >= 0
    }

    /// `normalize(int)`.
    fn normalize(&self, c: i32) -> i32 {
        if self.lower_case {
            case_folding::to_lower_case(c)
        } else {
            c
        }
    }

    /// Toggle consumer workflow checking: if your test consumes tokenstreams
    /// normally you should leave this enabled.
    pub fn set_enable_checks(&mut self, enable_checks: bool) {
        self.enable_checks = enable_checks;
    }

    /// The consumer workflow state, for tests.
    pub fn stream_state(&self) -> State {
        self.stream_state
    }
}

fn is_high_surrogate(c: i32) -> bool {
    (0xd800..=0xdbff).contains(&c)
}

fn is_low_surrogate(c: i32) -> bool {
    (0xdc00..=0xdfff).contains(&c)
}

impl TokenizerLogic for MockTokenizerLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.stream_state != State::Reset && self.stream_state != State::Increment {
            self.fail(format!(
                "incrementToken() called while in wrong state: {}",
                self.stream_state
            ))?;
        }
        source.clear_attributes();
        self.term_length16 = 0;
        loop {
            let start_offset;
            let mut cp;
            if self.buffered_code_point >= 0 {
                cp = self.buffered_code_point;
                start_offset = self.buffered_off;
                self.buffered_code_point = -1;
            } else {
                start_offset = self.off;
                cp = self.read_code_point(input)?;
            }
            if cp < 0 {
                break;
            } else if self.is_token_char(cp) {
                let mut end_offset;
                loop {
                    let normalized = self.normalize(cp);
                    let c = char::from_u32(normalized as u32)
                        .expect("INVARIANT: a code point read from a valid string");
                    self.term_att.char_term_mut().append_char(c);
                    self.term_length16 += c.len_utf16() as i32;
                    end_offset = self.off;
                    if self.term_length16 >= self.max_token_length {
                        break;
                    }
                    cp = self.read_code_point(input)?;
                    if !(cp >= 0 && self.is_token_char(cp)) {
                        break;
                    }
                }

                if self.term_length16 < self.max_token_length {
                    // buffer up, in case the "rejected" char can start a new word of its own
                    self.buffered_code_point = cp;
                    self.buffered_off = end_offset;
                } else {
                    // otherwise, it's because we hit term limit.
                    self.buffered_code_point = -1;
                }
                let corrected_start_offset =
                    Tokenizer::<Self>::correct_offset_of(Some(&*input), start_offset);
                let corrected_end_offset =
                    Tokenizer::<Self>::correct_offset_of(Some(&*input), end_offset);
                if corrected_start_offset < 0 {
                    Self::fail_always(format!(
                        "invalid start offset: {}, before correction: {}",
                        corrected_start_offset, start_offset
                    ))?;
                }
                if corrected_end_offset < 0 {
                    Self::fail_always(format!(
                        "invalid end offset: {}, before correction: {}",
                        corrected_end_offset, end_offset
                    ))?;
                }
                if corrected_start_offset < self.last_offset {
                    Self::fail_always(format!(
                        "start offset went backwards: {}, before correction: {}, lastOffset: {}",
                        corrected_start_offset, start_offset, self.last_offset
                    ))?;
                }
                self.last_offset = corrected_start_offset;
                if corrected_end_offset < corrected_start_offset {
                    Self::fail_always(format!(
                        "end offset: {} is before start offset: {}",
                        corrected_end_offset, corrected_start_offset
                    ))?;
                }
                self.offset_att
                    .offset_mut()
                    .set_offset(corrected_start_offset, corrected_end_offset);
                if self.state == -1 || self.run_automaton.is_accept(self.state) {
                    // either we hit a reject state (longest match), or end-of-text, but in an accept state
                    self.stream_state = State::Increment;
                    return Ok(true);
                }
            }
        }
        self.stream_state = State::IncrementFalse;
        Ok(false)
    }

    fn end(&mut self, _source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        let final_offset = Tokenizer::<Self>::correct_offset_of(input, self.off);
        self.offset_att
            .offset_mut()
            .set_offset(final_offset, final_offset);
        // some tokenizers, such as limiting tokenizers, call end() before incrementToken() returns
        // false.
        // these tests should disable this check (in general you should consume the entire stream)
        let result = if self.stream_state != State::IncrementFalse {
            self.fail(format!(
                "end() called in wrong state={}!",
                self.stream_state
            ))
        } else {
            Ok(())
        };
        self.stream_state = State::End;
        result
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.state = 0;
        self.off = 0;
        self.last_offset = 0;
        self.buffered_code_point = -1;
        self.pending_low_surrogate = None;
        let result = if self.stream_state == State::Reset {
            self.fail("double reset()".to_string())
        } else {
            Ok(())
        };
        self.stream_state = State::Reset;
        result
    }

    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        // in some exceptional cases (e.g. TestIndexWriterExceptions) a test can prematurely close()
        // these tests should disable this check, by default we check the normal workflow.
        // TODO: investigate the CachingTokenFilter "double-close"... for now we ignore this
        let result = if !(self.stream_state == State::End || self.stream_state == State::Close) {
            self.fail(format!(
                "close() called in wrong state: {}",
                self.stream_state
            ))
        } else {
            Ok(())
        };
        self.stream_state = State::Close;
        result
    }

    fn set_reader_test_point(&mut self) -> Result<()> {
        let result = if self.stream_state != State::Close {
            self.fail(format!(
                "setReader() called in wrong state: {}",
                self.stream_state
            ))
        } else {
            Ok(())
        };
        self.stream_state = State::SetReader;
        result
    }
}

/// Tokenizer for testing.
///
/// This tokenizer is a replacement for [`WHITESPACE`], [`SIMPLE`], and
/// [`KEYWORD`] tokenizers. If you are writing a component such as a
/// TokenFilter, it's a great idea to test it wrapping this tokenizer instead
/// for extra checks. This tokenizer has the following behavior:
///
/// * An internal state-machine is used for checking consumer consistency.
///   These checks can be disabled with
///   [`set_enable_checks`](Self::set_enable_checks).
/// * For convenience, optionally lowercases terms that it outputs.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockTokenizer`.
#[derive(Debug)]
pub struct MockTokenizer {
    inner: Tokenizer<MockTokenizerLogic>,
}

impl MockTokenizer {
    /// `MockTokenizer(AttributeFactory factory, CharacterRunAutomaton
    /// runAutomaton, boolean lowerCase, int maxTokenLength)`.
    ///
    /// The tokenizer's random is seeded from the next `nextLong()` of the
    /// calling thread's randomized context.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the factory provides no
    /// `CharTermAttribute` or `OffsetAttribute`.
    ///
    /// # Panics
    ///
    /// Outside a randomized context, as Java's `RandomizedContext.current()`.
    pub fn with_factory(
        factory: Arc<dyn AttributeFactory>,
        run_automaton: Arc<CharacterRunAutomaton>,
        lower_case: bool,
        max_token_length: i32,
    ) -> Result<MockTokenizer> {
        let mut source = AttributeSource::new_with_factory(factory);
        let term_att = Att::add(&mut source, char_term_id())?;
        let offset_att = Att::add(&mut source, offset_id())?;
        let random = JdkRandom::new(RandomizedContext::current().get_random().next_long());
        let logic = MockTokenizerLogic {
            run_automaton,
            lower_case,
            max_token_length,
            state: 0,
            term_att,
            offset_att,
            off: 0,
            buffered_code_point: -1,
            buffered_off: -1,
            stream_state: State::Close,
            last_offset: 0,
            enable_checks: true,
            random,
            pending_low_surrogate: None,
            term_length16: 0,
        };
        Ok(MockTokenizer {
            inner: Tokenizer::with_source(source, logic),
        })
    }

    /// `MockTokenizer(CharacterRunAutomaton runAutomaton, boolean lowerCase,
    /// int maxTokenLength)`: over
    /// [`BaseTokenStreamTestCase::new_attribute_factory`].
    ///
    /// # Errors
    ///
    /// As [`with_factory`](Self::with_factory).
    pub fn with_max_token_length(
        run_automaton: Arc<CharacterRunAutomaton>,
        lower_case: bool,
        max_token_length: i32,
    ) -> Result<MockTokenizer> {
        Self::with_factory(
            BaseTokenStreamTestCase::new_attribute_factory(),
            run_automaton,
            lower_case,
            max_token_length,
        )
    }

    /// `MockTokenizer(CharacterRunAutomaton runAutomaton, boolean
    /// lowerCase)`.
    ///
    /// # Errors
    ///
    /// As [`with_factory`](Self::with_factory).
    pub fn new(
        run_automaton: Arc<CharacterRunAutomaton>,
        lower_case: bool,
    ) -> Result<MockTokenizer> {
        Self::with_max_token_length(run_automaton, lower_case, DEFAULT_MAX_TOKEN_LENGTH)
    }

    /// `MockTokenizer()`: `MockTokenizer(WHITESPACE, true)`.
    ///
    /// # Errors
    ///
    /// As [`with_factory`](Self::with_factory).
    pub fn whitespace_lower_case() -> Result<MockTokenizer> {
        Self::new(Arc::clone(&WHITESPACE), true)
    }

    /// `MockTokenizer(AttributeFactory factory, CharacterRunAutomaton
    /// runAutomaton, boolean lowerCase)`.
    ///
    /// # Errors
    ///
    /// As [`with_factory`](Self::with_factory).
    pub fn with_factory_default_length(
        factory: Arc<dyn AttributeFactory>,
        run_automaton: Arc<CharacterRunAutomaton>,
        lower_case: bool,
    ) -> Result<MockTokenizer> {
        Self::with_factory(factory, run_automaton, lower_case, DEFAULT_MAX_TOKEN_LENGTH)
    }

    /// `MockTokenizer(AttributeFactory factory)`: `MockTokenizer(factory,
    /// WHITESPACE, true)`.
    ///
    /// # Errors
    ///
    /// As [`with_factory`](Self::with_factory).
    pub fn with_factory_only(factory: Arc<dyn AttributeFactory>) -> Result<MockTokenizer> {
        Self::with_factory_default_length(factory, Arc::clone(&WHITESPACE), true)
    }

    /// `Tokenizer.setReader(Reader)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when the previous input was not closed, or
    /// when the workflow check of `setReaderTestPoint` fails.
    pub fn set_reader(&mut self, reader: Box<dyn CharReader>) -> Result<()> {
        self.inner.set_reader(reader)
    }

    /// Toggle consumer workflow checking: if your test consumes tokenstreams
    /// normally you should leave this enabled.
    pub fn set_enable_checks(&mut self, enable_checks: bool) {
        self.inner.logic_mut().set_enable_checks(enable_checks);
    }

    /// The consumer workflow state.
    pub fn stream_state(&self) -> State {
        self.inner.logic().stream_state()
    }
}

impl TokenStream for MockTokenizer {
    fn increment_token(&mut self) -> Result<bool> {
        self.inner.increment_token()
    }

    fn end(&mut self) -> Result<()> {
        self.inner.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.inner.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.inner.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.inner.attribute_source_mut()
    }
}
