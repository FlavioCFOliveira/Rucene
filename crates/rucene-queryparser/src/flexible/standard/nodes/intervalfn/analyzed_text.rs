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

//! Port of `AnalyzedText.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use super::interval_function::IntervalFunction;

/// Node that represents `Intervals.analyzedText(String, Analyzer, String,
/// int, boolean)`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.AnalyzedText`.
#[derive(Debug)]
pub struct AnalyzedText {
    term: String,
}

impl AnalyzedText {
    /// Equivalent to `new AnalyzedText(String term)`.
    pub fn new(term: impl Into<String>) -> Self {
        Self { term: term.into() }
    }

    /// Equivalent to the package-private static `requiresQuotes(String
    /// term)`: whether the term holds a character of the class `[\\s]`
    /// (`Pattern.compile("[\\s]").matcher(term).find()`), a space, tab, line
    /// feed, line tabulation, form feed or carriage return.
    pub(crate) fn requires_quotes(term: &str) -> bool {
        term.chars()
            .any(|c| matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r'))
    }
}

impl IntervalFunction for AnalyzedText {
    /// Equivalent to `toIntervalSource(String field, Analyzer analyzer)`:
    /// `Intervals.analyzedText(term, analyzer, field, 0, true)`, its
    /// `IOException` rethrown as a `RuntimeException`.
    fn to_interval_source(
        &self,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let gaps = 0;
        let ordered = true;
        Intervals::analyzed_text(&self.term, analyzer, field, gaps, ordered).map_err(|e| {
            if e.is_io_exception() {
                LuceneError::runtime_exception(e)
            } else {
                e
            }
        })
    }

    /// Equivalent to `AnalyzedText.toString()`.
    fn to_java_string(&self) -> String {
        if Self::requires_quotes(&self.term) {
            format!("\"{}\"", self.term)
        } else {
            self.term.clone()
        }
    }
}
