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

//! Port of `org.apache.lucene.tests.analysis.MockTokenFilter`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::{TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;
use rucene::util::automaton::{Automata, CharacterRunAutomaton, Operations};

use super::attributes::{char_term_id, pos_inc_id, term_freq_id, Att};

/// A function mapping a term to its custom term frequency.
pub type CustomTermFreq = Arc<dyn Fn(&str) -> i32 + Send + Sync>;

/// Empty set of stopwords.
///
/// Equivalent to `MockTokenFilter.EMPTY_STOPSET`.
pub static EMPTY_STOPSET: LazyLock<Arc<CharacterRunAutomaton>> = LazyLock::new(|| {
    Arc::new(
        CharacterRunAutomaton::new(Automata::make_empty())
            .expect("INVARIANT: the empty automaton is deterministic"),
    )
});

/// Set of common english stopwords.
///
/// Equivalent to `MockTokenFilter.ENGLISH_STOPSET`.
pub static ENGLISH_STOPSET: LazyLock<Arc<CharacterRunAutomaton>> = LazyLock::new(|| {
    let words = [
        "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "into", "is",
        "it", "no", "not", "of", "on", "or", "such", "that", "the", "their", "then", "there",
        "these", "they", "this", "to", "was", "will", "with",
    ];
    let strings: Vec<_> = words.iter().map(|w| Automata::make_string(w)).collect();
    let a = Operations::determinize(&Operations::union(&strings), DEFAULT_DETERMINIZE_WORK_LIMIT)
        .expect("INVARIANT: the stop set determinizes");
    Arc::new(CharacterRunAutomaton::new(a).expect("INVARIANT: a deterministic automaton"))
});

/// The logic of [`MockTokenFilter`].
pub struct MockTokenFilterLogic {
    filter: Arc<CharacterRunAutomaton>,
    term_att: Att,
    pos_incr_att: Option<Att>,
    term_freq_att: Option<Att>,
    custom_term_freq: Option<CustomTermFreq>,
    skipped_positions: i32,
}

impl std::fmt::Debug for MockTokenFilterLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockTokenFilter")
            .field("skipped_positions", &self.skipped_positions)
            .finish_non_exhaustive()
    }
}

impl TokenFilterLogic for MockTokenFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        // TODO: fix me when posInc=false, to work like FilteringTokenFilter in that case and not return
        // initial token with posInc=0 ever

        // return the first non-stop word found
        self.skipped_positions = 0;
        while input.increment_token()? {
            let accepted = {
                let term = self.term_att.char_term();
                self.filter.run_chars(term.buffer(), 0, term.length())
            };
            if !accepted {
                if let (Some(att), Some(f)) = (&self.term_freq_att, &self.custom_term_freq) {
                    let freq = f(&self.term_att.term_string());
                    att.term_freq_mut().set_term_frequency(freq);
                }
                if let Some(att) = &self.pos_incr_att {
                    let inc = att.pos_inc().get_position_increment();
                    att.pos_inc_mut()
                        .set_position_increment(inc + self.skipped_positions);
                }
                return Ok(true);
            }
            if let Some(att) = &self.pos_incr_att {
                self.skipped_positions += att.pos_inc().get_position_increment();
            }
        }
        // reached EOS -- return false
        Ok(false)
    }

    fn end(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()?;
        if let Some(att) = &self.pos_incr_att {
            let inc = att.pos_inc().get_position_increment();
            att.pos_inc_mut()
                .set_position_increment(inc + self.skipped_positions);
        }
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.skipped_positions = 0;
        Ok(())
    }
}

/// A tokenfilter for testing that removes terms accepted by a DFA.
///
/// * Union a list of singletons to act like a stopfilter.
/// * Use the complement to act like a keepwordfilter
/// * Use a regex like `.{12,}` to act like a lengthfilter
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockTokenFilter`.
pub struct MockTokenFilter;

impl MockTokenFilter {
    /// Create a new MockTokenFilter.
    ///
    /// `filter` is the DFA representing the terms that should be removed.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory provides no
    /// `CharTermAttribute` or `PositionIncrementAttribute`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        input: Box<dyn TokenStream>,
        filter: Arc<CharacterRunAutomaton>,
    ) -> Result<TokenFilter<MockTokenFilterLogic>> {
        Self::with_custom_term_freq(input, filter, None)
    }

    /// Create a new MockTokenFilter that will generate custom term
    /// frequencies (`customTermFreq`, optional, maps a term to its score).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the stream's factory provides no
    /// `CharTermAttribute`, `PositionIncrementAttribute` or
    /// `TermFrequencyAttribute`.
    pub fn with_custom_term_freq(
        mut input: Box<dyn TokenStream>,
        filter: Arc<CharacterRunAutomaton>,
        custom_term_freq: Option<CustomTermFreq>,
    ) -> Result<TokenFilter<MockTokenFilterLogic>> {
        let source = input.attribute_source_mut();
        let term_att = Att::add(source, char_term_id())?;
        let (pos_incr_att, term_freq_att) = if custom_term_freq.is_some() {
            (None, Some(Att::add(source, term_freq_id())?))
        } else {
            (Some(Att::add(source, pos_inc_id())?), None)
        };
        Ok(TokenFilter::new(
            input,
            MockTokenFilterLogic {
                filter,
                term_att,
                pos_incr_att,
                term_freq_att,
                custom_term_freq,
                skipped_positions: 0,
            },
        ))
    }
}
