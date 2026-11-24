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

//! Port of `MemoryIndexOffsetStrategy.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::analysis::filtering_token_filter::{
    new_filtering_token_filter_with, FilteringTokenFilterLogic,
};
use rucene::analysis::tokenattributes::CharTermAttribute;
use rucene::analysis::{Analyzer, AttributeHandle, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::index::LeafReader;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::automata::MAX_STRING_UNION_TERM_LENGTH;
use rucene::util::BytesRef;
use rucene_memory::MemoryIndex;

use crate::uhighlight::analysis_offset_strategy::AnalysisOffsetStrategy;
use crate::uhighlight::char_array_matcher::{self, CharArrayMatcher};
use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::multi_term_highlighting::MultiTermHighlighting;
use crate::uhighlight::offsets_enum::OffsetsEnum;
use crate::uhighlight::overlay_single_doc_terms_leaf_reader::OverlaySingleDocTermsLeafReader;
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::OffsetSource;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Uses an `Analyzer` on content to get offsets and then populates a
/// `MemoryIndex`.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.MemoryIndexOffsetStrategy`.
pub struct MemoryIndexOffsetStrategy {
    base: AnalysisOffsetStrategy,
    memory_index: Mutex<MemoryIndex>,
    mem_index_leaf_reader: Arc<dyn LeafReader>,
    pre_mem_index_filter_automaton: Option<Arc<dyn CharArrayMatcher>>,
}

impl fmt::Debug for MemoryIndexOffsetStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryIndexOffsetStrategy")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// The combined matcher of `buildCombinedAutomaton`: any of the matchers.
#[derive(Debug)]
struct AnyOfMatcher {
    all_automata: Vec<Arc<dyn CharArrayMatcher>>,
}

impl CharArrayMatcher for AnyOfMatcher {
    fn match_chars(&self, chars: &[u16], offset: usize, length: usize) -> bool {
        for automaton in &self.all_automata {
            if automaton.match_chars(chars, offset, length) {
                return true;
            }
        }
        false
    }
}

impl MemoryIndexOffsetStrategy {
    /// Equivalent to `new MemoryIndexOffsetStrategy(UHComponents, Analyzer)`.
    ///
    /// # Errors
    ///
    /// As [`AnalysisOffsetStrategy::new`], and what building the automata
    /// raises.
    pub fn new(components: UHComponents, analyzer: Arc<dyn Analyzer>) -> Result<Self> {
        let store_payloads = components.phrase_helper().has_position_sensitivity(); // might be needed
        let memory_index = MemoryIndex::with_offsets_and_payloads(true, store_payloads); // true==store offsets
        let searcher = memory_index.create_searcher()?;
        let mem_index_leaf_reader = Arc::clone(searcher.get_top_reader_context())
            .as_leaf()
            .ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "the MemoryIndex reader is not a LeafReader",
                )
            })?
            .leaf_reader(); // appears to be re-usable
                            // preFilter for MemoryIndex
        let pre_mem_index_filter_automaton = Self::build_combined_automaton(&components)?;
        Ok(Self {
            base: AnalysisOffsetStrategy::new(components, analyzer)?,
            memory_index: Mutex::new(memory_index),
            mem_index_leaf_reader,
            pre_mem_index_filter_automaton,
        })
    }

    /// Build one `CharArrayMatcher` matching any term the query might match.
    fn build_combined_automaton(
        components: &UHComponents,
    ) -> Result<Option<Arc<dyn CharArrayMatcher>>> {
        // We don't know enough about the query to do this confidently
        let (Some(terms), Some(automata)) = (components.terms(), components.automata()) else {
            return Ok(None);
        };

        let mut all_automata: Vec<Arc<dyn CharArrayMatcher>> = Vec::new();
        if !terms.is_empty() {
            // Filter out any long terms that would otherwise cause
            // exceptions if we tried to build an automaton on them
            let filtered_terms: Vec<BytesRef> = terms
                .iter()
                .filter(|b| b.length < MAX_STRING_UNION_TERM_LENGTH)
                .cloned()
                .collect();
            all_automata.push(char_array_matcher::from_terms(&filtered_terms)?);
        }
        for automaton in automata {
            all_automata.push(Arc::clone(automaton) as Arc<dyn CharArrayMatcher>);
        }
        for span_query in components.phrase_helper().get_span_queries() {
            for automaton in MultiTermHighlighting::extract_automata(
                &**span_query,
                components.field_matcher(),
                true,
            ) {
                // true==lookInSpan
                all_automata.push(automaton as Arc<dyn CharArrayMatcher>);
            }
        }

        if all_automata.len() == 1 {
            return Ok(all_automata.pop());
        }
        // TODO it'd be nice if we could get at the underlying Automaton in
        // CharacterRunAutomaton so that we could union them all. But it's not
        // exposed, and sometimes the automaton is byte (not char) oriented

        // Return an aggregate CharArrayMatcher of others
        Ok(Some(Arc::new(AnyOfMatcher { all_automata })))
    }

    /// Equivalent to the private static `newKeepWordFilter(TokenStream,
    /// CharArrayMatcher)`.
    fn new_keep_word_filter(
        token_stream: Box<dyn TokenStream>,
        matcher: Arc<dyn CharArrayMatcher>,
    ) -> Result<Box<dyn TokenStream>> {
        // it'd be nice to use KeepWordFilter but it demands a CharArraySet.
        // TODO make KeepWordFilter accept a CharArrayMatcher
        Ok(Box::new(new_filtering_token_filter_with(
            token_stream,
            |source| {
                Ok(KeepWordLogic {
                    char_att: AttributeHandle::add(source)?,
                    matcher,
                })
            },
        )?))
    }
}

/// The anonymous `FilteringTokenFilter` of `newKeepWordFilter`.
#[derive(Debug)]
struct KeepWordLogic {
    char_att: AttributeHandle<dyn CharTermAttribute>,
    matcher: Arc<dyn CharArrayMatcher>,
}

impl FilteringTokenFilterLogic for KeepWordLogic {
    fn accept(&mut self, _source: &AttributeSource) -> Result<bool> {
        let att = self.char_att.borrow();
        let chars: String = att.buffer()[..att.length()].iter().collect();
        let units: Vec<u16> = chars.encode_utf16().collect();
        Ok(self.matcher.match_chars(&units, 0, units.len()))
    }
}

impl Deref for MemoryIndexOffsetStrategy {
    type Target = AnalysisOffsetStrategy;

    fn deref(&self) -> &AnalysisOffsetStrategy {
        &self.base
    }
}

impl FieldOffsetStrategy for MemoryIndexOffsetStrategy {
    fn components(&self) -> Option<&UHComponents> {
        Some(&self.base.components)
    }

    fn get_offset_source(&self) -> OffsetSource {
        self.base.get_offset_source()
    }

    fn get_offsets_enum(
        &self,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        content: &str,
    ) -> Result<Box<dyn OffsetsEnum>> {
        // note: don't need LimitTokenOffsetFilter since content is already
        // truncated to maxLength
        let mut token_stream = self.base.token_stream(content)?;

        // Filter the tokenStream to applicable terms
        if let Some(matcher) = &self.pre_mem_index_filter_automaton {
            token_stream = Self::new_keep_word_filter(token_stream, Arc::clone(matcher))?;
        }
        {
            let mut memory_index = lock(&self.memory_index);
            memory_index.reset();
            memory_index.add_field_token_stream(self.get_field()?, &mut *token_stream)?;
            // note: calls tokenStream.reset() & close()
        }

        match reader {
            None => self.create_offsets_enum_from_reader(&self.mem_index_leaf_reader, 0),
            Some(reader) => {
                let overlay: Arc<dyn LeafReader> = Arc::new(OverlaySingleDocTermsLeafReader::new(
                    Arc::clone(reader),
                    Arc::clone(&self.mem_index_leaf_reader),
                    self.get_field()?,
                    doc_id,
                )?);
                self.create_offsets_enum_from_reader(&overlay, doc_id)
            }
        }
    }
}
