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

//! An Analyzer that uses UnicodeWhitespaceTokenizer, ported from
//! `org.apache.lucene.analysis.core.UnicodeWhitespaceAnalyzer`.

use std::sync::Arc;

use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;

use super::unicode_whitespace_tokenizer::new_unicode_whitespace_tokenizer;

/// An Analyzer that uses `UnicodeWhitespaceTokenizer`.
///
/// Equivalent to `org.apache.lucene.analysis.core.UnicodeWhitespaceAnalyzer`
/// (since 5.4.0).
#[derive(Debug)]
pub struct UnicodeWhitespaceAnalyzer {
    state: AnalyzerState,
}

impl Default for UnicodeWhitespaceAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for UnicodeWhitespaceAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.core.UnicodeWhitespaceAnalyzer"
    }

    /// Equivalent to `UnicodeWhitespaceAnalyzer.createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        Ok(TokenStreamComponents::from_tokenizer(Arc::new(
            AtomicRefCell::new(new_unicode_whitespace_tokenizer()),
        )))
    }
}

impl UnicodeWhitespaceAnalyzer {
    /// Creates a new `UnicodeWhitespaceAnalyzer`.
    ///
    /// Equivalent to `UnicodeWhitespaceAnalyzer()`, whose `Analyzer()` uses
    /// `GLOBAL_REUSE_STRATEGY`.
    pub fn new() -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}
