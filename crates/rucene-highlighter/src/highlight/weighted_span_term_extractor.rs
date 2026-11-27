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

//! Port of `WeightedSpanTermExtractor.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::collections::hash_map::{self, HashMap};
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use rucene::analysis::caching_token_filter::CachingTokenFilter;
use rucene::analysis::TokenStream;
use rucene::error::{LuceneError, Result};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::{
    BinaryDocValues, FieldInfos, IndexReader, LeafReader, LeafReaderContext, NumericDocValues,
    SortedDocValues, Term, Terms,
};
use rucene::search::boolean_query::BooleanQuery;
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::index_or_doc_values_query::IndexOrDocValuesQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::multi_phrase_query::MultiPhraseQuery;
use rucene::search::multi_term_query::scoring_boolean_rewrite;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::term_collector;
use rucene::search::score_mode::ScoreMode;
use rucene::search::synonym_query::SynonymQuery;
use rucene::search::NO_MORE_DOCS;
use rucene_memory::MemoryIndex;
use rucene_queries::common_terms_query::CommonTermsQuery;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_queries::spans::field_masking_span_query::FieldMaskingSpanQuery;
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_position_check_query::SpanPositionCheckQuery;
use rucene_queries::spans::span_query::{as_span_query, to_span_query_arc, SpanQuery};
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::spans::NO_MORE_POSITIONS;

use crate::highlight::offset_limit_token_filter::OffsetLimitTokenFilter;
use crate::highlight::position_span::PositionSpan;
use crate::highlight::term_vector_leaf_reader::TermVectorLeafReader;
use crate::highlight::token_stream_from_term_vector::TokenStreamFromTermVector;
use crate::highlight::weighted_span_term::WeightedSpanTerm;

/// `(SpanQuery) q`: the cast, or Java's `ClassCastException`.
fn cast_span_query(query: &dyn Query) -> Result<Arc<dyn SpanQuery>> {
    to_span_query_arc(query).ok_or_else(|| {
        LuceneError::unchecked(
            "java.lang.ClassCastException",
            format!(
                "class {} cannot be cast to class org.apache.lucene.queries.spans.SpanQuery",
                query.java_class_name()
            ),
        )
    })
}

/// A token stream borrowed for the length of a call, as Java hands an
/// object it keeps to a consumer that does not.
#[derive(Debug)]
struct BorrowedTokenStream<'a>(&'a mut dyn TokenStream);

impl TokenStream for BorrowedTokenStream<'_> {
    fn increment_token(&mut self) -> Result<bool> {
        self.0.increment_token()
    }

    fn end(&mut self) -> Result<()> {
        self.0.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.0.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.0.close()
    }

    fn attribute_source(&self) -> &rucene::util::attribute::AttributeSource {
        self.0.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut rucene::util::attribute::AttributeSource {
        self.0.attribute_source_mut()
    }
}

/// The protected methods of `WeightedSpanTermExtractor` a Java subclass
/// overrides. Each default is the base class's body, reachable as the
/// extractor's `super_*` method.
pub trait WeightedSpanTermExtractorOverrides: fmt::Debug + Send + Sync {
    /// Equivalent to the protected `extract(Query, float, Map)`.
    ///
    /// # Errors
    ///
    /// As [`WeightedSpanTermExtractor::super_extract`].
    fn extract(
        &self,
        this: &mut WeightedSpanTermExtractor,
        query: &Arc<dyn Query>,
        boost: f32,
        terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        this.super_extract(query, boost, terms)
    }

    /// Equivalent to the protected `isQueryUnsupported(Class)`; the query
    /// stands for its class.
    fn is_query_unsupported(&self, this: &WeightedSpanTermExtractor, query: &dyn Query) -> bool {
        this.super_is_query_unsupported(query)
    }

    /// Equivalent to the protected `extractUnknownQuery(Query, Map)`.
    ///
    /// # Errors
    ///
    /// What the subclass raises.
    fn extract_unknown_query(
        &self,
        _this: &mut WeightedSpanTermExtractor,
        _query: &Arc<dyn Query>,
        _terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        // for sub-classing to extract custom queries
        Ok(())
    }

    /// Equivalent to the protected `extractWeightedSpanTerms(Map, SpanQuery,
    /// float)`.
    ///
    /// # Errors
    ///
    /// As [`WeightedSpanTermExtractor::super_extract_weighted_span_terms`].
    fn extract_weighted_span_terms(
        &self,
        this: &mut WeightedSpanTermExtractor,
        terms: &mut PositionCheckingMap,
        span_query: Arc<dyn SpanQuery>,
        boost: f32,
    ) -> Result<()> {
        this.super_extract_weighted_span_terms(terms, span_query, boost)
    }

    /// Equivalent to the protected `extractWeightedTerms(Map, Query, float)`.
    ///
    /// # Errors
    ///
    /// As [`WeightedSpanTermExtractor::super_extract_weighted_terms`].
    fn extract_weighted_terms(
        &self,
        this: &mut WeightedSpanTermExtractor,
        terms: &mut PositionCheckingMap,
        query: &Arc<dyn Query>,
        boost: f32,
    ) -> Result<()> {
        this.super_extract_weighted_terms(terms, query, boost)
    }

    /// Equivalent to the protected `fieldNameComparator(String)`.
    fn field_name_comparator(
        &self,
        this: &WeightedSpanTermExtractor,
        field_name_to_check: Option<&str>,
    ) -> bool {
        this.super_field_name_comparator(field_name_to_check)
    }

    /// Equivalent to the protected `getLeafContext()`.
    ///
    /// # Errors
    ///
    /// As [`WeightedSpanTermExtractor::super_get_leaf_context`].
    fn get_leaf_context(
        &self,
        this: &mut WeightedSpanTermExtractor,
    ) -> Result<Arc<LeafReaderContext>> {
        this.super_get_leaf_context()
    }

    /// Equivalent to the protected `collectSpanQueryFields(SpanQuery, Set)`.
    fn collect_span_query_fields(
        &self,
        this: &WeightedSpanTermExtractor,
        span_query: &dyn SpanQuery,
        field_names: &mut HashSet<Option<String>>,
    ) {
        this.super_collect_span_query_fields(span_query, field_names)
    }

    /// Equivalent to the protected `mustRewriteQuery(SpanQuery)`.
    fn must_rewrite_query(
        &self,
        this: &WeightedSpanTermExtractor,
        span_query: &dyn SpanQuery,
    ) -> bool {
        this.super_must_rewrite_query(span_query)
    }
}

/// Class used to extract `WeightedSpanTerm`s from a `Query` based on whether
/// `Term`s from the `Query` are contained in a supplied `TokenStream`.
///
/// In order to support additional, by default unsupported queries,
/// subclasses can override `extract(Query, float, Map)` for extracting
/// wrapped or delegate queries and `extractUnknownQuery(Query, Map)` to
/// process custom leaf queries; the port takes those overrides as a
/// [`WeightedSpanTermExtractorOverrides`].
///
/// Equivalent to `org.apache.lucene.search.highlight.WeightedSpanTermExtractor`.
pub struct WeightedSpanTermExtractor {
    field_name: Option<String>,
    token_stream: Option<Box<dyn TokenStream>>, // set after getWeightedSpanTerms* methods
    default_field: Option<String>,
    expand_multi_term_query: bool,
    cached_token_stream: bool,
    wrap_to_caching: bool,
    max_doc_chars_to_analyze: i32,
    use_payloads: bool,
    internal_reader: Option<Arc<dyn LeafReader>>,
    overrides: Option<Arc<dyn WeightedSpanTermExtractorOverrides>>,
}

impl fmt::Debug for WeightedSpanTermExtractor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeightedSpanTermExtractor")
            .field("field_name", &self.field_name)
            .field("default_field", &self.default_field)
            .field("expand_multi_term_query", &self.expand_multi_term_query)
            .field("cached_token_stream", &self.cached_token_stream)
            .field("wrap_to_caching", &self.wrap_to_caching)
            .field("max_doc_chars_to_analyze", &self.max_doc_chars_to_analyze)
            .field("use_payloads", &self.use_payloads)
            .finish_non_exhaustive()
    }
}

impl Default for WeightedSpanTermExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl WeightedSpanTermExtractor {
    /// Equivalent to `new WeightedSpanTermExtractor()`.
    pub fn new() -> Self {
        Self::with_default_field(None)
    }

    /// Equivalent to `new WeightedSpanTermExtractor(String)`.
    pub fn with_default_field(default_field: Option<&str>) -> Self {
        Self {
            field_name: None,
            token_stream: None,
            default_field: default_field.map(str::to_string),
            expand_multi_term_query: false,
            cached_token_stream: false,
            wrap_to_caching: true,
            max_doc_chars_to_analyze: 0,
            use_payloads: false,
            internal_reader: None,
            overrides: None,
        }
    }

    /// Sets the overridden protected methods, as a Java subclass does.
    pub fn set_overrides(&mut self, overrides: Arc<dyn WeightedSpanTermExtractorOverrides>) {
        self.overrides = Some(overrides);
    }

    /// Fills a `Map` with `WeightedSpanTerm`s using the terms from the
    /// supplied `Query`.
    ///
    /// Equivalent to the protected `extract(Query, float, Map)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn extract(
        &mut self,
        query: &Arc<dyn Query>,
        boost: f32,
        terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        match self.overrides.clone() {
            Some(overrides) => overrides.extract(self, query, boost, terms),
            None => self.super_extract(query, boost, terms),
        }
    }

    /// The base class's `extract(Query, float, Map)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors, and the `IllegalArgumentException` of a phrase
    /// whose terms are not of one field.
    pub fn super_extract(
        &mut self,
        query: &Arc<dyn Query>,
        boost: f32,
        terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        let any = query.as_any();
        if let Some(boost_query) = any.downcast_ref::<BoostQuery>() {
            self.extract(
                &boost_query.get_query(),
                boost * boost_query.get_boost(),
                terms,
            )?;
        } else if let Some(boolean_query) = any.downcast_ref::<BooleanQuery>() {
            for clause in boolean_query.clauses() {
                if !clause.is_prohibited() {
                    self.extract(clause.query(), boost, terms)?;
                }
            }
        } else if let Some(phrase_query) = any.downcast_ref::<PhraseQuery>() {
            let phrase_query_terms = phrase_query.get_terms();
            if phrase_query_terms.len() == 1 {
                self.extract_weighted_span_terms(
                    terms,
                    Arc::new(SpanTermQuery::new(phrase_query_terms[0].clone())),
                    boost,
                )?;
            } else {
                let mut clauses: Vec<Arc<dyn SpanQuery>> =
                    Vec::with_capacity(phrase_query_terms.len());
                for term in phrase_query_terms {
                    clauses.push(Arc::new(SpanTermQuery::new(term.clone())));
                }

                // sum position increments beyond 1
                let mut position_gaps = 0;
                let positions = phrase_query.get_positions();
                if positions.len() >= 2 {
                    // positions are in increasing order. max(0,...) is just
                    // a safeguard.
                    position_gaps = 0.max(
                        positions[positions.len() - 1] - positions[0] - positions.len() as i32 + 1,
                    );
                }

                // if original slop is 0 then require inOrder
                let inorder = phrase_query.get_slop() == 0;

                let sp =
                    SpanNearQuery::new(clauses, phrase_query.get_slop() + position_gaps, inorder)?;
                self.extract_weighted_span_terms(terms, Arc::new(sp), boost)?;
            }
        } else if let Some(iodv) = any.downcast_ref::<IndexOrDocValuesQuery>() {
            let index_query = Arc::clone(iodv.get_index_query());
            self.extract(&index_query, boost, terms)?;
        } else if query.as_term_query().is_some() || any.is::<SynonymQuery>() {
            self.extract_weighted_terms(terms, query, boost)?;
        } else if let Some(span_query) = as_span_query(&**query) {
            let span_query = cast_span_query(span_query)?;
            self.extract_weighted_span_terms(terms, span_query, boost)?;
        } else if let Some(csq) = any.downcast_ref::<ConstantScoreQuery>() {
            let q = csq.get_query();
            self.extract(&q, boost, terms)?;
        } else if any.is::<CommonTermsQuery>() {
            // specialized since rewriting would change the result query
            // this query is index sensitive.
            self.extract_weighted_terms(terms, query, boost)?;
        } else if let Some(dmq) = any.downcast_ref::<DisjunctionMaxQuery>() {
            for clause in dmq.get_disjuncts() {
                self.extract(clause, boost, terms)?;
            }
        } else if let Some(mpq) = any.downcast_ref::<MultiPhraseQuery>() {
            let term_arrays = mpq.get_term_arrays();
            let positions = mpq.get_positions();
            if !positions.is_empty() {
                let mut max_position = positions[positions.len() - 1];
                for &position in &positions[..positions.len() - 1] {
                    if position > max_position {
                        max_position = position;
                    }
                }

                let mut disjunct_lists: Vec<Option<Vec<Arc<dyn SpanQuery>>>> =
                    vec![None; (max_position + 1) as usize];
                let mut distinct_positions = 0;

                for (i, term_array) in term_arrays.iter().enumerate() {
                    let slot = &mut disjunct_lists[positions[i] as usize];
                    if slot.is_none() {
                        *slot = Some(Vec::with_capacity(term_array.len()));
                        distinct_positions += 1;
                    }
                    let disjuncts = slot.as_mut().expect("INVARIANT: set above");
                    for a_term_array in term_array {
                        disjuncts.push(Arc::new(SpanTermQuery::new(a_term_array.clone())));
                    }
                }

                let mut position_gaps = 0;
                let mut clauses: Vec<Arc<dyn SpanQuery>> = Vec::with_capacity(distinct_positions);
                for disjuncts in disjunct_lists {
                    match disjuncts {
                        Some(disjuncts) => clauses.push(Arc::new(SpanOrQuery::new(disjuncts)?)),
                        None => position_gaps += 1,
                    }
                }

                if clauses.len() == 1 {
                    let clause = clauses.pop().expect("INVARIANT: one clause");
                    self.extract_weighted_span_terms(terms, clause, boost)?;
                } else {
                    let slop = mpq.get_slop();
                    let inorder = slop == 0;

                    let sp = SpanNearQuery::new(clauses, slop + position_gaps, inorder)?;
                    self.extract_weighted_span_terms(terms, Arc::new(sp), boost)?;
                }
            }
        } else if any.is::<MatchAllDocsQuery>() {
            // nothing
        } else if let Some(fsq) = any.downcast_ref::<FunctionScoreQuery>() {
            let wrapped = Arc::clone(fsq.get_wrapped_query());
            self.extract(&wrapped, boost, terms)?;
        } else if self.is_query_unsupported(&**query) {
            // nothing
        } else {
            if let Some(mtq) = query.as_multi_term_query() {
                if !self.expand_multi_term_query
                    || !self.field_name_comparator(Some(mtq.get_field()))
                {
                    return Ok(());
                }
            }
            let reader: Arc<dyn IndexReader> =
                self.get_leaf_context()?.leaf_reader().as_index_reader();
            let rewritten: Option<Arc<dyn Query>> = match query.as_multi_term_query() {
                Some(mtq) => Some(
                    scoring_boolean_rewrite()
                        .rewrite(&IndexSearcher::new(reader)?, mtq.to_multi_term_query_arc())?,
                ),
                None => query.rewrite(&IndexSearcher::new(reader)?)?,
            };
            match rewritten {
                Some(rewritten) if !Arc::ptr_eq(&rewritten, query) => {
                    self.extract(&rewritten, boost, terms)?;
                }
                _ => self.extract_unknown_query(query, terms)?,
            }
        }
        Ok(())
    }

    /// Equivalent to the protected `isQueryUnsupported(Class)`: the query
    /// stands for its class.
    pub fn is_query_unsupported(&self, query: &dyn Query) -> bool {
        match &self.overrides {
            Some(overrides) => overrides.is_query_unsupported(self, query),
            None => self.super_is_query_unsupported(query),
        }
    }

    /// The base class's `isQueryUnsupported(Class)`.
    pub fn super_is_query_unsupported(&self, query: &dyn Query) -> bool {
        // spatial queries do not support highlighting:
        let name = query.java_class_name();
        if name.starts_with("org.apache.lucene.spatial.") {
            return true;
        }
        // spatial3d queries are also not supported:
        if name.starts_with("org.apache.lucene.spatial3d.") {
            return true;
        }
        false
    }

    /// Equivalent to the protected `extractUnknownQuery(Query, Map)`.
    ///
    /// # Errors
    ///
    /// What an override raises.
    pub fn extract_unknown_query(
        &mut self,
        query: &Arc<dyn Query>,
        terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        match self.overrides.clone() {
            Some(overrides) => overrides.extract_unknown_query(self, query, terms),
            // for sub-classing to extract custom queries
            None => Ok(()),
        }
    }

    /// Fills a `Map` with `WeightedSpanTerm`s using the terms from the
    /// supplied `SpanQuery`.
    ///
    /// Equivalent to the protected `extractWeightedSpanTerms(Map, SpanQuery,
    /// float)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn extract_weighted_span_terms(
        &mut self,
        terms: &mut PositionCheckingMap,
        span_query: Arc<dyn SpanQuery>,
        boost: f32,
    ) -> Result<()> {
        match self.overrides.clone() {
            Some(overrides) => {
                overrides.extract_weighted_span_terms(self, terms, span_query, boost)
            }
            None => self.super_extract_weighted_span_terms(terms, span_query, boost),
        }
    }

    /// The base class's `extractWeightedSpanTerms(Map, SpanQuery, float)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn super_extract_weighted_span_terms(
        &mut self,
        terms: &mut PositionCheckingMap,
        span_query: Arc<dyn SpanQuery>,
        boost: f32,
    ) -> Result<()> {
        let mut query_field_names: HashSet<Option<String>> = HashSet::new();
        self.collect_span_query_fields(&*span_query, &mut query_field_names);
        if let Some(field_name) = &self.field_name {
            if !query_field_names.contains(&Some(field_name.clone()))
                && self
                    .default_field
                    .as_ref()
                    .is_none_or(|df| !query_field_names.contains(&Some(df.clone())))
            {
                return Ok(());
            }
        }

        // rewrite possible MultiTermQueries to SpanQuery
        let must_rewrite_query = self.must_rewrite_query(&*span_query);
        let context = self.get_leaf_context()?;
        let mut searcher = IndexSearcher::from_context(context.clone(), None)?;
        searcher.set_query_cache(None);
        let query: Arc<dyn SpanQuery> = if must_rewrite_query {
            cast_span_query(&*searcher.rewrite(span_query as Arc<dyn Query>)?)?
        } else {
            span_query
        };

        let mut non_weighted_terms: HashSet<Term> = HashSet::new();
        query.visit(&mut term_collector(&mut non_weighted_terms));
        if non_weighted_terms.is_empty() {
            return Ok(());
        }

        let mut span_positions: Vec<PositionSpan> = Vec::new();

        let rewritten = cast_span_query(&*searcher.rewrite(query as Arc<dyn Query>)?)?;
        let w = rewritten.create_span_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
        let Some(mut spans) = w.get_spans(&context, Postings::Positions)? else {
            return Ok(());
        };

        let accept_docs = context.leaf_reader().get_live_docs();

        // collect span positions
        while spans.next_doc()? != NO_MORE_DOCS {
            if let Some(accept_docs) = &accept_docs {
                if !accept_docs.get(spans.doc_id() as usize) {
                    continue;
                }
            }

            while spans.next_start_position()? != NO_MORE_POSITIONS {
                span_positions.push(PositionSpan::new(
                    spans.start_position(),
                    spans.end_position() - 1,
                ));
            }
        }

        if span_positions.is_empty() {
            // no spans found
            return Ok(());
        }

        for query_term in &non_weighted_terms {
            if self.field_name_comparator(query_term.field_opt()) {
                let text = query_term.text();
                match terms.get_mut(&text) {
                    None => {
                        let mut weighted_span_term = WeightedSpanTerm::new(boost, text.clone());
                        weighted_span_term.add_position_spans(&span_positions);
                        weighted_span_term.position_sensitive = true;
                        terms.put(text, weighted_span_term);
                    }
                    Some(weighted_span_term) => {
                        if !span_positions.is_empty() {
                            weighted_span_term.add_position_spans(&span_positions);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Fills a `Map` with `WeightedSpanTerm`s using the terms from the
    /// supplied `Query`.
    ///
    /// Equivalent to the protected `extractWeightedTerms(Map, Query,
    /// float)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn extract_weighted_terms(
        &mut self,
        terms: &mut PositionCheckingMap,
        query: &Arc<dyn Query>,
        boost: f32,
    ) -> Result<()> {
        match self.overrides.clone() {
            Some(overrides) => overrides.extract_weighted_terms(self, terms, query, boost),
            None => self.super_extract_weighted_terms(terms, query, boost),
        }
    }

    /// The base class's `extractWeightedTerms(Map, Query, float)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn super_extract_weighted_terms(
        &mut self,
        terms: &mut PositionCheckingMap,
        query: &Arc<dyn Query>,
        boost: f32,
    ) -> Result<()> {
        let mut non_weighted_terms: HashSet<Term> = HashSet::new();
        let searcher = IndexSearcher::from_context(self.get_leaf_context()?, None)?;
        searcher
            .rewrite(Arc::clone(query))?
            .visit(&mut term_collector(&mut non_weighted_terms));

        for query_term in &non_weighted_terms {
            if self.field_name_comparator(query_term.field_opt()) {
                let weighted_span_term = WeightedSpanTerm::new(boost, query_term.text());
                terms.put(query_term.text(), weighted_span_term);
            }
        }
        Ok(())
    }

    /// Necessary to implement matches for queries against `defaultField`.
    ///
    /// Equivalent to the protected `fieldNameComparator(String)`.
    pub fn field_name_comparator(&self, field_name_to_check: Option<&str>) -> bool {
        match &self.overrides {
            Some(overrides) => overrides.field_name_comparator(self, field_name_to_check),
            None => self.super_field_name_comparator(field_name_to_check),
        }
    }

    /// The base class's `fieldNameComparator(String)`.
    pub fn super_field_name_comparator(&self, field_name_to_check: Option<&str>) -> bool {
        self.field_name.is_none()
            || self.field_name.as_deref() == field_name_to_check
            || (self.default_field.is_some()
                && self.default_field.as_deref() == field_name_to_check)
    }

    /// Equivalent to the protected `getLeafContext()`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn get_leaf_context(&mut self) -> Result<Arc<LeafReaderContext>> {
        match self.overrides.clone() {
            Some(overrides) => overrides.get_leaf_context(self),
            None => self.super_get_leaf_context(),
        }
    }

    /// The base class's `getLeafContext()`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn super_get_leaf_context(&mut self) -> Result<Arc<LeafReaderContext>> {
        if self.internal_reader.is_none() {
            let mut cache_it = self.wrap_to_caching
                && !self
                    .token_stream
                    .as_ref()
                    .and_then(|ts| ts.as_any())
                    .is_some_and(|any| any.is::<CachingTokenFilter>());

            // If it's from term vectors, simply wrap the underlying Terms in a
            // reader
            let term_vector_terms: Option<Arc<dyn Terms>> = self
                .token_stream
                .as_ref()
                .and_then(|ts| ts.as_any())
                .and_then(|any| any.downcast_ref::<TokenStreamFromTermVector>())
                .map(|ts| Arc::clone(ts.get_term_vector_terms()));
            if let Some(term_vector_terms) = term_vector_terms {
                cache_it = false;
                if term_vector_terms.has_positions() && term_vector_terms.has_offsets() {
                    self.internal_reader = Some(Arc::new(TermVectorLeafReader::new(
                        DelegatingLeafReader::FIELD_NAME,
                        term_vector_terms,
                    )));
                }
            }

            // Use MemoryIndex (index/invert this tokenStream now)
            if self.internal_reader.is_none() {
                let mut indexer = MemoryIndex::with_offsets_and_payloads(true, self.use_payloads); // offsets and payloads
                if cache_it {
                    debug_assert!(!self.cached_token_stream);
                    let token_stream = self
                        .token_stream
                        .take()
                        .ok_or_else(LuceneError::null_pointer)?;
                    let mut caching = CachingTokenFilter::new(Box::new(
                        OffsetLimitTokenFilter::new(token_stream, self.max_doc_chars_to_analyze),
                    ));
                    self.cached_token_stream = true;
                    let added = indexer
                        .add_field_token_stream(DelegatingLeafReader::FIELD_NAME, &mut caching);
                    self.token_stream = Some(Box::new(caching));
                    added?;
                } else {
                    let token_stream = self
                        .token_stream
                        .as_deref_mut()
                        .ok_or_else(LuceneError::null_pointer)?;
                    let mut filter = OffsetLimitTokenFilter::new(
                        Box::new(BorrowedTokenStream(token_stream)),
                        self.max_doc_chars_to_analyze,
                    );
                    indexer
                        .add_field_token_stream(DelegatingLeafReader::FIELD_NAME, &mut filter)?;
                }
                let searcher = indexer.create_searcher()?;
                // MEM index has only atomic ctx
                let leaf = Arc::clone(searcher.get_top_reader_context())
                    .as_leaf()
                    .ok_or_else(|| {
                        LuceneError::unchecked(
                            "java.lang.ClassCastException",
                            "the MemoryIndex top-level context is not a LeafReaderContext",
                        )
                    })?;
                self.internal_reader = Some(leaf.leaf_reader());
            }

            // Now wrap it so we always use a common field.
            let inner = self.internal_reader.take().expect("INVARIANT: set above");
            self.internal_reader = Some(Arc::new(DelegatingLeafReader::new(inner)?));
        }

        let reader = self.internal_reader.as_ref().expect("INVARIANT: set above");
        Arc::clone(reader)
            .as_index_reader()
            .get_context()?
            .as_leaf()
            .ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "the context of a LeafReader is not a LeafReaderContext",
                )
            })
    }

    /// Creates a Map of `WeightedSpanTerm`s from the given `Query` and
    /// `TokenStream`.
    ///
    /// Equivalent to `getWeightedSpanTerms(Query, float, TokenStream)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn get_weighted_span_terms(
        &mut self,
        query: &Arc<dyn Query>,
        boost: f32,
        token_stream: Box<dyn TokenStream>,
    ) -> Result<PositionCheckingMap> {
        self.get_weighted_span_terms_for_field(query, boost, token_stream, None)
    }

    /// Creates a Map of `WeightedSpanTerm`s from the given `Query` and
    /// `TokenStream`; `field_name` restricts the terms used based on field
    /// name (`None` for all).
    ///
    /// The extractor keeps `token_stream`; [`take_token_stream`](Self::take_token_stream)
    /// hands it back (wrapped in a caching filter when
    /// [`is_cached_token_stream`](Self::is_cached_token_stream)).
    ///
    /// Equivalent to `getWeightedSpanTerms(Query, float, TokenStream,
    /// String)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn get_weighted_span_terms_for_field(
        &mut self,
        query: &Arc<dyn Query>,
        boost: f32,
        token_stream: Box<dyn TokenStream>,
        field_name: Option<&str>,
    ) -> Result<PositionCheckingMap> {
        self.field_name = field_name.map(str::to_string);

        let mut terms = PositionCheckingMap::new();
        self.token_stream = Some(token_stream);
        let extracted = self.extract(query, boost, &mut terms);
        let closed = self.close_internal_reader();
        extracted?;
        closed?;

        Ok(terms)
    }

    /// Creates a Map of `WeightedSpanTerm`s from the given `Query` and
    /// `TokenStream`. Uses a supplied `IndexReader` to properly weight terms
    /// (for gradient highlighting).
    ///
    /// Equivalent to `getWeightedSpanTermsWithScores(Query, float,
    /// TokenStream, String, IndexReader)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn get_weighted_span_terms_with_scores(
        &mut self,
        query: &Arc<dyn Query>,
        boost: f32,
        token_stream: Box<dyn TokenStream>,
        field_name: Option<&str>,
        reader: &dyn IndexReader,
    ) -> Result<PositionCheckingMap> {
        self.field_name = field_name.map(str::to_string);
        self.token_stream = Some(token_stream);

        let mut terms = PositionCheckingMap::new();
        self.extract(query, boost, &mut terms)?;

        let total_num_docs = reader.max_doc();
        let weighted_terms: Vec<String> = terms.keys().cloned().collect();
        let scored = (|| -> Result<()> {
            for key in &weighted_terms {
                let weighted_span_term = terms.get_mut(key).expect("INVARIANT: a key of the map");
                let doc_freq = reader.doc_freq(&Term::from_text_nullable_field(
                    self.field_name.as_deref(),
                    weighted_span_term.get_term(),
                ))?;
                // IDF algorithm taken from ClassicSimilarity class
                let idf = ((total_num_docs as f64 / (doc_freq as f64 + 1.0)).ln() + 1.0) as f32;
                weighted_span_term.weight *= idf;
            }
            Ok(())
        })();
        let closed = self.close_internal_reader();
        scored?;
        closed?;

        Ok(terms)
    }

    /// `IOUtils.close(internalReader)`: closes the internal reader, which
    /// stays set (closed), as Java leaves the field set.
    fn close_internal_reader(&self) -> Result<()> {
        match &self.internal_reader {
            Some(reader) => Arc::clone(reader).as_index_reader().close(),
            None => Ok(()),
        }
    }

    /// Equivalent to the protected `collectSpanQueryFields(SpanQuery, Set)`;
    /// a span query without a field adds `None` (Java's `null`).
    pub fn collect_span_query_fields(
        &self,
        span_query: &dyn SpanQuery,
        field_names: &mut HashSet<Option<String>>,
    ) {
        match &self.overrides {
            Some(overrides) => overrides.collect_span_query_fields(self, span_query, field_names),
            None => self.super_collect_span_query_fields(span_query, field_names),
        }
    }

    /// The base class's `collectSpanQueryFields(SpanQuery, Set)`.
    pub fn super_collect_span_query_fields(
        &self,
        span_query: &dyn SpanQuery,
        field_names: &mut HashSet<Option<String>>,
    ) {
        let any = span_query.as_any();
        if let Some(q) = any.downcast_ref::<FieldMaskingSpanQuery>() {
            self.collect_span_query_fields(&**q.get_masked_query(), field_names);
        } else if let Some(q) = any.downcast_ref::<SpanFirstQuery>() {
            self.collect_span_query_fields(&**q.get_match(), field_names);
        } else if let Some(q) = any.downcast_ref::<SpanNearQuery>() {
            for clause in q.get_clauses() {
                self.collect_span_query_fields(&*clause, field_names);
            }
        } else if let Some(q) = any.downcast_ref::<SpanNotQuery>() {
            self.collect_span_query_fields(&**q.get_include(), field_names);
        } else if let Some(q) = any.downcast_ref::<SpanOrQuery>() {
            for clause in q.get_clauses() {
                self.collect_span_query_fields(&*clause, field_names);
            }
        } else {
            field_names.insert(span_query.get_field().map(str::to_string));
        }
    }

    /// Equivalent to the protected `mustRewriteQuery(SpanQuery)`.
    pub fn must_rewrite_query(&self, span_query: &dyn SpanQuery) -> bool {
        match &self.overrides {
            Some(overrides) => overrides.must_rewrite_query(self, span_query),
            None => self.super_must_rewrite_query(span_query),
        }
    }

    /// The base class's `mustRewriteQuery(SpanQuery)`.
    pub fn super_must_rewrite_query(&self, span_query: &dyn SpanQuery) -> bool {
        let any = span_query.as_any();
        if !self.expand_multi_term_query {
            false // Will throw UnsupportedOperationException in case of a SpanRegexQuery.
        } else if let Some(q) = any.downcast_ref::<FieldMaskingSpanQuery>() {
            self.must_rewrite_query(&**q.get_masked_query())
        } else if let Some(q) = any.downcast_ref::<SpanFirstQuery>() {
            self.must_rewrite_query(&**q.get_match())
        } else if let Some(q) = any.downcast_ref::<SpanNearQuery>() {
            for clause in q.get_clauses() {
                if self.must_rewrite_query(&*clause) {
                    return true;
                }
            }
            false
        } else if let Some(span_not_query) = any.downcast_ref::<SpanNotQuery>() {
            self.must_rewrite_query(&**span_not_query.get_include())
                || self.must_rewrite_query(&**span_not_query.get_exclude())
        } else if let Some(q) = any.downcast_ref::<SpanOrQuery>() {
            for clause in q.get_clauses() {
                if self.must_rewrite_query(&*clause) {
                    return true;
                }
            }
            false
        } else {
            !any.is::<SpanTermQuery>()
        }
    }

    /// Equivalent to `getExpandMultiTermQuery()`.
    pub fn get_expand_multi_term_query(&self) -> bool {
        self.expand_multi_term_query
    }

    /// Equivalent to `setExpandMultiTermQuery(boolean)`.
    pub fn set_expand_multi_term_query(&mut self, expand_multi_term_query: bool) {
        self.expand_multi_term_query = expand_multi_term_query;
    }

    /// Equivalent to `isUsePayloads()`.
    pub fn is_use_payloads(&self) -> bool {
        self.use_payloads
    }

    /// Equivalent to `setUsePayloads(boolean)`.
    pub fn set_use_payloads(&mut self, use_payloads: bool) {
        self.use_payloads = use_payloads;
    }

    /// Equivalent to `isCachedTokenStream()`.
    pub fn is_cached_token_stream(&self) -> bool {
        self.cached_token_stream
    }

    /// Returns the tokenStream which may have been wrapped in a
    /// CachingTokenFilter. It's an AttributeSource that may contain the same
    /// attributes as the passed in tokenStream, and the anlyzed tokens may be
    /// read again from it.
    ///
    /// Equivalent to `getTokenStream()`.
    pub fn get_token_stream(&mut self) -> Option<&mut (dyn TokenStream + 'static)> {
        debug_assert!(self.token_stream.is_some());
        self.token_stream.as_deref_mut()
    }

    /// [`get_token_stream`](Self::get_token_stream), handing the stream back
    /// to the caller, which owns it in Rust.
    pub fn take_token_stream(&mut self) -> Option<Box<dyn TokenStream>> {
        self.token_stream.take()
    }

    /// By default, `TokenStream`s that are not of the type
    /// `CachingTokenFilter` are wrapped in a `CachingTokenFilter` to ensure an
    /// efficient reset - if you are already using a different caching
    /// `TokenStream` impl and you don't want it to be wrapped, set this to
    /// false. This setting is ignored when a term vector based TokenStream is
    /// supplied, since it can be reset efficiently.
    ///
    /// Equivalent to `setWrapIfNotCachingTokenFilter(boolean)`.
    pub fn set_wrap_if_not_caching_token_filter(&mut self, wrap: bool) {
        self.wrap_to_caching = wrap;
    }

    /// A threshold of number of characters to analyze. When a TokenStream
    /// based on term vectors with offsets and positions are supplied, this
    /// setting does not apply.
    ///
    /// Equivalent to the protected final `setMaxDocCharsToAnalyze(int)`.
    pub fn set_max_doc_chars_to_analyze(&mut self, max_doc_chars_to_analyze: i32) {
        self.max_doc_chars_to_analyze = max_doc_chars_to_analyze;
    }
}

/// This reader will just delegate every call to a single field in the
/// wrapped LeafReader. This way we only need to build this field once rather
/// than N-Times
///
/// Equivalent to the package-private nested class
/// `WeightedSpanTermExtractor.DelegatingLeafReader`.
pub(crate) struct DelegatingLeafReader {
    base: FilterLeafReader,
}

impl DelegatingLeafReader {
    pub(crate) const FIELD_NAME: &'static str = "shadowed_field";

    fn new(input: Arc<dyn LeafReader>) -> Result<Self> {
        Ok(Self {
            base: FilterLeafReader::new(input)?,
        })
    }

    fn input(&self) -> &Arc<dyn LeafReader> {
        self.base.get_delegate()
    }
}

impl fmt::Debug for DelegatingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DelegatingLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for DelegatingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl LeafReader for DelegatingLeafReader {
    crate::forward_leaf_reader!(filter base;
        core,
        filter_leaf_reader_delegate,
        do_close,
        term_vectors,
        num_docs,
        max_doc,
        stored_fields,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_live_docs,
        get_point_values,
        check_integrity,
        get_meta_data,
    );

    fn get_field_infos(&self) -> FieldInfos {
        let field_info = self
            .base
            .get_field_infos()
            .field_info(Self::FIELD_NAME)
            .cloned()
            .expect("NullPointerException: the shadowed field has a FieldInfo");
        FieldInfos::new(vec![field_info])
            .expect("INVARIANT: a single field is a valid FieldInfos")
            .with_field_info_name_override(Self::FIELD_NAME)
    }

    fn terms(&self, _field: &str) -> Result<Option<Arc<dyn Terms>>> {
        self.input().terms(Self::FIELD_NAME)
    }

    fn get_numeric_doc_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.input().get_numeric_doc_values(Self::FIELD_NAME)
    }

    fn get_binary_doc_values(&self, _field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.input().get_binary_doc_values(Self::FIELD_NAME)
    }

    fn get_sorted_doc_values(&self, _field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.input().get_sorted_doc_values(Self::FIELD_NAME)
    }

    fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.input().get_norm_values(Self::FIELD_NAME)
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// This class makes sure that if both position sensitive and insensitive
/// versions of the same term are added, the position insensitive one wins.
///
/// Equivalent to the protected nested class
/// `WeightedSpanTermExtractor.PositionCheckingMap<K>`, a `HashMap` keyed by
/// the term text, whose `put` and `putAll` carry the position insensitivity
/// of a replaced term.
#[derive(Debug, Clone, Default)]
pub struct PositionCheckingMap {
    map: HashMap<String, WeightedSpanTerm>,
}

impl PositionCheckingMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty map sized for `capacity` terms (`HashMap.newHashMap`).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
        }
    }

    /// Equivalent to `putAll(Map)`.
    pub fn put_all(&mut self, m: impl IntoIterator<Item = (String, WeightedSpanTerm)>) {
        for (key, value) in m {
            self.put(key, value);
        }
    }

    /// Equivalent to `put(K, WeightedSpanTerm)`.
    pub fn put(&mut self, key: String, mut new_term: WeightedSpanTerm) -> Option<WeightedSpanTerm> {
        if let Some(prev_term) = self.map.get(&key) {
            if !prev_term.position_sensitive {
                new_term.position_sensitive = false;
            }
        }
        self.map.insert(key, new_term)
    }

    /// `get(Object)`.
    pub fn get(&self, key: &str) -> Option<&WeightedSpanTerm> {
        self.map.get(key)
    }

    /// `get(Object)`, for a caller that changes the term Java returns by
    /// reference.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut WeightedSpanTerm> {
        self.map.get_mut(key)
    }

    /// `keySet()`.
    pub fn keys(&self) -> hash_map::Keys<'_, String, WeightedSpanTerm> {
        self.map.keys()
    }

    /// `values()`.
    pub fn values(&self) -> hash_map::Values<'_, String, WeightedSpanTerm> {
        self.map.values()
    }

    /// `entrySet()`.
    pub fn iter(&self) -> hash_map::Iter<'_, String, WeightedSpanTerm> {
        self.map.iter()
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.map.len()
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// `containsKey(Object)`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// `remove(Object)`.
    pub fn remove(&mut self, key: &str) -> Option<WeightedSpanTerm> {
        self.map.remove(key)
    }
}

impl IntoIterator for PositionCheckingMap {
    type Item = (String, WeightedSpanTerm);
    type IntoIter = hash_map::IntoIter<String, WeightedSpanTerm>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.into_iter()
    }
}

#[allow(unused_imports)]
use SpanPositionCheckQuery as _;
