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

//! Port of `org.apache.lucene.tests.analysis.MockAnalyzer`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::{
    Analyzer, AnalyzerState, PerFieldReuseStrategy, TokenStream, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::automaton::CharacterRunAutomaton;

use super::mock_fixed_length_payload_filter::MockFixedLengthPayloadFilter;
use super::mock_lower_case_filter::MockLowerCaseFilter;
use super::mock_token_filter::{CustomTermFreq, MockTokenFilter, EMPTY_STOPSET};
use super::mock_tokenizer::{MockTokenizer, DEFAULT_MAX_TOKEN_LENGTH, WHITESPACE};
use super::mock_variable_length_payload_filter::MockVariableLengthPayloadFilter;
use super::{components, shared};
use crate::util::lucene_test_case::VERBOSE;
use crate::util::LuceneTestCase;

/// Analyzer for testing.
///
/// This analyzer is a replacement for Whitespace/Simple/KeywordAnalyzers for
/// unit tests. If you are testing a custom component such as a queryparser or
/// analyzer-wrapper that consumes analysis streams, it's a great idea to test
/// it with this analyzer instead. MockAnalyzer has the following behavior:
///
/// * By default, the assertions in [`MockTokenizer`] are turned on for extra
///   checks that the consumer is consuming properly. These checks can be
///   disabled with [`set_enable_checks`](Self::set_enable_checks).
/// * Payload data is randomly injected into the stream for more thorough
///   testing of payloads.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockAnalyzer`.
pub struct MockAnalyzer {
    state: AnalyzerState,
    run_automaton: Arc<CharacterRunAutomaton>,
    lower_case: bool,
    filter: Arc<CharacterRunAutomaton>,
    custom_term_freq: Option<CustomTermFreq>,
    position_increment_gap: AtomicI32,
    offset_gap: Mutex<Option<i32>>,
    random: Arc<dyn Random>,
    previous_mappings: Mutex<HashMap<String, i32>>,
    enable_checks: AtomicBool,
    max_token_length: AtomicI32,
}

impl std::fmt::Debug for MockAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockAnalyzer")
            .field("lower_case", &self.lower_case)
            .finish_non_exhaustive()
    }
}

impl MockAnalyzer {
    /// Creates a new MockAnalyzer.
    ///
    /// * `random` — Random for payloads behavior
    /// * `run_automaton` — DFA describing how tokenization should happen
    ///   (e.g. `[a-zA-Z]+`)
    /// * `lower_case` — true if the tokenizer should lowercase terms
    /// * `filter` — DFA describing how terms should be filtered (set of
    ///   stopwords, etc)
    /// * `custom_term_freq` — optional mapping of a term to its frequency
    pub fn with_custom_term_freq(
        random: &dyn Random,
        run_automaton: Arc<CharacterRunAutomaton>,
        lower_case: bool,
        filter: Arc<CharacterRunAutomaton>,
        custom_term_freq: Option<CustomTermFreq>,
    ) -> MockAnalyzer {
        MockAnalyzer {
            state: AnalyzerState::new(Box::new(PerFieldReuseStrategy)),
            // TODO: this should be solved in a different way; Random should not be shared (!).
            random: Arc::new(JdkRandom::new(random.next_long())),
            run_automaton,
            lower_case,
            filter,
            custom_term_freq,
            position_increment_gap: AtomicI32::new(0),
            offset_gap: Mutex::new(None),
            previous_mappings: Mutex::new(HashMap::new()),
            enable_checks: AtomicBool::new(true),
            max_token_length: AtomicI32::new(DEFAULT_MAX_TOKEN_LENGTH),
        }
    }

    /// `MockAnalyzer(random, runAutomaton, lowerCase, filter)`.
    pub fn with_filter(
        random: &dyn Random,
        run_automaton: Arc<CharacterRunAutomaton>,
        lower_case: bool,
        filter: Arc<CharacterRunAutomaton>,
    ) -> MockAnalyzer {
        Self::with_custom_term_freq(random, run_automaton, lower_case, filter, None)
    }

    /// `MockAnalyzer(random, runAutomaton, lowerCase)`: no stopwords.
    pub fn with_automaton(
        random: &dyn Random,
        run_automaton: Arc<CharacterRunAutomaton>,
        lower_case: bool,
    ) -> MockAnalyzer {
        Self::with_filter(
            random,
            run_automaton,
            lower_case,
            Arc::clone(&EMPTY_STOPSET),
        )
    }

    /// Create a Whitespace-lowercasing analyzer with no stopwords removal and
    /// custom term frequencies.
    pub fn with_term_freq(random: &dyn Random, custom_term_freq: CustomTermFreq) -> MockAnalyzer {
        Self::with_custom_term_freq(
            random,
            Arc::clone(&WHITESPACE),
            true,
            Arc::clone(&EMPTY_STOPSET),
            Some(custom_term_freq),
        )
    }

    /// Create a Whitespace-lowercasing analyzer with no stopwords removal.
    pub fn new(random: &dyn Random) -> MockAnalyzer {
        Self::with_custom_term_freq(
            random,
            Arc::clone(&WHITESPACE),
            true,
            Arc::clone(&EMPTY_STOPSET),
            None,
        )
    }

    /// `maybePayload(TokenFilter, String)`: the payload filter this field is
    /// assigned on its first use, kept for every later use.
    fn maybe_payload(
        &self,
        stream: Box<dyn TokenStream>,
        field_name: &str,
    ) -> Result<Box<dyn TokenStream>> {
        let val = {
            let mut previous_mappings = self
                .previous_mappings
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match previous_mappings.get(field_name) {
                Some(val) => *val,
                None => {
                    let mut val = -1; // no payloads
                    if LuceneTestCase::rarely_with(&*self.random) {
                        match self.random.next_int_bound(3) {
                            0 => val = -1,                             // no payloads
                            1 => val = i32::MAX,                       // variable length payload
                            2 => val = self.random.next_int_bound(12), // fixed length payload
                            _ => unreachable!(),
                        }
                    }
                    if *VERBOSE {
                        if val == i32::MAX {
                            println!(
                                "MockAnalyzer: field={} gets variable length payloads",
                                field_name
                            );
                        } else if val != -1 {
                            println!(
                                "MockAnalyzer: field={} gets fixed length={} payloads",
                                field_name, val
                            );
                        }
                    }
                    previous_mappings.insert(field_name.to_string(), val); // save it so we are consistent for this field
                    val
                }
            }
        };
        Ok(if val == -1 {
            stream
        } else if val == i32::MAX {
            Box::new(MockVariableLengthPayloadFilter::new(
                Arc::clone(&self.random),
                stream,
            )?)
        } else {
            Box::new(MockFixedLengthPayloadFilter::new(
                Arc::clone(&self.random),
                stream,
                val,
            )?)
        })
    }

    /// `setPositionIncrementGap(int)`.
    pub fn set_position_increment_gap(&self, position_increment_gap: i32) {
        self.position_increment_gap
            .store(position_increment_gap, Ordering::Relaxed);
    }

    /// Set a new offset gap which will then be added to the offset when
    /// several fields with the same name are indexed.
    pub fn set_offset_gap(&self, offset_gap: i32) {
        *self.offset_gap.lock().unwrap_or_else(|e| e.into_inner()) = Some(offset_gap);
    }

    /// Toggle consumer workflow checking: if your test consumes tokenstreams
    /// normally you should leave this enabled.
    pub fn set_enable_checks(&self, enable_checks: bool) {
        self.enable_checks.store(enable_checks, Ordering::Relaxed);
    }

    /// Toggle maxTokenLength for MockTokenizer.
    pub fn set_max_token_length(&self, length: i32) {
        self.max_token_length.store(length, Ordering::Relaxed);
    }
}

impl Analyzer for MockAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.analysis.MockAnalyzer"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let mut tokenizer = MockTokenizer::with_max_token_length(
            Arc::clone(&self.run_automaton),
            self.lower_case,
            self.max_token_length.load(Ordering::Relaxed),
        )?;
        tokenizer.set_enable_checks(self.enable_checks.load(Ordering::Relaxed));
        let tokenizer = Rc::new(RefCell::new(tokenizer));
        let filt = MockTokenFilter::with_custom_term_freq(
            shared(&tokenizer),
            Arc::clone(&self.filter),
            self.custom_term_freq.clone(),
        )?;
        let sink = self.maybe_payload(Box::new(filt), field_name)?;
        let sink: Rc<RefCell<dyn TokenStream>> = Rc::new(RefCell::new(BoxedStream(sink)));
        Ok(components(tokenizer, sink))
    }

    fn normalize_token_stream(
        &self,
        _field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        if self.lower_case {
            match MockLowerCaseFilter::new(input) {
                Ok(filter) => Box::new(filter),
                Err(e) => panic!("{}", e),
            }
        } else {
            input
        }
    }

    fn get_position_increment_gap(&self, _field_name: &str) -> i32 {
        self.position_increment_gap.load(Ordering::Relaxed)
    }

    /// Get the offset gap between tokens in fields if several fields with the
    /// same name were added.
    fn get_offset_gap(&self, _field_name: &str) -> i32 {
        self.offset_gap
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unwrap_or(1)
    }
}

/// A boxed stream as a sized [`TokenStream`], to share it behind an `Rc`.
#[derive(Debug)]
pub struct BoxedStream(pub Box<dyn TokenStream>);

impl TokenStream for BoxedStream {
    fn increment_token(&mut self) -> Result<bool> {
        self.0.increment_token()
    }
    fn end(&mut self) -> Result<()> {
        self.0.end()
    }
    fn reset(&mut self) -> Result<()> {
        self.0.reset()
    }
    fn close(&mut self) -> Result<()> {
        self.0.close()
    }
    fn attribute_source(&self) -> &rucene::util::attribute::AttributeSource {
        self.0.attribute_source()
    }
    fn attribute_source_mut(&mut self) -> &mut rucene::util::attribute::AttributeSource {
        self.0.attribute_source_mut()
    }
    fn as_unwrappable(&self) -> Option<&dyn rucene::util::attribute::Unwrappable<dyn TokenStream>> {
        self.0.as_unwrappable()
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.as_any()
    }
}
