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

//! The port of `SrndTermQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{IndexReader, MultiTerms, SeekStatus, Term};
use rucene::search::Query;
use rucene::util::BytesRef;

use super::basic_query_factory::BasicQueryFactory;
use super::distance_sub_query::DistanceSubQuery;
use super::simple_term::{self, MatchingTermVisitor, SimpleTerm};
use super::span_near_clause_factory::SpanNearClauseFactory;
use super::srnd_query::{SrndQuery, SrndQueryState};

/// Simple single-term clause.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.SrndTermQuery`.
#[derive(Debug, Clone)]
pub struct SrndTermQuery {
    srnd: SrndQueryState,
    quoted: bool,
    term_text: String,
}

impl SrndTermQuery {
    /// Equivalent to `new SrndTermQuery(String, boolean)`.
    pub fn new(term_text: &str, quoted: bool) -> Self {
        Self {
            srnd: SrndQueryState::new(),
            quoted,
            term_text: term_text.to_string(),
        }
    }

    /// Equivalent to `getTermText()`.
    pub fn get_term_text(&self) -> &str {
        &self.term_text
    }

    /// Equivalent to `getLuceneTerm(String)`.
    pub fn get_lucene_term(&self, field_name: &str) -> Term {
        Term::new(
            field_name.to_string(),
            BytesRef::from_char_sequence(self.get_term_text()),
        )
    }
}

impl SimpleTerm for SrndTermQuery {
    fn is_quoted(&self) -> bool {
        self.quoted
    }

    fn to_string_unquoted(&self) -> String {
        self.get_term_text().to_string()
    }

    fn visit_matching_terms(
        &self,
        reader: &Arc<dyn IndexReader>,
        field_name: &str,
        mtv: &mut dyn MatchingTermVisitor,
    ) -> Result<()> {
        /* check term presence in index here for symmetry with other SimpleTerm's */
        if let Some(terms) = MultiTerms::get_terms(reader, field_name)? {
            let mut terms_enum = terms.iterator()?;
            let status =
                terms_enum.seek_ceil(&BytesRef::from_char_sequence(self.get_term_text()))?;
            if status == SeekStatus::FOUND {
                mtv.visit_matching_term(self.get_lucene_term(field_name))?;
            }
        }
        Ok(())
    }
}

impl SrndQuery for SrndTermQuery {
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
        "org.apache.lucene.queryparser.surround.query.SrndTermQuery"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_distance_sub_query(&self) -> Option<&dyn DistanceSubQuery> {
        Some(self)
    }
}

impl DistanceSubQuery for SrndTermQuery {
    fn distance_sub_query_not_allowed(&self) -> Option<String> {
        simple_term::distance_sub_query_not_allowed(self)
    }

    fn add_span_queries(&self, sncf: &mut SpanNearClauseFactory) -> Result<()> {
        simple_term::add_span_queries(self, sncf)
    }
}
