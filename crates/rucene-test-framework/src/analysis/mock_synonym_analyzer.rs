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

//! Port of `org.apache.lucene.tests.analysis.MockSynonymAnalyzer`.

use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use super::mock_synonym_filter::MockSynonymFilter;
use super::mock_tokenizer::MockTokenizer;
use super::{components, shared, sink};

/// adds synonym of "dog" for "dogs", and synonym of "cavy" for "guinea pig".
///
/// Equivalent to `org.apache.lucene.tests.analysis.MockSynonymAnalyzer`.
#[derive(Debug)]
pub struct MockSynonymAnalyzer {
    state: AnalyzerState,
}

impl MockSynonymAnalyzer {
    /// `new MockSynonymAnalyzer()`.
    pub fn new() -> MockSynonymAnalyzer {
        MockSynonymAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Default for MockSynonymAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for MockSynonymAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.analysis.MockSynonymAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::whitespace_lower_case()?));
        let filter = MockSynonymFilter::new(shared(&tokenizer))?;
        Ok(components(tokenizer, sink(filter)))
    }
}
