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

//! Port of `TokenGroup.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use rucene::analysis::tokenattributes::{CharTermAttribute, OffsetAttribute};
use rucene::analysis::{AttributeHandle, TokenStream};
use rucene::error::Result;

const MAX_NUM_TOKENS_PER_GROUP: usize = 50;

/// One, or several overlapping tokens, along with the score(s) and the scope
/// of the original text.
///
/// Equivalent to `org.apache.lucene.search.highlight.TokenGroup`.
#[derive(Debug)]
pub struct TokenGroup {
    scores: [f32; MAX_NUM_TOKENS_PER_GROUP],
    num_tokens: i32,
    start_offset: i32,
    end_offset: i32,
    tot: f32,
    match_start_offset: i32,
    match_end_offset: i32,
    offset_att: AttributeHandle<dyn OffsetAttribute>,
}

impl TokenGroup {
    /// Equivalent to `new TokenGroup(TokenStream)`.
    ///
    /// # Errors
    ///
    /// What `addAttribute` raises.
    pub fn new(token_stream: &mut dyn TokenStream) -> Result<Self> {
        let offset_att = AttributeHandle::add(token_stream.attribute_source_mut())?;
        let _: AttributeHandle<dyn CharTermAttribute> =
            AttributeHandle::add(token_stream.attribute_source_mut())?;
        Ok(Self {
            scores: [0.0; MAX_NUM_TOKENS_PER_GROUP],
            num_tokens: 0,
            start_offset: 0,
            end_offset: 0,
            tot: 0.0,
            match_start_offset: 0,
            match_end_offset: 0,
            offset_att,
        })
    }

    /// Equivalent to the package-private `addToken(float)`.
    pub(crate) fn add_token(&mut self, score: f32) {
        if (self.num_tokens as usize) < MAX_NUM_TOKENS_PER_GROUP {
            let (term_start_offset, term_end_offset) = {
                let att = self.offset_att.borrow();
                (att.start_offset(), att.end_offset())
            };
            if self.num_tokens == 0 {
                self.start_offset = term_start_offset;
                self.match_start_offset = term_start_offset;
                self.end_offset = term_end_offset;
                self.match_end_offset = term_end_offset;
                self.tot += score;
            } else {
                self.start_offset = self.start_offset.min(term_start_offset);
                self.end_offset = self.end_offset.max(term_end_offset);
                if score > 0.0 {
                    if self.tot == 0.0 {
                        self.match_start_offset = term_start_offset;
                        self.match_end_offset = term_end_offset;
                    } else {
                        self.match_start_offset = self.match_start_offset.min(term_start_offset);
                        self.match_end_offset = self.match_end_offset.max(term_end_offset);
                    }
                    self.tot += score;
                }
            }
            self.scores[self.num_tokens as usize] = score;
            self.num_tokens += 1;
        }
    }

    /// Equivalent to the package-private `isDistinct()`.
    pub(crate) fn is_distinct(&self) -> bool {
        self.offset_att.borrow().start_offset() >= self.end_offset
    }

    /// Equivalent to the package-private `clear()`.
    pub(crate) fn clear(&mut self) {
        self.num_tokens = 0;
        self.tot = 0.0;
    }

    /// Returns the score for a given token in this group.
    ///
    /// Equivalent to `getScore(int)`.
    ///
    /// # Panics
    ///
    /// When `index` is outside `0..50`, where Java throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn get_score(&self, index: i32) -> f32 {
        self.scores[index as usize]
    }

    /// Returns the earliest start offset in the original text of a matching
    /// token in this group (score &gt; 0), or if there are none then the
    /// earliest offset of any token in the group.
    ///
    /// Equivalent to `getStartOffset()`.
    pub fn get_start_offset(&self) -> i32 {
        self.match_start_offset
    }

    /// Returns the latest end offset in the original text of a matching
    /// token in this group (score &gt; 0), or if there are none then
    /// `get_end_offset()`.
    ///
    /// Equivalent to `getEndOffset()`.
    pub fn get_end_offset(&self) -> i32 {
        self.match_end_offset
    }

    /// Returns the number of tokens in this group.
    ///
    /// Equivalent to `getNumTokens()`.
    pub fn get_num_tokens(&self) -> i32 {
        self.num_tokens
    }

    /// Returns all tokens' scores summed up.
    ///
    /// Equivalent to `getTotalScore()`.
    pub fn get_total_score(&self) -> f32 {
        self.tot
    }
}