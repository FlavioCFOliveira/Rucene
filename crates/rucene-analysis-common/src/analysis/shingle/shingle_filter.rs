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

//! Constructs shingles (token n-grams) from a token stream, ported from
//! `org.apache.lucene.analysis.shingle.ShingleFilter`.

use std::collections::VecDeque;

use rucene::analysis::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
    TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeSource, CapturedState};

/// filler token for when positionIncrement is more than 1
///
/// Equivalent to `ShingleFilter.DEFAULT_FILLER_TOKEN`.
pub const DEFAULT_FILLER_TOKEN: &str = "_";

/// default maximum shingle size is 2.
///
/// Equivalent to `ShingleFilter.DEFAULT_MAX_SHINGLE_SIZE`.
pub const DEFAULT_MAX_SHINGLE_SIZE: i32 = 2;

/// default minimum shingle size is 2.
///
/// Equivalent to `ShingleFilter.DEFAULT_MIN_SHINGLE_SIZE`.
pub const DEFAULT_MIN_SHINGLE_SIZE: i32 = 2;

/// default token type attribute value is "shingle"
///
/// Equivalent to `ShingleFilter.DEFAULT_TOKEN_TYPE`.
pub const DEFAULT_TOKEN_TYPE: &str = "shingle";

/// The default string to use when joining adjacent tokens to form a shingle
///
/// Equivalent to `ShingleFilter.DEFAULT_TOKEN_SEPARATOR`.
pub const DEFAULT_TOKEN_SEPARATOR: &str = " ";

/// The term and the offsets of `source`, which an `InputWindowToken` holds.
fn term_of(source: &AttributeSource) -> Vec<char> {
    let term = source
        .get_attribute_interface::<dyn CharTermAttribute>()
        .expect("INVARIANT: an input window token holds a CharTermAttribute");
    term.buffer()[..term.length()].to_vec()
}

fn start_offset_of(source: &AttributeSource) -> i32 {
    source
        .get_attribute_interface::<dyn OffsetAttribute>()
        .expect("INVARIANT: the source holds an OffsetAttribute")
        .start_offset()
}

fn end_offset_of(source: &AttributeSource) -> i32 {
    source
        .get_attribute_interface::<dyn OffsetAttribute>()
        .expect("INVARIANT: the source holds an OffsetAttribute")
        .end_offset()
}

fn set_offset_of(source: &AttributeSource, start: i32, end: i32) {
    source
        .get_attribute_interface_mut::<dyn OffsetAttribute>()
        .expect("INVARIANT: the source holds an OffsetAttribute")
        .set_offset(start, end);
}

fn position_increment_of(source: &AttributeSource) -> i32 {
    source
        .get_attribute_interface::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
        .get_position_increment()
}

/// The body of [`ShingleFilter`]: what Lucene's `ShingleFilter` adds to
/// `TokenFilter`, with its setters.
#[derive(Debug)]
pub struct ShingleFilterLogic {
    /// The sequence of input stream tokens (or filler tokens, if necessary)
    /// that will be composed to form output shingles.
    input_window: VecDeque<InputWindowToken>,
    /// The number of input tokens in the next output token. This is the "n"
    /// in "token n-grams".
    gram_size: CircularSequence,
    /// Shingle and unigram text is composed here.
    gram_builder: Vec<char>,
    /// The token type attribute value to use - default is "shingle"
    token_type: String,
    /// The string to use when joining adjacent tokens to form a shingle
    token_separator: String,
    /// The string to insert for each position at which there is no token
    /// (i.e., when position increment is greater than one).
    filler_token: Vec<char>,
    /// By default, we output unigrams (individual tokens) as well as
    /// shingles (token n-grams).
    output_unigrams: bool,
    /// By default, we don't override behavior of outputUnigrams.
    output_unigrams_if_no_shingles: bool,
    /// maximum shingle size (number of tokens)
    max_shingle_size: i32,
    /// minimum shingle size (number of tokens)
    min_shingle_size: i32,
    /// The remaining number of filler tokens to be inserted into the input
    /// stream from which shingles are composed, to handle position
    /// increments greater than one.
    num_filler_tokens_to_insert: i32,
    /// When the next input stream token has a position increment greater
    /// than one, it is stored in this field until sufficient filler tokens
    /// have been inserted to account for the position increment.
    next_input_stream_token: Option<AttributeSource>,
    /// Whether or not there is a next input stream token.
    is_next_input_stream_token: bool,
    /// Whether at least one unigram or shingle has been output at the
    /// current position.
    is_output_here: bool,
    /// true if no shingles have been output yet (for
    /// outputUnigramsIfNoShingles).
    pub(crate) no_shingle_output: bool,
    /// Holds the State after input.end() was called, so we can restore it in
    /// our end() impl.
    end_state: Option<CapturedState>,
    exhausted: bool,
}

impl ShingleFilterLogic {
    /// Set the type of the shingle tokens produced by this filter. (default:
    /// "shingle")
    ///
    /// Equivalent to `ShingleFilter.setTokenType(String)`; Java's
    /// `NullPointerException` for `null` has no counterpart.
    pub fn set_token_type(&mut self, token_type: impl Into<String>) {
        self.token_type = token_type.into();
    }

    /// Shall the output stream contain the input tokens (unigrams) as well
    /// as shingles? (default: true.)
    ///
    /// Equivalent to `ShingleFilter.setOutputUnigrams(boolean)`.
    pub fn set_output_unigrams(&mut self, output_unigrams: bool) {
        self.output_unigrams = output_unigrams;
        self.gram_size = CircularSequence::new(self.output_unigrams, self.min_shingle_size);
    }

    /// Shall we override the behavior of outputUnigrams==false for those
    /// times when no shingles are available (because there are fewer than
    /// minShingleSize tokens in the input stream)? (default: false.)
    ///
    /// Note that if outputUnigrams==true, then unigrams are always output,
    /// regardless of whether any shingles are available.
    ///
    /// Equivalent to `ShingleFilter.setOutputUnigramsIfNoShingles(boolean)`.
    pub fn set_output_unigrams_if_no_shingles(&mut self, output_unigrams_if_no_shingles: bool) {
        self.output_unigrams_if_no_shingles = output_unigrams_if_no_shingles;
    }

    /// Set the max shingle size (default: 2)
    ///
    /// Equivalent to `ShingleFilter.setMaxShingleSize(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "Max shingle size must be >= 2".
    pub fn set_max_shingle_size(&mut self, max_shingle_size: i32) -> Result<()> {
        if max_shingle_size < 2 {
            return Err(LuceneError::illegal_argument(
                "Max shingle size must be >= 2",
            ));
        }
        self.max_shingle_size = max_shingle_size;
        Ok(())
    }

    /// Set the min shingle size (default: 2).
    ///
    /// This method requires that the passed in minShingleSize is not greater
    /// than maxShingleSize, so make sure that maxShingleSize is set before
    /// calling this method.
    ///
    /// The unigram output option is independent of the min shingle size.
    ///
    /// Equivalent to `ShingleFilter.setMinShingleSize(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "Min shingle size must be >= 2" or "Min
    /// shingle size must be <= max shingle size".
    pub fn set_min_shingle_size(&mut self, min_shingle_size: i32) -> Result<()> {
        if min_shingle_size < 2 {
            return Err(LuceneError::illegal_argument(
                "Min shingle size must be >= 2",
            ));
        }
        if min_shingle_size > self.max_shingle_size {
            return Err(LuceneError::illegal_argument(
                "Min shingle size must be <= max shingle size",
            ));
        }
        self.min_shingle_size = min_shingle_size;
        self.gram_size = CircularSequence::new(self.output_unigrams, self.min_shingle_size);
        Ok(())
    }

    /// Sets the string to use when joining adjacent tokens to form a shingle
    ///
    /// Equivalent to `ShingleFilter.setTokenSeparator(String)`: `None` (Java's
    /// `null`) is the empty string.
    pub fn set_token_separator(&mut self, token_separator: Option<&str>) {
        self.token_separator = token_separator.unwrap_or("").to_string();
    }

    /// Sets the string to insert for each position at which there is no
    /// token (i.e., when position increment is greater than one).
    ///
    /// Equivalent to `ShingleFilter.setFillerToken(String)`: `None` (Java's
    /// `null`) is the empty string.
    pub fn set_filler_token(&mut self, filler_token: Option<&str>) {
        self.filler_token = filler_token.unwrap_or("").chars().collect();
    }

    /// Get the next token from the input stream.
    ///
    /// If the next token has `positionIncrement > 1`, `positionIncrement -
    /// 1` filler tokens are inserted first.
    ///
    /// Equivalent to the private `getNextToken(InputWindowToken)`: `target`
    /// is the token to fill (a new one when `None`); the filled token is
    /// returned, `None` at the end of the stream.
    fn get_next_token(
        &mut self,
        source: &AttributeSource,
        input: &mut dyn TokenStream,
        target: Option<InputWindowToken>,
    ) -> Result<Option<InputWindowToken>> {
        let new_target;
        if self.num_filler_tokens_to_insert > 0 {
            let next_input_stream_token = self
                .next_input_stream_token
                .as_ref()
                .expect("INVARIANT: filler tokens are inserted before a held token");
            let mut token = match target {
                None => InputWindowToken::new(next_input_stream_token.clone_attributes()),
                Some(target) => {
                    next_input_stream_token.copy_to(&target.att_source)?;
                    target
                }
            };
            // A filler token occupies no space
            let start = start_offset_of(&token.att_source);
            set_offset_of(&token.att_source, start, start);
            token.set_term(&self.filler_token);
            token.is_filler = true;
            self.num_filler_tokens_to_insert -= 1;
            new_target = Some(token);
        } else if self.is_next_input_stream_token {
            let next_input_stream_token = self
                .next_input_stream_token
                .as_ref()
                .expect("INVARIANT: the held token is set with its flag");
            let mut token = match target {
                None => InputWindowToken::new(next_input_stream_token.clone_attributes()),
                Some(target) => {
                    next_input_stream_token.copy_to(&target.att_source)?;
                    target
                }
            };
            self.is_next_input_stream_token = false;
            token.is_filler = false;
            new_target = Some(token);
        } else if !self.exhausted {
            if input.increment_token()? {
                let mut token = match target {
                    None => InputWindowToken::new(source.clone_attributes()),
                    Some(target) => {
                        source.copy_to(&target.att_source)?;
                        target
                    }
                };
                let position_increment = position_increment_of(source);
                if position_increment > 1 {
                    // Each output shingle must contain at least one input token,
                    // so no more than (maxShingleSize - 1) filler tokens will be inserted.
                    self.num_filler_tokens_to_insert =
                        (position_increment - 1).min(self.max_shingle_size - 1);
                    // Save the current token as the next input stream token
                    match &self.next_input_stream_token {
                        None => self.next_input_stream_token = Some(source.clone_attributes()),
                        Some(next) => source.copy_to(next)?,
                    }
                    self.is_next_input_stream_token = true;
                    // A filler token occupies no space
                    let start = start_offset_of(source);
                    set_offset_of(&token.att_source, start, start);
                    token.set_term(&self.filler_token);
                    token.is_filler = true;
                    self.num_filler_tokens_to_insert -= 1;
                } else {
                    token.is_filler = false;
                }
                new_target = Some(token);
            } else {
                self.exhausted = true;
                input.end()?;
                self.end_state = source.capture_state();
                self.num_filler_tokens_to_insert =
                    position_increment_of(source).min(self.max_shingle_size - 1);
                if self.num_filler_tokens_to_insert > 0 {
                    let mut next = AttributeSource::new_with_factory(source.attribute_factory());
                    next.add_attribute_interface::<dyn CharTermAttribute>()?;
                    next.add_attribute_interface::<dyn OffsetAttribute>()?;
                    let end = end_offset_of(source);
                    set_offset_of(&next, end, end);
                    self.next_input_stream_token = Some(next);
                    // Recurse/loop just once:
                    return self.get_next_token(source, input, target);
                } else {
                    new_target = None;
                }
            }
        } else {
            new_target = None;
        }
        Ok(new_target)
    }

    /// Fills `inputWindow` with input stream tokens, if available, shifting
    /// to the right if the window was previously full.
    ///
    /// Resets `gramSize` to its minimum value.
    ///
    /// Equivalent to the private `shiftInputWindow()`.
    fn shift_input_window(
        &mut self,
        source: &AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<()> {
        let mut first_token = self.input_window.pop_front();
        while (self.input_window.len() as i32) < self.max_shingle_size {
            if let Some(token) = first_token.take() {
                // recycle the firstToken, if available
                match self.get_next_token(source, input, Some(token))? {
                    // the firstToken becomes the last
                    Some(token) => self.input_window.push_back(token),
                    None => break, // end of input stream
                }
            } else {
                match self.get_next_token(source, input, None)? {
                    Some(next_token) => self.input_window.push_back(next_token),
                    None => break, // end of input stream
                }
            }
        }
        if self.output_unigrams_if_no_shingles
            && self.no_shingle_output
            && self.gram_size.min_value > 1
            && (self.input_window.len() as i32) < self.min_shingle_size
        {
            self.gram_size.min_value = 1;
        }
        self.gram_size.reset();
        self.is_output_here = false;
        Ok(())
    }
}

impl TokenFilterLogic for ShingleFilterLogic {
    /// Equivalent to `ShingleFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let mut token_available = false;
        let mut built_gram_size = 0;
        if self.gram_size.at_min_value()
            || (self.input_window.len() as i32) < self.gram_size.get_value()
        {
            self.shift_input_window(source, input)?;
            self.gram_builder.clear();
        } else {
            built_gram_size = self.gram_size.get_previous_value();
        }
        if self.input_window.len() as i32 >= self.gram_size.get_value() {
            let mut is_all_filler = true;
            let mut next_token: Option<usize> = None;
            let mut gram_num = 1;
            let mut index = 0;
            while index < self.input_window.len() && built_gram_size < self.gram_size.get_value() {
                next_token = Some(index);
                let token = &self.input_window[index];
                index += 1;
                if built_gram_size < gram_num {
                    if built_gram_size > 0 {
                        self.gram_builder.extend(self.token_separator.chars());
                    }
                    self.gram_builder.extend(term_of(&token.att_source));
                    built_gram_size += 1;
                }
                if is_all_filler && token.is_filler {
                    if gram_num == self.gram_size.get_value() {
                        self.gram_size
                            .advance(self.min_shingle_size, self.max_shingle_size);
                    }
                } else {
                    is_all_filler = false;
                }
                gram_num += 1;
            }
            if !is_all_filler && built_gram_size == self.gram_size.get_value() {
                self.input_window
                    .front()
                    .expect("INVARIANT: the window holds the gram's tokens")
                    .att_source
                    .copy_to(source)?;
                source
                    .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                    .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                    .set_position_increment(if self.is_output_here { 0 } else { 1 });
                source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute")
                    .copy_buffer(&self.gram_builder, 0, self.gram_builder.len());
                if self.gram_size.get_value() > 1 {
                    source
                        .get_attribute_interface_mut::<dyn TypeAttribute>()
                        .expect("INVARIANT: the constructor added the TypeAttribute")
                        .set_type(self.token_type.clone());
                    self.no_shingle_output = false;
                }
                let next_token =
                    &self.input_window[next_token.expect("INVARIANT: the loop ran at least once")];
                let end_offset = end_offset_of(&next_token.att_source);
                let start_offset = start_offset_of(source);
                set_offset_of(source, start_offset, end_offset);
                let position_length = if self.output_unigrams {
                    built_gram_size
                } else {
                    // position length for this token is the number of position created by shingles of smaller
                    // size.
                    1.max((built_gram_size - self.min_shingle_size) + 1)
                };
                source
                    .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
                    .expect("INVARIANT: the constructor added the PositionLengthAttribute")
                    .set_position_length(position_length);
                self.is_output_here = true;
                self.gram_size
                    .advance(self.min_shingle_size, self.max_shingle_size);
                token_available = true;
            }
        }
        Ok(token_available)
    }

    /// Equivalent to `ShingleFilter.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        if !self.exhausted {
            input.end()
        } else {
            if let Some(end_state) = &self.end_state {
                source.restore_state(end_state)?;
            }
            Ok(())
        }
    }

    /// Equivalent to `ShingleFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.gram_size.reset();
        self.input_window.clear();
        self.next_input_stream_token = None;
        self.is_next_input_stream_token = false;
        self.num_filler_tokens_to_insert = 0;
        self.is_output_here = false;
        self.no_shingle_output = true;
        self.exhausted = false;
        self.end_state = None;
        if self.output_unigrams_if_no_shingles && !self.output_unigrams {
            // Fix up gramSize if minValue was reset for outputUnigramsIfNoShingles
            self.gram_size.min_value = self.min_shingle_size;
        }
        Ok(())
    }
}

/// A ShingleFilter constructs shingles (token n-grams) from a token stream.
/// In other words, it creates combinations of tokens as a single token.
///
/// For example, the sentence "please divide this sentence into shingles"
/// might be tokenized into shingles "please divide", "divide this", "this
/// sentence", "sentence into", and "into shingles".
///
/// This filter handles position increments > 1 by inserting filler tokens
/// (tokens with termtext "_"). It does not handle a position increment of 0.
///
/// Equivalent to `org.apache.lucene.analysis.shingle.ShingleFilter`; its
/// setters are those of [`ShingleFilterLogic`], reached through
/// [`TokenFilter::logic_mut`].
pub type ShingleFilter = TokenFilter<ShingleFilterLogic>;

/// Constructs a ShingleFilter with the specified shingle size from the
/// `TokenStream` `input`
///
/// * `input` input stream
/// * `min_shingle_size` minimum shingle size produced by the filter.
/// * `max_shingle_size` maximum shingle size produced by the filter.
///
/// Equivalent to `ShingleFilter(TokenStream, int, int)`, which adds the
/// `CharTermAttribute`, `OffsetAttribute`, `PositionIncrementAttribute`,
/// `PositionLengthAttribute` and `TypeAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws, and what
/// [`ShingleFilterLogic::set_max_shingle_size`] and
/// [`ShingleFilterLogic::set_min_shingle_size`] return.
pub fn new_shingle_filter(
    input: Box<dyn TokenStream>,
    min_shingle_size: i32,
    max_shingle_size: i32,
) -> Result<ShingleFilter> {
    let logic = ShingleFilterLogic {
        input_window: VecDeque::new(),
        // Java's field is `null` until `setMinShingleSize` sets it below.
        gram_size: CircularSequence::new(true, 0),
        gram_builder: Vec::new(),
        token_type: DEFAULT_TOKEN_TYPE.to_string(),
        token_separator: DEFAULT_TOKEN_SEPARATOR.to_string(),
        filler_token: DEFAULT_FILLER_TOKEN.chars().collect(),
        output_unigrams: true,
        output_unigrams_if_no_shingles: false,
        max_shingle_size: 0,
        min_shingle_size: 0,
        num_filler_tokens_to_insert: 0,
        next_input_stream_token: None,
        is_next_input_stream_token: false,
        is_output_here: false,
        no_shingle_output: true,
        end_state: None,
        exhausted: false,
    };
    let mut filter = TokenFilter::new(input, logic);
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    filter.logic_mut().set_max_shingle_size(max_shingle_size)?;
    filter.logic_mut().set_min_shingle_size(min_shingle_size)?;
    Ok(filter)
}

/// Constructs a ShingleFilter with the specified shingle size from the
/// `TokenStream` `input`
///
/// * `input` input stream
/// * `max_shingle_size` maximum shingle size produced by the filter.
///
/// Equivalent to `ShingleFilter(TokenStream, int)`, `this(input,
/// DEFAULT_MIN_SHINGLE_SIZE, maxShingleSize)`.
///
/// # Errors
///
/// As [`new_shingle_filter`].
pub fn new_shingle_filter_with_max(
    input: Box<dyn TokenStream>,
    max_shingle_size: i32,
) -> Result<ShingleFilter> {
    new_shingle_filter(input, DEFAULT_MIN_SHINGLE_SIZE, max_shingle_size)
}

/// Construct a ShingleFilter with default shingle size: 2.
///
/// Equivalent to `ShingleFilter(TokenStream)`.
///
/// # Errors
///
/// As [`new_shingle_filter`].
pub fn new_shingle_filter_default(input: Box<dyn TokenStream>) -> Result<ShingleFilter> {
    new_shingle_filter(input, DEFAULT_MIN_SHINGLE_SIZE, DEFAULT_MAX_SHINGLE_SIZE)
}

/// Construct a ShingleFilter with the specified token type for shingle
/// tokens and the default shingle size: 2
///
/// Equivalent to `ShingleFilter(TokenStream, String)`.
///
/// # Errors
///
/// As [`new_shingle_filter`].
pub fn new_shingle_filter_with_token_type(
    input: Box<dyn TokenStream>,
    token_type: &str,
) -> Result<ShingleFilter> {
    let mut filter = new_shingle_filter(input, DEFAULT_MIN_SHINGLE_SIZE, DEFAULT_MAX_SHINGLE_SIZE)?;
    filter.logic_mut().set_token_type(token_type);
    Ok(filter)
}

/// An instance of this class is used to maintain the number of input stream
/// tokens that will be used to compose the next unigram or shingle:
/// `gramSize`.
///
/// `gramSize` will take on values from the circular sequence **{ [ 1, ]
/// minShingleSize [ , ... , maxShingleSize ] }**.
///
/// 1 is included in the circular sequence only if outputUnigrams = true.
///
/// Equivalent to `org.apache.lucene.analysis.shingle.ShingleFilter$CircularSequence`,
/// an inner class: the outer fields it reads (`outputUnigrams`,
/// `minShingleSize`, `maxShingleSize`) are passed to the constructor and to
/// [`advance`](Self::advance).
#[derive(Debug, Clone)]
pub(crate) struct CircularSequence {
    value: i32,
    previous_value: i32,
    min_value: i32,
}

impl CircularSequence {
    /// Equivalent to `CircularSequence()`.
    fn new(output_unigrams: bool, min_shingle_size: i32) -> Self {
        let mut sequence = Self {
            value: 0,
            previous_value: 0,
            min_value: if output_unigrams { 1 } else { min_shingle_size },
        };
        sequence.reset();
        sequence
    }

    /// the current value.
    ///
    /// Equivalent to `getValue()`.
    fn get_value(&self) -> i32 {
        self.value
    }

    /// Increments this circular number's value to the next member in the
    /// circular sequence `gramSize` will take on values from the circular
    /// sequence **{ [ 1, ] minShingleSize [ , ... , maxShingleSize ] }**.
    ///
    /// 1 is included in the circular sequence only if outputUnigrams = true.
    ///
    /// Equivalent to `advance()`.
    fn advance(&mut self, min_shingle_size: i32, max_shingle_size: i32) {
        self.previous_value = self.value;
        if self.value == 1 {
            self.value = min_shingle_size;
        } else if self.value == max_shingle_size {
            self.reset();
        } else {
            self.value += 1;
        }
    }

    /// Sets this circular number's value to the first member of the circular
    /// sequence
    ///
    /// Equivalent to `reset()`.
    fn reset(&mut self) {
        self.value = self.min_value;
        self.previous_value = self.min_value;
    }

    /// Returns true if the current value is the first member of the circular
    /// sequence.
    ///
    /// If outputUnigrams = true, the first member of the circular sequence
    /// will be 1; otherwise, it will be minShingleSize.
    ///
    /// Equivalent to `atMinValue()`.
    fn at_min_value(&self) -> bool {
        self.value == self.min_value
    }

    /// the value this instance had before the last advance() call
    ///
    /// Equivalent to `getPreviousValue()`.
    fn get_previous_value(&self) -> i32 {
        self.previous_value
    }
}

/// A token of the input window: its attributes and whether it is a filler.
///
/// Equivalent to `org.apache.lucene.analysis.shingle.ShingleFilter$InputWindowToken`;
/// Java's `termAtt` and `offsetAtt` fields are read from `att_source` when
/// needed.
#[derive(Debug)]
pub(crate) struct InputWindowToken {
    att_source: AttributeSource,
    is_filler: bool,
}

impl InputWindowToken {
    /// Equivalent to `InputWindowToken(AttributeSource)`.
    fn new(att_source: AttributeSource) -> Self {
        Self {
            att_source,
            is_filler: false,
        }
    }

    /// `termAtt.copyBuffer(buffer, 0, buffer.length)`.
    fn set_term(&mut self, buffer: &[char]) {
        self.att_source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: an input window token holds a CharTermAttribute")
            .copy_buffer(buffer, 0, buffer.len());
    }
}
