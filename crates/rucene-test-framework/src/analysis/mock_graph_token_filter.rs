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

//! Port of `org.apache.lucene.tests.analysis.MockGraphTokenFilter`.

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::{TokenFilter, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

use super::attributes::{char_term_id, Att};
use super::lookahead_token_filter::{
    new_lookahead_filter, LookaheadLogic, LookaheadTokenFilter, Position,
};
use crate::util::TestUtil;

const DEBUG: bool = false;

/// The subclass part of [`MockGraphTokenFilter`].
#[derive(Debug)]
pub struct MockGraphTokenFilterLogic {
    term_att: Att,
    seed: i64,
    random: Option<JdkRandom>,
}

impl LookaheadLogic for MockGraphTokenFilterLogic {
    type Pos = Position;

    fn after_position(
        f: &mut LookaheadTokenFilter<Self>,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<()> {
        if DEBUG {
            println!("MockGraphTF.afterPos");
        }
        let random = f
            .logic
            .random
            .as_ref()
            .expect("java.lang.NullPointerException: reset() was not called");
        if random.next_int_bound(7) == 5 {
            let pos_length = TestUtil::next_int(random, 1, 5);

            if DEBUG {
                println!("  do insert! posLen={}", pos_length);
            }

            let end_pos = f.output_pos + pos_length;
            let end_offset_at =
                |f: &mut LookaheadTokenFilter<Self>| f.positions.get(end_pos).end_offset;

            // Look ahead as needed until we figure out the right
            // endOffset:
            while !f.end && end_offset_at(f) == -1 && f.input_pos <= end_pos {
                if !f.peek_token(source, input)? {
                    break;
                }
            }

            let end_offset = end_offset_at(f);
            if end_offset != -1 {
                // Notify super class that we are injecting a token:
                f.insert_token(source);
                source.clear_attributes();
                f.pos_len_att.pos_len_mut().set_position_length(pos_length);
                let random = f.logic.random.as_ref().unwrap();
                let text = TestUtil::random_unicode_string(random);
                f.logic.term_att.char_term_mut().append_string(Some(&text));
                f.pos_inc_att.pos_inc_mut().set_position_increment(0);
                let output_pos = f.output_pos;
                let start_offset = f.positions.get(output_pos).start_offset;
                f.offset_att
                    .offset_mut()
                    .set_offset(start_offset, end_offset);
                if DEBUG {
                    println!(
                        "  inject: outputPos={} startOffset={} endOffset={} posLength={}",
                        f.output_pos, start_offset, end_offset, pos_length
                    );
                }
                // TODO: set TypeAtt too?
            } else {
                // Either 1) the tokens ended before our posLength,
                // or 2) our posLength ended inside a hole from the
                // input.  In each case we just skip the inserted
                // token.
            }
        }
        Ok(())
    }

    fn after_reset(f: &mut LookaheadTokenFilter<Self>) -> Result<()> {
        // NOTE: must be "deterministically random" because
        // BaseTokenStreamTestCase pulls tokens twice on the
        // same input and asserts they are the same:
        f.logic.random = Some(JdkRandom::new(f.logic.seed));
        Ok(())
    }

    fn after_close(f: &mut LookaheadTokenFilter<Self>) -> Result<()> {
        f.logic.random = None;
        Ok(())
    }

    fn increment_token(
        f: &mut LookaheadTokenFilter<Self>,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if DEBUG {
            println!(
                "MockGraphTF.incr inputPos={} outputPos={}",
                f.input_pos, f.output_pos
            );
        }
        if f.logic.random.is_none() {
            return Err(LuceneError::illegal_state(
                "incrementToken called in wrong state!",
            ));
        }
        f.next_token(source, input)
    }
}

/// Randomly inserts overlapped (posInc=0) tokens with posLength sometimes >
/// 1. The chain must have an OffsetAttribute.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockGraphTokenFilter`.
pub struct MockGraphTokenFilter;

impl MockGraphTokenFilter {
    /// `MockGraphTokenFilter(Random random, TokenStream input)`: draws the
    /// seed of the filter's own random.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory lacks one of the
    /// attributes the filter uses.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        random: &dyn Random,
        input: Box<dyn TokenStream>,
    ) -> Result<TokenFilter<LookaheadTokenFilter<MockGraphTokenFilterLogic>>> {
        new_lookahead_filter(input, |source| {
            let term_att = Att::add(source, char_term_id())?;
            Ok(MockGraphTokenFilterLogic {
                term_att,
                seed: random.next_long(),
                random: None,
            })
        })
    }
}
