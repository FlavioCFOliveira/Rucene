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

//! Port of `FuzzyTerm.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::fuzzy_query::{
    DEFAULT_MAX_EDITS, DEFAULT_PREFIX_LENGTH, DEFAULT_TRANSPOSITIONS,
};
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use super::analyzed_text::AnalyzedText;
use super::interval_function::IntervalFunction;

/// An interval function equivalent to `Intervals.fuzzyTerm(String, int)`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.FuzzyTerm`.
#[derive(Debug)]
pub struct FuzzyTerm {
    term: String,
    max_edits: i32,
    max_expansions: i32,
}

impl FuzzyTerm {
    /// Equivalent to `new FuzzyTerm(String term, Integer maxEdits, Integer
    /// maxExpansions)`: `None` gives `FuzzyQuery.defaultMaxEdits` and
    /// `Intervals.DEFAULT_MAX_EXPANSIONS`.
    pub fn new(
        term: impl Into<String>,
        max_edits: Option<i32>,
        max_expansions: Option<i32>,
    ) -> Self {
        Self {
            term: term.into(),
            max_edits: max_edits.unwrap_or(DEFAULT_MAX_EDITS),
            max_expansions: max_expansions.unwrap_or(Intervals::DEFAULT_MAX_EXPANSIONS),
        }
    }
}

impl IntervalFunction for FuzzyTerm {
    fn to_interval_source(
        &self,
        _field: &str,
        _analyzer: &dyn Analyzer,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Intervals::fuzzy_term_with(
            &self.term,
            self.max_edits,
            DEFAULT_PREFIX_LENGTH,
            DEFAULT_TRANSPOSITIONS,
            self.max_expansions,
        )
    }

    /// Equivalent to `toString()`: `String.format(Locale.ROOT,
    /// "fn:fuzzyTerm(%s %d%s)", ...)`, the maximum expansions right after
    /// the maximum edits.
    fn to_java_string(&self) -> String {
        let term = if AnalyzedText::requires_quotes(&self.term) {
            format!("\"{}\"", self.term)
        } else {
            self.term.clone()
        };
        format!(
            "fn:fuzzyTerm({term} {}{})",
            self.max_edits, self.max_expansions
        )
    }
}
