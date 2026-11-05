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

//! Port of `org.apache.lucene.tests.analysis.MockRandomLookaheadTokenFilter`.

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::{TokenFilter, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::attributes::{char_term_id, Att};
use super::lookahead_token_filter::{
    new_lookahead_filter, LookaheadLogic, LookaheadTokenFilter, Position,
};

const DEBUG: bool = false;

/// The subclass part of [`MockRandomLookaheadTokenFilter`].
#[derive(Debug)]
pub struct MockRandomLookaheadTokenFilterLogic {
    term_att: Att,
    random: JdkRandom,
    seed: i64,
}

impl LookaheadLogic for MockRandomLookaheadTokenFilterLogic {
    type Pos = Position;

    fn after_position(
        f: &mut LookaheadTokenFilter<Self>,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<()> {
        if !f.end && f.logic.random.next_int_bound(4) == 2 {
            f.peek_token(source, input)?;
        }
        Ok(())
    }

    fn increment_token(
        f: &mut LookaheadTokenFilter<Self>,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if DEBUG {
            println!("\n{:?}: MRLTF.incrToken", std::thread::current().name());
        }

        if !f.end {
            loop {
                if f.logic.random.next_int_bound(3) == 1 {
                    if !f.peek_token(source, input)? {
                        if DEBUG {
                            println!("  peek; inputPos={} END", f.input_pos);
                        }
                        break;
                    }
                    if DEBUG {
                        println!(
                            "  peek; inputPos={} token={}",
                            f.input_pos,
                            f.logic.term_att.term_string()
                        );
                    }
                } else {
                    if DEBUG {
                        println!("  done peek");
                    }
                    break;
                }
            }
        }

        let result = f.next_token(source, input)?;
        if DEBUG {
            if result {
                println!(
                    "  return nextToken token={}",
                    f.logic.term_att.term_string()
                );
            } else {
                println!("  return nextToken END");
            }
        }
        Ok(result)
    }

    fn after_reset(f: &mut LookaheadTokenFilter<Self>) -> Result<()> {
        f.logic.random.set_seed(f.logic.seed);
        Ok(())
    }
}

/// Uses [`LookaheadTokenFilter`] to randomly peek at future tokens.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockRandomLookaheadTokenFilter`.
pub struct MockRandomLookaheadTokenFilter;

impl MockRandomLookaheadTokenFilter {
    /// `MockRandomLookaheadTokenFilter(Random random, TokenStream in)`: draws
    /// the seed of the filter's own random.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory lacks one of the
    /// attributes the filter uses.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        random: &dyn Random,
        input: Box<dyn TokenStream>,
    ) -> Result<TokenFilter<LookaheadTokenFilter<MockRandomLookaheadTokenFilterLogic>>> {
        new_lookahead_filter(input, |source| {
            let term_att = Att::add(source, char_term_id())?;
            let seed = random.next_long();
            Ok(MockRandomLookaheadTokenFilterLogic {
                term_att,
                random: JdkRandom::new(seed),
                seed,
            })
        })
    }
}
