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

//! An analyzer that filters `LetterTokenizer` with `LowerCaseFilter` and
//! `StopFilter`, ported from `org.apache.lucene.analysis.core.StopAnalyzer`.

use std::path::Path;
use std::sync::Arc;

use rucene::analysis::{
    Analyzer, AnalyzerState, CharArraySet, CharReader, GlobalReuseStrategy, SharedTokenStream,
    StopwordAnalyzerBase, TokenStream, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;

use super::letter_tokenizer::new_letter_tokenizer;
use super::lower_case_filter::LowerCaseFilter;
use super::stop_filter::StopFilter;

/// Filters `LetterTokenizer` with `LowerCaseFilter` and `StopFilter`.
///
/// Equivalent to `org.apache.lucene.analysis.core.StopAnalyzer` (since 3.1),
/// which extends `StopwordAnalyzerBase`: the stop words live in the
/// [`StopwordAnalyzerBase`] this analyzer holds, as Java's live in the
/// inherited field.
#[derive(Debug)]
pub struct StopAnalyzer {
    state: AnalyzerState,
    base: StopwordAnalyzerBase,
}

impl StopAnalyzer {
    /// Builds an analyzer with the stop words from the given set.
    ///
    /// Equivalent to `StopAnalyzer(CharArraySet)`, `super(stopWords)`:
    /// [`StopwordAnalyzerBase::new`] keeps an unmodifiable copy of the set.
    pub fn new(stop_words: CharArraySet) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            base: StopwordAnalyzerBase::new(stop_words),
        }
    }

    /// Builds an analyzer with the stop words from the given path, read with
    /// `WordlistLoader.getWordSet(Reader)`.
    ///
    /// Equivalent to `StopAnalyzer(Path)`, `this(loadStopwordSet(stopwordsFile))`.
    ///
    /// # Errors
    ///
    /// What [`StopwordAnalyzerBase::load_stopword_set_from_path`] returns.
    pub fn from_path(stopwords_file: &Path) -> Result<Self> {
        Ok(Self::new(
            StopwordAnalyzerBase::load_stopword_set_from_path(stopwords_file)?,
        ))
    }

    /// Builds an analyzer with the stop words from the given reader, read
    /// with `WordlistLoader.getWordSet(Reader)`; the reader is closed.
    ///
    /// Equivalent to `StopAnalyzer(Reader)`, `this(loadStopwordSet(stopwords))`.
    ///
    /// # Errors
    ///
    /// What [`StopwordAnalyzerBase::load_stopword_set`] returns.
    pub fn from_reader(stopwords: &mut dyn CharReader) -> Result<Self> {
        Ok(Self::new(StopwordAnalyzerBase::load_stopword_set(
            stopwords,
        )?))
    }

    /// Returns the stop-word set used by this analyzer.
    ///
    /// Equivalent to the inherited `StopwordAnalyzerBase.getStopwordSet()`.
    pub fn stopwords(&self) -> &CharArraySet {
        self.base.stopwords()
    }
}

impl Analyzer for StopAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.StopAnalyzer"
    }

    /// Creates `TokenStreamComponents` used to tokenize all the text in the
    /// provided `Reader`: a `LetterTokenizer` filtered with
    /// `LowerCaseFilter` and `StopFilter`.
    ///
    /// Equivalent to `StopAnalyzer.createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(new_letter_tokenizer()));
        let tokenizer_for_source = Arc::clone(&tokenizer);
        let source = Box::new(move |reader: Box<dyn CharReader>| {
            tokenizer_for_source.borrow_mut().set_reader(reader)
        });
        let lower = LowerCaseFilter::new(Box::new(SharedTokenStream::new(tokenizer)))?;
        let stop = StopFilter::new(Box::new(lower), self.base.stopwords().clone())?;
        let sink: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(stop));
        Ok(TokenStreamComponents::new(source, sink))
    }

    /// Equivalent to `StopAnalyzer.normalize(String, TokenStream)`, `new
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
            LowerCaseFilter::new(input)
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        )
    }
}
