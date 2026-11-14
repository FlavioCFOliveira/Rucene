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

//! `Analyzer` for Dutch language, ported from
//! `org.apache.lucene.analysis.nl.DutchAnalyzer`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::wordlist_loader::get_snowball_word_set_stream;
use rucene::analysis::{
    new_lower_case_filter, new_standard_tokenizer, new_stop_filter, Analyzer, AnalyzerState,
    CharArrayMap, CharArraySet, CharReader, GlobalReuseStrategy, SharedTokenStream, TokenStream,
    TokenStreamComponents,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::IOUtils;

use crate::analysis::miscellaneous::set_keyword_marker_filter::new_set_keyword_marker_filter;
use crate::analysis::miscellaneous::stemmer_override_filter::{
    new_stemmer_override_filter, Builder, StemmerOverrideMap,
};
use crate::analysis::snowball::snowball_filter::new_snowball_filter;
use crate::jdk::string::to_rust_string;
use crate::resources::get_resource_as_stream;
use crate::tartarus::snowball::ext::dutch_stemmer::DutchStemmer;

/// File containing default Dutch stopwords.
///
/// Equivalent to `DutchAnalyzer.DEFAULT_STOPWORD_FILE`.
pub const DEFAULT_STOPWORD_FILE: &str = "dutch_stop.txt";

/// The holder of the default stop-words set and stem dictionary, loaded
/// once, the first time either is read.
///
/// Equivalent to `org.apache.lucene.analysis.nl.DutchAnalyzer.DefaultSetHolder`,
/// whose static initializer this type's [`DEFAULT_STOP_SET`] and
/// [`DEFAULT_STEM_DICT`] run.
#[derive(Debug)]
pub(crate) struct DefaultSetHolder;

/// `DefaultSetHolder.DEFAULT_STOP_SET`:
/// `WordlistLoader.getSnowballWordSet(IOUtils.requireResourceNonNull(
/// SnowballFilter.class.getResourceAsStream(DEFAULT_STOPWORD_FILE),
/// DEFAULT_STOPWORD_FILE))`.
///
/// # Panics
///
/// As Java's static initializer throws `UncheckedIOException("Unable to
/// load default stopword set")`, when the resource cannot be read; it is
/// part of the crate, as it is of Java's JAR.
static DEFAULT_STOP_SET: LazyLock<CharArraySet> = LazyLock::new(DefaultSetHolder::load_stop_set);

/// `DefaultSetHolder.DEFAULT_STEM_DICT`: a `CharArrayMap<String>(4,
/// false)` holding the four entries the static initializer puts.
static DEFAULT_STEM_DICT: LazyLock<CharArrayMap<String>> =
    LazyLock::new(DefaultSetHolder::load_stem_dict);

impl DefaultSetHolder {
    /// The first part of the static initializer of `DefaultSetHolder`.
    fn load_stop_set() -> CharArraySet {
        let load = || -> Result<CharArraySet> {
            let mut stream = IOUtils::require_resource_non_null(
                get_resource_as_stream(
                    "org.apache.lucene.analysis.snowball.SnowballFilter",
                    DEFAULT_STOPWORD_FILE,
                ),
                DEFAULT_STOPWORD_FILE,
            )?;
            get_snowball_word_set_stream(&mut stream, "UTF-8")
        };
        // default set should always be present as it is part of the
        // distribution (JAR)
        load().unwrap_or_else(|ex| panic!("Unable to load default stopword set: {ex}"))
    }

    /// The second part of the static initializer of `DefaultSetHolder`.
    fn load_stem_dict() -> CharArrayMap<String> {
        let mut dict = CharArrayMap::new(4, false);
        dict.put("fiets", "fiets".to_string()); // otherwise fiet
        dict.put("bromfiets", "bromfiets".to_string()); // otherwise bromfiet
        dict.put("ei", "eier".to_string());
        dict.put("kind", "kinder".to_string());
        dict
    }
}

/// `Analyzer` for Dutch language.
///
/// Supports an external list of stopwords (words that will not be indexed
/// at all), an external list of exclusions (word that will not be stemmed,
/// but indexed) and an external list of word-stem pairs that overrule the
/// algorithm (dictionary stemming). A default set of stopwords is used
/// unless an alternative list is specified, but the exclusion list is empty
/// by default.
///
/// Equivalent to `org.apache.lucene.analysis.nl.DutchAnalyzer` (since 3.1),
/// which extends `Analyzer` ("TODO: extend StopwordAnalyzerBase").
#[derive(Debug)]
pub struct DutchAnalyzer {
    state: AnalyzerState,
    /// Contains the stopwords used with the StopFilter.
    stoptable: CharArraySet,
    /// Contains words that should be indexed but not stemmed.
    excltable: CharArraySet,
    /// Java's `stemdict`, null (`None`) for an empty override dictionary.
    stemdict: Option<Arc<StemmerOverrideMap>>,
}

impl Default for DutchAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl DutchAnalyzer {
    /// Returns an unmodifiable instance of the default stop-words set.
    ///
    /// * returns: an unmodifiable instance of the default stop-words set.
    ///
    /// Equivalent to `DutchAnalyzer.getDefaultStopSet()`.
    pub fn get_default_stop_set() -> &'static CharArraySet {
        &DEFAULT_STOP_SET
    }

    /// Builds an analyzer with the default stop words
    /// ([`get_default_stop_set`](Self::get_default_stop_set)) and a few
    /// default entries for the stem exclusion table.
    ///
    /// Equivalent to `DutchAnalyzer()`.
    pub fn new() -> Self {
        Self::with_stem_exclusion_table(DEFAULT_STOP_SET.clone(), &CharArraySet::empty_set())
    }

    /// Equivalent to `DutchAnalyzer(CharArraySet)`, `this(stopwords,
    /// CharArraySet.EMPTY_SET, DefaultSetHolder.DEFAULT_STEM_DICT)`.
    pub fn with_stop_words(stopwords: CharArraySet) -> Self {
        Self::with_stem_exclusion_table(stopwords, &CharArraySet::empty_set())
    }

    /// Equivalent to `DutchAnalyzer(CharArraySet, CharArraySet)`,
    /// `this(stopwords, stemExclusionTable, DefaultSetHolder.DEFAULT_STEM_DICT)`.
    pub fn with_stem_exclusion_table(
        stopwords: CharArraySet,
        stem_exclusion_table: &CharArraySet,
    ) -> Self {
        Self::with_stem_override_dict(stopwords, stem_exclusion_table, &DEFAULT_STEM_DICT)
            .expect("INVARIANT: the default stem dictionary builds")
    }

    /// Equivalent to `DutchAnalyzer(CharArraySet, CharArraySet,
    /// CharArrayMap<String>)`: the analyzer keeps unmodifiable copies of
    /// the sets, and, unless `stem_override_dict` is empty, a
    /// `StemmerOverrideMap` built from its entries without ignoring case.
    ///
    /// # Errors
    ///
    /// `RuntimeException("can not build stem dict")`, caused by the error,
    /// when the map cannot be built.
    pub fn with_stem_override_dict(
        stopwords: CharArraySet,
        stem_exclusion_table: &CharArraySet,
        stem_override_dict: &CharArrayMap<String>,
    ) -> Result<Self> {
        let stoptable = CharArraySet::unmodifiable_set(Some(stopwords.copy()));
        let excltable = CharArraySet::unmodifiable_set(Some(stem_exclusion_table.copy()));
        let stemdict = if stem_override_dict.is_empty() {
            None
        } else {
            // we don't need to ignore case here since we lowercase in this analyzer anyway
            let mut builder = Builder::with_ignore_case(false);
            let build = |builder: &mut Builder| -> Result<StemmerOverrideMap> {
                for (next_key, value) in stem_override_dict.iter() {
                    builder.add(&to_rust_string(next_key), value)?;
                }
                builder.build()
            };
            match build(&mut builder) {
                Ok(map) => Some(Arc::new(map)),
                Err(ex) => {
                    return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                        class: "java.lang.RuntimeException",
                        message: Some("can not build stem dict".to_string()),
                        cause: Some(Arc::new(ex)),
                    }))
                }
            }
        };
        Ok(Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            stoptable,
            excltable,
            stemdict,
        })
    }
}

impl Analyzer for DutchAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.nl.DutchAnalyzer"
    }

    /// Returns a (possibly reused) `TokenStream` which tokenizes all the
    /// text in the provided `Reader`.
    ///
    /// * returns: A `TokenStream` built from a `StandardTokenizer` filtered
    ///   with `LowerCaseFilter`, `StopFilter`, `SetKeywordMarkerFilter` if a
    ///   stem exclusion set is provided, `StemmerOverrideFilter`, and
    ///   `SnowballFilter`
    ///
    /// Equivalent to `DutchAnalyzer.createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let source = Arc::new(AtomicRefCell::new(new_standard_tokenizer()));
        let source_for_reader = Arc::clone(&source);
        let set_reader = Box::new(move |reader: Box<dyn CharReader>| {
            source_for_reader.borrow_mut().set_reader(reader)
        });
        let mut result: Box<dyn TokenStream> = Box::new(new_lower_case_filter(Box::new(
            SharedTokenStream::new(source),
        ))?);
        result = Box::new(new_stop_filter(result, self.stoptable.clone())?);
        if !self.excltable.is_empty() {
            result = Box::new(new_set_keyword_marker_filter(
                result,
                self.excltable.clone(),
            )?);
        }
        if let Some(stemdict) = &self.stemdict {
            result = Box::new(new_stemmer_override_filter(result, Arc::clone(stemdict))?);
        }
        let result: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(
            new_snowball_filter(result, Box::new(DutchStemmer::new()))?,
        ));
        Ok(TokenStreamComponents::new(set_reader, result))
    }

    /// Equivalent to `DutchAnalyzer.normalize(String, TokenStream)`, `new
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
