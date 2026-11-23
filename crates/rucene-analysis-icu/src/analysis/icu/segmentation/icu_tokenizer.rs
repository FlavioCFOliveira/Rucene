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

//! Breaks text into words according to UAX #29: Unicode Text Segmentation,
//! ported from `org.apache.lucene.analysis.icu.segmentation.ICUTokenizer`.

use std::any::TypeId;
use std::borrow::Cow;
use std::sync::Arc;

use icu4j::lang::u_character::UCharacter;
use icu4j::text::break_iterator::DONE;
use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute, Tokenizer,
    TokenizerLogic, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeFactory, AttributeSource};
use rucene_analysis_common::jdk::character::utf16_to_term_chars;

use super::composite_break_iterator::CompositeBreakIterator;
use super::default_icu_tokenizer_config::DefaultICUTokenizerConfig;
use super::icu_tokenizer_config::ICUTokenizerConfig;
use crate::analysis::icu::tokenattributes::script_attribute::ScriptAttribute;
use crate::analysis::icu::tokenattributes::script_attribute_impl::ScriptAttributeImpl;

/// `IOBUFFER`: the size of the read buffer.
const IOBUFFER: usize = 4096;

/// Breaks text into words according to UAX #29: Unicode Text Segmentation
/// (<http://www.unicode.org/reports/tr29/>)
///
/// Words are broken across script boundaries, then segmented according to
/// the BreakIterator and typing provided by the [`ICUTokenizerConfig`].
///
/// See [`ICUTokenizerConfig`].
///
/// Equivalent to the body of `org.apache.lucene.analysis.icu.segmentation.ICUTokenizer`
/// (a final `Tokenizer`, `@lucene.experimental`): its fields and its
/// overrides of `incrementToken()`, `reset()` and `end()`. The tokenizer is
/// [`ICUTokenizer`].
///
/// **Divergence from Lucene 10.5.0 (inherited).** Java's break iterators
/// read the tokenizer's `char[]` buffer itself; here they share a snapshot
/// of it (`Arc<[u16]>`) taken whenever the buffer is set on them (`reset`
/// and every `refill`), the shape of the iterators in
/// [`char_array_iterator`](super::char_array_iterator). Lucene sets the text
/// again after every change to the buffer, so the iterators see the same
/// characters.
#[derive(Debug)]
pub struct ICUTokenizerLogic {
    buffer: Vec<u16>,
    /// true length of text in the buffer
    length: i32,
    /// length in buffer that can be evaluated safely, up to a safe end point
    usable_length: i32,
    /// accumulated offset of previous buffers for this reader, for offsetAtt
    offset: i32,

    breaker: CompositeBreakIterator, /* tokenizes a char[] of text */
    config: Arc<dyn ICUTokenizerConfig>,
}

impl ICUTokenizerLogic {
    /// The config this tokenizer segments with.
    pub fn config(&self) -> &Arc<dyn ICUTokenizerConfig> {
        &self.config
    }

    /// The buffer as the text of the break iterators: a snapshot of the
    /// whole array, as Java hands them the array itself.
    fn text(&self) -> Arc<[u16]> {
        Arc::from(&self.buffer[..])
    }

    /*
     * This tokenizes text based upon the longest matching rule, and because of
     * this, isn't friendly to a Reader.
     *
     * Text is read from the input stream in 4kB chunks. Within a 4kB chunk of
     * text, the last unambiguous break point is found (in this implementation:
     * white space character) Any remaining characters represent possible partial
     * words, so are appended to the front of the next chunk.
     *
     * There is the possibility that there are no unambiguous break points within
     * an entire 4kB chunk of text (binary data). So there is a maximum word limit
     * of 4kB since it will not try to grow the buffer in this case.
     */

    /// Returns the last unambiguous break position in the text: the
    /// position of character, or -1 if one does not exist.
    ///
    /// Equivalent to `findSafeEnd()`.
    fn find_safe_end(&self) -> i32 {
        let mut i = self.length - 1;
        while i >= 0 {
            if UCharacter::is_whitespace(i32::from(self.buffer[i as usize])) {
                return i + 1;
            }
            i -= 1;
        }
        -1
    }

    /// Refill the buffer, accumulating the offset and setting usableLength
    /// to the last unambiguous break position.
    ///
    /// Equivalent to `refill()`.
    ///
    /// # Errors
    ///
    /// If there is a low-level I/O error.
    fn refill(&mut self, input: &mut dyn CharReader) -> Result<()> {
        self.offset += self.usable_length;
        let leftover = self.length - self.usable_length;
        self.buffer.copy_within(
            self.usable_length as usize..(self.usable_length + leftover) as usize,
            0,
        );
        let requested = self.buffer.len() as i32 - leftover;
        let returned = Self::read(input, &mut self.buffer, leftover, requested)?;
        self.length = returned + leftover;
        if returned < requested {
            /* reader has been emptied, process the rest */
            self.usable_length = self.length;
        } else {
            /* still more data to be read, find a safe-stopping place */
            self.usable_length = self.find_safe_end();
            if self.usable_length < 0 {
                /*
                 * more than IOBUFFER of text without space,
                 * gonna possibly truncate tokens
                 */
                self.usable_length = self.length;
            }
        }

        let text = self.text();
        self.breaker.set_text(text, 0, self.usable_length.max(0));
        Ok(())
    }

    // TODO: refactor to a shared readFully somewhere
    // (NGramTokenizer does this too):
    /// commons-io's readFully, but without bugs if offset != 0
    ///
    /// Equivalent to `read(Reader, char[], int, int)`.
    fn read(input: &mut dyn CharReader, buffer: &mut [u16], offset: i32, length: i32) -> Result<i32> {
        debug_assert!(length >= 0, "length must not be negative: {length}");

        let mut remaining = length;
        while remaining > 0 {
            let location = length - remaining;
            let from = (offset + location) as usize;
            let count = input.read(&mut buffer[from..from + remaining as usize])?;
            if count == 0 {
                // EOF (`-1 == count` in Java; a `CharReader` answers 0)
                break;
            }
            remaining -= count as i32;
        }
        Ok(length - remaining)
    }

    /*
     * return true if there is a token from the buffer, or null if it is
     * exhausted.
     */
    /// Equivalent to `incrementTokenBuffer()`.
    fn increment_token_buffer(&mut self, source: &mut AttributeSource, input: &dyn CharReader) -> bool {
        let mut start = self.breaker.current();
        debug_assert!(start != DONE);

        // find the next set of boundaries, skipping over non-tokens (rule status 0)
        let mut end = self.breaker.next();
        while end != DONE && self.breaker.get_rule_status() == 0 {
            start = end;
            end = self.breaker.next();
        }

        if end == DONE {
            return false; // BreakIterator exhausted
        }

        let chars = utf16_to_term_chars(&self.buffer[start as usize..end as usize]);
        source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute")
            .copy_buffer(&chars, 0, chars.len());
        let start_offset = Tokenizer::<Self>::correct_offset_of(Some(input), self.offset + start);
        let end_offset = Tokenizer::<Self>::correct_offset_of(Some(input), self.offset + end);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(start_offset, end_offset);
        let ty = self
            .config
            .get_type(self.breaker.get_script_code(), self.breaker.get_rule_status());
        {
            let mut type_att = source
                .get_attribute_interface_mut::<dyn TypeAttribute>()
                .expect("INVARIANT: the constructor added the TypeAttribute");
            match ty {
                Cow::Borrowed(constant) => type_att.set_type_static(constant),
                Cow::Owned(value) => type_att.set_type(value),
            }
        }
        source
            .get_attribute_interface_mut::<dyn ScriptAttribute>()
            .expect("INVARIANT: the constructor added the ScriptAttribute")
            .set_code(self.breaker.get_script_code());

        true
    }
}

impl TokenizerLogic for ICUTokenizerLogic {
    /// Equivalent to `ICUTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        if self.length == 0 {
            self.refill(reader)?;
        }
        while !self.increment_token_buffer(source, reader) {
            self.refill(reader)?;
            if self.length <= 0 {
                // no more bytes to read;
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Equivalent to `ICUTokenizer.reset()`, after the inherited
    /// `Tokenizer.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        let text = self.text();
        self.breaker.set_text(text, 0, 0);
        self.length = 0;
        self.usable_length = 0;
        self.offset = 0;
        Ok(())
    }

    /// Equivalent to `ICUTokenizer.end()`, after the inherited
    /// `TokenStream.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        let final_offset = if self.length < 0 {
            self.offset
        } else {
            self.offset + self.length
        };
        let corrected = Tokenizer::<Self>::correct_offset_of(input, final_offset);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(corrected, corrected);
        Ok(())
    }
}

/// Breaks text into words according to UAX #29: Unicode Text Segmentation.
///
/// Equivalent to `org.apache.lucene.analysis.icu.segmentation.ICUTokenizer`;
/// see [`ICUTokenizerLogic`].
pub type ICUTokenizer = Tokenizer<ICUTokenizerLogic>;

/// `addAttribute(ScriptAttribute.class)`.
///
/// **Divergence from Lucene 10.5.0 (inherited).** Java's
/// `DefaultAttributeFactory` resolves `ScriptAttributeImpl` reflectively
/// from the interface name; `rucene`'s default factory is an explicit
/// registry of `lucene-core`'s implementations. The source's factory is
/// asked first, and when it cannot implement the interface, the
/// implementation the reflective lookup produces is installed directly (as
/// `rucene-analysis-common`'s `ConcatenateGraphFilter` installs its own
/// attribute).
fn add_script_attribute(source: &mut AttributeSource) -> Result<()> {
    match source.add_attribute_by_id(TypeId::of::<dyn ScriptAttribute>()) {
        Ok(_) => Ok(()),
        Err(e) if e.is_illegal_argument_exception() => {
            source.add_attribute_impl_instance(Box::new(ScriptAttributeImpl::new()));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// The attribute source and logic of an ICUTokenizer over `factory`:
/// `super(factory)` and the field initializers, then the constructor body.
///
/// # Errors
///
/// What adding the attributes raises.
pub fn icu_tokenizer_parts(
    factory: Arc<dyn AttributeFactory>,
    config: Arc<dyn ICUTokenizerConfig>,
) -> Result<(AttributeSource, ICUTokenizerLogic)> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    add_script_attribute(&mut source)?;
    let breaker = CompositeBreakIterator::new(Arc::clone(&config));
    Ok((
        source,
        ICUTokenizerLogic {
            buffer: vec![0u16; IOBUFFER],
            length: 0,
            usable_length: 0,
            offset: 0,
            breaker,
            config,
        },
    ))
}

/// Construct a new ICUTokenizer that breaks text into words from the given
/// Reader.
///
/// The default script-specific handling is used.
///
/// The default attribute factory is used.
///
/// See [`DefaultICUTokenizerConfig`].
///
/// Equivalent to `ICUTokenizer()`, `this(new DefaultICUTokenizerConfig(true,
/// true))`.
///
/// # Errors
///
/// What adding the attributes raises.
pub fn new_icu_tokenizer() -> Result<ICUTokenizer> {
    new_icu_tokenizer_with_config(Arc::new(DefaultICUTokenizerConfig::new(true, true)))
}

/// Construct a new ICUTokenizer that breaks text into words from the given
/// Reader, using a tailored BreakIterator configuration.
///
/// The default attribute factory is used.
///
/// * `config`: Tailored BreakIterator configuration
///
/// Equivalent to `ICUTokenizer(ICUTokenizerConfig)`,
/// `this(DEFAULT_TOKEN_ATTRIBUTE_FACTORY, config)`.
///
/// # Errors
///
/// What adding the attributes raises.
pub fn new_icu_tokenizer_with_config(config: Arc<dyn ICUTokenizerConfig>) -> Result<ICUTokenizer> {
    new_icu_tokenizer_with_factory(default_token_attribute_factory(), config)
}

/// Construct a new ICUTokenizer that breaks text into words from the given
/// Reader, using a tailored BreakIterator configuration.
///
/// * `factory`: AttributeFactory to use
/// * `config`: Tailored BreakIterator configuration
///
/// Equivalent to `ICUTokenizer(AttributeFactory, ICUTokenizerConfig)`.
///
/// # Errors
///
/// What adding the attributes raises.
pub fn new_icu_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
    config: Arc<dyn ICUTokenizerConfig>,
) -> Result<ICUTokenizer> {
    let (source, logic) = icu_tokenizer_parts(factory, config)?;
    Ok(Tokenizer::with_source(source, logic))
}
