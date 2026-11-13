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

//! An analyzer that filters `LetterTokenizer` with `LowerCaseFilter`,
//! ported from `org.apache.lucene.analysis.core.SimpleAnalyzer`.

use std::sync::Arc;

use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, SharedTokenStream, TokenStream,
    TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;

use super::letter_tokenizer::new_letter_tokenizer;
use super::lower_case_filter::LowerCaseFilter;

/// An `Analyzer` that filters `LetterTokenizer` with `LowerCaseFilter`.
///
/// Equivalent to `org.apache.lucene.analysis.core.SimpleAnalyzer` (since
/// 3.1).
#[derive(Debug)]
pub struct SimpleAnalyzer {
    state: AnalyzerState,
}

impl SimpleAnalyzer {
    /// Creates a new [`SimpleAnalyzer`].
    ///
    /// Equivalent to `SimpleAnalyzer()`, whose `Analyzer()` uses
    /// `GLOBAL_REUSE_STRATEGY`.
    pub fn new() -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Default for SimpleAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for SimpleAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.SimpleAnalyzer"
    }

    /// Equivalent to `SimpleAnalyzer.createComponents(String)`:
    /// `new TokenStreamComponents(tokenizer, new LowerCaseFilter(tokenizer))`,
    /// the tokenizer's `setReader` as the source.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(new_letter_tokenizer()));
        let tokenizer_for_source = Arc::clone(&tokenizer);
        let source = Box::new(move |reader: Box<dyn CharReader>| {
            tokenizer_for_source.borrow_mut().set_reader(reader)
        });
        let filter = LowerCaseFilter::new(Box::new(SharedTokenStream::new(tokenizer)))?;
        let sink: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(filter));
        Ok(TokenStreamComponents::new(source, sink))
    }

    /// Equivalent to `SimpleAnalyzer.normalize(String, TokenStream)`, `new
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
