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

//! Port of `org.apache.lucene.tests.analysis.LookaheadTokenFilter`.

use std::fmt::Debug;

use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene::util::ring_buffer::{Resettable, RollingBuffer};

use super::attributes::{offset_id, pos_inc_id, pos_len_id, Att};

const DEBUG: bool = false;

/// Holds all state for a single position; subclass this to record other
/// state at each position.
///
/// Equivalent to `LookaheadTokenFilter.Position`.
#[derive(Debug)]
pub struct Position {
    /// Buffered input tokens at this position.
    pub input_tokens: Vec<CapturedState>,
    /// Next buffered token to be returned to consumer.
    pub next_read: usize,
    /// Any token leaving from this position should have this startOffset.
    pub start_offset: i32,
    /// Any token arriving to this position should have this endOffset.
    pub end_offset: i32,
}

impl Default for Position {
    fn default() -> Self {
        Position {
            input_tokens: Vec::new(),
            next_read: 0,
            start_offset: -1,
            end_offset: -1,
        }
    }
}

impl Resettable for Position {
    fn reset(&mut self) {
        self.input_tokens.clear();
        self.next_read = 0;
        self.start_offset = -1;
        self.end_offset = -1;
    }
}

impl Position {
    /// `add(AttributeSource.State)`.
    pub fn add(&mut self, state: CapturedState) {
        self.input_tokens.push(state);
    }

    /// `nextState()`.
    pub fn next_state(&mut self) -> &CapturedState {
        java_assert_unchecked!(self.next_read < self.input_tokens.len());
        let i = self.next_read;
        self.next_read += 1;
        &self.input_tokens[i]
    }
}

/// A position type of a [`LookaheadTokenFilter`]: [`Position`] or a
/// "subclass" holding one.
///
/// Java's `T extends LookaheadTokenFilter.Position`; Rust has no inheritance,
/// so a subclass exposes the `Position` it contains.
pub trait LookaheadPosition: Resettable + Default + Debug {
    /// The inherited `Position` fields.
    fn position(&self) -> &Position;
    /// The inherited `Position` fields, mutably.
    fn position_mut(&mut self) -> &mut Position;
}

impl LookaheadPosition for Position {
    fn position(&self) -> &Position {
        self
    }
    fn position_mut(&mut self) -> &mut Position {
        self
    }
}

/// The abstract members of `LookaheadTokenFilter` a subclass supplies.
///
/// Each hook receives the whole filter, so that it can call the
/// "protected" members (`peekToken`, `nextToken`, `insertToken`, the
/// `positions` buffer) as a Java subclass does.
pub trait LookaheadLogic: Debug + Sized + Send + Sync {
    /// The position type, Java's `T`.
    type Pos: LookaheadPosition + Send + Sync;

    /// `incrementToken()`.
    ///
    /// # Errors
    ///
    /// What the input or the subclass throws.
    fn increment_token(
        f: &mut LookaheadTokenFilter<Self>,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool>;

    /// This is called when all input tokens leaving a given position have
    /// been returned. Override this and call insertToken and then set
    /// whichever token's attributes you want, if you want to inject a token
    /// starting from this position.
    ///
    /// # Errors
    ///
    /// What the input or the subclass throws.
    fn after_position(
        _f: &mut LookaheadTokenFilter<Self>,
        _source: &mut AttributeSource,
        _input: &mut dyn TokenStream,
    ) -> Result<()> {
        Ok(())
    }

    /// The subclass's `reset()` after `super.reset()`.
    ///
    /// # Errors
    ///
    /// What the subclass throws.
    fn after_reset(_f: &mut LookaheadTokenFilter<Self>) -> Result<()> {
        Ok(())
    }

    /// The subclass's `close()` after `super.close()`.
    ///
    /// # Errors
    ///
    /// What the subclass throws.
    fn after_close(_f: &mut LookaheadTokenFilter<Self>) -> Result<()> {
        Ok(())
    }
}

/// An abstract TokenFilter to make it easier to build graph token filters
/// requiring some lookahead. This class handles the details of buffering up
/// tokens, recording them by position, restoring them, providing access to
/// them, etc.
///
/// Equivalent to `org.apache.lucene.tests.analysis.LookaheadTokenFilter`.
pub struct LookaheadTokenFilter<L: LookaheadLogic> {
    /// The subclass.
    pub logic: L,
    /// `posIncAtt`.
    pub pos_inc_att: Att,
    /// `posLenAtt`.
    pub pos_len_att: Att,
    /// `offsetAtt`.
    pub offset_att: Att,
    /// Position of last read input token.
    pub input_pos: i32,
    /// Position of next possible output token to return.
    pub output_pos: i32,
    /// True if we hit end from our input.
    pub end: bool,
    token_pending: bool,
    insert_pending: bool,
    /// The buffered positions.
    pub positions: RollingBuffer<L::Pos, fn() -> L::Pos>,
}

impl<L: LookaheadLogic> Debug for LookaheadTokenFilter<L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LookaheadTokenFilter")
            .field("logic", &self.logic)
            .field("input_pos", &self.input_pos)
            .field("output_pos", &self.output_pos)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}

/// Builds a lookahead filter over `input`: the base class's attributes are
/// added to the input first, then `make_logic` adds the subclass's and
/// builds it, Java's field initialization order.
///
/// # Errors
///
/// What adding an attribute or `make_logic` returns.
pub fn new_lookahead_filter<L: LookaheadLogic>(
    mut input: Box<dyn TokenStream>,
    make_logic: impl FnOnce(&mut AttributeSource) -> Result<L>,
) -> Result<TokenFilter<LookaheadTokenFilter<L>>> {
    let source = input.attribute_source_mut();
    let pos_inc_att = Att::add(source, pos_inc_id())?;
    let pos_len_att = Att::add(source, pos_len_id())?;
    let offset_att = Att::add(source, offset_id())?;
    let logic = make_logic(source)?;
    Ok(TokenFilter::new(
        input,
        LookaheadTokenFilter {
            logic,
            pos_inc_att,
            pos_len_att,
            offset_att,
            input_pos: 0,
            output_pos: 0,
            end: false,
            token_pending: false,
            insert_pending: false,
            positions: RollingBuffer::new(L::Pos::default as fn() -> L::Pos),
        },
    ))
}

impl<L: LookaheadLogic> LookaheadTokenFilter<L> {
    /// Call this only from within afterPosition, to insert a new token. After
    /// calling this you should set any necessary token you need.
    pub fn insert_token(&mut self, source: &AttributeSource) {
        if self.token_pending {
            let state = capture(source);
            self.positions.get(self.input_pos).position_mut().add(state);
            self.token_pending = false;
        }
        java_assert_unchecked!(!self.insert_pending);
        self.insert_pending = true;
    }

    /// Returns true if there is a new token.
    ///
    /// # Errors
    ///
    /// What the input throws.
    pub fn peek_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if DEBUG {
            println!(
                "LTF.peekToken inputPos={} outputPos={} tokenPending={}",
                self.input_pos, self.output_pos, self.token_pending
            );
        }
        java_assert_unchecked!(!self.end);
        java_assert_unchecked!(self.input_pos == -1 || self.output_pos <= self.input_pos);
        if self.token_pending {
            let state = capture(source);
            self.positions.get(self.input_pos).position_mut().add(state);
            self.token_pending = false;
        }
        let got_token = input.increment_token()?;
        if DEBUG {
            println!("  input.incrToken() returned {}", got_token);
        }
        if got_token {
            self.input_pos += self.pos_inc_att.pos_inc().get_position_increment();
            java_assert_unchecked!(self.input_pos >= 0);
            if DEBUG {
                println!("  now inputPos={}", self.input_pos);
            }

            let pos_len = self.pos_len_att.pos_len().get_position_length();
            let (start_offset, end_offset) = {
                let o = self.offset_att.offset();
                (o.start_offset(), o.end_offset())
            };
            let input_pos = self.input_pos;
            {
                let start_pos_data = self.positions.get(input_pos).position_mut();
                if start_pos_data.start_offset == -1 {
                    start_pos_data.start_offset = start_offset;
                } else {
                    // Make sure our input isn't messing up offsets:
                    java_assert_unchecked!(
                        start_pos_data.start_offset == start_offset,
                        "prev startOffset={} vs new startOffset={} inputPos={}",
                        start_pos_data.start_offset,
                        start_offset,
                        input_pos
                    );
                }
            }
            {
                let end_pos_data = self.positions.get(input_pos + pos_len).position_mut();
                if end_pos_data.end_offset == -1 {
                    end_pos_data.end_offset = end_offset;
                } else {
                    // Make sure our input isn't messing up offsets:
                    java_assert_unchecked!(
                        end_pos_data.end_offset == end_offset,
                        "prev endOffset={} vs new endOffset={} inputPos={}",
                        end_pos_data.end_offset,
                        end_offset,
                        input_pos
                    );
                }
            }

            self.token_pending = true;
        } else {
            self.end = true;
        }

        Ok(got_token)
    }

    /// Call this when you are done looking ahead; it will set the next token
    /// to return. Return the boolean back to the caller.
    ///
    /// # Errors
    ///
    /// What the input or the subclass throws.
    pub fn next_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if DEBUG {
            println!(
                "LTF.nextToken inputPos={} outputPos={} tokenPending={}",
                self.input_pos, self.output_pos, self.token_pending
            );
        }
        let mut pos = self.output_pos;
        self.positions.get(pos);

        // While loop here in case we have to
        // skip over a hole from the input:
        loop {
            // See if we have a previously buffered token to
            // return at the current position:
            let (next_read, buffered, start_offset) = {
                let p = self.positions.get(pos).position();
                (p.next_read, p.input_tokens.len(), p.start_offset)
            };
            if next_read < buffered {
                if DEBUG {
                    println!("  return previously buffered token");
                }
                // This position has buffered tokens to serve up:
                if self.token_pending {
                    let state = capture(source);
                    self.positions.get(self.input_pos).position_mut().add(state);
                    self.token_pending = false;
                }
                let output_pos = self.output_pos;
                let state = self
                    .positions
                    .get(output_pos)
                    .position_mut()
                    .next_state()
                    .clone();
                source.restore_state(&state)?;
                return Ok(true);
            }

            if self.input_pos == -1 || self.output_pos == self.input_pos {
                // No more buffered tokens:
                // We may still get input tokens at this position
                if self.token_pending {
                    // Fast path: just return token we had just incr'd,
                    // without having captured/restored its state:
                    if DEBUG {
                        println!("  pass-through: return pending token");
                    }
                    self.token_pending = false;
                    return Ok(true);
                } else if self.end || !self.peek_token(source, input)? {
                    if DEBUG {
                        println!("  END");
                    }
                    L::after_position(self, source, input)?;
                    if self.insert_pending {
                        // Subclass inserted a token at this same
                        // position:
                        if DEBUG {
                            println!("  return inserted token");
                        }
                        java_assert_unchecked!(self.inserted_token_consistent());
                        self.insert_pending = false;
                        return Ok(true);
                    }

                    return Ok(false);
                }
            } else {
                if start_offset != -1 {
                    // This position had at least one token leaving
                    if DEBUG {
                        println!("  call afterPosition");
                    }
                    L::after_position(self, source, input)?;
                    if self.insert_pending {
                        // Subclass inserted a token at this same
                        // position:
                        if DEBUG {
                            println!("  return inserted token");
                        }
                        java_assert_unchecked!(self.inserted_token_consistent());
                        self.insert_pending = false;
                        return Ok(true);
                    }
                }

                // Done with this position; move on:
                self.output_pos += 1;
                if DEBUG {
                    println!("  next position: outputPos={}", self.output_pos);
                }
                self.positions.free_before(self.output_pos);
                pos = self.output_pos;
                self.positions.get(pos);
            }
        }
    }

    // If subclass inserted a token, make sure it had in fact
    // looked ahead enough:
    fn inserted_token_consistent(&mut self) -> bool {
        let pos_len = self.pos_len_att.pos_len().get_position_length();
        let end_pos_data_end_offset = self
            .positions
            .get(self.output_pos + pos_len)
            .position()
            .end_offset;
        java_assert_unchecked!(end_pos_data_end_offset != -1);
        let end_offset = self.offset_att.offset().end_offset();
        java_assert_unchecked!(
            end_offset == end_pos_data_end_offset,
            "offsetAtt.endOffset={} vs expected={}",
            end_offset,
            end_pos_data_end_offset
        );
        true
    }
}

fn capture(source: &AttributeSource) -> CapturedState {
    source
        .capture_state()
        .expect("INVARIANT: a lookahead filter's source holds attributes")
}

impl<L: LookaheadLogic> TokenFilterLogic for LookaheadTokenFilter<L> {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        L::increment_token(self, source, input)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.positions.reset();
        self.input_pos = -1;
        self.output_pos = 0;
        self.token_pending = false;
        self.end = false;
        L::after_reset(self)
    }

    fn close(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.close()?;
        L::after_close(self)
    }
}
