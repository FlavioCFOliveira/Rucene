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

//! The types declared in `CompoundWordTokenFilterBase.java`, of the Java
//! package `org.apache.lucene.analysis.compound`.

use std::collections::VecDeque;
use std::fmt::Debug;

use rucene::analysis::{
    CharArraySet, CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, TokenFilter,
    TokenFilterLogic, TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeSource, CapturedState};

use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// The default for minimal word length that gets decomposed
///
/// Equivalent to `CompoundWordTokenFilterBase.DEFAULT_MIN_WORD_SIZE`.
pub const DEFAULT_MIN_WORD_SIZE: i32 = 5;

/// The default for minimal length of subwords that get propagated to the
/// output of this filter
///
/// Equivalent to `CompoundWordTokenFilterBase.DEFAULT_MIN_SUBWORD_SIZE`.
pub const DEFAULT_MIN_SUBWORD_SIZE: i32 = 2;

/// The default for maximal length of subwords that get propagated to the
/// output of this filter
///
/// Equivalent to `CompoundWordTokenFilterBase.DEFAULT_MAX_SUBWORD_SIZE`.
pub const DEFAULT_MAX_SUBWORD_SIZE: i32 = 15;

/// The current token as `decompose()` reads it: the term buffer of the
/// `CharTermAttribute`, as Java's UTF-16 units, and the offsets of the
/// `OffsetAttribute`.
///
/// **Divergence.** The core's `CharTermAttribute` holds Rust `char`s, where
/// Java's holds UTF-16 units; the filters index, measure and slice the term
/// in UTF-16 units, as Java does, through this view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompoundTermView {
    /// `termAtt.buffer()` up to `termAtt.length()`, as UTF-16 units.
    pub buffer: Vec<u16>,
    /// `offsetAtt.startOffset()`.
    pub start_offset: i32,
    /// `offsetAtt.endOffset()`.
    pub end_offset: i32,
}

impl CompoundTermView {
    /// Equivalent to `termAtt.length()`, in UTF-16 units.
    pub fn length(&self) -> usize {
        self.buffer.len()
    }

    /// The current token of `source`.
    fn of(source: &AttributeSource) -> Self {
        let term = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute");
        let buffer = chars_to_utf16(&term.buffer()[..term.length()]);
        drop(term);
        let offset = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute");
        Self {
            buffer,
            start_offset: offset.start_offset(),
            end_offset: offset.end_offset(),
        }
    }
}

/// Helper class to hold decompounded token information
///
/// Equivalent to the protected inner class
/// `org.apache.lucene.analysis.compound.CompoundWordTokenFilterBase$CompoundToken`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompoundToken {
    /// Equivalent to `CompoundToken.txt`, as UTF-16 units.
    pub txt: Vec<u16>,
    /// Equivalent to `CompoundToken.startOffset`.
    pub start_offset: i32,
    /// Equivalent to `CompoundToken.endOffset`.
    pub end_offset: i32,
}

impl CompoundToken {
    /// Construct the compound token based on a slice of the current
    /// `termAtt` (`term`).
    ///
    /// Equivalent to `CompoundToken(int offset, int length)`: the text is
    /// `termAtt.subSequence(offset, offset + length)`, the offsets those of
    /// the original word.
    ///
    /// # Panics
    ///
    /// For a range outside the term, where Java's `subSequence` throws
    /// `IndexOutOfBoundsException`.
    pub fn new(term: &CompoundTermView, offset: usize, length: usize) -> Self {
        Self {
            txt: term.buffer[offset..offset + length].to_vec(),
            // offsets of the original word
            start_offset: term.start_offset,
            end_offset: term.end_offset,
        }
    }
}

/// Base class for decomposition token filters.
///
/// Equivalent to `org.apache.lucene.analysis.compound.CompoundWordTokenFilterBase`,
/// an abstract `TokenFilter`: this struct holds its fields, a
/// [`CompoundWordTokenFilterBaseSubclass`] supplies the abstract
/// `decompose()`, and [`CompoundWordTokenFilterLogic`] is the filter body
/// (`incrementToken()`, `reset()`).
#[derive(Debug, Clone)]
pub struct CompoundWordTokenFilterBase {
    /// Equivalent to `CompoundWordTokenFilterBase.dictionary` (`None` for
    /// Java's `null`).
    pub dictionary: Option<CharArraySet>,
    /// Equivalent to `CompoundWordTokenFilterBase.tokens`.
    pub tokens: VecDeque<CompoundToken>,
    /// Equivalent to `CompoundWordTokenFilterBase.minWordSize`.
    pub min_word_size: i32,
    /// Equivalent to `CompoundWordTokenFilterBase.minSubwordSize`.
    pub min_subword_size: i32,
    /// Equivalent to `CompoundWordTokenFilterBase.maxSubwordSize`.
    pub max_subword_size: i32,
    /// Equivalent to `CompoundWordTokenFilterBase.onlyLongestMatch`.
    pub only_longest_match: bool,
    current: Option<CapturedState>,
}

impl CompoundWordTokenFilterBase {
    /// Equivalent to the body of `CompoundWordTokenFilterBase(TokenStream,
    /// CharArraySet, int, int, int, boolean)` after `super(input)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`minWordSize cannot be negative`,
    /// `minSubwordSize cannot be negative`, `maxSubwordSize cannot be
    /// negative`) for a negative size.
    pub fn new(
        dictionary: Option<CharArraySet>,
        min_word_size: i32,
        min_subword_size: i32,
        max_subword_size: i32,
        only_longest_match: bool,
    ) -> Result<Self> {
        if min_word_size < 0 {
            return Err(LuceneError::illegal_argument(
                "minWordSize cannot be negative",
            ));
        }
        if min_subword_size < 0 {
            return Err(LuceneError::illegal_argument(
                "minSubwordSize cannot be negative",
            ));
        }
        if max_subword_size < 0 {
            return Err(LuceneError::illegal_argument(
                "maxSubwordSize cannot be negative",
            ));
        }
        Ok(Self {
            dictionary,
            tokens: VecDeque::new(),
            min_word_size,
            min_subword_size,
            max_subword_size,
            only_longest_match,
            current: None,
        })
    }

    /// Equivalent to `dictionary.contains(termAtt.buffer(), off, len)`.
    ///
    /// # Panics
    ///
    /// When there is no dictionary, where Java throws `NullPointerException`.
    pub fn dictionary_contains(&self, text: &[u16], off: usize, len: usize) -> bool {
        self.dictionary
            .as_ref()
            .expect("INVARIANT: Java's dictionary.contains throws NullPointerException")
            .contains_chars(Some(text), off, len)
    }
}

/// The abstract method of `CompoundWordTokenFilterBase` a concrete filter
/// implements.
pub trait CompoundWordTokenFilterBaseSubclass: Debug + Send + Sync {
    /// Decomposes the current `termAtt` (`term`) and places
    /// [`CompoundToken`] instances in the `tokens` list of `base`. The
    /// original token may not be placed in the list, as it is automatically
    /// passed through this filter.
    ///
    /// Equivalent to the protected abstract `CompoundWordTokenFilterBase.decompose()`.
    fn decompose(&mut self, base: &mut CompoundWordTokenFilterBase, term: &CompoundTermView);
}

/// The body of a decomposition token filter: what Lucene's
/// `CompoundWordTokenFilterBase` adds to `TokenFilter`, over the concrete
/// filter's `decompose()`.
#[derive(Debug, Clone)]
pub struct CompoundWordTokenFilterLogic<S: CompoundWordTokenFilterBaseSubclass> {
    base: CompoundWordTokenFilterBase,
    subclass: S,
}

impl<S: CompoundWordTokenFilterBaseSubclass> CompoundWordTokenFilterLogic<S> {
    /// The fields of `CompoundWordTokenFilterBase`.
    pub fn base(&self) -> &CompoundWordTokenFilterBase {
        &self.base
    }

    /// The concrete filter.
    pub fn subclass(&self) -> &S {
        &self.subclass
    }
}

impl<S: CompoundWordTokenFilterBaseSubclass> TokenFilterLogic for CompoundWordTokenFilterLogic<S> {
    /// Equivalent to `CompoundWordTokenFilterBase.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if let Some(token) = self.base.tokens.pop_front() {
            let current = self
                .base
                .current
                .as_ref()
                .expect("INVARIANT: the state is captured when tokens are queued");
            source.restore_state(current)?; // keep all other attributes untouched
            {
                let mut term = source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                term.set_empty();
                let chars = utf16_to_term_chars(&token.txt);
                term.copy_buffer(&chars, 0, chars.len());
            }
            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .set_offset(token.start_offset, token.end_offset);
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(0);
            return Ok(true);
        }

        self.base.current = None; // not really needed, but for safety
        if input.increment_token()? {
            // Only words longer than minWordSize get processed
            let term = CompoundTermView::of(source);
            if term.length() as i64 >= self.base.min_word_size as i64 {
                self.subclass.decompose(&mut self.base, &term);
                // only capture the state if we really need it for producing new tokens
                if !self.base.tokens.is_empty() {
                    self.base.current = source.capture_state();
                }
            }
            // return original token:
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Equivalent to `CompoundWordTokenFilterBase.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.base.tokens.clear();
        self.base.current = None;
        Ok(())
    }
}

/// Builds a decomposition filter: the `TokenFilter` over `input`, with the
/// attributes `CompoundWordTokenFilterBase` adds (`CharTermAttribute`,
/// `OffsetAttribute`, `PositionIncrementAttribute`, in its field order),
/// then the base's checks.
///
/// Equivalent to `CompoundWordTokenFilterBase(TokenStream, CharArraySet,
/// int, int, int, boolean)`, which a concrete filter calls as `super(...)`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, and what [`CompoundWordTokenFilterBase::new`] returns.
pub fn new_compound_word_token_filter<S: CompoundWordTokenFilterBaseSubclass>(
    mut input: Box<dyn TokenStream>,
    dictionary: Option<CharArraySet>,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
    only_longest_match: bool,
    subclass: S,
) -> Result<TokenFilter<CompoundWordTokenFilterLogic<S>>> {
    {
        let source = input.attribute_source_mut();
        source.add_attribute_interface::<dyn CharTermAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    }
    let base = CompoundWordTokenFilterBase::new(
        dictionary,
        min_word_size,
        min_subword_size,
        max_subword_size,
        only_longest_match,
    )?;
    Ok(TokenFilter::new(
        input,
        CompoundWordTokenFilterLogic { base, subclass },
    ))
}
