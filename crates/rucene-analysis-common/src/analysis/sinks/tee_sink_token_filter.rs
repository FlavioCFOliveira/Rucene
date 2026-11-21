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

//! Sets aside the attribute states of a stream for other streams, ported
//! from `org.apache.lucene.analysis.sinks.TeeSinkTokenFilter`.

use std::sync::Arc;

use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::{AttributeSource, CapturedState, Unwrappable};

/// The body of [`TeeSinkTokenFilter`]'s `TokenFilter`: what Lucene's
/// `TeeSinkTokenFilter` adds to `incrementToken`, `end` and `reset`.
#[derive(Debug)]
pub struct TeeSinkTokenFilterLogic {
    cached_states: Arc<AtomicRefCell<States>>,
}

impl TokenFilterLogic for TeeSinkTokenFilterLogic {
    /// Equivalent to `TeeSinkTokenFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            self.cached_states.borrow_mut().add(source.capture_state());
            return Ok(true);
        }

        Ok(false)
    }

    /// Equivalent to the final `TeeSinkTokenFilter.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        self.cached_states
            .borrow_mut()
            .set_final_state(source.capture_state());
        Ok(())
    }

    /// Equivalent to `TeeSinkTokenFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        self.cached_states.borrow_mut().reset();
        input.reset()
    }
}

/// This TokenFilter provides the ability to set aside attribute states that
/// have already been analyzed. This is useful in situations where multiple
/// fields share many common analysis steps and then go their separate ways.
///
/// It is also useful for doing things like entity extraction or proper noun
/// analysis as part of the analysis workflow and saving off those tokens for
/// use in another field.
///
/// ```text
/// TeeSinkTokenFilter source1 = new TeeSinkTokenFilter(new WhitespaceTokenizer());
/// TeeSinkTokenFilter.SinkTokenStream sink1 = source1.newSinkTokenStream();
/// TeeSinkTokenFilter.SinkTokenStream sink2 = source1.newSinkTokenStream();
///
/// TokenStream final1 = new LowerCaseFilter(source1);
/// TokenStream final2 = new EntityDetect(sink1);
/// TokenStream final3 = new URLDetect(sink2);
///
/// d.add(new TextField("f1", final1));
/// d.add(new TextField("f2", final2));
/// d.add(new TextField("f3", final3));
/// ```
///
/// In this example, `sink1` and `sink2` will both get tokens from `source1`
/// after whitespace tokenization, and will further do additional token
/// filtering, e.g. detect entities and URLs.
///
/// **NOTE**: it is important, that tees are consumed before sinks, therefore
/// you should add them to the document before the sinks. In the above
/// example, *f1* is added before the other fields, and so by the time they
/// are processed, it has already been consumed, which is the correct way to
/// index the three streams. If for some reason you cannot ensure that, you
/// should call [`consume_all_tokens`](Self::consume_all_tokens) before adding
/// the sinks to document fields.
///
/// Equivalent to `org.apache.lucene.analysis.sinks.TeeSinkTokenFilter`, a
/// `TokenFilter`: the filter it is built on, with the methods Java adds.
/// The cached states are shared with the sinks, as Java's `States` object
/// is.
#[derive(Debug)]
pub struct TeeSinkTokenFilter {
    filter: TokenFilter<TeeSinkTokenFilterLogic>,
    cached_states: Arc<AtomicRefCell<States>>,
}

impl TeeSinkTokenFilter {
    /// Equivalent to `TeeSinkTokenFilter(TokenStream)`.
    pub fn new(input: Box<dyn TokenStream>) -> Self {
        let cached_states = Arc::new(AtomicRefCell::new(States::new()));
        Self {
            filter: TokenFilter::new(
                input,
                TeeSinkTokenFilterLogic {
                    cached_states: Arc::clone(&cached_states),
                },
            ),
            cached_states,
        }
    }

    /// Returns a new [`SinkTokenStream`] that receives all tokens consumed by
    /// this stream.
    ///
    /// Equivalent to `TeeSinkTokenFilter.newSinkTokenStream()`, `new
    /// SinkTokenStream(this.cloneAttributes(), cachedStates)`.
    pub fn new_sink_token_stream(&self) -> SinkTokenStream {
        SinkTokenStream::new(
            self.filter.attribute_source().clone_attributes(),
            Arc::clone(&self.cached_states),
        )
    }

    /// `TeeSinkTokenFilter` passes all tokens to the added sinks when itself
    /// is consumed. To be sure that all tokens from the input stream are
    /// passed to the sinks, you can call this methods. This instance is
    /// exhausted after this method returns, but all sinks are instant
    /// available.
    ///
    /// Equivalent to `TeeSinkTokenFilter.consumeAllTokens()`.
    ///
    /// # Errors
    ///
    /// What [`increment_token`](TokenStream::increment_token) returns.
    pub fn consume_all_tokens(&mut self) -> Result<()> {
        while self.increment_token()? {}
        Ok(())
    }
}

impl TokenStream for TeeSinkTokenFilter {
    fn increment_token(&mut self) -> Result<bool> {
        self.filter.increment_token()
    }

    fn end(&mut self) -> Result<()> {
        self.filter.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.filter.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.filter.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.filter.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.filter.attribute_source_mut()
    }

    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        self.filter.as_unwrappable()
    }
}

/// The iteration over the cached states a sink's `reset()` starts: Java's
/// `ArrayList` iterator, its cursor and the modification count it expects.
#[derive(Debug, Clone, Copy)]
struct StatesIterator {
    cursor: usize,
    expected_mod_count: u64,
}

/// TokenStream output from a tee.
///
/// Equivalent to `org.apache.lucene.analysis.sinks.TeeSinkTokenFilter$SinkTokenStream`,
/// whose `TokenStream(AttributeSource)` shares the attributes of the tee's
/// clone, the source this stream owns.
#[derive(Debug)]
pub struct SinkTokenStream {
    source: AttributeSource,
    cached_states: Arc<AtomicRefCell<States>>,
    it: Option<StatesIterator>,
}

impl SinkTokenStream {
    /// Equivalent to the private `SinkTokenStream(AttributeSource, States)`.
    fn new(source: AttributeSource, cached_states: Arc<AtomicRefCell<States>>) -> Self {
        Self {
            source,
            cached_states,
            it: None,
        }
    }
}

impl TokenStream for SinkTokenStream {
    /// Equivalent to the final `SinkTokenStream.incrementToken()`.
    ///
    /// # Errors
    ///
    /// Java's `NullPointerException` before the first `reset()` (`it` is
    /// `null`), and the `ConcurrentModificationException` of the `ArrayList`
    /// iterator when the tee added or cleared states since the `reset()`.
    fn increment_token(&mut self) -> Result<bool> {
        let Some(it) = self.it.as_mut() else {
            return Err(LuceneError::null_pointer());
        };
        let states = self.cached_states.borrow();
        if it.cursor == states.states.len() {
            return Ok(false);
        }
        if states.mod_count != it.expected_mod_count {
            return Err(LuceneError::unchecked(
                "java.util.ConcurrentModificationException",
                String::new(),
            ));
        }
        let state = states.states[it.cursor].as_ref();
        it.cursor += 1;
        // `restoreState(null)` does nothing.
        if let Some(state) = state {
            self.source.restore_state(state)?;
        }
        Ok(true)
    }

    /// Equivalent to `SinkTokenStream.end()`, which restores the tee's final
    /// state and does not call `super.end()`.
    fn end(&mut self) -> Result<()> {
        let states = self.cached_states.borrow();
        if let Some(final_state) = states.get_final_state() {
            self.source.restore_state(final_state)?;
        }
        Ok(())
    }

    /// Equivalent to the final `SinkTokenStream.reset()`, `it =
    /// cachedStates.getStates()`.
    fn reset(&mut self) -> Result<()> {
        self.it = Some(self.cached_states.borrow().get_states());
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// A convenience wrapper for storing the cached states as well the final
/// state of the stream.
///
/// Equivalent to `org.apache.lucene.analysis.sinks.TeeSinkTokenFilter$States`.
/// A state is `None` where Java's `captureState()` returns `null` (a source
/// without attributes); `mod_count` is the `ArrayList`'s, which its
/// iterators check.
#[derive(Debug, Default)]
pub(crate) struct States {
    states: Vec<Option<CapturedState>>,
    mod_count: u64,
    final_state: Option<CapturedState>,
}

impl States {
    /// Equivalent to `States()`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `States.setFinalState(State)`.
    fn set_final_state(&mut self, final_state: Option<CapturedState>) {
        self.final_state = final_state;
    }

    /// Equivalent to `States.getFinalState()`.
    fn get_final_state(&self) -> Option<&CapturedState> {
        self.final_state.as_ref()
    }

    /// Equivalent to `States.add(State)`.
    fn add(&mut self, state: Option<CapturedState>) {
        self.mod_count += 1;
        self.states.push(state);
    }

    /// Equivalent to `States.getStates()`, `states.iterator()`.
    fn get_states(&self) -> StatesIterator {
        StatesIterator {
            cursor: 0,
            expected_mod_count: self.mod_count,
        }
    }

    /// Equivalent to `States.reset()`.
    fn reset(&mut self) {
        self.final_state = None;
        self.mod_count += 1;
        self.states.clear();
    }
}
