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

//! Tokenizer that uses a `BreakIterator` to tokenize Thai text, ported from
//! `org.apache.lucene.analysis.th.ThaiTokenizer`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::{
    default_token_attribute_factory, CharReader, CharTermAttribute, OffsetAttribute, Tokenizer,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};

use crate::analysis::util::char_array_iterator::CharArrayIterator;
use crate::analysis::util::segmenting_tokenizer_base::{
    segmenting_tokenizer_base_source, SegmentingTokenizerBase, SegmentingTokenizerBaseAdapter,
    SegmentingTokenizerBaseLogic,
};
use crate::jdk::break_iterator::{get_sentence_instance, get_word_instance, BreakIterator, DONE};
use crate::jdk::character::{code_point_at, is_digit, is_letter, utf16_to_term_chars};
use crate::jdk::locale::Locale;

/// The static state of `ThaiTokenizer`: `proto`, the Thai word instance,
/// and `DBBI_AVAILABLE`, which the static initializer computes with it.
struct Prototype {
    proto: Box<dyn BreakIterator>,
    dbbi_available: bool,
}

/// `proto` and the static initializer that checks it.
static PROTO: LazyLock<Prototype> = LazyLock::new(|| {
    let locale = Locale::builder_from_language_tag("th")
        .expect("INVARIANT: `th` is a well-formed language tag");
    let mut proto = get_word_instance(&locale);
    // check that we have a working dictionary-based break iterator for thai
    proto.set_text_str("ภาษาไทย");
    let dbbi_available = proto
        .is_boundary(4)
        .expect("INVARIANT: 4 is inside the seven-unit text");
    Prototype {
        proto,
        dbbi_available,
    }
});

/// True if the JRE supports a working dictionary-based breakiterator for
/// Thai. If this is false, this tokenizer will not work at all!
///
/// Equivalent to `ThaiTokenizer.DBBI_AVAILABLE`: the Thai word instance of
/// the `BreakIterator` emulation ([`crate::jdk::break_iterator`]), set to
/// `"ภาษาไทย"`, answers whether 4 is a boundary.
pub static DBBI_AVAILABLE: LazyLock<bool> = LazyLock::new(|| PROTO.dbbi_available);

/// used for breaking the text into sentences: `ThaiTokenizer.sentenceProto`,
/// `BreakIterator.getSentenceInstance(Locale.ROOT)`.
static SENTENCE_PROTO: LazyLock<Box<dyn BreakIterator>> =
    LazyLock::new(|| get_sentence_instance(&Locale::default()));

/// The body of [`ThaiTokenizer`]: what Lucene's `ThaiTokenizer` adds to
/// `SegmentingTokenizerBase`.
#[derive(Debug)]
pub struct ThaiTokenizerLogic {
    word_breaker: Box<dyn BreakIterator>,
    wrapper: CharArrayIterator,
    /// The package-private `sentenceStart`.
    pub(crate) sentence_start: i32,
    /// The package-private `sentenceEnd`.
    pub(crate) sentence_end: i32,
}

impl SegmentingTokenizerBaseLogic for ThaiTokenizerLogic {
    /// Equivalent to `ThaiTokenizer.setNextSentence(int, int)`.
    fn set_next_sentence(&mut self, buffer: &[u16], sentence_start: i32, sentence_end: i32) {
        self.sentence_start = sentence_start;
        self.sentence_end = sentence_end;
        self.wrapper
            .set_text(buffer, sentence_start, sentence_end - sentence_start);
        self.word_breaker.set_text(Box::new(self.wrapper.clone()));
    }

    /// Equivalent to `ThaiTokenizer.incrementWord()`.
    fn increment_word(
        &mut self,
        source: &mut AttributeSource,
        buffer: &[u16],
        offset: i32,
        input: &dyn CharReader,
    ) -> bool {
        let mut start = self.word_breaker.current();
        if start == DONE {
            return false; // BreakIterator exhausted
        }

        // find the next set of boundaries, skipping over non-tokens
        let mut end = self.word_breaker.next();
        while end != DONE && {
            let cp = code_point_at(
                buffer,
                (self.sentence_start + start) as usize,
                self.sentence_end as usize,
            );
            !(is_letter(cp) || is_digit(cp))
        } {
            start = end;
            end = self.word_breaker.next();
        }

        if end == DONE {
            return false; // BreakIterator exhausted
        }

        source.clear_attributes();
        let from = (self.sentence_start + start) as usize;
        let chars = utf16_to_term_chars(&buffer[from..from + (end - start) as usize]);
        source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute")
            .copy_buffer(&chars, 0, chars.len());
        let start_offset = Tokenizer::<SegmentingTokenizerBaseAdapter<Self>>::correct_offset_of(
            Some(input),
            offset + self.sentence_start + start,
        );
        let end_offset = Tokenizer::<SegmentingTokenizerBaseAdapter<Self>>::correct_offset_of(
            Some(input),
            offset + self.sentence_start + end,
        );
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(start_offset, end_offset);
        true
    }
}

/// Tokenizer that use `BreakIterator` to tokenize Thai text.
///
/// WARNING: this tokenizer may not be supported by all JREs. It is known to
/// work with Sun/Oracle and Harmony JREs. If your application needs to be
/// fully portable, consider using ICUTokenizer instead, which uses an ICU
/// Thai `BreakIterator` that will always be available.
///
/// Equivalent to `org.apache.lucene.analysis.th.ThaiTokenizer`, which
/// extends `SegmentingTokenizerBase`: a [`SegmentingTokenizerBase`] over
/// [`ThaiTokenizerLogic`]. Its word breaks come from the Thai word instance
/// of the `BreakIterator` emulation, whose dictionary is the installed JDK's
/// `thai_dict`, or ICU's word list without one (see
/// [`crate::jdk::break_dictionary`]).
pub type ThaiTokenizer = SegmentingTokenizerBase<ThaiTokenizerLogic>;

/// The source and body of a `ThaiTokenizer` over `factory`: the
/// `SegmentingTokenizerBase` fields, then `ThaiTokenizer`'s.
///
/// # Errors
///
/// `UnsupportedOperationException("This JRE does not have support for Thai
/// segmentation")` when [`DBBI_AVAILABLE`] is false, and what `factory`
/// raises for an attribute it cannot implement.
pub(crate) fn thai_tokenizer_parts(
    factory: Arc<dyn AttributeFactory>,
) -> Result<(
    AttributeSource,
    SegmentingTokenizerBaseAdapter<ThaiTokenizerLogic>,
)> {
    let mut source = segmenting_tokenizer_base_source(factory)?;
    let sentence_breaker = SENTENCE_PROTO.clone_box();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    if !*DBBI_AVAILABLE {
        return Err(LuceneError::unsupported_operation(
            "This JRE does not have support for Thai segmentation".to_string(),
        ));
    }
    let logic = ThaiTokenizerLogic {
        word_breaker: PROTO.proto.clone_box(),
        wrapper: CharArrayIterator::new_word_instance(),
        sentence_start: 0,
        sentence_end: 0,
    };
    Ok((
        source,
        SegmentingTokenizerBaseAdapter::new(sentence_breaker, logic),
    ))
}

/// Creates a new ThaiTokenizer
///
/// Equivalent to `ThaiTokenizer()`, over
/// `TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY`.
///
/// # Errors
///
/// As [`new_thai_tokenizer_with_factory`].
pub fn new_thai_tokenizer() -> Result<ThaiTokenizer> {
    new_thai_tokenizer_with_factory(default_token_attribute_factory())
}

/// Creates a new ThaiTokenizer, supplying the AttributeFactory
///
/// Equivalent to `ThaiTokenizer(AttributeFactory)`.
///
/// # Errors
///
/// `UnsupportedOperationException("This JRE does not have support for Thai
/// segmentation")` when [`DBBI_AVAILABLE`] is false, and what `factory`
/// raises for an attribute it cannot implement.
pub fn new_thai_tokenizer_with_factory(
    factory: Arc<dyn AttributeFactory>,
) -> Result<ThaiTokenizer> {
    let (source, adapter) = thai_tokenizer_parts(factory)?;
    Ok(Tokenizer::with_source(source, adapter))
}
