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
//! Runs tokens through a filter chain only when a condition holds, ported
//! from `org.apache.lucene.analysis.miscellaneous.ConditionalTokenFilter`.

use std::fmt::Debug;
use std::sync::Arc;

use rucene::analysis::{OffsetAttribute, PositionIncrementAttribute, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::{AttributeSource, CapturedState};

/// The abstract `shouldFilter()` of [`ConditionalTokenFilter`], which a
/// subclass supplies.
pub trait ConditionalTokenFilterLogic: Debug + Send + Sync {
    /// Whether or not to execute the wrapped TokenFilter(s) for the current
    /// token.
    ///
    /// Equivalent to the abstract `ConditionalTokenFilter.shouldFilter()`;
    /// `source` holds the filter's attributes.
    ///
    /// # Errors
    ///
    /// What the subclass's check raises (Java's `IOException`).
    fn should_filter(&mut self, source: &AttributeSource) -> Result<bool>;
}

/// The state of a [`ConditionalTokenFilter`].
///
/// Equivalent to the private enum `org.apache.lucene.analysis.miscellaneous.ConditionalTokenFilter$TokenState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenState {
    /// `READING`.
    Reading,
    /// `PREBUFFERING`.
    Prebuffering,
    /// `DELEGATING`.
    Delegating,
}

/// The fields of `ConditionalTokenFilter` that its inner class
/// `OneTimeWrapper` reads and writes, with the filter's `input` and the
/// subclass's `shouldFilter()`.
///
/// Java's inner class reaches the enclosing instance's fields directly; the
/// delegate chain owns the wrapper here, so the filter and the wrapper share
/// these fields through an `Arc`. No borrow of them is held while either
/// calls into the other.
#[derive(Debug)]
struct SharedState<L> {
    input: Box<dyn TokenStream>,
    logic: L,
    state: TokenState,
    buffered_state: Option<CapturedState>,
    exhausted: bool,
    adjust_position: bool,
    end_state: Option<CapturedState>,
    end_offset: i32,
}

type Shared<L> = Arc<AtomicRefCell<SharedState<L>>>;

/// Calls the subclass's `shouldFilter()`.
fn should_filter<L: ConditionalTokenFilterLogic>(
    shared: &Shared<L>,
    source: &AttributeSource,
) -> Result<bool> {
    shared.borrow_mut().logic.should_filter(source)
}

fn pos_inc_att(
    source: &AttributeSource,
) -> rucene::util::atomic_ref_cell::AtomicRefMut<'_, dyn PositionIncrementAttribute> {
    source
        .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
}

fn end_offset_of(source: &AttributeSource) -> i32 {
    source
        .get_attribute_interface::<dyn OffsetAttribute>()
        .expect("INVARIANT: the constructor added the OffsetAttribute")
        .end_offset()
}

/// The stream the delegate filters read: it hands them the current token
/// once, then reports exhaustion.
///
/// Equivalent to the private inner class `org.apache.lucene.analysis.miscellaneous.ConditionalTokenFilter$OneTimeWrapper`,
/// a `TokenStream` over the attribute source of the filter's input.
#[derive(Debug)]
pub struct OneTimeWrapper<L: ConditionalTokenFilterLogic + 'static> {
    source: AttributeSource,
    shared: Shared<L>,
}

impl<L: ConditionalTokenFilterLogic + 'static> TokenStream for OneTimeWrapper<L> {
    /// Equivalent to `OneTimeWrapper.incrementToken()`.
    fn increment_token(&mut self) -> Result<bool> {
        {
            let mut shared = self.shared.borrow_mut();
            if shared.state == TokenState::Prebuffering {
                let mut pos_inc_att = pos_inc_att(&self.source);
                if pos_inc_att.get_position_increment() == 0 {
                    shared.adjust_position = true;
                    pos_inc_att.set_position_increment(1);
                }
                shared.state = TokenState::Delegating;
                return Ok(true);
            }
            debug_assert!(shared.state == TokenState::Delegating);
        }
        let more = self.shared.borrow_mut().input.increment_token()?;
        if more {
            if should_filter(&self.shared, &self.source)? {
                return Ok(true);
            }
            let end_offset = end_offset_of(&self.source);
            let buffered_state = self.source.capture_state();
            let mut shared = self.shared.borrow_mut();
            shared.end_offset = end_offset;
            shared.buffered_state = buffered_state;
        } else {
            self.shared.borrow_mut().exhausted = true;
        }
        Ok(false)
    }

    /// Equivalent to `OneTimeWrapper.reset()`: clearing attributes etc is
    /// done by the parent stream, so must be avoided here.
    fn reset(&mut self) -> Result<()> {
        Ok(())
    }

    /// Equivalent to `OneTimeWrapper.end()`: imitate `Tokenizer.end()` call
    /// - endAttributes, set final offset.
    fn end(&mut self) -> Result<()> {
        let exhausted = self.shared.borrow().exhausted;
        if exhausted {
            if self.shared.borrow().end_state.is_none() {
                self.shared.borrow_mut().input.end()?;
                let end_state = self.source.capture_state();
                self.shared.borrow_mut().end_state = end_state;
            }
            let end_offset = end_offset_of(&self.source);
            self.shared.borrow_mut().end_offset = end_offset;
        }
        self.source.end_attributes();
        let end_offset = self.shared.borrow().end_offset;
        self.source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(end_offset, end_offset);
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// Allows skipping TokenFilters based on the current set of attributes.
///
/// To use, implement the `shouldFilter()` method
/// ([`ConditionalTokenFilterLogic::should_filter`]). If it returns `true`,
/// then calling `incrementToken()` will use the wrapped TokenFilter(s) to
/// make changes to the tokenstream. If it returns `false`, then the wrapped
/// filter(s) will be skipped.
///
/// Equivalent to the abstract `org.apache.lucene.analysis.miscellaneous.ConditionalTokenFilter`,
/// a
/// `TokenFilter`: the subclass is `L`.
///
/// **Divergence from Lucene 10.5.0.** The filter's `input` is shared with
/// its `OneTimeWrapper` (see `SharedState`), so the filter cannot lend it
/// out: it does not answer `TokenStream::as_unwrappable` (Java's
/// `TokenFilter.unwrap()`).
#[derive(Debug)]
pub struct ConditionalTokenFilter<L: ConditionalTokenFilterLogic + 'static> {
    source: AttributeSource,
    shared: Shared<L>,
    delegate: Box<dyn TokenStream>,
    last_token_filtered: bool,
}

/// Create a new [`ConditionalTokenFilter`]: `input` is the input token
/// stream, `input_factory` a factory function to create the wrapped filter(s),
/// and `logic` the subclass's `shouldFilter()`.
///
/// Equivalent to the protected `ConditionalTokenFilter(TokenStream,
/// Function<TokenStream, TokenStream>)`: `super(input)`, the field
/// initializer that adds the `PositionIncrementAttribute`, then
/// `this.delegate = inputFactory.apply(new OneTimeWrapper(this.input))`.
///
/// # Errors
///
/// What the attribute factory raises, and what `input_factory` raises.
pub fn new_conditional_token_filter<L, F>(
    input: Box<dyn TokenStream>,
    input_factory: F,
    logic: L,
) -> Result<ConditionalTokenFilter<L>>
where
    L: ConditionalTokenFilterLogic + 'static,
    F: FnOnce(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>,
{
    let mut source = AttributeSource::new_from(input.attribute_source());
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    let mut wrapper_source = AttributeSource::new_from(input.attribute_source());
    wrapper_source.add_attribute_interface::<dyn OffsetAttribute>()?;
    wrapper_source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    let shared = Arc::new(AtomicRefCell::new(SharedState {
        input,
        logic,
        state: TokenState::Reading,
        buffered_state: None,
        exhausted: false,
        adjust_position: false,
        end_state: None,
        end_offset: 0,
    }));
    let wrapper = OneTimeWrapper {
        source: wrapper_source,
        shared: Arc::clone(&shared),
    };
    let delegate = input_factory(Box::new(wrapper))?;
    Ok(ConditionalTokenFilter {
        source,
        shared,
        delegate,
        last_token_filtered: false,
    })
}

impl<L: ConditionalTokenFilterLogic + 'static> ConditionalTokenFilter<L> {
    /// Calls `f` with the subclass's logic.
    pub fn with_logic<R>(&self, f: impl FnOnce(&L) -> R) -> R {
        f(&self.shared.borrow().logic)
    }

    /// Equivalent to the private `ConditionalTokenFilter.endDelegating()`.
    fn end_delegating(&mut self) -> Result<bool> {
        if self.shared.borrow().buffered_state.is_none() {
            debug_assert!(self.shared.borrow().exhausted);
            return Ok(false);
        }
        self.delegate.end()?;
        let pos_inc = pos_inc_att(&self.source).get_position_increment();
        let buffered_state = self
            .shared
            .borrow_mut()
            .buffered_state
            .take()
            .expect("INVARIANT: checked above");
        self.source.restore_state(&buffered_state)?;
        let mut pos_inc_att = pos_inc_att(&self.source);
        let buffered = pos_inc_att.get_position_increment();
        pos_inc_att.set_position_increment(buffered + pos_inc);
        let mut shared = self.shared.borrow_mut();
        if shared.adjust_position {
            let current = pos_inc_att.get_position_increment();
            pos_inc_att.set_position_increment(current - 1);
            shared.adjust_position = false;
        }
        Ok(true)
    }
}

impl<L: ConditionalTokenFilterLogic + 'static> TokenStream for ConditionalTokenFilter<L> {
    /// Equivalent to the final `ConditionalTokenFilter.incrementToken()`.
    fn increment_token(&mut self) -> Result<bool> {
        self.last_token_filtered = false;
        loop {
            let state = self.shared.borrow().state;
            if state == TokenState::Reading {
                let buffered_state = self.shared.borrow_mut().buffered_state.take();
                if let Some(buffered_state) = buffered_state {
                    self.source.restore_state(&buffered_state)?;
                    self.last_token_filtered = false;
                    return Ok(true);
                }
                if self.shared.borrow().exhausted {
                    return Ok(false);
                }
                let more = self.shared.borrow_mut().input.increment_token()?;
                if !more {
                    self.shared.borrow_mut().exhausted = true;
                    return Ok(false);
                }
                if should_filter(&self.shared, &self.source)? {
                    self.last_token_filtered = true;
                    self.shared.borrow_mut().state = TokenState::Prebuffering;
                    // we determine that the delegate has emitted all the tokens it can at the
                    // current position when OneTimeWrapper.incrementToken() is called in
                    // DELEGATING state.  To signal this back to the delegate, we return false,
                    // so we now need to reset it to ensure that it can continue to emit more
                    // tokens
                    self.delegate.reset()?;
                    let more = self.delegate.increment_token()?;
                    if more {
                        let mut shared = self.shared.borrow_mut();
                        shared.state = TokenState::Delegating;
                        if shared.adjust_position {
                            let mut pos_inc_att = pos_inc_att(&self.source);
                            let pos_inc = pos_inc_att.get_position_increment();
                            pos_inc_att.set_position_increment(pos_inc - 1);
                        }
                        shared.adjust_position = false;
                    } else {
                        self.shared.borrow_mut().state = TokenState::Reading;
                        return self.end_delegating();
                    }
                    return Ok(true);
                }
                return Ok(true);
            }
            if state == TokenState::Delegating {
                self.last_token_filtered = true;
                if self.delegate.increment_token()? {
                    return Ok(true);
                }
                // no more cached tokens
                self.shared.borrow_mut().state = TokenState::Reading;
                return self.end_delegating();
            }
        }
    }

    /// Equivalent to `ConditionalTokenFilter.reset()`.
    fn reset(&mut self) -> Result<()> {
        self.shared.borrow_mut().input.reset()?;
        self.delegate.reset()?;
        let mut shared = self.shared.borrow_mut();
        shared.state = TokenState::Reading;
        self.last_token_filtered = false;
        shared.buffered_state = None;
        shared.exhausted = false;
        shared.adjust_position = false;
        shared.end_offset = -1;
        shared.end_state = None;
        Ok(())
    }

    /// Equivalent to `ConditionalTokenFilter.end()`.
    fn end(&mut self) -> Result<()> {
        let end_state = self.shared.borrow_mut().end_state.take();
        match end_state {
            None => {
                self.shared.borrow_mut().input.end()?;
                self.shared.borrow_mut().end_state = self.source.capture_state();
            }
            Some(end_state) => {
                self.source.restore_state(&end_state)?;
                self.shared.borrow_mut().end_state = Some(end_state);
            }
        }
        let end_offset = self
            .source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .ok_or_else(|| {
                LuceneError::illegal_argument(
                    "This AttributeSource does not have the attribute 'org.apache.lucene.analysis.tokenattributes.OffsetAttribute'.",
                )
            })?
            .end_offset();
        self.shared.borrow_mut().end_offset = end_offset;
        if self.last_token_filtered {
            self.delegate.end()?;
            self.shared.borrow_mut().end_state = self.source.capture_state();
        }
        Ok(())
    }

    /// Equivalent to `ConditionalTokenFilter.close()`.
    fn close(&mut self) -> Result<()> {
        self.shared.borrow_mut().input.close()?;
        self.delegate.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
