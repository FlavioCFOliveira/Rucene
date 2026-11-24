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

//! Port of `MissingAnalyzer.java`: an `Analyzer` that throws a runtime
//! exception when used for anything.

use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, TokenStreamComponents,
};
use rucene::error::{LuceneError, Result};
use rucene_test_framework::util::java_exceptions::rethrow_java;

/// `final class MissingAnalyzer extends Analyzer`.
#[derive(Debug)]
pub struct MissingAnalyzer {
    state: AnalyzerState,
}

impl MissingAnalyzer {
    /// `new MissingAnalyzer()`.
    pub fn new() -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

fn missing(field_name: Option<&str>) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.RuntimeException",
        format!(
            "Field must have an explicit Analyzer: {}",
            field_name.unwrap_or("null")
        ),
    )
}

impl Analyzer for MissingAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.matchhighlight.MissingAnalyzer"
    }

    fn init_reader(
        &self,
        field_name: Option<&str>,
        _reader: Box<dyn CharReader>,
    ) -> Box<dyn CharReader> {
        rethrow_java(missing(field_name))
    }

    fn create_components(&self, field_name: Option<&str>) -> Result<TokenStreamComponents> {
        Err(missing(field_name))
    }

    fn get_offset_gap(&self, _field_name: Option<&str>) -> i32 {
        0
    }
}
