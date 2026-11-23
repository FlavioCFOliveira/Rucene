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

//! Port of `QueryTermExtractor.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::{EmptyQueryVisitor, QueryVisitor};

use crate::highlight::weighted_term::WeightedTerm;

/// Utility class used to extract the terms used in a query, plus any weights.
/// This class will not find terms for MultiTermQuery, TermRangeQuery and
/// PrefixQuery classes so the caller must pass a rewritten query (see
/// Query.rewrite) to obtain a list of expanded terms.
///
/// Equivalent to `org.apache.lucene.search.highlight.QueryTermExtractor`.
///
/// Java's private static `EMPTY_INDEXSEARCHER` (an `IndexSearcher` over an
/// empty `MultiReader`) is read by no method, so the port has no counterpart.
#[derive(Debug)]
pub struct QueryTermExtractor {
    _private: (),
}

impl QueryTermExtractor {
    /// Extracts all terms texts of a given Query into an array of
    /// WeightedTerms.
    ///
    /// Equivalent to the static `getTerms(Query)`.
    pub fn get_terms(query: &dyn Query) -> Vec<WeightedTerm> {
        Self::get_terms_prohibited(query, false)
    }

    /// Extracts all terms texts of a given Query into an array of
    /// WeightedTerms, with an IDF-based weight.
    ///
    /// Equivalent to the static `getIdfWeightedTerms(Query, IndexReader,
    /// String)`: an `IOException` of `docFreq` leaves the term's weight
    /// unchanged, as Java ignores it.
    ///
    /// # Errors
    ///
    /// A non-I/O failure of the reader's `docFreq`.
    pub fn get_idf_weighted_terms(
        query: &dyn Query,
        reader: &dyn IndexReader,
        field_name: &str,
    ) -> Result<Vec<WeightedTerm>> {
        let mut terms = Self::get_terms_for_field(query, false, Some(field_name));
        let total_num_docs = reader.max_doc();
        for term in &mut terms {
            match reader.doc_freq(&Term::from_text(field_name.to_string(), &term.term)) {
                Ok(doc_freq) => {
                    // IDF algorithm taken from ClassicSimilarity class
                    let idf = ((total_num_docs as f64 / (doc_freq as f64 + 1.0)).ln() + 1.0) as f32;
                    term.weight *= idf;
                }
                // ignore
                Err(e) if e.is_io_exception() => {}
                Err(e) => return Err(e),
            }
        }
        Ok(terms)
    }

    /// Extracts all terms texts of a given Query into an array of
    /// WeightedTerms; `field_name` (`None` for Java's `null`) restricts the
    /// terms to one field.
    ///
    /// Equivalent to the static `getTerms(Query, boolean, String)`. Java
    /// collects the terms into a `HashSet` compared by identity, so every
    /// extracted term is kept; the array order follows the identity hash
    /// codes, which Java leaves unspecified (they vary between runs), so the
    /// port's extraction order is one of the orders Java can produce.
    pub fn get_terms_for_field(
        query: &dyn Query,
        prohibited: bool,
        field_name: Option<&str>,
    ) -> Vec<WeightedTerm> {
        let mut terms = Vec::new();
        query.visit(&mut BoostedTermExtractor::new(
            1.0,
            &mut terms,
            prohibited,
            field_name.map(str::to_string),
        ));
        terms
    }

    /// Extracts all terms texts of a given Query into an array of
    /// WeightedTerms.
    ///
    /// Equivalent to the static `getTerms(Query, boolean)`.
    pub fn get_terms_prohibited(query: &dyn Query, prohibited: bool) -> Vec<WeightedTerm> {
        Self::get_terms_for_field(query, prohibited, None)
    }
}

/// Equivalent to the private nested class `QueryTermExtractor.BoostedTermExtractor`.
struct BoostedTermExtractor<'t> {
    boost: f32,
    terms: &'t mut Vec<WeightedTerm>,
    include_prohibited: bool,
    /// The field selector: `None` accepts every field (`f -> true`), a name
    /// accepts that field (`fieldName::equals`).
    field_selector: Option<String>,
}

impl<'t> BoostedTermExtractor<'t> {
    fn new(
        boost: f32,
        terms: &'t mut Vec<WeightedTerm>,
        include_prohibited: bool,
        field_selector: Option<String>,
    ) -> Self {
        Self {
            boost,
            terms,
            include_prohibited,
            field_selector,
        }
    }
}

impl QueryVisitor for BoostedTermExtractor<'_> {
    fn accept_field(&self, field: &str) -> bool {
        match &self.field_selector {
            None => true,
            Some(name) => name == field,
        }
    }

    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        for term in terms {
            self.terms.push(WeightedTerm::new(self.boost, term.text()));
        }
    }

    fn get_sub_visitor<'a>(
        &'a mut self,
        occur: Occur,
        parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        if let Some(boost_query) = parent.as_any().downcast_ref::<BoostQuery>() {
            let newboost = self.boost * boost_query.get_boost();
            return Box::new(BoostedTermExtractor::new(
                newboost,
                &mut *self.terms,
                self.include_prohibited,
                self.field_selector.clone(),
            ));
        }
        if occur == Occur::MUST_NOT && !self.include_prohibited {
            return Box::new(EmptyQueryVisitor);
        }
        Box::new(self)
    }
}

#[allow(unused_imports)]
use Arc as _;