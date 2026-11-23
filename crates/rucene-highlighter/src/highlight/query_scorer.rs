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

//! Port of `QueryScorer.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::any::Any;
use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::analysis::tokenattributes::{CharTermAttribute, PositionIncrementAttribute};
use rucene::analysis::{AttributeHandle, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::index::IndexReader;
use rucene::search::query::Query;

use crate::highlight::scorer::Scorer;
use crate::highlight::text_fragment::TextFragment;
use crate::highlight::weighted_span_term::WeightedSpanTerm;
use crate::highlight::weighted_span_term_extractor::{
    PositionCheckingMap, WeightedSpanTermExtractor,
};
use crate::java::term_text;

/// The factory of [`QueryScorer::new_term_extractor`]: Java's protected
/// `newTermExtractor(String)`, which a subclass overrides.
pub type TermExtractorFactory =
    Arc<dyn Fn(Option<&str>) -> WeightedSpanTermExtractor + Send + Sync>;

struct State {
    total_score: f32,
    found_terms: Option<HashSet<String>>,
    field_weighted_span_terms: Option<PositionCheckingMap>,
    max_term_weight: f32,
    position: i32,
    default_field: Option<String>,
    term_att: Option<AttributeHandle<dyn CharTermAttribute>>,
    pos_inc_att: Option<AttributeHandle<dyn PositionIncrementAttribute>>,
    expand_multi_term_query: bool,
    query: Option<Arc<dyn Query>>,
    field: Option<String>,
    reader: Option<Arc<dyn IndexReader>>,
    skip_init_extractor: bool,
    wrap_to_caching: bool,
    max_chars_to_analyze: i32,
    use_payloads: bool,
    term_extractor_factory: Option<TermExtractorFactory>,
}

/// `Scorer` implementation which scores text fragments by the number of
/// unique query terms found. This class converts appropriate `Query`s to
/// `SpanQuery`s and attempts to score only those terms that participated in
/// generating the 'hit' on the document.
///
/// Equivalent to `org.apache.lucene.search.highlight.QueryScorer`. A value is
/// a handle onto one scorer, as a Java reference is: a clone is the same
/// scorer, so a `Highlighter` and a `SimpleSpanFragmenter` share it as Java
/// shares the object.
#[derive(Clone)]
pub struct QueryScorer {
    state: Arc<Mutex<State>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl fmt::Debug for QueryScorer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = lock(&self.state);
        f.debug_struct("QueryScorer")
            .field("query", &state.query)
            .field("field", &state.field)
            .field("default_field", &state.default_field)
            .finish_non_exhaustive()
    }
}

impl QueryScorer {
    fn with_state(
        query: Option<Arc<dyn Query>>,
        field: Option<&str>,
        reader: Option<Arc<dyn IndexReader>>,
        default_field: Option<&str>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                total_score: 0.0,
                found_terms: None,
                field_weighted_span_terms: None,
                max_term_weight: 0.0,
                position: -1,
                default_field: default_field.map(str::to_string),
                term_att: None,
                pos_inc_att: None,
                expand_multi_term_query: true,
                query,
                field: field.map(str::to_string),
                reader,
                skip_init_extractor: false,
                wrap_to_caching: true,
                max_chars_to_analyze: 0,
                use_payloads: false,
                term_extractor_factory: None,
            })),
        }
    }

    /// Equivalent to `new QueryScorer(Query)`: `query` is the Query to use
    /// for highlighting.
    pub fn new(query: Arc<dyn Query>) -> Self {
        Self::with_state(Some(query), None, None, None)
    }

    /// Equivalent to `new QueryScorer(Query, String)`: `field` is the Field
    /// to highlight - pass `None` to ignore fields.
    pub fn with_field(query: Arc<dyn Query>, field: Option<&str>) -> Self {
        Self::with_state(Some(query), field, None, None)
    }

    /// Equivalent to `new QueryScorer(Query, IndexReader, String)`: `reader`
    /// is used for term idf.
    pub fn with_reader(
        query: Arc<dyn Query>,
        reader: Option<Arc<dyn IndexReader>>,
        field: Option<&str>,
    ) -> Self {
        Self::with_state(Some(query), field, reader, None)
    }

    /// Equivalent to `new QueryScorer(Query, IndexReader, String, String)`:
    /// `default_field` is the default field for queries with the field name
    /// unspecified.
    pub fn with_reader_and_default_field(
        query: Arc<dyn Query>,
        reader: Option<Arc<dyn IndexReader>>,
        field: Option<&str>,
        default_field: Option<&str>,
    ) -> Self {
        Self::with_state(Some(query), field, reader, default_field)
    }

    /// Equivalent to `new QueryScorer(Query, String, String)`.
    pub fn with_field_and_default_field(
        query: Arc<dyn Query>,
        field: Option<&str>,
        default_field: Option<&str>,
    ) -> Self {
        Self::with_state(Some(query), field, None, default_field)
    }

    /// Equivalent to `new QueryScorer(WeightedSpanTerm[])`: `weighted_terms`
    /// is an array of terms to use for highlighting.
    pub fn from_weighted_span_terms(weighted_terms: Vec<WeightedSpanTerm>) -> Self {
        let this = Self::with_state(None, None, None, None);
        {
            let mut state = lock(&this.state);
            let mut map = PositionCheckingMap::with_capacity(weighted_terms.len());
            let mut max_term_weight = 0.0f32;
            for weighted_term in weighted_terms {
                let existing_term = map.get(weighted_term.get_term());
                if existing_term.is_none_or(|existing| existing.weight < weighted_term.weight) {
                    // if a term is defined more than once, always use the
                    // highest scoring weight
                    max_term_weight = max_term_weight.max(weighted_term.get_weight());
                    map.put(weighted_term.get_term().to_string(), weighted_term);
                }
            }
            state.field_weighted_span_terms = Some(map);
            state.max_term_weight = max_term_weight;
            state.skip_init_extractor = true;
        }
        this
    }

    /// Sets the factory of the extractor `init` uses, as a Java subclass
    /// overrides the protected `newTermExtractor(String)`.
    pub fn set_term_extractor_factory(&mut self, factory: TermExtractorFactory) {
        lock(&self.state).term_extractor_factory = Some(factory);
    }

    /// Returns the highest weighted term. If you are using gradient
    /// highlights you should use this value to work out the max value for the
    /// gradient.
    ///
    /// Equivalent to `getMaxTermWeight()`.
    pub fn get_max_term_weight(&self) -> f32 {
        lock(&self.state).max_term_weight
    }

    /// Retrieve the `WeightedSpanTerm` for the specified token. Useful for
    /// passing Span information to a `Fragmenter`.
    ///
    /// Equivalent to `getWeightedSpanTerm(String)`; the port answers a copy of
    /// the term Java returns by reference.
    ///
    /// # Panics
    ///
    /// Before the terms are known (`init` or the array constructor), where
    /// Java throws `NullPointerException`.
    pub fn get_weighted_span_term(&self, token: &str) -> Option<WeightedSpanTerm> {
        lock(&self.state)
            .field_weighted_span_terms
            .as_ref()
            .expect("NullPointerException: fieldWeightedSpanTerms is null")
            .get(token)
            .cloned()
    }

    /// Equivalent to the protected `newTermExtractor(String)`.
    pub fn new_term_extractor(&self, default_field: Option<&str>) -> WeightedSpanTermExtractor {
        let factory = lock(&self.state).term_extractor_factory.clone();
        match factory {
            Some(factory) => factory(default_field),
            None => WeightedSpanTermExtractor::with_default_field(default_field),
        }
    }

    /// Equivalent to the private `initExtractor(TokenStream)`.
    fn init_extractor(
        &self,
        state: &mut State,
        token_stream: Box<dyn TokenStream>,
    ) -> Result<Box<dyn TokenStream>> {
        let mut qse = match &state.term_extractor_factory {
            Some(factory) => factory(state.default_field.as_deref()),
            None => WeightedSpanTermExtractor::with_default_field(state.default_field.as_deref()),
        };
        qse.set_max_doc_chars_to_analyze(state.max_chars_to_analyze);
        qse.set_expand_multi_term_query(state.expand_multi_term_query);
        qse.set_wrap_if_not_caching_token_filter(state.wrap_to_caching);
        qse.set_use_payloads(state.use_payloads);
        let query = state
            .query
            .clone()
            .expect("INVARIANT: a scorer that runs its extractor has a query");
        let terms = match &state.reader {
            None => qse.get_weighted_span_terms_for_field(
                &query,
                1.0,
                token_stream,
                state.field.as_deref(),
            )?,
            Some(reader) => qse.get_weighted_span_terms_with_scores(
                &query,
                1.0,
                token_stream,
                state.field.as_deref(),
                &**reader,
            )?,
        };
        state.field_weighted_span_terms = Some(terms);
        // Java returns the extractor's caching stream when it cached one, and
        // `null` (keep reading the original) otherwise; the extractor holds
        // whichever it is.
        Ok(qse
            .take_token_stream()
            .expect("INVARIANT: the extractor holds the stream it was given"))
    }

    /// Equivalent to `isExpandMultiTermQuery()`.
    pub fn is_expand_multi_term_query(&self) -> bool {
        lock(&self.state).expand_multi_term_query
    }

    /// Controls whether or not multi-term queries are expanded against a
    /// `MemoryIndex` `IndexReader`.
    ///
    /// Equivalent to `setExpandMultiTermQuery(boolean)`.
    pub fn set_expand_multi_term_query(&mut self, expand_multi_term_query: bool) {
        lock(&self.state).expand_multi_term_query = expand_multi_term_query;
    }

    /// Whether or not we should capture payloads in `MemoryIndex` at each
    /// position so that queries can access them. This does not apply to term
    /// vector based TokenStreams, which support payloads only when the term
    /// vector has them.
    ///
    /// Equivalent to `isUsePayloads()`.
    pub fn is_use_payloads(&self) -> bool {
        lock(&self.state).use_payloads
    }

    /// Equivalent to `setUsePayloads(boolean)`.
    pub fn set_use_payloads(&mut self, use_payloads: bool) {
        lock(&self.state).use_payloads = use_payloads;
    }

    /// By default, `TokenStream`s that are not of the type
    /// `CachingTokenFilter` are wrapped in a `CachingTokenFilter` to ensure
    /// an efficient reset - if you are already using a different caching
    /// `TokenStream` impl and you don't want it to be wrapped, set this to
    /// false. Note that term-vector based tokenstreams are detected and won't
    /// be wrapped either.
    ///
    /// Equivalent to `setWrapIfNotCachingTokenFilter(boolean)`.
    pub fn set_wrap_if_not_caching_token_filter(&mut self, wrap: bool) {
        lock(&self.state).wrap_to_caching = wrap;
    }

    /// Equivalent to `setMaxDocCharsToAnalyze(int)`.
    pub fn set_max_doc_chars_to_analyze(&mut self, max_doc_chars_to_analyze: i32) {
        lock(&self.state).max_chars_to_analyze = max_doc_chars_to_analyze;
    }
}

impl Scorer for QueryScorer {
    fn get_fragment_score(&self) -> f32 {
        lock(&self.state).total_score
    }

    /// # Errors
    ///
    /// Java's `NullPointerException` before `init` and `start_fragment`.
    fn get_token_score(&mut self) -> Result<f32> {
        let mut state = lock(&self.state);
        let state = &mut *state;
        state.position += state
            .pos_inc_att
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?
            .borrow()
            .get_position_increment();
        let term_text = term_text(
            &*state
                .term_att
                .as_ref()
                .ok_or_else(LuceneError::null_pointer)?
                .borrow(),
        );

        let Some(weighted_span_term) = state
            .field_weighted_span_terms
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?
            .get(&term_text)
        else {
            return Ok(0.0);
        };

        if weighted_span_term.position_sensitive
            && !weighted_span_term.check_position(state.position)
        {
            return Ok(0.0);
        }

        let score = weighted_span_term.get_weight();

        // found a query term - is it unique in this doc?
        if state
            .found_terms
            .as_mut()
            .ok_or_else(LuceneError::null_pointer)?
            .insert(term_text)
        {
            state.total_score += score;
        }

        Ok(score)
    }

    fn init(&mut self, token_stream: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let mut token_stream = token_stream;
        let mut state = lock(&self.state);
        state.position = -1;
        state.term_att = Some(AttributeHandle::add(token_stream.attribute_source_mut())?);
        state.pos_inc_att = Some(AttributeHandle::add(token_stream.attribute_source_mut())?);

        if !state.skip_init_extractor {
            if let Some(terms) = state.field_weighted_span_terms.as_mut() {
                terms.clear();
            }
            return self.init_extractor(&mut state, token_stream);
        }
        Ok(token_stream)
    }

    fn start_fragment(&mut self, _new_fragment: &TextFragment) {
        let mut state = lock(&self.state);
        state.found_terms = Some(HashSet::new());
        state.total_score = 0.0;
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
