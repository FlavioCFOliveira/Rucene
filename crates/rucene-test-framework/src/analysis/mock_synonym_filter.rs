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

//! Port of `org.apache.lucene.tests.analysis.MockSynonymFilter`.

use std::collections::VecDeque;

use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use super::attributes::{char_term_id, offset_id, pos_inc_id, pos_len_id, type_id, Att};

/// The logic of [`MockSynonymFilter`].
#[derive(Debug)]
pub struct MockSynonymFilterLogic {
    term_att: Att,
    pos_inc_att: Att,
    offset_att: Att,
    pos_len_att: Att,
    type_att: Att,
    token_queue: VecDeque<AttributeSource>,
    end_of_input: bool,
}

impl MockSynonymFilterLogic {
    fn add_synonym(
        &mut self,
        source: &AttributeSource,
        synonym_text: &str,
        pos_len: i32,
        end_offset: i32,
    ) {
        self.term_att.set_term(synonym_text);
        self.pos_inc_att.pos_inc_mut().set_position_increment(0);
        self.pos_len_att.pos_len_mut().set_position_length(pos_len);
        let start = self.offset_att.offset().start_offset();
        self.offset_att.offset_mut().set_offset(start, end_offset);
        self.type_att.type_att_mut().set_type("SYNONYM".to_string());
        self.token_queue.push_back(source.clone_attributes());
    }

    fn add_synonym_and_restore_orig_token(
        &mut self,
        source: &AttributeSource,
        synonym_text: &str,
        pos_len: i32,
        end_offset: i32,
    ) -> Result<()> {
        let orig_token = source.clone_attributes();
        self.add_synonym(source, synonym_text, pos_len, end_offset);
        orig_token.copy_to(source)?;
        Ok(())
    }
}

impl TokenFilterLogic for MockSynonymFilterLogic {
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.token_queue.clear();
        self.end_of_input = false;
        Ok(())
    }

    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if let Some(token) = self.token_queue.pop_front() {
            token.copy_to(source)?;
            return Ok(true);
        }
        if !self.end_of_input && input.increment_token()? {
            let term = self.term_att.term_string();
            if term == "dogs" {
                let end = self.offset_att.offset().end_offset();
                self.add_synonym_and_restore_orig_token(source, "dog", 1, end)?;
            } else if term == "guinea" {
                let first_saved_token = source.clone_attributes();
                if input.increment_token()? {
                    let term = self.term_att.term_string();
                    if term == "pig" {
                        let second_saved_token = source.clone_attributes();
                        let second_end_offset = self.offset_att.offset().end_offset();
                        first_saved_token.copy_to(source)?;
                        self.add_synonym(source, "cavy", 2, second_end_offset);
                        self.token_queue.push_back(second_saved_token);
                    } else if term == "dogs" {
                        self.token_queue.push_back(source.clone_attributes());
                        let end = self.offset_att.offset().end_offset();
                        self.add_synonym(source, "dog", 1, end);
                    }
                } else {
                    self.end_of_input = true;
                }
                first_saved_token.copy_to(source)?;
            }
            Ok(true)
        } else {
            self.end_of_input = true;
            Ok(false)
        }
    }
}

/// adds synonym of "dog" for "dogs", and synonym of "cavy" for "guinea pig".
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockSynonymFilter`.
pub struct MockSynonymFilter;

impl MockSynonymFilter {
    /// `MockSynonymFilter(TokenStream input)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory lacks one of the
    /// attributes the filter uses.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<MockSynonymFilterLogic>> {
        let source = input.attribute_source_mut();
        let term_att = Att::add(source, char_term_id())?;
        let pos_inc_att = Att::add(source, pos_inc_id())?;
        let offset_att = Att::add(source, offset_id())?;
        let pos_len_att = Att::add(source, pos_len_id())?;
        let type_att = Att::add(source, type_id())?;
        Ok(TokenFilter::new(
            input,
            MockSynonymFilterLogic {
                term_att,
                pos_inc_att,
                offset_att,
                pos_len_att,
                type_att,
                token_queue: VecDeque::new(),
                end_of_input: false,
            },
        ))
    }
}
