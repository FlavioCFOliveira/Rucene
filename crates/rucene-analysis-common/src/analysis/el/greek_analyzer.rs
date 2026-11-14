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

//! `Analyzer` for the Greek language, ported from
//! `org.apache.lucene.analysis.el.GreekAnalyzer`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::wordlist_loader::get_word_set_stream_with_comment;
use rucene::analysis::{
    new_standard_tokenizer, new_stop_filter, Analyzer, AnalyzerState, CharArraySet, CharReader,
    GlobalReuseStrategy, SharedTokenStream, StopwordAnalyzerBase, TokenStream,
    TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::IOUtils;

use super::greek_lower_case_filter::new_greek_lower_case_filter;
use super::greek_stem_filter::new_greek_stem_filter;
use crate::resources::get_resource_as_stream;

/// File containing default Greek stopwords.
///
/// Equivalent to `GreekAnalyzer.DEFAULT_STOPWORD_FILE`.
pub const DEFAULT_STOPWORD_FILE: &str = "stopwords.txt";

/// The holder of the default stop-words set, loaded once, the first time it
/// is read.
///
/// Equivalent to `org.apache.lucene.analysis.el.GreekAnalyzer.DefaultSetHolder`,
/// whose static initializer this type's [`DEFAULT_SET`] runs.
#[derive(Debug)]
pub(crate) struct DefaultSetHolder;

/// `DefaultSetHolder.DEFAULT_SET`:
/// `WordlistLoader.getWordSet(IOUtils.requireResourceNonNull(
/// GreekAnalyzer.class.getResourceAsStream(DEFAULT_STOPWORD_FILE),
/// DEFAULT_STOPWORD_FILE), "#")`.
///
/// # Panics
///
/// As Java's static initializer throws `UncheckedIOException("Unable to
/// load default stopword set")`, when the resource cannot be read; it is
/// part of the crate, as it is of Java's JAR.
static DEFAULT_SET: LazyLock<CharArraySet> = LazyLock::new(DefaultSetHolder::load);

impl DefaultSetHolder {
    /// The static initializer of `DefaultSetHolder`.
    fn load() -> CharArraySet {
        let load = || -> Result<CharArraySet> {
            let mut stream = IOUtils::require_resource_non_null(
                get_resource_as_stream(
                    "org.apache.lucene.analysis.el.GreekAnalyzer",
                    DEFAULT_STOPWORD_FILE,
                ),
                DEFAULT_STOPWORD_FILE,
            )?;
            get_word_set_stream_with_comment(&mut stream, "UTF-8", "#")
        };
        // default set should always be present as it is part of the
        // distribution (JAR)
        load().unwrap_or_else(|ex| panic!("Unable to load default stopword set: {ex}"))
    }
}

/// `Analyzer` for the Greek language.
///
/// Supports an external list of stopwords (words that will not be indexed
/// at all). A default set of stopwords is used unless an alternative list is
/// specified.
///
/// **NOTE**: This class uses the same `org.apache.lucene.util.Version`
/// dependent settings as `StandardAnalyzer`.
///
/// Equivalent to `org.apache.lucene.analysis.el.GreekAnalyzer` (since 3.1),
/// which extends `StopwordAnalyzerBase`: the stop words live in the
/// [`StopwordAnalyzerBase`] this analyzer holds, as Java's live in the
/// inherited field.
#[derive(Debug)]
pub struct GreekAnalyzer {
    state: AnalyzerState,
    base: StopwordAnalyzerBase,
}

impl Default for GreekAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl GreekAnalyzer {
    /// Returns a set of default Greek-stopwords
    ///
    /// * returns: a set of default Greek-stopwords
    ///
    /// Equivalent to `GreekAnalyzer.getDefaultStopSet()`.
    pub fn get_default_stop_set() -> &'static CharArraySet {
        &DEFAULT_SET
    }

    /// Builds an analyzer with the default stop words.
    ///
    /// Equivalent to `GreekAnalyzer()`.
    pub fn new() -> Self {
        Self::with_stop_words(DEFAULT_SET.clone())
    }

    /// Builds an analyzer with the given stop words.
    ///
    /// **NOTE:** The stopwords set should be pre-processed with the logic of
    /// `GreekLowerCaseFilter` for best results.
    ///
    /// * `stopwords`: a stopword set
    ///
    /// Equivalent to `GreekAnalyzer(CharArraySet)`.
    pub fn with_stop_words(stopwords: CharArraySet) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            base: StopwordAnalyzerBase::new(stopwords),
        }
    }

    /// Returns the stop-word set used by this analyzer.
    ///
    /// Equivalent to the inherited `StopwordAnalyzerBase.getStopwordSet()`.
    pub fn stopwords(&self) -> &CharArraySet {
        self.base.stopwords()
    }
}

impl Analyzer for GreekAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.el.GreekAnalyzer"
    }

    /// Creates `org.apache.lucene.analysis.Analyzer.TokenStreamComponents`
    /// used to tokenize all the text in the provided `Reader`.
    ///
    /// * returns: `org.apache.lucene.analysis.Analyzer.TokenStreamComponents`
    ///   built from a `StandardTokenizer` filtered with
    ///   `GreekLowerCaseFilter`, `StopFilter`, and `GreekStemFilter`
    ///
    /// Equivalent to `GreekAnalyzer.createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let source = Arc::new(AtomicRefCell::new(new_standard_tokenizer()));
        let source_for_reader = Arc::clone(&source);
        let set_reader = Box::new(move |reader: Box<dyn CharReader>| {
            source_for_reader.borrow_mut().set_reader(reader)
        });
        let mut result: Box<dyn TokenStream> = Box::new(new_greek_lower_case_filter(Box::new(
            SharedTokenStream::new(source),
        ))?);
        result = Box::new(new_stop_filter(result, self.base.stopwords().clone())?);
        let result: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(new_greek_stem_filter(result)?));
        Ok(TokenStreamComponents::new(set_reader, result))
    }

    /// Equivalent to `GreekAnalyzer.normalize(String, TokenStream)`, `new
    /// GreekLowerCaseFilter(in)`.
    fn normalize_token_stream(
        &self,
        _field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        // `Analyzer.normalize` hands this a `StringTokenStream`, which holds
        // the `CharTermAttribute` already, so the filter's `addAttribute`
        // finds it and cannot fail.
        Box::new(
            new_greek_lower_case_filter(input)
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        )
    }
}
