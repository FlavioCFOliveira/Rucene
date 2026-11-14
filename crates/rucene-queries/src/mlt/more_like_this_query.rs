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

//! Port of `MoreLikeThisQuery.java`, of the Java package
//! `org.apache.lucene.queries.mlt`.

use std::any::Any;
use std::collections::HashSet;
use std::sync::Arc;

use rucene::analysis::{Analyzer, StringCharReader};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::boolean_query::{BooleanQuery, Builder as BooleanQueryBuilder};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::java_collections::{
    java_float_hash_code, java_identity_hash_code_stand_in, java_string_hash_code,
};

use crate::mlt::more_like_this::MoreLikeThis;

/// A simple wrapper for [`MoreLikeThis`] for use in scenarios where a
/// [`Query`] object is required, e.g. in custom query parsers.
///
/// Equivalent to `org.apache.lucene.queries.mlt.MoreLikeThisQuery`.
///
/// **Divergences from Lucene 10.5.0.**
///
/// * The stop words are a `HashSet<String>`, as in [`MoreLikeThis`].
/// * Java compares the analyzers with `Analyzer.equals`, which is identity,
///   and hashes them with their identity hash (`MoreLikeThisQuery.java:157,
///   177`). Identity is [`Arc::ptr_eq`] here, and the identity hash is
///   [`java_identity_hash_code_stand_in`] over the analyzer's Java class name,
///   which is consistent with it.
#[derive(Debug, Clone)]
pub struct MoreLikeThisQuery {
    like_text: String,
    more_like_fields: Vec<String>,
    analyzer: Arc<dyn Analyzer>,
    field_name: String,
    percent_terms_to_match: f32,
    min_term_frequency: i32,
    max_query_terms: i32,
    stop_words: Option<Arc<HashSet<String>>>,
    min_doc_freq: i32,
}

impl MoreLikeThisQuery {
    /// Equivalent to `MoreLikeThisQuery(String, String[], Analyzer, String)`.
    ///
    /// * `more_like_fields` — fields used for similarity measure.
    pub fn new(
        like_text: impl Into<String>,
        more_like_fields: Vec<String>,
        analyzer: Arc<dyn Analyzer>,
        field_name: impl Into<String>,
    ) -> Self {
        Self {
            like_text: like_text.into(),
            more_like_fields,
            analyzer,
            field_name: field_name.into(),
            percent_terms_to_match: 0.3,
            min_term_frequency: 1,
            max_query_terms: 5,
            stop_words: None,
            min_doc_freq: -1,
        }
    }

    /// Equivalent to `MoreLikeThisQuery.getPercentTermsToMatch()`.
    pub fn get_percent_terms_to_match(&self) -> f32 {
        self.percent_terms_to_match
    }

    /// Equivalent to `MoreLikeThisQuery.setPercentTermsToMatch(float)`.
    pub fn set_percent_terms_to_match(&mut self, percent_terms_to_match: f32) {
        self.percent_terms_to_match = percent_terms_to_match;
    }

    /// Equivalent to `MoreLikeThisQuery.getAnalyzer()`.
    pub fn get_analyzer(&self) -> &Arc<dyn Analyzer> {
        &self.analyzer
    }

    /// Equivalent to `MoreLikeThisQuery.setAnalyzer(Analyzer)`.
    pub fn set_analyzer(&mut self, analyzer: Arc<dyn Analyzer>) {
        self.analyzer = analyzer;
    }

    /// Equivalent to `MoreLikeThisQuery.getLikeText()`.
    pub fn get_like_text(&self) -> &str {
        &self.like_text
    }

    /// Equivalent to `MoreLikeThisQuery.setLikeText(String)`.
    pub fn set_like_text(&mut self, like_text: impl Into<String>) {
        self.like_text = like_text.into();
    }

    /// Equivalent to `MoreLikeThisQuery.getMaxQueryTerms()`.
    pub fn get_max_query_terms(&self) -> i32 {
        self.max_query_terms
    }

    /// Equivalent to `MoreLikeThisQuery.setMaxQueryTerms(int)`.
    pub fn set_max_query_terms(&mut self, max_query_terms: i32) {
        self.max_query_terms = max_query_terms;
    }

    /// Equivalent to `MoreLikeThisQuery.getMinTermFrequency()`.
    pub fn get_min_term_frequency(&self) -> i32 {
        self.min_term_frequency
    }

    /// Equivalent to `MoreLikeThisQuery.setMinTermFrequency(int)`.
    pub fn set_min_term_frequency(&mut self, min_term_frequency: i32) {
        self.min_term_frequency = min_term_frequency;
    }

    /// Equivalent to `MoreLikeThisQuery.getMoreLikeFields()`.
    pub fn get_more_like_fields(&self) -> &[String] {
        &self.more_like_fields
    }

    /// Equivalent to `MoreLikeThisQuery.setMoreLikeFields(String[])`.
    pub fn set_more_like_fields(&mut self, more_like_fields: Vec<String>) {
        self.more_like_fields = more_like_fields;
    }

    /// Equivalent to `MoreLikeThisQuery.getStopWords()`.
    pub fn get_stop_words(&self) -> Option<&Arc<HashSet<String>>> {
        self.stop_words.as_ref()
    }

    /// Equivalent to `MoreLikeThisQuery.setStopWords(Set<?>)`.
    pub fn set_stop_words(&mut self, stop_words: Option<Arc<HashSet<String>>>) {
        self.stop_words = stop_words;
    }

    /// Equivalent to `MoreLikeThisQuery.getMinDocFreq()`.
    pub fn get_min_doc_freq(&self) -> i32 {
        self.min_doc_freq
    }

    /// Equivalent to `MoreLikeThisQuery.setMinDocFreq(int)`.
    pub fn set_min_doc_freq(&mut self, min_doc_freq: i32) {
        self.min_doc_freq = min_doc_freq;
    }

    /// The identity hash of the analyzer; see the type documentation.
    fn analyzer_hash(&self) -> i32 {
        java_identity_hash_code_stand_in(self.analyzer.java_class_name())
    }

    /// `Set.hashCode()` of the stop words, `0` for `null`: the sum of the
    /// elements' hash codes.
    fn stop_words_hash(&self) -> i32 {
        self.stop_words.as_ref().map_or(0, |stop_words| {
            stop_words
                .iter()
                .fold(0i32, |h, word| h.wrapping_add(java_string_hash_code(word)))
        })
    }

    /// Equivalent to the private `MoreLikeThisQuery.equalsTo(MoreLikeThisQuery)`.
    fn equals_to(&self, other: &MoreLikeThisQuery) -> bool {
        self.max_query_terms == other.max_query_terms
            && self.min_doc_freq == other.min_doc_freq
            && self.min_term_frequency == other.min_term_frequency
            && java_float_hash_code(self.percent_terms_to_match)
                == java_float_hash_code(other.percent_terms_to_match)
            && Arc::ptr_eq(&self.analyzer, &other.analyzer)
            && self.field_name == other.field_name
            && self.like_text == other.like_text
            && self.more_like_fields == other.more_like_fields
            && self.stop_words.as_deref() == other.stop_words.as_deref()
    }
}

impl Query for MoreLikeThisQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!("like:{}", self.like_text)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let mut mlt = MoreLikeThis::new(Arc::clone(index_searcher.get_index_reader()));

        mlt.set_field_names(Some(self.more_like_fields.clone()));
        mlt.set_analyzer(Some(Arc::clone(&self.analyzer)));
        mlt.set_min_term_freq(self.min_term_frequency);
        if self.min_doc_freq >= 0 {
            mlt.set_min_doc_freq(self.min_doc_freq);
        }
        mlt.set_max_query_terms(self.max_query_terms);
        mlt.set_stop_words(self.stop_words.clone());
        let like = mlt.like_readers(
            &self.field_name,
            vec![Box::new(StringCharReader::new(self.like_text.clone()))],
        )?;
        // `(BooleanQuery) mlt.like(...)`.
        let bq = like
            .as_any()
            .downcast_ref::<BooleanQuery>()
            .ok_or_else(|| {
                LuceneError::from(LuceneErrorKind::Unchecked {
                    class: "java.lang.ClassCastException",
                    message: Some(format!(
                        "class {} cannot be cast to class org.apache.lucene.search.BooleanQuery",
                        like.java_class_name()
                    )),
                    cause: None,
                })
            })?;
        let mut new_bq = BooleanQueryBuilder::new();
        for clause in bq.clauses() {
            new_bq.add_clause(clause.clone())?;
        }
        // make at least half the terms match
        new_bq.set_minimum_number_should_match(
            (bq.clauses().len() as f32 * self.percent_terms_to_match) as i32,
        );
        Ok(Some(Arc::new(new_bq.build())))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<MoreLikeThisQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    fn query_hash(&self) -> i32 {
        let prime = 31i32;
        let mut result = self.class_hash();
        // `Objects.hash(analyzer, fieldName, likeText, stopWords)`.
        let mut objects_hash = 1i32;
        objects_hash = objects_hash
            .wrapping_mul(31)
            .wrapping_add(self.analyzer_hash());
        objects_hash = objects_hash
            .wrapping_mul(31)
            .wrapping_add(java_string_hash_code(&self.field_name));
        objects_hash = objects_hash
            .wrapping_mul(31)
            .wrapping_add(java_string_hash_code(&self.like_text));
        objects_hash = objects_hash
            .wrapping_mul(31)
            .wrapping_add(self.stop_words_hash());
        result = prime.wrapping_mul(result).wrapping_add(objects_hash);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.max_query_terms);
        result = prime.wrapping_mul(result).wrapping_add(self.min_doc_freq);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.min_term_frequency);
        // `Arrays.hashCode(moreLikeFields)`.
        let fields_hash = self.more_like_fields.iter().fold(1i32, |h, f| {
            h.wrapping_mul(31).wrapping_add(java_string_hash_code(f))
        });
        result = prime.wrapping_mul(result).wrapping_add(fields_hash);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_float_hash_code(self.percent_terms_to_match));
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.mlt.MoreLikeThisQuery"
    }
}
