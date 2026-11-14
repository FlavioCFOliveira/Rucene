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

//! Breaks text into sentences with a `BreakIterator`, ported from
//! `org.apache.lucene.analysis.util.SegmentingTokenizerBase`.

use std::fmt::Debug;
use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, CharReader, OffsetAttribute, Tokenizer, TokenizerLogic,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeFactory, AttributeSource};

use super::char_array_iterator::CharArrayIterator;
use crate::jdk::break_iterator::{BreakIterator, DONE};

/// `SegmentingTokenizerBase.BUFFERMAX`: the size of the buffer, 1024 units.
pub const BUFFERMAX: usize = 1024;

/// The members of `SegmentingTokenizerBase` a subclass supplies: its two
/// abstract methods, and the methods it may override.
pub trait SegmentingTokenizerBaseLogic: Debug + Send + Sync {
    /// Provides the next input sentence for analysis.
    ///
    /// Equivalent to the abstract `setNextSentence(int, int)`. Java's
    /// subclass reads the protected `buffer` field; here it is `buffer`,
    /// the base's buffer of UTF-16 units.
    fn set_next_sentence(&mut self, buffer: &[u16], sentence_start: i32, sentence_end: i32);

    /// Returns true if another word is available.
    ///
    /// Equivalent to the abstract `incrementWord()`. Java's subclass reads the
    /// protected `buffer` and `offset` fields, sets its attributes, and
    /// corrects offsets with the inherited `correctOffset`; here they are
    /// `buffer`, `offset`, `source` and `input` (correct through
    /// `Tokenizer::correct_offset_of(Some(input), ..)`).
    fn increment_word(
        &mut self,
        source: &mut AttributeSource,
        buffer: &[u16],
        offset: i32,
        input: &dyn CharReader,
    ) -> bool;

    /// For sentence tokenization, these are the unambiguous break positions.
    ///
    /// Equivalent to the protected, overridable `isSafeEnd(char)`, whose
    /// default this is: CR, LF, NEL, LINE SEPARATOR and PARAGRAPH SEPARATOR.
    fn is_safe_end(&self, ch: u16) -> bool {
        matches!(ch, 0x000D | 0x000A | 0x0085 | 0x2028 | 0x2029)
    }

    /// What a subclass's `reset()` does after `super.reset()`; nothing by
    /// default.
    ///
    /// # Errors
    ///
    /// What the subclass's reset raises.
    fn reset(&mut self, _source: &mut AttributeSource) -> Result<()> {
        Ok(())
    }

    /// What a subclass's `close()` does after `super.close()`; nothing by
    /// default.
    ///
    /// # Errors
    ///
    /// What the subclass's close raises.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The body of [`SegmentingTokenizerBase`]: what Lucene's
/// `SegmentingTokenizerBase` adds to `Tokenizer`, over the subclass's
/// [`SegmentingTokenizerBaseLogic`].
#[derive(Debug)]
pub struct SegmentingTokenizerBaseAdapter<T: SegmentingTokenizerBaseLogic> {
    logic: T,
    /// The protected `buffer`, UTF-16 units.
    buffer: Vec<u16>,
    /// true length of text in the buffer
    length: i32,
    /// length in buffer that can be evaluated safely, up to a safe end point
    usable_length: i32,
    /// accumulated offset of previous buffers for this reader, for offsetAtt
    offset: i32,
    iterator: Box<dyn BreakIterator>,
    wrapper: CharArrayIterator,
}

impl<T: SegmentingTokenizerBaseLogic> SegmentingTokenizerBaseAdapter<T> {
    /// The fields of a `SegmentingTokenizerBase` built over `iterator`, the
    /// `BreakIterator` used for sentence segmentation.
    ///
    /// Note that you should never share BreakIterators across different
    /// TokenStreams, instead a newly created or cloned one should always be
    /// provided to this constructor.
    pub fn new(iterator: Box<dyn BreakIterator>, logic: T) -> Self {
        Self {
            logic,
            buffer: vec![0; BUFFERMAX],
            length: 0,
            usable_length: 0,
            offset: 0,
            iterator,
            wrapper: CharArrayIterator::new_sentence_instance(),
        }
    }

    /// The subclass's logic.
    pub fn logic(&self) -> &T {
        &self.logic
    }

    /// The subclass's logic, mutably.
    pub fn logic_mut(&mut self) -> &mut T {
        &mut self.logic
    }

    /// The protected `buffer`.
    pub fn buffer(&self) -> &[u16] {
        &self.buffer
    }

    /// The protected `offset`: the accumulated offset of previous buffers for
    /// this reader.
    pub fn offset(&self) -> i32 {
        self.offset
    }

    /// Returns the last unambiguous break position in the text.
    ///
    /// Equivalent to `findSafeEnd()`.
    fn find_safe_end(&self) -> i32 {
        for i in (0..self.length.max(0) as usize).rev() {
            if self.logic.is_safe_end(self.buffer[i]) {
                return i as i32 + 1;
            }
        }
        -1
    }

    /// Refill the buffer, accumulating the offset and setting usableLength to
    /// the last unambiguous break position.
    ///
    /// Equivalent to `refill()`.
    fn refill(&mut self, input: &mut dyn CharReader) -> Result<()> {
        self.offset += self.usable_length;
        let leftover = self.length - self.usable_length;
        self.buffer.copy_within(
            self.usable_length as usize..(self.usable_length + leftover) as usize,
            0,
        );
        let requested = self.buffer.len() as i32 - leftover;
        let returned = read(
            input,
            &mut self.buffer,
            leftover as usize,
            requested as usize,
        )?;
        self.length = if returned < 0 {
            leftover
        } else {
            returned + leftover
        };
        if returned < requested {
            // reader has been emptied, process the rest
            self.usable_length = self.length;
        } else {
            // still more data to be read, find a safe-stopping place
            self.usable_length = self.find_safe_end();
            if self.usable_length < 0 {
                // more than IOBUFFER of text without breaks, gonna possibly
                // truncate tokens
                self.usable_length = self.length;
            }
        }

        self.wrapper
            .set_text(&self.buffer, 0, self.usable_length.max(0));
        self.iterator.set_text(Box::new(self.wrapper.clone()));
        Ok(())
    }

    /// return true if there is a token from the buffer, or null if it is
    /// exhausted.
    ///
    /// Equivalent to `incrementSentence()`.
    fn increment_sentence(&mut self, source: &mut AttributeSource, input: &dyn CharReader) -> bool {
        if self.length == 0 {
            // we must refill the buffer
            return false;
        }

        loop {
            let start = self.iterator.current();

            if start == DONE {
                return false; // BreakIterator exhausted
            }

            // find the next set of boundaries
            let end = self.iterator.next();

            if end == DONE {
                return false; // BreakIterator exhausted
            }

            self.logic.set_next_sentence(&self.buffer, start, end);
            if self
                .logic
                .increment_word(source, &self.buffer, self.offset, input)
            {
                return true;
            }
        }
    }
}

/// commons-io's readFully, but without bugs if offset != 0.
///
/// Equivalent to the private static `read(Reader, char[], int, int)`.
fn read(
    input: &mut dyn CharReader,
    buffer: &mut [u16],
    offset: usize,
    length: usize,
) -> Result<i32> {
    let mut remaining = length;
    while remaining > 0 {
        let location = length - remaining;
        let count = input.read(&mut buffer[offset + location..offset + location + remaining])?;
        if count == 0 {
            // EOF: a `CharReader` answers 0 where Java's answers -1, and the
            // request is never empty here.
            break;
        }
        remaining -= count;
    }
    Ok((length - remaining) as i32)
}

impl<T: SegmentingTokenizerBaseLogic> TokenizerLogic for SegmentingTokenizerBaseAdapter<T> {
    /// Equivalent to the final `SegmentingTokenizerBase.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.length == 0
            || !self
                .logic
                .increment_word(source, &self.buffer, self.offset, &*input)
        {
            while !self.increment_sentence(source, &*input) {
                self.refill(input)?;
                if self.length <= 0 {
                    // no more bytes to read;
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    /// Equivalent to the final `SegmentingTokenizerBase.end()`;
    /// `Tokenizer::end` has run `super.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        let final_offset = Tokenizer::<Self>::correct_offset_of(
            input,
            if self.length < 0 {
                self.offset
            } else {
                self.offset + self.length
            },
        );
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(final_offset, final_offset);
        Ok(())
    }

    /// Equivalent to `SegmentingTokenizerBase.reset()`; `Tokenizer::reset`
    /// has run `super.reset()`, and the subclass's reset follows.
    fn reset(&mut self, source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.wrapper.set_text(&self.buffer, 0, 0);
        self.iterator.set_text(Box::new(self.wrapper.clone()));
        self.length = 0;
        self.usable_length = 0;
        self.offset = 0;
        self.logic.reset(source)
    }

    /// `Tokenizer.close()`, which `SegmentingTokenizerBase` does not
    /// override, and the subclass's close after it.
    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        self.logic.close()
    }
}

/// Breaks text into sentences with a `BreakIterator` and allows subclasses
/// to decompose these sentences into words.
///
/// This can be used by subclasses that need sentence context for
/// tokenization purposes, such as CJK segmenters.
///
/// Additionally it can be used by subclasses that want to mark sentence
/// boundaries (with a custom attribute, extra token, position increment, etc)
/// for downstream processing.
///
/// Equivalent to `org.apache.lucene.analysis.util.SegmentingTokenizerBase`
/// (`@lucene.experimental`): a [`Tokenizer`] whose body is a
/// [`SegmentingTokenizerBaseAdapter`] over the subclass's
/// [`SegmentingTokenizerBaseLogic`].
pub type SegmentingTokenizerBase<T> = Tokenizer<SegmentingTokenizerBaseAdapter<T>>;

/// The source of a `SegmentingTokenizerBase` over `factory`, holding the
/// attribute its field adds, the `OffsetAttribute`; a subclass adds its own
/// attributes after it, as its field initializers run after the base's.
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws.
pub fn segmenting_tokenizer_base_source(
    factory: Arc<dyn AttributeFactory>,
) -> Result<AttributeSource> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    Ok(source)
}

/// Construct a new SegmenterBase, using the provided BreakIterator for
/// sentence segmentation, over a source holding the subclass's attributes.
///
/// Equivalent to `SegmentingTokenizerBase(BreakIterator)` and
/// `SegmentingTokenizerBase(AttributeFactory, BreakIterator)`: `source` is
/// [`segmenting_tokenizer_base_source`] of the attribute factory
/// (`TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY` for the first), with the
/// subclass's attributes added after.
pub fn new_segmenting_tokenizer_base<T: SegmentingTokenizerBaseLogic>(
    source: AttributeSource,
    iterator: Box<dyn BreakIterator>,
    logic: T,
) -> SegmentingTokenizerBase<T> {
    Tokenizer::with_source(source, SegmentingTokenizerBaseAdapter::new(iterator, logic))
}

/// [`segmenting_tokenizer_base_source`] over
/// `TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY`, the source of
/// `SegmentingTokenizerBase(BreakIterator)`.
pub fn default_segmenting_tokenizer_base_source() -> AttributeSource {
    segmenting_tokenizer_base_source(default_token_attribute_factory())
        .expect("INVARIANT: the default token attribute factory implements the core attributes")
}
