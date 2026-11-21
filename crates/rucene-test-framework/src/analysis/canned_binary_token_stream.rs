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

//! Port of `org.apache.lucene.tests.analysis.CannedBinaryTokenStream`.

use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::attributes::{
    bytes_term_id, offset_id, pos_inc_id, pos_len_id, term_to_bytes_ref_id, Att,
};
use super::token::Token;

/// Represents a binary token.
///
/// Equivalent to `CannedBinaryTokenStream.BinaryToken`.
#[derive(Clone, Debug)]
pub struct BinaryToken {
    /// The term bytes.
    pub term: BytesRef,
    /// The position increment.
    pub pos_inc: i32,
    /// The position length.
    pub pos_len: i32,
    /// The start offset.
    pub start_offset: i32,
    /// The end offset.
    pub end_offset: i32,
}

impl BinaryToken {
    /// `BinaryToken(BytesRef term)`: position increment and length `1`.
    pub fn new(term: BytesRef) -> BinaryToken {
        Self::with_positions(term, 1, 1)
    }

    /// `BinaryToken(BytesRef term, int posInc, int posLen)`.
    pub fn with_positions(term: BytesRef, pos_inc: i32, pos_len: i32) -> BinaryToken {
        BinaryToken {
            term,
            pos_inc,
            pos_len,
            start_offset: 0,
            end_offset: 0,
        }
    }
}

/// TokenStream from a canned list of binary (BytesRef-based) tokens.
///
/// Equivalent to `org.apache.lucene.tests.analysis.CannedBinaryTokenStream`.
#[derive(Debug)]
pub struct CannedBinaryTokenStream {
    source: AttributeSource,
    tokens: Vec<BinaryToken>,
    upto: usize,
    term_att: Att,
    pos_incr_att: Att,
    pos_length_att: Att,
    offset_att: Att,
}

impl CannedBinaryTokenStream {
    /// `CannedBinaryTokenStream(BinaryToken... tokens)`.
    pub fn new(tokens: Vec<BinaryToken>) -> CannedBinaryTokenStream {
        let mut source = AttributeSource::new_with_factory(Token::token_attribute_factory());
        let term_att = Att::add(&mut source, bytes_term_id())
            .expect("INVARIANT: the default factory provides BytesTermAttribute");
        let pos_incr_att = Att::add(&mut source, pos_inc_id())
            .expect("INVARIANT: Token provides PositionIncrementAttribute");
        let pos_length_att = Att::add(&mut source, pos_len_id())
            .expect("INVARIANT: Token provides PositionLengthAttribute");
        let offset_att =
            Att::add(&mut source, offset_id()).expect("INVARIANT: Token provides OffsetAttribute");
        java_assert_unchecked!(Att::get(&source, term_to_bytes_ref_id())
            .is_some_and(|bytes_att| bytes_att.same(&term_att)));
        CannedBinaryTokenStream {
            source,
            tokens,
            upto: 0,
            term_att,
            pos_incr_att,
            pos_length_att,
            offset_att,
        }
    }
}

impl TokenStream for CannedBinaryTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.upto < self.tokens.len() {
            let token = &self.tokens[self.upto];
            self.upto += 1;
            // TODO: can we just capture/restoreState so
            // we get all attrs...?
            self.source.clear_attributes();
            self.term_att
                .bytes_term_mut()
                .set_bytes_ref(token.term.clone());
            self.pos_incr_att
                .pos_inc_mut()
                .set_position_increment(token.pos_inc);
            self.pos_length_att
                .pos_len_mut()
                .set_position_length(token.pos_len);
            self.offset_att
                .offset_mut()
                .set_offset(token.start_offset, token.end_offset);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
