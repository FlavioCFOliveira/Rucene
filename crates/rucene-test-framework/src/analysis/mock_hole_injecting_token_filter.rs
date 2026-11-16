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

//! Port of `org.apache.lucene.tests.analysis.MockHoleInjectingTokenFilter`.

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::attributes::{pos_inc_id, pos_len_id, Att};
use crate::index::base_index_file_format_test_case::npe_with;
use crate::util::TestUtil;

/// The logic of [`MockHoleInjectingTokenFilter`].
#[derive(Debug)]
pub struct MockHoleInjectingTokenFilterLogic {
    random_seed: i64,
    random: Option<JdkRandom>,
    pos_inc_att: Att,
    pos_len_att: Att,
    max_pos: i32,
    pos: i32,
}

impl TokenFilterLogic for MockHoleInjectingTokenFilterLogic {
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.random = Some(JdkRandom::new(self.random_seed));
        self.max_pos = -1;
        self.pos = -1;
        Ok(())
    }

    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let pos_inc = self.pos_inc_att.pos_inc().get_position_increment();
            let mut next_pos = self.pos.wrapping_add(pos_inc);
            // `random` is `null` until `reset()`; Java dereferences it only
            // when the first two conditions hold.
            let random = || {
                npe_with(self.random.as_ref(), || {
                    "Cannot invoke \"java.util.Random.nextInt(int)\" because \"this.random\" is null"
                        .to_string()
                })
            };
            // Carefully inject a hole only where it won't mess up
            // the graph:
            if pos_inc > 0 && self.max_pos <= next_pos && random().next_int_bound(5) == 3 {
                let hole_size = TestUtil::next_int(random(), 1, 5);
                self.pos_inc_att
                    .pos_inc_mut()
                    .set_position_increment(pos_inc.wrapping_add(hole_size));
                next_pos = next_pos.wrapping_add(hole_size);
            }
            self.pos = next_pos;
            let pos_len = self.pos_len_att.pos_len().get_position_length();
            self.max_pos = self.max_pos.max(self.pos.wrapping_add(pos_len));
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Randomly injects holes (similar to what a stopfilter would do).
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockHoleInjectingTokenFilter`.
pub struct MockHoleInjectingTokenFilter;

impl MockHoleInjectingTokenFilter {
    /// `MockHoleInjectingTokenFilter(Random random, TokenStream in)`: draws
    /// the seed of the filter's own random.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory provides no
    /// position attributes.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        random: &dyn Random,
        mut input: Box<dyn TokenStream>,
    ) -> Result<TokenFilter<MockHoleInjectingTokenFilterLogic>> {
        let source = input.attribute_source_mut();
        let pos_inc_att = Att::add(source, pos_inc_id())?;
        let pos_len_att = Att::add(source, pos_len_id())?;
        let random_seed = random.next_long();
        Ok(TokenFilter::new(
            input,
            MockHoleInjectingTokenFilterLogic {
                random_seed,
                random: None,
                pos_inc_att,
                pos_len_att,
                max_pos: 0,
                pos: 0,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use randomizedtesting::jdk::exception::Throwable;
    use randomizedtesting::jdk::random::JdkRandom;
    use rucene::analysis::TokenStream;

    use super::MockHoleInjectingTokenFilter;
    use crate::analysis::{CannedTokenStream, Token};

    /// Before `reset()` the filter's `random` is `null`; Java dereferences it
    /// only when `posInc > 0 && maxPos <= nextPos`
    /// (`MockHoleInjectingTokenFilter.java:64`), so a token with `posInc == 0`
    /// passes, and one that reaches the draw throws the helpful
    /// `NullPointerException`. The port threw on every token, with an
    /// invented message.
    #[test]
    fn random_is_dereferenced_only_when_a_hole_may_be_drawn() {
        let canned = CannedTokenStream::new(vec![
            Token::with_pos_inc("a", 0, 0, 1),
            Token::with_pos_inc("b", 1, 2, 3),
        ]);
        let mut f =
            MockHoleInjectingTokenFilter::new(&JdkRandom::new(0), Box::new(canned)).unwrap();
        assert!(f.increment_token().unwrap());
        let t = match catch_unwind(AssertUnwindSafe(|| f.increment_token())) {
            Ok(_) => panic!("expected the NullPointerException"),
            Err(payload) => Throwable::from_panic(payload),
        };
        assert_eq!(t.class_name(), "java.lang.NullPointerException");
        assert_eq!(
            t.message(),
            Some("Cannot invoke \"java.util.Random.nextInt(int)\" because \"this.random\" is null")
        );
    }
}
