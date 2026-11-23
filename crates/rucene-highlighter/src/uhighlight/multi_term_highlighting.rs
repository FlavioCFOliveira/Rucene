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

//! Port of `MultiTermHighlighting.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::sync::Arc;

use rucene::search::automaton_query::AutomatonQuery;
use rucene::search::boolean_clause::Occur;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::{EmptyQueryVisitor, QueryVisitor};
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::util::automaton::byte_run_automaton::ByteRunAutomaton;
use rucene_queries::spans::span_query::as_span_query;

use crate::uhighlight::labelled_char_array_matcher::{
    wrap_byte_run_automaton, LabelledCharArrayMatcher,
};
use crate::uhighlight::uh_components::FieldMatcher;

/// Support for highlighting multi-term queries.
///
/// Equivalent to the package-private final class
/// `org.apache.lucene.search.uhighlight.MultiTermHighlighting`.
#[derive(Debug)]
pub(crate) struct MultiTermHighlighting {
    _private: (),
}

impl MultiTermHighlighting {
    /// Extracts MultiTermQueries that match the provided field predicate.
    /// Returns equivalent automata that will match terms.
    ///
    /// Equivalent to the package-private static `extractAutomata(Query,
    /// Predicate, boolean)`.
    pub(crate) fn extract_automata(
        query: &dyn Query,
        field_matcher: &FieldMatcher,
        look_in_span: bool,
    ) -> Vec<Arc<dyn LabelledCharArrayMatcher>> {
        let mut collector = AutomataCollector::new(look_in_span, Arc::clone(field_matcher));
        query.visit(&mut collector);
        collector.run_automata
    }

    /// Indicates if the the leaf query (from `QueryVisitor.visitLeaf`) is a
    /// type of query that we can extract automata from.
    ///
    /// Equivalent to the static `canExtractAutomataFromLeafQuery(Query)`:
    /// `query instanceof AutomatonQuery || query instanceof FuzzyQuery`, the
    /// subclasses of `AutomatonQuery` (`PrefixQuery`, `WildcardQuery`,
    /// `RegexpQuery`, `TermRangeQuery`) included.
    pub fn can_extract_automata_from_leaf_query(query: &dyn Query) -> bool {
        let any = query.as_any();
        any.is::<AutomatonQuery>()
            || any.is::<PrefixQuery>()
            || any.is::<WildcardQuery>()
            || any.is::<RegexpQuery>()
            || any.is::<TermRangeQuery>()
            || any.is::<FuzzyQuery>()
    }
}

/// Equivalent to the private nested class
/// `MultiTermHighlighting.AutomataCollector`.
pub(crate) struct AutomataCollector {
    pub(crate) run_automata: Vec<Arc<dyn LabelledCharArrayMatcher>>,
    pub(crate) look_in_span: bool,
    pub(crate) field_matcher: FieldMatcher,
}

impl AutomataCollector {
    fn new(look_in_span: bool, field_matcher: FieldMatcher) -> Self {
        Self {
            run_automata: Vec::new(),
            look_in_span,
            field_matcher,
        }
    }
}

impl QueryVisitor for AutomataCollector {
    fn accept_field(&self, field: &str) -> bool {
        (self.field_matcher)(field)
    }

    fn get_sub_visitor<'a>(
        &'a mut self,
        occur: Occur,
        parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        if !self.look_in_span && as_span_query(parent).is_some() {
            return Box::new(EmptyQueryVisitor);
        }
        // super.getSubVisitor(occur, parent)
        if occur == Occur::MUST_NOT {
            return Box::new(EmptyQueryVisitor);
        }
        Box::new(self)
    }

    fn consume_terms_matching(
        &mut self,
        query: &dyn Query,
        _field: &str,
        automaton: &dyn Fn() -> ByteRunAutomaton,
    ) {
        self.run_automata.push(wrap_byte_run_automaton(
            &query.to_query_string(""),
            automaton(),
        ));
    }
}
