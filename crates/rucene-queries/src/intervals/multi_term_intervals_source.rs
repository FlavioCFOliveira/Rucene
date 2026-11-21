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

//! Port of `MultiTermIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::{MatchesIterator, MatchesUtils};
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::automaton::compiled_automaton::CompiledAutomaton;
use rucene::util::java_collections::java_string_hash_code;

use crate::intervals::disjunction_intervals_source::DisjunctionIntervalIterator;
use crate::intervals::interval_iterator::IntervalIterator;
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::interval_query::IntervalQuery;
use crate::intervals::intervals_source::{objects_hash, same_object, IntervalsSource};
use crate::intervals::term_intervals_source::TermIntervalsSource;

/// The intervals of the terms an automaton accepts, at most `maxExpansions`
/// of them.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.MultiTermIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct MultiTermIntervalsSource {
    automaton: CompiledAutomaton,
    max_expansions: i32,
    pattern: String,
}

impl MultiTermIntervalsSource {
    /// Equivalent to `MultiTermIntervalsSource(CompiledAutomaton, int,
    /// String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `max_expansions` is greater than
    /// `IndexSearcher.getMaxClauseCount()`.
    pub(crate) fn new(
        automaton: CompiledAutomaton,
        max_expansions: i32,
        pattern: String,
    ) -> Result<Self> {
        if max_expansions > IndexSearcher::get_max_clause_count() {
            return Err(LuceneError::illegal_argument(format!(
                "maxExpansions [{max_expansions}] cannot be greater than BooleanQuery.getMaxClauseCount [{}]",
                IndexSearcher::get_max_clause_count()
            )));
        }
        Ok(Self {
            automaton,
            max_expansions,
            pattern,
        })
    }
}

/// The anonymous `IntervalMatchesIterator` of
/// `MultiTermIntervalsSource.matches`, over the disjunction of the terms'
/// matches.
struct MultiTermMatchesIterator {
    mi: Box<dyn MatchesIterator>,
}

impl MatchesIterator for MultiTermMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.mi.next()
    }

    fn start_position(&self) -> i32 {
        self.mi.start_position()
    }

    fn end_position(&self) -> i32 {
        self.mi.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.mi.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.mi.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.mi.get_sub_matches()
    }

    fn get_query(&self) -> Option<Arc<dyn Query>> {
        self.mi.get_query()
    }
}

impl IntervalMatchesIterator for MultiTermMatchesIterator {
    fn gaps(&self) -> i32 {
        0
    }

    fn width(&self) -> i32 {
        1
    }
}

impl IntervalsSource for MultiTermIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let Some(terms) = ctx.leaf_reader().terms(field)? else {
            return Ok(None);
        };
        let mut sub_sources: Vec<Box<dyn IntervalIterator>> = Vec::new();
        let mut te = self.automaton.get_terms_enum(&*terms)?;
        let mut count = 0i32;
        while let Some(term) = te.next()?.cloned() {
            sub_sources.push(TermIntervalsSource::intervals_for_term(term, &mut *te)?);
            count += 1;
            if count > self.max_expansions {
                return Err(LuceneError::illegal_state(format!(
                    "Automaton [{}] expanded to too many terms (limit {})",
                    self.pattern, self.max_expansions
                )));
            }
        }
        if sub_sources.is_empty() {
            return Ok(None);
        }
        Ok(Some(Box::new(DisjunctionIntervalIterator::new(
            sub_sources,
        )?)))
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let Some(terms) = ctx.leaf_reader().terms(field)? else {
            return Ok(None);
        };
        let mut sub_matches: Vec<Box<dyn MatchesIterator>> = Vec::new();
        let mut te = self.automaton.get_terms_enum(&*terms)?;
        let mut count = 0i32;
        while let Some(term) = te.next()?.cloned() {
            if let Some(mi) = TermIntervalsSource::matches_for_term(&mut *te, doc, field)? {
                sub_matches.push(mi);
                let previous = count;
                count += 1;
                if previous > self.max_expansions {
                    return Err(LuceneError::illegal_state(format!(
                        "Automaton {term} expanded to too many terms (limit {})",
                        self.max_expansions
                    )));
                }
            }
        }
        let Some(mi) = MatchesUtils::disjunction(sub_matches)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(MultiTermMatchesIterator { mi })))
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        let automaton = self.automaton.clone();
        let query = IntervalQuery::new(field, self);
        automaton.visit(visitor, &query, field);
    }

    fn min_extent(&self) -> i32 {
        1
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<MultiTermIntervalsSource>()
                .is_some_and(|that| {
                    self.max_expansions == that.max_expansions
                        && self.automaton == that.automaton
                        && self.pattern == that.pattern
                })
    }

    /// `Objects.hash(automaton, maxExpansions, pattern)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[
            self.automaton.hash_code(),
            self.max_expansions,
            java_string_hash_code(&self.pattern),
        ])
    }

    fn to_source_string(&self) -> String {
        format!("MultiTerm({})", self.pattern)
    }
}
