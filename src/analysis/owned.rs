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

//! Token streams driven by one consumer with their attributes borrowed in
//! place (rmp #679/#682, user decision 2026-11-26: "extend generic").
//!
//! **No counterpart in Lucene 10.5.0.** Lucene's stages share one
//! `AttributeSource` by reference and step through it at no cost; the port's
//! shared stream keeps each attribute implementation in an
//! `Arc<AtomicRefCell>` and borrows it at every token and stage, and wraps
//! its stages in shared cells. An analyzer whose components this module can
//! reproduce describes them as owned stages ([`OwnedStages`],
//! [`Analyzer::owned_stages`](crate::analysis::Analyzer::owned_stages)): the
//! same stepping logic as its tokenizer and filters, applied to the
//! attributes by `&mut`. An [`OwnedTokenChain`] drives those stages for one
//! consumer at a time (the indexing chain of one `DocumentsWriterPerThread`):
//!
//! * its attribute source is the source of components the analyzer builds
//!   for it (`createComponents`), so the implementations, the interfaces and
//!   their order are exactly those of the shared stream, and a consumer that
//!   reads the source (`FieldInvertState.getAttributeSource()`) sees what it
//!   sees there;
//! * for one text value it borrows the source's implementation cells once
//!   — the default factory's `PackedTokenAttributeImpl`, and the
//!   `KeywordAttributeImpl` when a stage added one — runs the stages, and
//!   releases them before anything else reads the source.
//!
//! A source holding any other implementation, an analyzer that does not
//! describe its stages, or one that is closed, stays on the shared stream.
//! Every stage reproduces the steps of the Lucene class it names.

use std::fmt::Debug;

use crate::analysis::tokenattributes::KeywordAttributeImpl;
use crate::analysis::tokenattributes::{CharTermAttribute, PackedTokenAttributeImpl};
use crate::analysis::{Analyzer, AttributeHandle, TokenStreamComponents};
use crate::error::Result;
use crate::index::{IndexableField, IndexableFieldType};
use crate::util::atomic_ref_cell::AtomicRefCell;
use crate::util::attribute::{AttributeImpl, AttributeSource};
use std::sync::Arc;

/// The attributes of an owned chain, borrowed for one text value.
///
/// `term` is the default factory's implementation of every core token
/// attribute; `keyword` is the `KeywordAttributeImpl` when a stage added the
/// `KeywordAttribute`.
#[derive(Debug)]
pub struct TokenAttrs<'a> {
    /// The packed token attributes.
    pub term: &'a mut PackedTokenAttributeImpl,
    /// The keyword attribute, when the source holds one.
    pub keyword: Option<&'a mut KeywordAttributeImpl>,
}

impl TokenAttrs<'_> {
    /// `AttributeSource.clearAttributes()`: every implementation of the
    /// source, cleared.
    #[inline]
    pub fn clear(&mut self) {
        AttributeImpl::clear(self.term);
        if let Some(keyword) = self.keyword.as_deref_mut() {
            AttributeImpl::clear(keyword);
        }
    }

    /// `AttributeSource.endAttributes()`.
    #[inline]
    pub fn end(&mut self) {
        AttributeImpl::end(self.term);
        if let Some(keyword) = self.keyword.as_deref_mut() {
            AttributeImpl::end(keyword);
        }
    }

    /// `keywordAttr.isKeyword()`; `false` for a source without the
    /// attribute, where no stage reads it.
    #[inline]
    pub fn is_keyword(&self) -> bool {
        use crate::analysis::tokenattributes::KeywordAttribute;
        self.keyword.as_deref().is_some_and(|k| k.is_keyword())
    }
}

/// A tokenizer's stepping logic over owned attributes, with its own reader.
///
/// Equivalent to the tokenizer of an analyzer's components: `setReader`
/// takes the text (as `Analyzer.tokenStream(String, String)` hands it a
/// `ReusableStringReader`), and the four lifecycle calls are those of
/// `Tokenizer`, `clearAttributes()`/`endAttributes()` included.
pub trait OwnedTokenizer: Debug + Send {
    /// The components' `setReader` (the source lambda's settings, then
    /// `Tokenizer.setReader`) over `text`.
    ///
    /// # Errors
    ///
    /// What the source lambda raises, and Java's `IllegalStateException`
    /// when the previous input was not closed.
    fn set_text(&mut self, text: &str) -> Result<()>;

    /// `Tokenizer.reset()` and the subclass's reset.
    ///
    /// # Errors
    ///
    /// What the subclass's reset raises.
    fn reset(&mut self) -> Result<()>;

    /// `incrementToken()`, `clearAttributes()` first.
    ///
    /// # Errors
    ///
    /// What reading the input raises.
    fn increment_token(&mut self, attrs: &mut TokenAttrs<'_>) -> Result<bool>;

    /// `end()`: `endAttributes()`, then the subclass's end-of-stream state.
    ///
    /// # Errors
    ///
    /// What the subclass's end raises.
    fn end(&mut self, attrs: &mut TokenAttrs<'_>) -> Result<()>;

    /// `Tokenizer.close()` and the subclass's close.
    ///
    /// # Errors
    ///
    /// What closing the input raises.
    fn close(&mut self) -> Result<()>;
}

/// The input of an [`OwnedTokenFilter`]: the stages below it.
pub trait OwnedInput {
    /// `input.incrementToken()`.
    ///
    /// # Errors
    ///
    /// What the stages below raise.
    fn increment_token(&mut self, attrs: &mut TokenAttrs<'_>) -> Result<bool>;

    /// `input.end()`.
    ///
    /// # Errors
    ///
    /// What the stages below raise.
    fn end(&mut self, attrs: &mut TokenAttrs<'_>) -> Result<()>;
}

/// A token filter's stepping logic over owned attributes.
///
/// Equivalent to a `TokenFilter` subclass of the components:
/// `incrementToken()` and `end()` call the input as the Java class calls
/// `input`; `reset()` and `close()` are the subclass's own work, run after
/// the stages below, as `TokenFilter.reset()` calls `input.reset()` first.
pub trait OwnedTokenFilter: Debug + Send {
    /// `incrementToken()`.
    ///
    /// # Errors
    ///
    /// What the filter or its input raises.
    fn increment_token(
        &mut self,
        attrs: &mut TokenAttrs<'_>,
        input: &mut dyn OwnedInput,
    ) -> Result<bool>;

    /// `end()`; the default is `TokenFilter.end()`, `input.end()`.
    ///
    /// # Errors
    ///
    /// What the filter or its input raises.
    fn end(&mut self, attrs: &mut TokenAttrs<'_>, input: &mut dyn OwnedInput) -> Result<()> {
        input.end(attrs)
    }

    /// The subclass's part of `reset()` (after `input.reset()`).
    ///
    /// # Errors
    ///
    /// What the filter's reset raises.
    fn reset(&mut self) -> Result<()> {
        Ok(())
    }

    /// The subclass's part of `close()` (after `input.close()`).
    ///
    /// # Errors
    ///
    /// What the filter's close raises.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// An analyzer's components as owned stages: the tokenizer and the filters,
/// innermost first.
#[derive(Debug)]
pub struct OwnedStages {
    /// The tokenizer.
    pub tokenizer: Box<dyn OwnedTokenizer>,
    /// The filters, in the order the components wrap them.
    pub filters: Vec<Box<dyn OwnedTokenFilter>>,
}

impl OwnedStages {
    /// The stages of a tokenizer with no filter.
    pub fn new(tokenizer: Box<dyn OwnedTokenizer>) -> Self {
        Self {
            tokenizer,
            filters: Vec::new(),
        }
    }

    /// These stages with `filter` wrapping them.
    pub fn with_filter(mut self, filter: Box<dyn OwnedTokenFilter>) -> Self {
        self.filters.push(filter);
        self
    }
}

/// The stages below a filter.
struct Below<'s> {
    filters: &'s mut [Box<dyn OwnedTokenFilter>],
    tokenizer: &'s mut dyn OwnedTokenizer,
}

impl OwnedInput for Below<'_> {
    #[inline]
    fn increment_token(&mut self, attrs: &mut TokenAttrs<'_>) -> Result<bool> {
        match self.filters.split_last_mut() {
            Some((last, rest)) => last.increment_token(
                attrs,
                &mut Below {
                    filters: rest,
                    tokenizer: &mut *self.tokenizer,
                },
            ),
            None => self.tokenizer.increment_token(attrs),
        }
    }

    fn end(&mut self, attrs: &mut TokenAttrs<'_>) -> Result<()> {
        match self.filters.split_last_mut() {
            Some((last, rest)) => last.end(
                attrs,
                &mut Below {
                    filters: rest,
                    tokenizer: &mut *self.tokenizer,
                },
            ),
            None => self.tokenizer.end(attrs),
        }
    }
}

/// The attribute source an [`OwnedTokenChain`] works in: the source of the
/// analyzer's reusable components (the instance its shared stream uses),
/// with the handles of its implementation cells, or `None` when it holds an
/// implementation the owned stages do not drive.
struct View {
    components: Arc<AtomicRefCell<TokenStreamComponents>>,
    cells: Option<ViewCells>,
}

struct ViewCells {
    source: AttributeSource,
    term: AttributeHandle<dyn CharTermAttribute>,
    keyword: Option<AttributeHandle<dyn crate::analysis::tokenattributes::KeywordAttribute>>,
}

impl ViewCells {
    /// The cells of `components`' source, when it holds only the default
    /// factory's `PackedTokenAttributeImpl` and at most one
    /// `KeywordAttributeImpl`.
    fn of(components: &Arc<AtomicRefCell<TokenStreamComponents>>) -> Option<Self> {
        let stream = components.borrow().token_stream();
        let source = AttributeSource::new_from(stream.borrow().attribute_source());
        let mut packed = 0;
        let mut keywords = 0;
        for cell in source.attribute_impls_iter() {
            let att = cell.borrow();
            let any = att.as_any();
            if any.is::<PackedTokenAttributeImpl>() {
                packed += 1;
            } else if any.is::<KeywordAttributeImpl>() {
                keywords += 1;
            } else {
                return None;
            }
        }
        if packed != 1 || keywords > 1 {
            return None;
        }
        let term = AttributeHandle::get(&source)?;
        let keyword = AttributeHandle::get(&source);
        if keywords == 1 && keyword.is_none() {
            return None;
        }
        Some(Self {
            source,
            term,
            keyword,
        })
    }
}

/// An analyzer's token stream driven by one consumer: its owned stages,
/// working in the attribute source of the analyzer's own reusable
/// components.
///
/// The chain uses the components instance the analyzer's reuse strategy
/// holds for the field on the current thread — the one
/// `Analyzer.tokenStream` would use, created and stored there when missing,
/// as `tokenStream` does — so a field inverted through the shared stream and
/// one inverted through the owned chain write the same attribute source, as
/// Lucene's single components instance does. That instance, once stored for
/// a thread (and a field, under a per-field strategy), is never replaced
/// while the analyzer is open (`Analyzer.close()` empties the store, and a
/// closed analyzer takes no owned chain), so the chain looks it up again
/// only when it runs on another thread ([`bind`](Self::bind)).
pub struct OwnedTokenChain {
    stages: OwnedStages,
    view: Option<View>,
    /// The thread the view was bound on ([`current_thread_key`]).
    bound_thread: u64,
    /// Between `reset()` and `close()`: a throwable that escaped the
    /// consumer leaves it set, and the next value closes the chain first,
    /// as Java's try-with-resources closed the stream.
    open: bool,
}

impl Debug for OwnedTokenChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedTokenChain")
            .field("stages", &self.stages)
            .finish_non_exhaustive()
    }
}

impl OwnedTokenChain {
    /// The owned chain of `analyzer` for `field_name`, or `None` when the
    /// analyzer does not describe its stages.
    pub fn build(analyzer: &dyn Analyzer, field_name: Option<&str>) -> Option<Self> {
        let stages = analyzer.owned_stages(field_name)?.ok()?;
        Some(Self {
            stages,
            view: None,
            bound_thread: 0,
            open: false,
        })
    }

    /// Binds the chain to the reusable components
    /// `Analyzer.tokenStream(field_name, ...)` uses on this thread (`Analyzer.java:153-198`: `reuseStrategy
    /// .getReusableComponents`, else `createComponents` and
    /// `setReusableComponents`). Answers whether the chain can run in their
    /// attribute source; `false` keeps the value on the shared stream.
    ///
    /// # Errors
    ///
    /// What `createComponents` raises, as `tokenStream` raises it.
    pub fn bind(&mut self, analyzer: &dyn Analyzer, field_name: Option<&str>) -> Result<bool> {
        let thread = current_thread_key();
        if self.bound_thread != thread {
            let state = analyzer.analyzer_state();
            let strategy = state.reuse_strategy();
            let components = match strategy.get_reusable_components(state, field_name) {
                Some(components) => components,
                None => {
                    let components =
                        Arc::new(AtomicRefCell::new(analyzer.create_components(field_name)?));
                    strategy.set_reusable_components(state, field_name, Arc::clone(&components));
                    components
                }
            };
            if !matches!(&self.view, Some(view) if Arc::ptr_eq(&view.components, &components)) {
                let cells = ViewCells::of(&components);
                self.view = Some(View { components, cells });
            }
            self.bound_thread = thread;
        }
        Ok(self.view.as_ref().is_some_and(|view| view.cells.is_some()))
    }

    fn cells(&self) -> &ViewCells {
        self.view
            .as_ref()
            .and_then(|view| view.cells.as_ref())
            .expect("INVARIANT: bind answered true before the chain is used")
    }

    /// The chain's attribute source: the shared stream's own (after a
    /// [`bind`](Self::bind) that answered `true`).
    pub fn attribute_source(&self) -> &AttributeSource {
        &self.cells().source
    }

    /// `Analyzer.tokenStream(fieldName, text)` on reused components: the
    /// components' `setReader` over `text`. A chain a throwable left open is
    /// closed first.
    ///
    /// # Errors
    ///
    /// What the tokenizer's `setReader` raises.
    pub fn set_text(&mut self, text: &str) -> Result<()> {
        if self.open {
            self.open = false;
            let _ = self.close_stages();
        }
        self.stages.tokenizer.set_text(text)
    }

    /// `stream.reset()`: the tokenizer's, then each filter's, innermost
    /// first.
    ///
    /// # Errors
    ///
    /// What a stage's reset raises.
    pub fn reset(&mut self) -> Result<()> {
        self.open = true;
        self.stages.tokenizer.reset()?;
        for filter in &mut self.stages.filters {
            filter.reset()?;
        }
        Ok(())
    }

    /// The token loop: `while (stream.incrementToken())` with `on_token`
    /// reading each token, then `stream.end()`; returns the end-of-stream
    /// position increment and end offset. The attributes are borrowed for
    /// the loop and released on return.
    ///
    /// # Errors
    ///
    /// What a stage or `on_token` raises.
    pub fn for_each_token(
        &mut self,
        mut on_token: impl FnMut(&PackedTokenAttributeImpl) -> Result<()>,
    ) -> Result<(i32, i32)> {
        use crate::analysis::tokenattributes::{OffsetAttribute, PositionIncrementAttribute};
        let cells = self
            .view
            .as_ref()
            .and_then(|view| view.cells.as_ref())
            .expect("INVARIANT: bind answered true before the chain is used");
        let mut term_impl = cells.term.borrow_impl_mut();
        let mut keyword_impl = cells.keyword.as_ref().map(AttributeHandle::borrow_impl_mut);
        let term = term_impl
            .as_any_mut()
            .downcast_mut::<PackedTokenAttributeImpl>()
            .expect("INVARIANT: bind checked the implementation");
        let keyword = keyword_impl.as_mut().map(|k| {
            k.as_any_mut()
                .downcast_mut::<KeywordAttributeImpl>()
                .expect("INVARIANT: bind checked the implementation")
        });
        let mut attrs = TokenAttrs { term, keyword };
        let OwnedStages { tokenizer, filters } = &mut self.stages;
        let mut top = Below {
            filters: filters.as_mut_slice(),
            tokenizer: tokenizer.as_mut(),
        };
        while top.increment_token(&mut attrs)? {
            on_token(attrs.term)?;
        }
        top.end(&mut attrs)?;
        Ok((attrs.term.get_position_increment(), attrs.term.end_offset()))
    }

    /// `stream.close()`.
    ///
    /// # Errors
    ///
    /// What closing a stage raises; the first error wins.
    pub fn close(&mut self) -> Result<()> {
        self.open = false;
        self.close_stages()
    }

    fn close_stages(&mut self) -> Result<()> {
        let mut outcome = self.stages.tokenizer.close();
        for filter in &mut self.stages.filters {
            let closed = filter.close();
            if outcome.is_ok() {
                outcome = closed;
            }
        }
        outcome
    }
}

/// A key of the current thread, unique among the process's threads and
/// never 0: the analyzer's reusable components are stored per thread.
fn current_thread_key() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    thread_local! {
        static KEY: u64 = NEXT.fetch_add(1, Ordering::Relaxed);
    }
    KEY.with(|key| *key)
}

/// The text `field.tokenStream(analyzer, reuse)` hands to
/// `analyzer.tokenStream(name, text)`, or `None`:
///
/// * a tokenized [`Field`](crate::document::Field) with a string (or number)
///   value, whose `tokenStream` does so when it holds neither a token-stream
///   nor a reader value (`Field.java:465-498`);
/// * a [`TextField`](crate::document::TextField) with a string value, whose
///   `tokenStream` does the same.
pub fn analyzed_text(field: &dyn IndexableField) -> Option<crate::index::StringValueRef<'_>> {
    let any = field.as_any()?;
    // PERF: the string is lent by the field (`string_value_ref`), not copied.
    if let Some(field) = any.downcast_ref::<crate::document::Field>() {
        if !field.field_type().tokenized() {
            return None;
        }
        // A string (or number) value excludes a token-stream and a reader
        // value: they are variants of the one field value.
        return IndexableField::string_value_ref(field);
    }
    // A `TextField`'s string value excludes its token-stream and reader
    // values in the same way.
    IndexableField::string_value_ref(any.downcast_ref::<crate::document::TextField>()?)
}

/// One token as the tests compare it: term, position increment, position
/// length, start and end offsets, type, term frequency and keyword flag.
pub type TokenRecord = (String, i32, i32, i32, i32, String, i32, bool);

/// The tokens and the end state (position increment, start and end offset)
/// of `text` through the analyzer's shared stream and through its owned
/// chain, for the owned-chain identity tests of every analyzer that
/// describes its stages; `None` when the analyzer has no owned chain.
///
/// # Errors
///
/// What either stream raises.
#[allow(clippy::type_complexity)]
pub fn owned_and_shared_tokens(
    analyzer: &dyn Analyzer,
    field_name: &str,
    text: &str,
) -> Result<
    Option<(
        (Vec<TokenRecord>, (i32, i32, i32)),
        (Vec<TokenRecord>, (i32, i32, i32)),
    )>,
> {
    use crate::analysis::tokenattributes::{
        KeywordAttribute, OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
        TermFrequencyAttribute, TypeAttribute,
    };
    fn record(source: &AttributeSource) -> TokenRecord {
        let term = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("a token stream has a term attribute");
        let text: String = term.buffer()[..term.length()].iter().collect();
        drop(term);
        let pi = source
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .map_or(1, |a| a.get_position_increment());
        let pl = source
            .get_attribute_interface::<dyn PositionLengthAttribute>()
            .map_or(1, |a| a.get_position_length());
        let (so, eo) = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .map_or((0, 0), |a| (a.start_offset(), a.end_offset()));
        let ty = source
            .get_attribute_interface::<dyn TypeAttribute>()
            .map_or(String::new(), |a| a.type_value().to_string());
        let tf = source
            .get_attribute_interface::<dyn TermFrequencyAttribute>()
            .map_or(1, |a| a.get_term_frequency());
        let kw = source
            .get_attribute_interface::<dyn KeywordAttribute>()
            .is_some_and(|a| a.is_keyword());
        (text, pi, pl, so, eo, ty, tf, kw)
    }
    fn end_state(source: &AttributeSource) -> (i32, i32, i32) {
        let pi = source
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .map_or(0, |a| a.get_position_increment());
        let (so, eo) = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .map_or((0, 0), |a| (a.start_offset(), a.end_offset()));
        (pi, so, eo)
    }
    let Some(mut chain) = OwnedTokenChain::build(analyzer, Some(field_name)) else {
        return Ok(None);
    };
    let shared = {
        let stream = analyzer.token_stream_from_str(field_name, text)?;
        let mut stream = stream.borrow_mut();
        stream.reset()?;
        let mut tokens = Vec::new();
        while stream.increment_token()? {
            tokens.push(record(stream.attribute_source()));
        }
        stream.end()?;
        let end = end_state(stream.attribute_source());
        stream.close()?;
        (tokens, end)
    };
    if !chain.bind(analyzer, Some(field_name))? {
        return Ok(None);
    }
    chain.set_text(text)?;
    chain.reset()?;
    let mut tokens = Vec::new();
    chain.for_each_token(|att| {
        use crate::analysis::tokenattributes::{
            OffsetAttribute as _, PositionIncrementAttribute as _, PositionLengthAttribute as _,
            TermFrequencyAttribute as _, TypeAttribute as _,
        };
        tokens.push((
            att.buffer()[..att.length()].iter().collect::<String>(),
            att.get_position_increment(),
            att.get_position_length(),
            att.start_offset(),
            att.end_offset(),
            att.type_value().to_string(),
            att.get_term_frequency(),
            false,
        ));
        Ok(())
    })?;
    // The keyword flag is not read inside the loop (the keyword cell is
    // borrowed there): no owned stage sets it, and an analyzer with a
    // keyword-marker filter describes no owned stages.
    let end = end_state(chain.attribute_source());
    chain.close()?;
    Ok(Some((shared, (tokens, end))))
}
