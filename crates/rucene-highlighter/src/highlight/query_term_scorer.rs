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

//! Port of `QueryTermScorer.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::any::Any;
use std::collections::{HashMap, HashSet};

use rucene::analysis::tokenattributes::CharTermAttribute;
use rucene::analysis::{AttributeHandle, TokenStream};
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::search::query::Query;

use crate::highlight::query_term_extractor::QueryTermExtractor;
use crate::highlight::scorer::Scorer;
use crate::highlight::text_fragment::TextFragment;
use crate::highlight::weighted_term::WeightedTerm;
use crate::java::term_text;

/// `Scorer` implementation which scores text fragments by the number of
/// unique query terms found. This class uses the `QueryTermExtractor` class
/// to process determine the query terms and their boosts to be used.
///
/// Equivalent to `org.apache.lucene.search.highlight.QueryTermScorer`.
#[derive(Debug)]
pub struct QueryTermScorer {
    pub(crate) current_text_fragment: Option<TextFragment>,
    pub(crate) unique_terms_in_fragment: Option<HashSet<String>>,
    pub(crate) total_score: f32,
    pub(crate) max_term_weight: f32,
    terms_to_find: HashMap<String, WeightedTerm>,
    term_att: Option<AttributeHandle<dyn CharTermAttribute>>,
}

impl QueryTermScorer {
    /// Equivalent to `new QueryTermScorer(Query)`: `query` is a Lucene query
    /// (ideally rewritten using query.rewrite before being passed to this
    /// class and the searcher).
    pub fn new(query: &dyn Query) -> Self {
        Self::from_weighted_terms(QueryTermExtractor::get_terms(query))
    }

    /// Equivalent to `new QueryTermScorer(Query, String)`.
    pub fn with_field(query: &dyn Query, field_name: &str) -> Self {
        Self::from_weighted_terms(QueryTermExtractor::get_terms_for_field(
            query,
            false,
            Some(field_name),
        ))
    }

    /// Equivalent to `new QueryTermScorer(Query, IndexReader, String)`:
    /// `reader` is used to compute IDF which can be used to a) score
    /// selected fragments better b) use graded highlights eg set font color
    /// intensity.
    ///
    /// # Errors
    ///
    /// As [`QueryTermExtractor::get_idf_weighted_terms`].
    pub fn with_reader(
        query: &dyn Query,
        reader: &dyn IndexReader,
        field_name: &str,
    ) -> Result<Self> {
        Ok(Self::from_weighted_terms(
            QueryTermExtractor::get_idf_weighted_terms(query, reader, field_name)?,
        ))
    }

    /// Equivalent to `new QueryTermScorer(WeightedTerm[])`.
    pub fn from_weighted_terms(weighted_terms: Vec<WeightedTerm>) -> Self {
        let mut terms_to_find: HashMap<String, WeightedTerm> = HashMap::new();
        let mut max_term_weight = 0.0f32;
        for weighted_term in weighted_terms {
            let existing_term = terms_to_find.get(&weighted_term.term);
            if existing_term.is_none_or(|existing| existing.weight < weighted_term.weight) {
                // if a term is defined more than once, always use the highest
                // scoring weight
                max_term_weight = max_term_weight.max(weighted_term.get_weight());
                terms_to_find.insert(weighted_term.term.clone(), weighted_term);
            }
        }
        Self {
            current_text_fragment: None,
            unique_terms_in_fragment: None,
            total_score: 0.0,
            max_term_weight,
            terms_to_find,
            term_att: None,
        }
    }

    /// Equivalent to `allFragmentsProcessed()`, which does nothing.
    pub fn all_fragments_processed(&self) {
        // this class has no special operations to perform at end of
        // processing
    }

    /// Returns the highest weighted term. If you are using gradient
    /// highlights you should use this value to work out the max value for the
    /// gradient.
    ///
    /// Equivalent to `getMaxTermWeight()`.
    pub fn get_max_term_weight(&self) -> f32 {
        self.max_term_weight
    }
}

impl Scorer for QueryTermScorer {
    fn init(&mut self, token_stream: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let mut token_stream = token_stream;
        self.term_att = Some(AttributeHandle::add(token_stream.attribute_source_mut())?);
        Ok(token_stream)
    }

    fn start_fragment(&mut self, new_fragment: &TextFragment) {
        self.unique_terms_in_fragment = Some(HashSet::new());
        self.current_text_fragment = Some(new_fragment.clone());
        self.total_score = 0.0;
    }

    /// # Panics
    ///
    /// Before `init` and `start_fragment`, where Java throws
    /// `NullPointerException`.
    fn get_token_score(&mut self) -> f32 {
        let term_text = term_text(
            &*self
                .term_att
                .as_ref()
                .expect("NullPointerException: init(TokenStream) was not called")
                .borrow(),
        );

        let Some(query_term) = self.terms_to_find.get(&term_text) else {
            return 0.0;
        };
        let weight = query_term.get_weight();
        // found a query term - is it unique in this doc?
        let unique = self
            .unique_terms_in_fragment
            .as_mut()
            .expect("NullPointerException: startFragment(TextFragment) was not called");
        if !unique.contains(&term_text) {
            self.total_score += weight;
            unique.insert(term_text);
        }
        weight
    }

    fn get_fragment_score(&self) -> f32 {
        self.total_score
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}