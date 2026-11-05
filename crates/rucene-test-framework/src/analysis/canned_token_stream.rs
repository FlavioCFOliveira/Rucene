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

//! Port of `org.apache.lucene.tests.analysis.CannedTokenStream`.

use rucene::analysis::TokenStream;
use rucene::error::Result;
use rucene::util::attribute::{AttributeImpl, AttributeSource};

use super::attributes::{offset_id, pos_inc_id, Att};
use super::token::Token;

/// TokenStream from a canned list of Tokens.
///
/// Equivalent to `org.apache.lucene.tests.analysis.CannedTokenStream`.
#[derive(Debug)]
pub struct CannedTokenStream {
    source: AttributeSource,
    tokens: Vec<Token>,
    upto: usize,
    offset_att: Att,
    pos_incr_att: Att,
    final_offset: i32,
    final_pos_inc: i32,
}

impl CannedTokenStream {
    /// `CannedTokenStream(Token... tokens)`.
    pub fn new(tokens: Vec<Token>) -> CannedTokenStream {
        Self::with_final(0, 0, tokens)
    }

    /// If you want trailing holes, pass a non-zero finalPosInc.
    pub fn with_final(
        final_pos_inc: i32,
        final_offset: i32,
        tokens: Vec<Token>,
    ) -> CannedTokenStream {
        let mut source = AttributeSource::new_with_factory(Token::token_attribute_factory());
        let offset_att =
            Att::add(&mut source, offset_id()).expect("INVARIANT: Token provides OffsetAttribute");
        let pos_incr_att = Att::add(&mut source, pos_inc_id())
            .expect("INVARIANT: Token provides PositionIncrementAttribute");
        CannedTokenStream {
            source,
            tokens,
            upto: 0,
            offset_att,
            pos_incr_att,
            final_offset,
            final_pos_inc,
        }
    }
}

impl TokenStream for CannedTokenStream {
    fn end(&mut self) -> Result<()> {
        self.source.end_attributes();
        self.pos_incr_att
            .pos_inc_mut()
            .set_position_increment(self.final_pos_inc);
        self.offset_att
            .offset_mut()
            .set_offset(self.final_offset, self.final_offset);
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.upto = 0;
        Ok(())
    }

    fn increment_token(&mut self) -> Result<bool> {
        if self.upto < self.tokens.len() {
            self.source.clear_attributes();
            // NOTE: this looks weird, casting offsetAtt to Token, but because we are using the Token
            // class's AttributeFactory, all attributes are
            // in fact backed by the Token class, so we just copy the current token into our Token:
            let token = &self.tokens[self.upto];
            self.upto += 1;
            token.copy_to(&mut **self.offset_att.cell().borrow_mut());
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
