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

//! Forms bigrams of CJK terms that are generated from StandardTokenizer or
//! ICUTokenizer, ported from `org.apache.lucene.analysis.cjk.CJKBigramFilter`.

use rucene::analysis::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
    TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute, HANGUL as STANDARD_HANGUL,
    HIRAGANA as STANDARD_HIRAGANA, IDEOGRAPHIC, KATAKANA as STANDARD_KATAKANA, TOKEN_TYPES,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};

use crate::jdk::character::{char_count, chars_to_utf16, code_point_at, to_term_char};

/// bigram flag for Han Ideographs: `CJKBigramFilter.HAN`.
pub const HAN: i32 = 1;

/// bigram flag for Hiragana: `CJKBigramFilter.HIRAGANA`.
pub const HIRAGANA: i32 = 2;

/// bigram flag for Katakana: `CJKBigramFilter.KATAKANA`.
pub const KATAKANA: i32 = 4;

/// bigram flag for Hangul: `CJKBigramFilter.HANGUL`.
pub const HANGUL: i32 = 8;

/// when we emit a bigram, it's then marked as this type:
/// `CJKBigramFilter.DOUBLE_TYPE`.
pub const DOUBLE_TYPE: &str = "<DOUBLE>";

/// when we emit a unigram, it's then marked as this type:
/// `CJKBigramFilter.SINGLE_TYPE`.
pub const SINGLE_TYPE: &str = "<SINGLE>";

/// Java's `type == doX`, where `doX` is one of `StandardTokenizer`'s
/// `TOKEN_TYPES` strings: whether `type_value` is that very string, compared
/// by reference as Java compares objects with `==`.
///
/// **Divergence from Lucene 10.5.0.** Java interns every string literal, so a
/// type set from any `"<IDEOGRAPHIC>"` literal of any class is the same
/// object as `StandardTokenizer.TOKEN_TYPES[IDEOGRAPHIC]`; Rust does not
/// intern, so only a type set from the element of
/// [`TOKEN_TYPES`](rucene::analysis::TOKEN_TYPES) itself (as
/// `StandardTokenizer` sets it, and as attribute copies keep it) is that
/// string here. A type built at run time is a different object in both
/// languages.
fn is_same_string(type_value: &str, constant: &'static str) -> bool {
    std::ptr::eq(type_value.as_ptr(), constant.as_ptr()) && type_value.len() == constant.len()
}

/// The body of [`CJKBigramFilter`]: what Lucene's `CJKBigramFilter` adds to
/// `TokenFilter`.
///
/// Java's sentinel `NO` (an `Object` no type equals) is `None`: a script set
/// to `None` is passed thru.
#[derive(Debug)]
pub struct CJKBigramFilterLogic {
    // these are set to either their type or NO if we want to pass them thru
    do_han: Option<&'static str>,
    do_hiragana: Option<&'static str>,
    do_katakana: Option<&'static str>,
    do_hangul: Option<&'static str>,

    // true if we should output unigram tokens always
    output_unigrams: bool,
    ngram_state: bool, // false = output unigram, true = output bigram

    // buffers containing codepoint and offsets in parallel
    buffer: Vec<i32>,
    start_offset: Vec<i32>,
    end_offset: Vec<i32>,
    // length of valid buffer
    buffer_len: usize,
    // current buffer index
    index: usize,

    // the last end offset, to determine if we should bigram across tokens
    last_end_offset: i32,

    exhausted: bool,

    // rarely used: only for "lone cjk characters", where we emit unigrams
    lone_state: Option<CapturedState>,
}

impl CJKBigramFilterLogic {
    /// The fields of a `CJKBigramFilter` over `flags` and `output_unigrams`.
    fn new(flags: i32, output_unigrams: bool) -> Self {
        let pick = |flag: i32, token_type: usize| {
            if flags & flag == 0 {
                None
            } else {
                Some(TOKEN_TYPES[token_type])
            }
        };
        Self {
            do_han: pick(HAN, IDEOGRAPHIC),
            do_hiragana: pick(HIRAGANA, STANDARD_HIRAGANA),
            do_katakana: pick(KATAKANA, STANDARD_KATAKANA),
            do_hangul: pick(HANGUL, STANDARD_HANGUL),
            output_unigrams,
            ngram_state: false,
            buffer: vec![0; 8],
            start_offset: vec![0; 8],
            end_offset: vec![0; 8],
            buffer_len: 0,
            index: 0,
            last_end_offset: 0,
            exhausted: false,
            lone_state: None,
        }
    }

    /// `type == doHan || type == doHiragana || type == doKatakana || type ==
    /// doHangul`.
    fn is_cjk_type(&self, type_value: &str) -> bool {
        [
            self.do_han,
            self.do_hiragana,
            self.do_katakana,
            self.do_hangul,
        ]
        .into_iter()
        .flatten()
        .any(|constant| is_same_string(type_value, constant))
    }

    /// looks at next input token, returning false is none is available
    ///
    /// Equivalent to `doNext()`.
    fn do_next(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if let Some(lone_state) = self.lone_state.take() {
            source.restore_state(&lone_state)?;
            Ok(true)
        } else if self.exhausted {
            Ok(false)
        } else if input.increment_token()? {
            Ok(true)
        } else {
            self.exhausted = true;
            Ok(false)
        }
    }

    /// refills buffers with new data from the current token.
    ///
    /// Equivalent to `refill()`. Java walks the term's UTF-16 units with
    /// `Character.codePointAt`; here the term's units are those of its
    /// `char`s.
    fn refill(&mut self, source: &AttributeSource) {
        // compact buffers to keep them smallish if they become large
        // just a safety check, but technically we only need the last codepoint
        if self.buffer_len > 64 {
            let last = self.buffer_len - 1;
            self.buffer[0] = self.buffer[last];
            self.start_offset[0] = self.start_offset[last];
            self.end_offset[0] = self.end_offset[last];
            self.buffer_len = 1;
            self.index -= last;
        }

        let term_buffer = {
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            chars_to_utf16(&term_att.buffer()[..term_att.length()])
        };
        let len = term_buffer.len();
        let (mut start, end) = {
            let offset_att = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute");
            (offset_att.start_offset(), offset_att.end_offset())
        };

        let new_size = self.buffer_len + len;
        // `ArrayUtil.grow`: only the length matters, the arrays being read
        // up to `bufferLen`.
        if self.buffer.len() < new_size {
            self.buffer.resize(new_size, 0);
            self.start_offset.resize(new_size, 0);
            self.end_offset.resize(new_size, 0);
        }
        self.last_end_offset = end;

        if end - start != len as i32 {
            // crazy offsets (modified by synonym or charfilter): just preserve
            let mut i = 0;
            while i < len {
                let cp = code_point_at(&term_buffer, i, len);
                self.buffer[self.buffer_len] = cp;
                self.start_offset[self.buffer_len] = start;
                self.end_offset[self.buffer_len] = end;
                self.buffer_len += 1;
                i += char_count(cp);
            }
        } else {
            // normal offsets
            let mut i = 0;
            while i < len {
                let cp = code_point_at(&term_buffer, i, len);
                self.buffer[self.buffer_len] = cp;
                let cp_len = char_count(cp);
                self.start_offset[self.buffer_len] = start;
                start += cp_len as i32;
                self.end_offset[self.buffer_len] = start;
                self.buffer_len += 1;
                i += cp_len;
            }
        }
    }

    /// Flushes a bigram token to output from our buffer This is the normal
    /// case, e.g. ABC -&gt; AB BC
    ///
    /// Equivalent to `flushBigram()`.
    fn flush_bigram(&mut self, source: &AttributeSource) {
        source.clear_attributes();
        {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            // maximum bigram length in code units (2 supplementaries)
            let chars = [
                to_term_char(self.buffer[self.index]),
                to_term_char(self.buffer[self.index + 1]),
            ];
            term_att.copy_buffer(&chars, 0, 2);
        }
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(
                self.start_offset[self.index],
                self.end_offset[self.index + 1],
            );
        source
            .get_attribute_interface_mut::<dyn TypeAttribute>()
            .expect("INVARIANT: the constructor added the TypeAttribute")
            .set_type_static(DOUBLE_TYPE);
        // when outputting unigrams, all bigrams are synonyms that span two
        // unigrams
        if self.output_unigrams {
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(0);
            source
                .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
                .expect("INVARIANT: the constructor added the PositionLengthAttribute")
                .set_position_length(2);
        }
        self.index += 1;
    }

    /// Flushes a unigram token to output from our buffer. This happens when
    /// we encounter isolated CJK characters, either the whole CJK string is a
    /// single character, or we encounter a CJK character surrounded by space,
    /// punctuation, english, etc, but not beside any other CJK.
    ///
    /// Equivalent to `flushUnigram()`.
    fn flush_unigram(&mut self, source: &AttributeSource) {
        source.clear_attributes();
        {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            // maximum unigram length (2 surrogates)
            let chars = [to_term_char(self.buffer[self.index])];
            term_att.copy_buffer(&chars, 0, 1);
        }
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(self.start_offset[self.index], self.end_offset[self.index]);
        source
            .get_attribute_interface_mut::<dyn TypeAttribute>()
            .expect("INVARIANT: the constructor added the TypeAttribute")
            .set_type_static(SINGLE_TYPE);
        self.index += 1;
    }

    /// True if we have multiple codepoints sitting in our buffer
    ///
    /// Equivalent to `hasBufferedBigram()`.
    fn has_buffered_bigram(&self) -> bool {
        self.buffer_len as i64 - self.index as i64 > 1
    }

    /// True if we have a single codepoint sitting in our buffer, where its
    /// future (whether it is emitted as unigram or forms a bigram) depends
    /// upon not-yet-seen inputs.
    ///
    /// Equivalent to `hasBufferedUnigram()`.
    fn has_buffered_unigram(&self) -> bool {
        if self.output_unigrams {
            // when outputting unigrams always
            self.buffer_len as i64 - self.index as i64 == 1
        } else {
            // otherwise it's only when we have a lone CJK character
            self.buffer_len == 1 && self.index == 0
        }
    }
}

impl TokenFilterLogic for CJKBigramFilterLogic {
    /// Equivalent to `CJKBigramFilter.incrementToken()`.
    ///
    /// much of this complexity revolves around handling the special case of
    /// a "lone cjk character" where cjktokenizer would output a unigram. this
    /// is also the only time we ever have to captureState.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        loop {
            if self.has_buffered_bigram() {
                // case 1: we have multiple remaining codepoints buffered,
                // so we can emit a bigram here.

                if self.output_unigrams {
                    // when also outputting unigrams, we output the unigram first,
                    // then rewind back to revisit the bigram.
                    // so an input of ABC is A + (rewind)AB + B + (rewind)BC + C
                    // the logic in hasBufferedUnigram ensures we output the C,
                    // even though it did actually have adjacent CJK characters.

                    if self.ngram_state {
                        self.flush_bigram(source);
                    } else {
                        self.flush_unigram(source);
                        self.index -= 1;
                    }
                    self.ngram_state = !self.ngram_state;
                } else {
                    self.flush_bigram(source);
                }
                return Ok(true);
            } else if self.do_next(source, input)? {
                // case 2: look at the token type. should we form any n-grams?

                let is_cjk = {
                    let type_att = source
                        .get_attribute_interface::<dyn TypeAttribute>()
                        .expect("INVARIANT: the constructor added the TypeAttribute");
                    self.is_cjk_type(type_att.type_value())
                };
                if is_cjk {
                    // acceptable CJK type: we form n-grams from these.
                    // as long as the offsets are aligned, we just add these to our current buffer.
                    // otherwise, we clear the buffer and start over.

                    let start_offset = source
                        .get_attribute_interface::<dyn OffsetAttribute>()
                        .expect("INVARIANT: the constructor added the OffsetAttribute")
                        .start_offset();
                    if start_offset != self.last_end_offset {
                        // unaligned, clear queue
                        if self.has_buffered_unigram() {
                            // we have a buffered unigram, and we peeked ahead to see if we could form
                            // a bigram, but we can't, because the offsets are unaligned. capture the state
                            // of this peeked data to be revisited next time thru the loop, and dump our unigram.

                            self.lone_state = source.capture_state();
                            self.flush_unigram(source);
                            return Ok(true);
                        }
                        self.index = 0;
                        self.buffer_len = 0;
                    }
                    self.refill(source);
                } else {
                    // not a CJK type: we just return these as-is.

                    if self.has_buffered_unigram() {
                        // we have a buffered unigram, and we peeked ahead to see if we could form
                        // a bigram, but we can't, because it's not a CJK type. capture the state
                        // of this peeked data to be revisited next time thru the loop, and dump our unigram.

                        self.lone_state = source.capture_state();
                        self.flush_unigram(source);
                        return Ok(true);
                    }
                    return Ok(true);
                }
            } else {
                // case 3: we have only zero or 1 codepoints buffered,
                // so not enough to form a bigram. But, we also have no
                // more input. So if we have a buffered codepoint, emit
                // a unigram, otherwise, it's end of stream.

                if self.has_buffered_unigram() {
                    self.flush_unigram(source); // flush our remaining unigram
                    return Ok(true);
                }
                return Ok(false);
            }
        }
    }

    /// Equivalent to `CJKBigramFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.buffer_len = 0;
        self.index = 0;
        self.last_end_offset = 0;
        self.lone_state = None;
        self.exhausted = false;
        self.ngram_state = false;
        Ok(())
    }
}

/// Forms bigrams of CJK terms that are generated from StandardTokenizer or
/// ICUTokenizer.
///
/// CJK types are set by these tokenizers, but you can also use
/// [`new_cjk_bigram_filter_with_flags`] to explicitly control which of the
/// CJK scripts are turned into bigrams.
///
/// By default, when a CJK character has no adjacent characters to form a
/// bigram, it is output in unigram form. If you want to always output both
/// unigrams and bigrams, set the `outputUnigrams` flag in
/// [`new_cjk_bigram_filter_with_unigrams`]. This can be used for a combined
/// unigram+bigram approach.
///
/// Unlike ICUTokenizer, StandardTokenizer does not split at script
/// boundaries. Korean Hangul characters are treated the same as many other
/// scripts' letters, and as a result, StandardTokenizer can produce tokens
/// that mix Hangul and non-Hangul characters, e.g. "한국abc". Such
/// mixed-script tokens are typed as `<ALPHANUM>` rather than `<HANGUL>`, and
/// as a result, will not be converted to bigrams by CJKBigramFilter.
///
/// In all cases, all non-CJK input is passed thru unmodified.
///
/// Equivalent to `org.apache.lucene.analysis.cjk.CJKBigramFilter`
/// (`@IgnoreRandomChains`, LUCENE-8092: doesn't handle graph inputs).
pub type CJKBigramFilter = TokenFilter<CJKBigramFilterLogic>;

/// Calls [`new_cjk_bigram_filter_with_flags`]`(input, HAN | HIRAGANA |
/// KATAKANA | HANGUL)`.
///
/// Equivalent to `CJKBigramFilter(TokenStream)`.
///
/// # Errors
///
/// What `addAttribute` raises for an attribute the input's factory cannot
/// implement.
pub fn new_cjk_bigram_filter(input: Box<dyn TokenStream>) -> Result<CJKBigramFilter> {
    new_cjk_bigram_filter_with_flags(input, HAN | HIRAGANA | KATAKANA | HANGUL)
}

/// Calls [`new_cjk_bigram_filter_with_unigrams`]`(input, flags, false)`.
///
/// Equivalent to `CJKBigramFilter(TokenStream, int)`.
///
/// # Errors
///
/// As [`new_cjk_bigram_filter`].
pub fn new_cjk_bigram_filter_with_flags(
    input: Box<dyn TokenStream>,
    flags: i32,
) -> Result<CJKBigramFilter> {
    new_cjk_bigram_filter_with_unigrams(input, flags, false)
}

/// Create a new CJKBigramFilter, specifying which writing systems should be
/// bigrammed, and whether or not unigrams should also be output.
///
/// `flags` is an OR'ed set from [`HAN`], [`HIRAGANA`], [`KATAKANA`],
/// [`HANGUL`]; `output_unigrams` is true if unigrams
/// for the selected writing systems should also be output. when this is
/// false, this is only done when there are no adjacent characters to form a
/// bigram.
///
/// Equivalent to `CJKBigramFilter(TokenStream, int, boolean)`.
///
/// # Errors
///
/// As [`new_cjk_bigram_filter`].
pub fn new_cjk_bigram_filter_with_unigrams(
    input: Box<dyn TokenStream>,
    flags: i32,
    output_unigrams: bool,
) -> Result<CJKBigramFilter> {
    let mut filter = TokenFilter::new(input, CJKBigramFilterLogic::new(flags, output_unigrams));
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    Ok(filter)
}
