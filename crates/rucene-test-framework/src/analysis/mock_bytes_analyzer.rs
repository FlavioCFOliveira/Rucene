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

//! Port of `org.apache.lucene.tests.analysis.MockBytesAnalyzer`.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::mock_tokenizer::{MockTokenizer, DEFAULT_MAX_TOKEN_LENGTH, KEYWORD};
use super::mock_utf16_term_attribute_impl::MockUTF16TermAttributeImpl;
use super::tokenizer_components;

/// Analyzer for testing that encodes terms as UTF-16 bytes.
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockBytesAnalyzer`.
#[derive(Debug)]
pub struct MockBytesAnalyzer {
    state: AnalyzerState,
}

impl MockBytesAnalyzer {
    /// `new MockBytesAnalyzer()`.
    pub fn new() -> MockBytesAnalyzer {
        MockBytesAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Default for MockBytesAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for MockBytesAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.analysis.MockBytesAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let t = MockTokenizer::with_factory(
            MockUTF16TermAttributeImpl::utf16_term_attribute_factory(),
            Arc::clone(&KEYWORD),
            false,
            DEFAULT_MAX_TOKEN_LENGTH,
        )?;
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(t))))
    }

    fn attribute_factory(&self, _field_name: &str) -> Arc<dyn AttributeFactory> {
        MockUTF16TermAttributeImpl::utf16_term_attribute_factory()
    }
}
