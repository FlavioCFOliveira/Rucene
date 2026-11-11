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

//! Port of `org.apache.lucene.tests.analysis.ValidatingTokenFilter`.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::io::Write;

use rucene::analysis::tokenattributes::{OffsetAttribute, PositionIncrementAttribute};
use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeSource, Unwrappable};

use super::attributes::{char_term_id, offset_id, pos_inc_id, pos_len_id, Att};
use super::token::Token;

const MAX_DEBUG_TOKENS: usize = 20;

/// The logic of [`ValidatingTokenFilter`].
#[derive(Debug)]
pub struct ValidatingTokenFilterLogic {
    pos: i32,
    last_start_offset: i32,
    // Maps position to the start/end offset:
    pos_to_start_offset: HashMap<i32, i32>,
    pos_to_end_offset: HashMap<i32, i32>,
    pos_inc_att: Option<Att>,
    pos_len_att: Option<Att>,
    offset_att: Option<Att>,
    term_att: Option<Att>,
    // record the last MAX_DEBUG_TOKENS tokens seen so they can be dumped on failure
    tokens: VecDeque<Token>,
    name: String,
}

impl ValidatingTokenFilterLogic {
    fn term(&self) -> String {
        self.term_att
            .as_ref()
            .map(Att::term_string)
            .unwrap_or_else(|| "null".to_string())
    }

    fn add_token(&mut self, start_offset: i32, end_offset: i32, pos_inc: i32) {
        if self.tokens.len() == MAX_DEBUG_TOKENS {
            self.tokens.pop_front();
        }
        let term = self.term();
        self.tokens.push_back(Token::with_pos_inc(
            &term,
            pos_inc,
            start_offset,
            end_offset,
        ));
    }

    /// `dump()`.
    pub fn dump(&self) -> String {
        let mut buf = String::new();
        buf.push_str(&self.name);
        buf.push_str(": ");
        for token in &self.tokens {
            buf.push_str(&format!(
                "{}<[{}-{}] +{}> ",
                token.term(),
                token.start_offset(),
                token.end_offset(),
                token.get_position_increment()
            ));
        }
        buf
    }
}

impl TokenFilterLogic for ValidatingTokenFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }

        let mut start_offset = 0;
        let mut end_offset = 0;
        let mut pos_inc = 0;

        if let Some(att) = &self.pos_inc_att {
            pos_inc = att.pos_inc().get_position_increment();
        }
        if let Some(att) = &self.offset_att {
            let o = att.offset();
            start_offset = o.start_offset();
            end_offset = o.end_offset();
        }

        let pos_len = match &self.pos_len_att {
            None => 1,
            Some(att) => att.pos_len().get_position_length(),
        };

        self.add_token(start_offset, end_offset, pos_inc);

        if self.pos_inc_att.is_some() {
            self.pos += pos_inc;
            if self.pos == -1 {
                eprintln!("{}", self.dump());
                return Err(LuceneError::illegal_state(format!(
                    "{}: first posInc must be > 0",
                    self.name
                )));
            }
        }

        if let Some(att) = &self.offset_att {
            if start_offset < self.last_start_offset {
                eprintln!("{}", self.dump());
                return Err(LuceneError::illegal_state(format!(
                    "{}: offsets must not go backwards startOffset={} is < lastStartOffset={}",
                    self.name, start_offset, self.last_start_offset
                )));
            }
            self.last_start_offset = att.offset().start_offset();
        }

        if self.offset_att.is_some() && self.pos_inc_att.is_some() {
            match self.pos_to_start_offset.get(&self.pos) {
                None => {
                    // First time we've seen a token leaving from this position:
                    self.pos_to_start_offset.insert(self.pos, start_offset);
                }
                Some(&old_start_offset) => {
                    // We've seen a token leaving from this position
                    // before; verify the startOffset is the same:
                    if old_start_offset != start_offset {
                        eprintln!("{}", self.dump());
                        return Err(LuceneError::illegal_state(format!(
                            "{}: inconsistent startOffset at pos={}: {} vs {}; token={}",
                            self.name,
                            self.pos,
                            old_start_offset,
                            start_offset,
                            self.term()
                        )));
                    }
                }
            }

            let end_pos = self.pos + pos_len;

            match self.pos_to_end_offset.get(&end_pos) {
                None => {
                    // First time we've seen a token arriving to this position:
                    self.pos_to_end_offset.insert(end_pos, end_offset);
                }
                Some(&old_end_offset) => {
                    // We've seen a token arriving to this position
                    // before; verify the endOffset is the same:
                    if old_end_offset != end_offset {
                        eprintln!("{}", self.dump());
                        return Err(LuceneError::illegal_state(format!(
                            "{}: inconsistent endOffset at pos={}: {} vs {}; token={}",
                            self.name,
                            end_pos,
                            old_end_offset,
                            end_offset,
                            self.term()
                        )));
                    }
                }
            }
        }

        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.pos = -1;
        self.pos_to_start_offset.clear();
        self.pos_to_end_offset.clear();
        self.last_start_offset = 0;
        self.tokens.clear();
        Ok(())
    }
}

/// A TokenFilter that checks consistency of the tokens (eg offsets are
/// consistent with one another).
///
/// Equivalent to `org.apache.lucene.tests.analysis.ValidatingTokenFilter`.
#[derive(Debug)]
pub struct ValidatingTokenFilter {
    inner: TokenFilter<ValidatingTokenFilterLogic>,
}

impl ValidatingTokenFilter {
    /// The name arg is used to identify this stage when throwing exceptions
    /// (useful if you have more than one instance in your chain).
    pub fn new(input: Box<dyn TokenStream>, name: &str) -> ValidatingTokenFilter {
        let source = input.attribute_source();
        let logic = ValidatingTokenFilterLogic {
            pos: 0,
            last_start_offset: 0,
            pos_to_start_offset: HashMap::new(),
            pos_to_end_offset: HashMap::new(),
            pos_inc_att: Att::get(source, pos_inc_id()),
            pos_len_att: Att::get(source, pos_len_id()),
            offset_att: Att::get(source, offset_id()),
            term_att: Att::get(source, char_term_id()),
            tokens: VecDeque::new(),
            name: name.to_string(),
        };
        ValidatingTokenFilter {
            inner: TokenFilter::new(input, logic),
        }
    }

    /// `dump()`: the name and the last tokens seen.
    pub fn dump(&self) -> String {
        self.inner_logic().dump()
    }

    fn inner_logic(&self) -> &ValidatingTokenFilterLogic {
        self.inner.logic()
    }

    /// Prints the `dump()` of every `ValidatingTokenFilter` of the chain
    /// ending at `input`, innermost first.
    pub fn dump_validating_token_filters(input: &dyn TokenStream, out: &mut dyn Write) {
        if let Some(wrapper) = input.as_unwrappable() {
            Self::dump_validating_token_filters(wrapper.unwrap(), out);
            if let Some(v) = input
                .as_any()
                .and_then(|a| a.downcast_ref::<ValidatingTokenFilter>())
            {
                let _ = writeln!(out, "{}", v.dump());
            }
        }
    }
}

impl TokenStream for ValidatingTokenFilter {
    fn increment_token(&mut self) -> Result<bool> {
        self.inner.increment_token()
    }

    fn end(&mut self) -> Result<()> {
        // TODO: what else to validate
        // TODO: check that endOffset is >= max(endOffset)
        // we've seen
        self.inner.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.inner.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.inner.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.inner.attribute_source_mut()
    }

    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        self.inner.as_unwrappable()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
