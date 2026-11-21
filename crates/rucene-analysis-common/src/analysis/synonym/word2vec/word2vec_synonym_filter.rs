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

//! Applies single-token synonyms from a Word2Vec trained network to an
//! incoming `TokenStream`, ported from
//! `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymFilter`.

use std::collections::VecDeque;
use std::sync::Arc;

use rucene::analysis::{
    CharTermAttribute, PositionIncrementAttribute, PositionLengthAttribute, TokenFilter,
    TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene::util::BytesRefBuilder;

use super::term_and_boost::TermAndBoost;
use super::word2vec_synonym_provider::Word2VecSynonymProvider;
use crate::analysis::synonym::synonym_graph_filter::SynonymGraphFilterLogic;

/// The body of [`Word2VecSynonymFilter`]: what Lucene's
/// `Word2VecSynonymFilter` adds to `TokenFilter`.
#[derive(Debug)]
pub struct Word2VecSynonymFilterLogic {
    synonym_provider: Arc<Word2VecSynonymProvider>,
    max_synonyms_per_term: i32,
    min_accepted_similarity: f32,
    synonym_buffer: VecDeque<TermAndBoost>,
    last_state: Option<CapturedState>,
}

impl TokenFilterLogic for Word2VecSynonymFilterLogic {
    /// Equivalent to `Word2VecSynonymFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if let Some(synonym) = self.synonym_buffer.pop_front() {
            source.clear_attributes();
            if let Some(last_state) = &self.last_state {
                source.restore_state(last_state)?;
            }
            let term = synonym.term().utf8_to_string()?;
            {
                let mut term_att = source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                term_att.set_empty();
                term_att.append_string(Some(&term));
            }
            source
                .get_attribute_interface_mut::<dyn TypeAttribute>()
                .expect("INVARIANT: the constructor added the TypeAttribute")
                .set_type(SynonymGraphFilterLogic::TYPE_SYNONYM.to_string());
            source
                .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
                .expect("INVARIANT: the constructor added the PositionLengthAttribute")
                .set_position_length(1);
            source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
                .set_position_increment(0);
            return Ok(true);
        }

        if input.increment_token()? {
            let mut bytes_ref_builder = BytesRefBuilder::new();
            {
                let term_att = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                bytes_ref_builder.copy_char_array(&term_att.buffer()[..term_att.length()]);
            }
            let term = bytes_ref_builder.get();
            let synonyms = self.synonym_provider.get_synonyms(
                &term,
                self.max_synonyms_per_term,
                self.min_accepted_similarity,
            )?;
            if !synonyms.is_empty() {
                self.last_state = source.capture_state();
                self.synonym_buffer.extend(synonyms);
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Equivalent to `Word2VecSynonymFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.synonym_buffer.clear();
        Ok(())
    }
}

/// Applies single-token synonyms from a Word2Vec trained network to an
/// incoming `TokenStream`.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymFilter`
/// (`@lucene.experimental`).
pub type Word2VecSynonymFilter = TokenFilter<Word2VecSynonymFilterLogic>;

/// Apply previously built synonymProvider to incoming tokens: `input` input
/// tokenstream, `synonym_provider` synonym provider,
/// `max_synonyms_per_term` maximum number of result returned by the
/// synonym search, `min_accepted_similarity` minimal value of cosine
/// similarity between the searched vector and the retrieved ones.
///
/// Equivalent to `Word2VecSynonymFilter(TokenStream, Word2VecSynonymProvider,
/// int, float)`, whose fields add the `CharTermAttribute`, the
/// `PositionIncrementAttribute`, the `PositionLengthAttribute` and the
/// `TypeAttribute`, in that order. Java's `IllegalArgumentException("The
/// SynonymProvider must be non-null")` has no counterpart, as the provider
/// cannot be null.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_word2vec_synonym_filter(
    input: Box<dyn TokenStream>,
    synonym_provider: Arc<Word2VecSynonymProvider>,
    max_synonyms_per_term: i32,
    min_accepted_similarity: f32,
) -> Result<Word2VecSynonymFilter> {
    let mut filter = TokenFilter::new(
        input,
        Word2VecSynonymFilterLogic {
            synonym_provider,
            max_synonyms_per_term,
            min_accepted_similarity,
            synonym_buffer: VecDeque::new(),
            last_state: None,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}
