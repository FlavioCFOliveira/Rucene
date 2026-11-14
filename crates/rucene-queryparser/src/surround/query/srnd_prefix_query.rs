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

//! The port of `SrndPrefixQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{IndexReader, MultiTerms, Term};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::Query;
use rucene::util::automaton::compiled_automaton::CompiledAutomaton;
use rucene::util::BytesRef;

use super::basic_query_factory::BasicQueryFactory;
use super::distance_sub_query::DistanceSubQuery;
use super::simple_term::{self, MatchingTermVisitor, SimpleTerm};
use super::span_near_clause_factory::SpanNearClauseFactory;
use super::srnd_query::{SrndQuery, SrndQueryState};

/// Query that matches String prefixes.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.SrndPrefixQuery`.
#[derive(Debug, Clone)]
pub struct SrndPrefixQuery {
    srnd: SrndQueryState,
    quoted: bool,
    compiled: Arc<CompiledAutomaton>,
    prefix: String,
    truncator: char,
}

impl SrndPrefixQuery {
    /// Equivalent to `new SrndPrefixQuery(String, boolean, char)`.
    ///
    /// # Errors
    ///
    /// The errors of the automaton compilation.
    pub fn new(prefix: &str, quoted: bool, truncator: char) -> Result<Self> {
        let compiled = CompiledAutomaton::new(
            PrefixQuery::to_automaton(&BytesRef::from_char_sequence(prefix)),
            true,
            true,
            true,
        )?;
        Ok(Self {
            srnd: SrndQueryState::new(),
            quoted,
            compiled: Arc::new(compiled),
            prefix: prefix.to_string(),
            truncator,
        })
    }

    /// Equivalent to `getPrefix()`.
    pub fn get_prefix(&self) -> &str {
        &self.prefix
    }

    /// Equivalent to `getSuffixOperator()`.
    pub fn get_suffix_operator(&self) -> char {
        self.truncator
    }
}

impl SimpleTerm for SrndPrefixQuery {
    fn is_quoted(&self) -> bool {
        self.quoted
    }

    fn to_string_unquoted(&self) -> String {
        self.get_prefix().to_string()
    }

    fn suffix_to_string(&self, r: &mut String) {
        r.push(self.get_suffix_operator());
    }

    fn visit_matching_terms(
        &self,
        reader: &Arc<dyn IndexReader>,
        field_name: &str,
        mtv: &mut dyn MatchingTermVisitor,
    ) -> Result<()> {
        if let Some(terms) = MultiTerms::get_terms(reader, field_name)? {
            let mut terms_enum = self.compiled.get_terms_enum(terms.as_ref())?;
            while let Some(br) = terms_enum.next()? {
                mtv.visit_matching_term(Term::new(field_name, BytesRef::deep_copy_of(&br)))?;
            }
        }
        Ok(())
    }
}

impl SrndQuery for SrndPrefixQuery {
    fn srnd_state(&self) -> &SrndQueryState {
        &self.srnd
    }

    fn make_lucene_query_field_no_boost(
        self: Arc<Self>,
        field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Arc<dyn Query>> {
        simple_term::make_lucene_query_field_no_boost(self, field_name, qf)
    }

    fn srnd_to_string(&self) -> String {
        simple_term::to_string(self)
    }

    fn clone_srnd(&self) -> Arc<dyn SrndQuery> {
        Arc::new(self.clone())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queryparser.surround.query.SrndPrefixQuery"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_distance_sub_query(&self) -> Option<&dyn DistanceSubQuery> {
        Some(self)
    }
}

impl DistanceSubQuery for SrndPrefixQuery {
    fn distance_sub_query_not_allowed(&self) -> Option<String> {
        simple_term::distance_sub_query_not_allowed(self)
    }

    fn add_span_queries(&self, sncf: &mut SpanNearClauseFactory) -> Result<()> {
        simple_term::add_span_queries(self, sncf)
    }
}
