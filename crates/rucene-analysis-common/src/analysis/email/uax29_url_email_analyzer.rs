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

//! Filters `UAX29URLEmailTokenizer` with `LowerCaseFilter` and `StopFilter`,
//! ported from `org.apache.lucene.analysis.email.UAX29URLEmailAnalyzer`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::analysis::{
    new_lower_case_filter, new_stop_filter, Analyzer, AnalyzerState, CharArraySet, CharReader,
    GlobalReuseStrategy, SharedTokenStream, StopwordAnalyzerBase, TokenStream,
    TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;

use super::uax29_url_email_tokenizer::new_uax29_url_email_tokenizer;
use crate::analysis::en::english_analyzer::ENGLISH_STOP_WORDS_SET;

/// Default maximum allowed token length
///
/// Equivalent to `UAX29URLEmailAnalyzer.DEFAULT_MAX_TOKEN_LENGTH`,
/// `StandardAnalyzer.DEFAULT_MAX_TOKEN_LENGTH`.
pub const DEFAULT_MAX_TOKEN_LENGTH: i32 = rucene::analysis::DEFAULT_MAX_TOKEN_LENGTH as i32;

/// Filters `UAX29URLEmailTokenizer` with `LowerCaseFilter` and `StopFilter`,
/// using a list of English stop words.
///
/// Equivalent to `org.apache.lucene.analysis.email.UAX29URLEmailAnalyzer`
/// (since 3.6.0), which extends `StopwordAnalyzerBase`: the stop words live in
/// the [`StopwordAnalyzerBase`] this analyzer holds, as Java's live in the
/// inherited field.
#[derive(Debug)]
pub struct UAX29URLEmailAnalyzer {
    state: AnalyzerState,
    base: StopwordAnalyzerBase,
    /// Java's `maxTokenLength` field. The components' source reads it again on
    /// every reader, as Java's lambda reads
    /// `UAX29URLEmailAnalyzer.this.maxTokenLength`, so it is shared with them.
    max_token_length: Arc<AtomicI32>,
}

impl Default for UAX29URLEmailAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl UAX29URLEmailAnalyzer {
    /// An unmodifiable set containing some common English words that are
    /// usually not useful for searching.
    ///
    /// Equivalent to `UAX29URLEmailAnalyzer.STOP_WORDS_SET`,
    /// `EnglishAnalyzer.ENGLISH_STOP_WORDS_SET`.
    pub fn stop_words_set() -> &'static CharArraySet {
        &ENGLISH_STOP_WORDS_SET
    }

    /// Builds an analyzer with the given stop words.
    ///
    /// Equivalent to `UAX29URLEmailAnalyzer(CharArraySet)`, `super(stopWords)`.
    pub fn with_stop_words(stop_words: CharArraySet) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            base: StopwordAnalyzerBase::new(stop_words),
            max_token_length: Arc::new(AtomicI32::new(DEFAULT_MAX_TOKEN_LENGTH)),
        }
    }

    /// Builds an analyzer with the default stop words
    /// ([`stop_words_set`](Self::stop_words_set)).
    ///
    /// Equivalent to `UAX29URLEmailAnalyzer()`.
    pub fn new() -> Self {
        Self::with_stop_words(ENGLISH_STOP_WORDS_SET.clone())
    }

    /// Builds an analyzer with the stop words from the given reader.
    ///
    /// Equivalent to `UAX29URLEmailAnalyzer(Reader)`,
    /// `this(loadStopwordSet(stopwords))`.
    ///
    /// # Errors
    ///
    /// What [`StopwordAnalyzerBase::load_stopword_set`] returns.
    pub fn from_reader(stopwords: &mut dyn CharReader) -> Result<Self> {
        Ok(Self::with_stop_words(
            StopwordAnalyzerBase::load_stopword_set(stopwords)?,
        ))
    }

    /// Set the max allowed token length. Tokens larger than this will be
    /// chopped up at this token length and emitted as multiple tokens. If you
    /// need to skip such large tokens, you could increase this max length, and
    /// then use `LengthFilter` to remove long tokens. The default is
    /// [`DEFAULT_MAX_TOKEN_LENGTH`].
    ///
    /// Equivalent to `UAX29URLEmailAnalyzer.setMaxTokenLength(int)`, which
    /// only stores the value: the tokenizer checks it when it receives it.
    pub fn set_max_token_length(&mut self, length: i32) {
        self.max_token_length.store(length, Ordering::Relaxed);
    }

    /// Equivalent to `UAX29URLEmailAnalyzer.getMaxTokenLength()`.
    pub fn get_max_token_length(&self) -> i32 {
        self.max_token_length.load(Ordering::Relaxed)
    }

    /// Returns the stop-word set used by this analyzer.
    ///
    /// Equivalent to the inherited `StopwordAnalyzerBase.getStopwordSet()`.
    pub fn stopwords(&self) -> &CharArraySet {
        self.base.stopwords()
    }
}

impl Analyzer for UAX29URLEmailAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.email.UAX29URLEmailAnalyzer"
    }

    /// Equivalent to `UAX29URLEmailAnalyzer.createComponents(String)`: a
    /// `UAX29URLEmailTokenizer` with the analyzer's maximum token length,
    /// filtered with `LowerCaseFilter` and `StopFilter`; the source applies
    /// the analyzer's current length again before every reader, as Java's
    /// `src.setMaxTokenLength(UAX29URLEmailAnalyzer.this.maxTokenLength)`
    /// lambda does.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let src = Arc::new(AtomicRefCell::new(new_uax29_url_email_tokenizer()));
        let max_token_length = Arc::clone(&self.max_token_length);
        src.borrow_mut()
            .logic_mut()
            .set_max_token_length(max_token_length.load(Ordering::Relaxed))?;
        let src_for_reader = Arc::clone(&src);
        let set_reader = Box::new(move |reader: Box<dyn CharReader>| {
            let mut src = src_for_reader.borrow_mut();
            src.logic_mut()
                .set_max_token_length(max_token_length.load(Ordering::Relaxed))?;
            src.set_reader(reader)
        });
        let tok: Box<dyn TokenStream> = Box::new(new_lower_case_filter(Box::new(
            SharedTokenStream::new(src),
        ))?);
        let tok: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(
            new_stop_filter(tok, self.base.stopwords().clone())?,
        ));
        Ok(TokenStreamComponents::new(set_reader, tok))
    }

    /// Equivalent to `UAX29URLEmailAnalyzer.normalize(String, TokenStream)`,
    /// `new LowerCaseFilter(in)`.
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
