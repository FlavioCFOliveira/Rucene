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

//! An Analyzer that uses WhitespaceTokenizer, ported from
//! `org.apache.lucene.analysis.core.WhitespaceAnalyzer`.

use std::sync::Arc;

use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;

use super::whitespace_tokenizer::new_whitespace_tokenizer_with_max_token_len;
use crate::analysis::util::char_tokenizer::DEFAULT_MAX_WORD_LEN;

/// An Analyzer that uses `WhitespaceTokenizer`.
///
/// Equivalent to `org.apache.lucene.analysis.core.WhitespaceAnalyzer` (since
/// 3.1).
#[derive(Debug)]
pub struct WhitespaceAnalyzer {
    state: AnalyzerState,
    max_token_length: i32,
}

impl Default for WhitespaceAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for WhitespaceAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.WhitespaceAnalyzer"
    }

    /// Equivalent to `WhitespaceAnalyzer.createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        Ok(TokenStreamComponents::from_tokenizer(Arc::new(
            AtomicRefCell::new(new_whitespace_tokenizer_with_max_token_len(
                self.max_token_length,
            )?),
        )))
    }
}

impl WhitespaceAnalyzer {
    /// Creates a new `WhitespaceAnalyzer` with a maximum token length of 255
    /// chars.
    ///
    /// Equivalent to `WhitespaceAnalyzer()`,
    /// `this(WhitespaceTokenizer.DEFAULT_MAX_WORD_LEN)`.
    pub fn new() -> Self {
        Self::with_max_token_length(DEFAULT_MAX_WORD_LEN)
    }

    /// Creates a new `WhitespaceAnalyzer` with a custom maximum token length:
    /// `max_token_length` is the maximum token length the analyzer will emit.
    /// It is checked when the components are created, where Java's
    /// `new WhitespaceTokenizer(maxTokenLength)` checks it.
    ///
    /// Equivalent to `WhitespaceAnalyzer(int)`, whose `Analyzer()` uses
    /// `GLOBAL_REUSE_STRATEGY`.
    pub fn with_max_token_length(max_token_length: i32) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            max_token_length,
        }
    }
}
