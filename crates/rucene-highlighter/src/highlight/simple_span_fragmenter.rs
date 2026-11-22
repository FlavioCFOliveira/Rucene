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

//! Port of `SimpleSpanFragmenter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute,
};
use rucene::analysis::{AttributeHandle, TokenStream};
use rucene::error::Result;

use crate::highlight::fragmenter::Fragmenter;
use crate::highlight::query_scorer::QueryScorer;
use crate::java::{term_text, utf16_len};

const DEFAULT_FRAGMENT_SIZE: i32 = 100;

/// `Fragmenter` implementation which breaks text up into same-size fragments
/// but does not split up `Spans`. This is a simple sample class.
///
/// Equivalent to `org.apache.lucene.search.highlight.SimpleSpanFragmenter`.
#[derive(Debug)]
pub struct SimpleSpanFragmenter {
    fragment_size: i32,
    current_num_frags: i32,
    position: i32,
    query_scorer: QueryScorer,
    wait_for_pos: i32,
    text_size: i32,
    term_att: Option<AttributeHandle<dyn CharTermAttribute>>,
    pos_inc_att: Option<AttributeHandle<dyn PositionIncrementAttribute>>,
    offset_att: Option<AttributeHandle<dyn OffsetAttribute>>,
}

impl SimpleSpanFragmenter {
    /// Equivalent to `new SimpleSpanFragmenter(QueryScorer)`: `query_scorer`
    /// is the handle of the `QueryScorer` that was used to score hits.
    pub fn new(query_scorer: QueryScorer) -> Self {
        Self::with_fragment_size(query_scorer, DEFAULT_FRAGMENT_SIZE)
    }

    /// Equivalent to `new SimpleSpanFragmenter(QueryScorer, int)`.
    pub fn with_fragment_size(query_scorer: QueryScorer, fragment_size: i32) -> Self {
        Self {
            fragment_size,
            current_num_frags: 0,
            position: -1,
            query_scorer,
            wait_for_pos: -1,
            text_size: 0,
            term_att: None,
            pos_inc_att: None,
            offset_att: None,
        }
    }
}

const NOT_STARTED: &str = "NullPointerException: start(String, TokenStream) was not called";

impl Fragmenter for SimpleSpanFragmenter {
    /// # Panics
    ///
    /// Before `start`, where Java throws `NullPointerException`.
    fn is_new_fragment(&mut self) -> bool {
        self.position += self
            .pos_inc_att
            .as_ref()
            .expect(NOT_STARTED)
            .borrow()
            .get_position_increment();

        if self.wait_for_pos <= self.position {
            self.wait_for_pos = -1;
        } else if self.wait_for_pos != -1 {
            return false;
        }

        let term = term_text(&*self.term_att.as_ref().expect(NOT_STARTED).borrow());
        if let Some(w_span_term) = self.query_scorer.get_weighted_span_term(&term) {
            for position_span in w_span_term.get_position_spans() {
                if position_span.start == self.position {
                    self.wait_for_pos = position_span.end + 1;
                    break;
                }
            }
        }

        let end_offset = self
            .offset_att
            .as_ref()
            .expect(NOT_STARTED)
            .borrow()
            .end_offset();
        let is_new_frag = end_offset >= self.fragment_size.wrapping_mul(self.current_num_frags)
            && (self.text_size - end_offset) >= ((self.fragment_size as u32) >> 1) as i32;

        if is_new_frag {
            self.current_num_frags += 1;
        }

        is_new_frag
    }

    fn start(&mut self, original_text: &str, token_stream: &mut dyn TokenStream) -> Result<()> {
        self.position = -1;
        self.current_num_frags = 1;
        self.text_size = utf16_len(original_text);
        let source = token_stream.attribute_source_mut();
        self.term_att = Some(AttributeHandle::add(source)?);
        self.pos_inc_att = Some(AttributeHandle::add(source)?);
        self.offset_att = Some(AttributeHandle::add(source)?);
        Ok(())
    }
}