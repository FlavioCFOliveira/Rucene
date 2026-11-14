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

//! `Analyzer` for Persian, ported from
//! `org.apache.lucene.analysis.fa.PersianAnalyzer`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::wordlist_loader::get_word_set_stream_with_comment;
use rucene::analysis::{
    new_lower_case_filter, new_standard_tokenizer, new_stop_filter, Analyzer, AnalyzerState,
    CharArraySet, CharReader, GlobalReuseStrategy, SharedTokenStream, StopwordAnalyzerBase,
    TokenStream, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::IOUtils;

use super::persian_char_filter::PersianCharFilter;
use super::persian_normalization_filter::new_persian_normalization_filter;
use super::persian_stem_filter::new_persian_stem_filter;
use crate::analysis::ar::arabic_normalization_filter::new_arabic_normalization_filter;
use crate::analysis::core::decimal_digit_filter::new_decimal_digit_filter;
use crate::analysis::miscellaneous::set_keyword_marker_filter::new_set_keyword_marker_filter;
use crate::resources::get_resource_as_stream;

/// File containing default Persian stopwords.
///
/// Default stopword list is from
/// <http://members.unine.ch/jacques.savoy/clef/index.html> The stopword list is
/// BSD-Licensed.
///
/// Equivalent to `PersianAnalyzer.DEFAULT_STOPWORD_FILE`.
pub const DEFAULT_STOPWORD_FILE: &str = "stopwords.txt";

/// The comment character in the stopwords file. All lines prefixed with this
/// will be ignored
///
/// Equivalent to `PersianAnalyzer.STOPWORDS_COMMENT`.
pub const STOPWORDS_COMMENT: &str = "#";

/// Atomically loads the DEFAULT_STOP_SET in a lazy fashion once the outer class
/// accesses the static final set the first time.;
///
/// The holder of the default stop-words set, loaded once, the first time it
/// is read.
///
/// Equivalent to `org.apache.lucene.analysis.fa.PersianAnalyzer.DefaultSetHolder`,
/// whose static initializer this type's [`DEFAULT_STOP_SET`] runs.
#[derive(Debug)]
pub(crate) struct DefaultSetHolder;

/// `DefaultSetHolder.DEFAULT_STOP_SET`:
/// `WordlistLoader.getWordSet(IOUtils.requireResourceNonNull(
/// PersianAnalyzer.class.getResourceAsStream(DEFAULT_STOPWORD_FILE),
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
                    "org.apache.lucene.analysis.fa.PersianAnalyzer",
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

/// `Analyzer` for Persian.
///
/// This Analyzer uses `PersianCharFilter` which implies tokenizing around
/// zero-width non-joiner in addition to whitespace. Some persian-specific
/// variant forms (such as farsi yeh and keheh) are standardized. "Stemming" is
/// accomplished via stopwords.
///
/// Equivalent to `org.apache.lucene.analysis.fa.PersianAnalyzer` (since 3.1),
/// which extends `StopwordAnalyzerBase`: the stop words live in the
/// [`StopwordAnalyzerBase`] this analyzer holds, as Java's live in the
/// inherited field.
#[derive(Debug)]
pub struct PersianAnalyzer {
    state: AnalyzerState,
    base: StopwordAnalyzerBase,
    stem_exclusion_set: CharArraySet,
}

impl Default for PersianAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl PersianAnalyzer {
    /// Returns an unmodifiable instance of the default stop-words set.
    ///
    /// * returns: an unmodifiable instance of the default stop-words set.
    ///
    /// Equivalent to `PersianAnalyzer.getDefaultStopSet()`.
    pub fn get_default_stop_set() -> &'static CharArraySet {
        &DEFAULT_STOP_SET
    }

    /// Builds an analyzer with the default stop words: `DEFAULT_STOPWORD_FILE`.
    ///
    /// Equivalent to `PersianAnalyzer()`.
    pub fn new() -> Self {
        Self::with_stop_words(DEFAULT_STOP_SET.clone())
    }

    /// Builds an analyzer with the given stop words
    ///
    /// * `stopwords`: a stopword set
    ///
    /// Equivalent to `PersianAnalyzer(CharArraySet)`, `this(stopwords,
    /// CharArraySet.EMPTY_SET)`.
    pub fn with_stop_words(stopwords: CharArraySet) -> Self {
        Self::with_stem_exclusion_set(stopwords, &CharArraySet::empty_set())
    }

    /// Builds an analyzer with the given stop word. If a none-empty stem
    /// exclusion set is provided this analyzer will add a
    /// `SetKeywordMarkerFilter` before `PersianStemFilter`.
    ///
    /// * `stopwords`: a stopword set
    /// * `stem_exclusion_set`: a set of terms not to be stemmed
    ///
    /// Equivalent to `PersianAnalyzer(CharArraySet, CharArraySet)`: the analyzer keeps
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

impl Analyzer for PersianAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.fa.PersianAnalyzer"
    }

    /// Creates `org.apache.lucene.analysis.Analyzer.TokenStreamComponents` used
    /// to tokenize all the text in the provided `Reader`.
    ///
    /// * returns: `org.apache.lucene.analysis.Analyzer.TokenStreamComponents`
    ///   built from a `StandardTokenizer` filtered with `LowerCaseFilter`,
    ///   `DecimalDigitFilter`, `ArabicNormalizationFilter`,
    ///   `PersianNormalizationFilter`, Persian Stop words, and
    ///   `PersianStemFilter`.
    ///
    /// Equivalent to `PersianAnalyzer.createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let source = Arc::new(AtomicRefCell::new(new_standard_tokenizer()));
        let source_for_reader = Arc::clone(&source);
        let set_reader = Box::new(move |reader: Box<dyn CharReader>| {
            source_for_reader.borrow_mut().set_reader(reader)
        });
        let mut result: Box<dyn TokenStream> = Box::new(new_lower_case_filter(Box::new(
            SharedTokenStream::new(source),
        ))?);
        result = Box::new(new_decimal_digit_filter(result)?);
        result = Box::new(new_arabic_normalization_filter(result)?);
        /* additional persian-specific normalization */
        result = Box::new(new_persian_normalization_filter(result)?);
        /*
         * the order here is important: the stopword list is normalized with the
         * above!
         */
        result = Box::new(new_stop_filter(result, self.base.stopwords().clone())?);
        if !self.stem_exclusion_set.is_empty() {
            result = Box::new(new_set_keyword_marker_filter(
                result,
                self.stem_exclusion_set.clone(),
            )?);
        }
        let result: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(new_persian_stem_filter(result)?));
        Ok(TokenStreamComponents::new(set_reader, result))
    }

    /// Equivalent to `PersianAnalyzer.normalize(String, TokenStream)`.
    fn normalize_token_stream(
        &self,
        _field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        // `Analyzer.normalize` hands this a `StringTokenStream`, which holds
        // the `CharTermAttribute` already, so the filters' `addAttribute`
        // find it and cannot fail.
        let mut result: Box<dyn TokenStream> = Box::new(
            new_lower_case_filter(input)
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        );
        result = Box::new(
            new_decimal_digit_filter(result)
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        );
        result = Box::new(
            new_arabic_normalization_filter(result)
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        );
        /* additional persian-specific normalization */
        result = Box::new(
            new_persian_normalization_filter(result)
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        );
        result
    }

    /// Wraps the Reader with `PersianCharFilter`
    ///
    /// Equivalent to `PersianAnalyzer.initReader(String, Reader)`.
    fn init_reader(&self, _field_name: &str, reader: Box<dyn CharReader>) -> Box<dyn CharReader> {
        Box::new(PersianCharFilter::new(reader))
    }
}
