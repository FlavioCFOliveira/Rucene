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

//! Port of `TokenStreamOffsetStrategy.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use rucene::analysis::tokenattributes::{CharTermAttribute, OffsetAttribute};
use rucene::analysis::{Analyzer, AttributeHandle, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::index::LeafReader;
use rucene::util::automaton::automata::Automata;
use rucene::util::automaton::character_run_automaton::CharacterRunAutomaton;
use rucene::util::BytesRef;

use crate::uhighlight::analysis_offset_strategy::AnalysisOffsetStrategy;
use crate::uhighlight::char_array_matcher::{CharArrayMatcher, RunAutomatonMatcher};
use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::labelled_char_array_matcher::{self, LabelledCharArrayMatcher};
use crate::uhighlight::offsets_enum::OffsetsEnum;
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::OffsetSource;

/// Analyzes the text, producing a single `OffsetsEnum` wrapping the
/// `TokenStream` filtered to terms in the query, including wildcards. It
/// can't handle position-sensitive queries (phrases). Passage accuracy
/// suffers because the freq() is unknown -- it's always
/// `i32::MAX` instead.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.TokenStreamOffsetStrategy`.
pub struct TokenStreamOffsetStrategy {
    base: AnalysisOffsetStrategy,
    combined_automata: Arc<Vec<Arc<dyn CharArrayMatcher>>>,
}

impl fmt::Debug for TokenStreamOffsetStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenStreamOffsetStrategy")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl TokenStreamOffsetStrategy {
    /// Equivalent to `new TokenStreamOffsetStrategy(UHComponents, Analyzer)`.
    ///
    /// # Errors
    ///
    /// As [`AnalysisOffsetStrategy::new`]; `NullPointerException` when the
    /// components carry no terms or automata.
    pub fn new(components: UHComponents, index_analyzer: Arc<dyn Analyzer>) -> Result<Self> {
        debug_assert!(!components.phrase_helper().has_position_sensitivity());
        let combined_automata = Self::convert_terms_to_matchers(
            components.terms().ok_or_else(LuceneError::null_pointer)?,
            components.automata().ok_or_else(LuceneError::null_pointer)?,
        )?;
        Ok(Self {
            base: AnalysisOffsetStrategy::new(components, index_analyzer)?,
            combined_automata: Arc::new(combined_automata),
        })
    }

    // TODO this is inefficient; instead build a union automata just for
    // terms part.
    fn convert_terms_to_matchers(
        terms: &[BytesRef],
        matchers: &[Arc<dyn LabelledCharArrayMatcher>],
    ) -> Result<Vec<Arc<dyn CharArrayMatcher>>> {
        let mut new_automata: Vec<Arc<dyn CharArrayMatcher>> =
            Vec::with_capacity(terms.len() + matchers.len());
        for term in terms {
            let term_string = term.utf8_to_string()?;
            let a = CharacterRunAutomaton::new(Automata::make_string(&term_string))?;
            new_automata.push(labelled_char_array_matcher::wrap(
                &term_string,
                Arc::new(RunAutomatonMatcher { automaton: a }),
            ) as Arc<dyn CharArrayMatcher>);
        }
        // Append existing automata (that which is used for MTQs)
        for matcher in matchers {
            new_automata.push(Arc::clone(matcher) as Arc<dyn CharArrayMatcher>);
        }
        Ok(new_automata)
    }
}

impl Deref for TokenStreamOffsetStrategy {
    type Target = AnalysisOffsetStrategy;

    fn deref(&self) -> &AnalysisOffsetStrategy {
        &self.base
    }
}

impl FieldOffsetStrategy for TokenStreamOffsetStrategy {
    fn components(&self) -> Option<&UHComponents> {
        Some(&self.base.components)
    }

    fn get_offset_source(&self) -> OffsetSource {
        self.base.get_offset_source()
    }

    fn get_offsets_enum(
        &self,
        _reader: Option<&Arc<dyn LeafReader>>,
        _doc_id: i32,
        content: &str,
    ) -> Result<Box<dyn OffsetsEnum>> {
        Ok(Box::new(TokenStreamOffsetsEnum::new(
            self.base.token_stream(content)?,
            Arc::clone(&self.combined_automata),
        )?))
    }
}

/// Equivalent to the private nested class
/// `TokenStreamOffsetStrategy.TokenStreamOffsetsEnum`.
pub(crate) struct TokenStreamOffsetsEnum {
    stream: Option<Box<dyn TokenStream>>, // becomes null when closed
    matchers: Arc<Vec<Arc<dyn CharArrayMatcher>>>,
    char_term_att: AttributeHandle<dyn CharTermAttribute>,
    offset_att: AttributeHandle<dyn OffsetAttribute>,

    current_match: i32,

    match_descriptions: RefCell<Vec<Option<BytesRef>>>,
}

impl TokenStreamOffsetsEnum {
    fn new(mut ts: Box<dyn TokenStream>, matchers: Arc<Vec<Arc<dyn CharArrayMatcher>>>) -> Result<Self> {
        let match_descriptions = RefCell::new(vec![None; matchers.len()]);
        let char_term_att = AttributeHandle::add(ts.attribute_source_mut())?;
        let offset_att = AttributeHandle::add(ts.attribute_source_mut())?;
        ts.reset()?;
        Ok(Self {
            stream: Some(ts),
            matchers,
            char_term_att,
            offset_att,
            current_match: -1,
            match_descriptions,
        })
    }
}

impl OffsetsEnum for TokenStreamOffsetsEnum {
    fn next_position(&mut self) -> Result<bool> {
        if let Some(stream) = self.stream.as_mut() {
            while stream.increment_token()? {
                let units: Vec<u16> = {
                    let att = self.char_term_att.borrow();
                    let chars: String = att.buffer()[..att.length()].iter().collect();
                    chars.encode_utf16().collect()
                };
                for (i, matcher) in self.matchers.iter().enumerate() {
                    if matcher.match_chars(&units, 0, units.len()) {
                        self.current_match = i as i32;
                        return Ok(true);
                    }
                }
            }
            stream.end()?;
            self.close()?;
        }
        Ok(false)
    }

    fn freq(&self) -> Result<i32> {
        Ok(i32::MAX) // lie
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(self.offset_att.borrow().start_offset())
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(self.offset_att.borrow().end_offset())
    }

    /// # Errors
    ///
    /// `ArrayIndexOutOfBoundsException` before a match.
    fn get_term(&self) -> Result<Option<BytesRef>> {
        let index = self.current_match;
        if index < 0 || index as usize >= self.matchers.len() {
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {index} out of bounds for length {}", self.matchers.len()),
            ));
        }
        let mut descriptions = self.match_descriptions.borrow_mut();
        let description = descriptions[index as usize].get_or_insert_with(|| {
            // lazily initialize
            BytesRef::new(self.matchers[index as usize].java_to_string().into_bytes())
        });
        Ok(Some(description.clone()))
    }

    fn close(&mut self) -> Result<()> {
        if let Some(mut stream) = self.stream.take() {
            stream.close()?;
        }
        Ok(())
    }

    fn simple_name(&self) -> &'static str {
        "TokenStreamOffsetsEnum"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}