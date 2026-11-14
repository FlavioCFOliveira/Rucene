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

//! `Analyzer` for English, ported from
//! `org.apache.lucene.analysis.en.EnglishAnalyzer`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::{
    new_lower_case_filter, new_standard_tokenizer, new_stop_filter, Analyzer, AnalyzerState,
    CharArraySet, CharReader, GlobalReuseStrategy, SharedTokenStream, StopwordAnalyzerBase,
    TokenStream, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;

use super::english_possessive_filter::new_english_possessive_filter;
use super::porter_stem_filter::new_porter_stem_filter;
use crate::analysis::miscellaneous::set_keyword_marker_filter::new_set_keyword_marker_filter;

/// An unmodifiable set containing some common English words that are not
/// usually useful for searching.
///
/// Equivalent to the public static field `ENGLISH_STOP_WORDS_SET` of
/// `org.apache.lucene.analysis.en.EnglishAnalyzer`, which its static
/// initializer builds as `CharArraySet.unmodifiableSet(new
/// CharArraySet(stopWords, false))`; rmp task #449 ported it ahead of the
/// class, for `StopFilterFactory`.
pub static ENGLISH_STOP_WORDS_SET: LazyLock<CharArraySet> = LazyLock::new(|| {
    let stop_words = [
        "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "into", "is",
        "it", "no", "not", "of", "on", "or", "such", "that", "the", "their", "then", "there",
        "these", "they", "this", "to", "was", "will", "with",
    ];
    let stop_set = CharArraySet::from_iter(stop_words, false);
    CharArraySet::unmodifiable_set(Some(stop_set))
});

/// `Analyzer` for English.
///
/// Equivalent to `org.apache.lucene.analysis.en.EnglishAnalyzer` (since
/// 3.1), which extends `StopwordAnalyzerBase`: the stop words live in the
/// [`StopwordAnalyzerBase`] this analyzer holds, as Java's live in the
/// inherited field.
#[derive(Debug)]
pub struct EnglishAnalyzer {
    state: AnalyzerState,
    base: StopwordAnalyzerBase,
    stem_exclusion_set: CharArraySet,
}

impl Default for EnglishAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl EnglishAnalyzer {
    /// Returns an unmodifiable instance of the default stop words set.
    ///
    /// Equivalent to `EnglishAnalyzer.getDefaultStopSet()`.
    pub fn get_default_stop_set() -> &'static CharArraySet {
        &ENGLISH_STOP_WORDS_SET
    }

    /// Builds an analyzer with the default stop words:
    /// [`get_default_stop_set`](Self::get_default_stop_set).
    ///
    /// Equivalent to `EnglishAnalyzer()`.
    pub fn new() -> Self {
        Self::with_stop_words(ENGLISH_STOP_WORDS_SET.clone())
    }

    /// Builds an analyzer with the given stop words.
    ///
    /// Equivalent to `EnglishAnalyzer(CharArraySet)`, `this(stopwords,
    /// CharArraySet.EMPTY_SET)`.
    pub fn with_stop_words(stopwords: CharArraySet) -> Self {
        Self::with_stem_exclusion_set(stopwords, &CharArraySet::empty_set())
    }

    /// Builds an analyzer with the given stop words. If a non-empty stem
    /// exclusion set is provided this analyzer will add a
    /// `SetKeywordMarkerFilter` before stemming.
    ///
    /// Equivalent to `EnglishAnalyzer(CharArraySet, CharArraySet)`: the
    /// analyzer keeps
    /// `CharArraySet.unmodifiableSet(CharArraySet.copy(stemExclusionSet))`.
    pub fn with_stem_exclusion_set(
        stopwords: CharArraySet,
        stem_exclusion_set: &CharArraySet,
    ) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            base: StopwordAnalyzerBase::new(stopwords),
            stem_exclusion_set: CharArraySet::unmodifiable_set(Some(stem_exclusion_set.copy())),
        }
    }

    /// Returns the stop-word set used by this analyzer.
    ///
    /// Equivalent to the inherited `StopwordAnalyzerBase.getStopwordSet()`.
    pub fn stopwords(&self) -> &CharArraySet {
        self.base.stopwords()
    }
}

impl Analyzer for EnglishAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.en.EnglishAnalyzer"
    }

    /// Creates a `TokenStreamComponents` which tokenizes all the text in the
    /// provided `Reader`: a `StandardTokenizer` filtered with
    /// `EnglishPossessiveFilter`, `LowerCaseFilter`, `StopFilter`,
    /// `SetKeywordMarkerFilter` if a stem exclusion set is provided and
    /// `PorterStemFilter`.
    ///
    /// Equivalent to `EnglishAnalyzer.createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let source = Arc::new(AtomicRefCell::new(new_standard_tokenizer()));
        let source_for_reader = Arc::clone(&source);
        let set_reader = Box::new(move |reader: Box<dyn CharReader>| {
            source_for_reader.borrow_mut().set_reader(reader)
        });
        let mut result: Box<dyn TokenStream> = Box::new(new_english_possessive_filter(Box::new(
            SharedTokenStream::new(source),
        ))?);
        result = Box::new(new_lower_case_filter(result)?);
        result = Box::new(new_stop_filter(result, self.base.stopwords().clone())?);
        if !self.stem_exclusion_set.is_empty() {
            result = Box::new(new_set_keyword_marker_filter(
                result,
                self.stem_exclusion_set.clone(),
            )?);
        }
        let result: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(new_porter_stem_filter(result)?));
        Ok(TokenStreamComponents::new(set_reader, result))
    }

    /// Equivalent to `EnglishAnalyzer.normalize(String, TokenStream)`, `new
    /// LowerCaseFilter(in)`.
    fn normalize_token_stream(
        &self,
        _field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        // `Analyzer.normalize` hands this a `StringTokenStream`, which holds
        // the `CharTermAttribute` already, so the filter's `addAttribute`
        // finds it and cannot fail.
        Box::new(
            new_lower_case_filter(input)
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        )
    }
}
