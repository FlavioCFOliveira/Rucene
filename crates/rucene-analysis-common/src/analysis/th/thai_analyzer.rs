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

//! `Analyzer` for Thai language, ported from
//! `org.apache.lucene.analysis.th.ThaiAnalyzer`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::wordlist_loader::get_word_set_stream_with_comment;
use rucene::analysis::{
    new_lower_case_filter, new_stop_filter, Analyzer, AnalyzerState, CharArraySet, CharReader,
    GlobalReuseStrategy, SharedTokenStream, StopwordAnalyzerBase, TokenStream,
    TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::IOUtils;

use super::thai_tokenizer::new_thai_tokenizer;
use crate::analysis::core::decimal_digit_filter::new_decimal_digit_filter;
use crate::resources::get_resource_as_stream;

/// File containing default Thai stopwords.
///
/// Equivalent to `ThaiAnalyzer.DEFAULT_STOPWORD_FILE`.
pub const DEFAULT_STOPWORD_FILE: &str = "stopwords.txt";

/// The comment character in the stopwords file. All lines prefixed with
/// this will be ignored.
///
/// Equivalent to `ThaiAnalyzer.STOPWORDS_COMMENT`.
const STOPWORDS_COMMENT: &str = "#";

/// Atomically loads the DEFAULT_STOP_SET in a lazy fashion once the outer
/// class accesses the static final set the first time.
///
/// Equivalent to `org.apache.lucene.analysis.th.ThaiAnalyzer.DefaultSetHolder`,
/// whose static initializer this type's [`DEFAULT_STOP_SET`] runs.
#[derive(Debug)]
pub(crate) struct DefaultSetHolder;

/// `DefaultSetHolder.DEFAULT_STOP_SET`:
/// `WordlistLoader.getWordSet(IOUtils.requireResourceNonNull(
/// ThaiAnalyzer.class.getResourceAsStream(DEFAULT_STOPWORD_FILE),
/// DEFAULT_STOPWORD_FILE), STOPWORDS_COMMENT)`.
///
/// # Panics
///
/// As Java's static initializer throws `UncheckedIOException("Unable to
/// load default stopword set")`, when the resource cannot be read; it is
/// part of the crate, as it is of Java's JAR.
static DEFAULT_STOP_SET: LazyLock<CharArraySet> = LazyLock::new(DefaultSetHolder::load);

impl DefaultSetHolder {
    /// The static initializer of `DefaultSetHolder`.
    fn load() -> CharArraySet {
        let load = || -> Result<CharArraySet> {
            let mut stream = IOUtils::require_resource_non_null(
                get_resource_as_stream(
                    "org.apache.lucene.analysis.th.ThaiAnalyzer",
                    DEFAULT_STOPWORD_FILE,
                ),
                DEFAULT_STOPWORD_FILE,
            )?;
            get_word_set_stream_with_comment(&mut stream, "UTF-8", STOPWORDS_COMMENT)
        };
        // default set should always be present as it is part of the
        // distribution (JAR)
        load().unwrap_or_else(|ex| panic!("Unable to load default stopword set: {ex}"))
    }
}

/// `Analyzer` for Thai language. It uses `java.text.BreakIterator` to break
/// words.
///
/// Equivalent to `org.apache.lucene.analysis.th.ThaiAnalyzer` (since 3.1),
/// which extends `StopwordAnalyzerBase`: the stop words live in the
/// [`StopwordAnalyzerBase`] this analyzer holds, as Java's live in the
/// inherited field.
#[derive(Debug)]
pub struct ThaiAnalyzer {
    state: AnalyzerState,
    base: StopwordAnalyzerBase,
}

impl Default for ThaiAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ThaiAnalyzer {
    /// Returns an unmodifiable instance of the default stop words set.
    ///
    /// Equivalent to `ThaiAnalyzer.getDefaultStopSet()`.
    pub fn get_default_stop_set() -> &'static CharArraySet {
        &DEFAULT_STOP_SET
    }

    /// Builds an analyzer with the default stop words.
    ///
    /// Equivalent to `ThaiAnalyzer()`.
    pub fn new() -> Self {
        Self::with_stop_words(DEFAULT_STOP_SET.clone())
    }

    /// Builds an analyzer with the given stop words.
    ///
    /// Equivalent to `ThaiAnalyzer(CharArraySet)`.
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

impl Analyzer for ThaiAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.th.ThaiAnalyzer"
    }

    /// Creates `TokenStreamComponents` used to tokenize all the text in the
    /// provided `Reader`: a `ThaiTokenizer` filtered with
    /// `LowerCaseFilter`, `DecimalDigitFilter` and `StopFilter`.
    ///
    /// Equivalent to `ThaiAnalyzer.createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let source = Arc::new(AtomicRefCell::new(new_thai_tokenizer()?));
        let source_for_reader = Arc::clone(&source);
        let set_reader = Box::new(move |reader: Box<dyn CharReader>| {
            source_for_reader.borrow_mut().set_reader(reader)
        });
        let mut result: Box<dyn TokenStream> = Box::new(new_lower_case_filter(Box::new(
            SharedTokenStream::new(source),
        ))?);
        result = Box::new(new_decimal_digit_filter(result)?);
        let result: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(
            new_stop_filter(result, self.base.stopwords().clone())?,
        ));
        Ok(TokenStreamComponents::new(set_reader, result))
    }

    /// Equivalent to `ThaiAnalyzer.normalize(String, TokenStream)`: `new
    /// DecimalDigitFilter(new LowerCaseFilter(in))`.
    fn normalize_token_stream(
        &self,
        _field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        // `Analyzer.normalize` hands this a `StringTokenStream`, which holds
        // the `CharTermAttribute` already, so the filters' `addAttribute`
        // find it and cannot fail.
        let result = new_lower_case_filter(input)
            .expect("INVARIANT: the normalization input holds the CharTermAttribute");
        Box::new(
            new_decimal_digit_filter(Box::new(result))
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        )
    }
}
