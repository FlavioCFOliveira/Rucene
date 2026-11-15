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

//! Port of `SpanTermQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::index::{LeafReaderContext, Term, TermStates, TermsEnum};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::Weight;

use crate::spans::span_query::{SpanQuery, TermStatesMap};
use crate::spans::span_weight::{
    new_span_weight, span_weight_explain, span_weight_matches, span_weight_scorer_supplier,
    Postings, SpanWeight, SpanWeightBase,
};
use crate::spans::spans::Spans;
use crate::spans::term_spans::TermSpans;

/// A guess of the relative cost of dealing with the term positions when using a
/// span near query instead of a phrase query.
///
/// Equivalent to the private `SpanTermQuery.PHRASE_TO_SPAN_TERM_POSITIONS_COST`.
const PHRASE_TO_SPAN_TERM_POSITIONS_COST: f32 = 4.0;

/// Equivalent to the private `SpanTermQuery.TERM_POSNS_SEEK_OPS_PER_DOC`.
const TERM_POSNS_SEEK_OPS_PER_DOC: i32 = 128;

/// Equivalent to the private `SpanTermQuery.TERM_OPS_PER_POS`.
const TERM_OPS_PER_POS: i32 = 7;

/// Matches spans containing a term.
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanTermQuery`. This should
/// not be used for terms that are indexed at position [`i32::MAX`].
#[derive(Debug, Clone)]
pub struct SpanTermQuery {
    /// The protected `term` field.
    term: Term,
    /// The protected `termStates` field, `None` where Java holds `null`.
    term_states: Option<Arc<TermStates>>,
}

impl SpanTermQuery {
    /// Constructs a query matching the named term's spans.
    ///
    /// Equivalent to `SpanTermQuery(Term)`.
    pub fn new(term: Term) -> Self {
        Self {
            term,
            term_states: None,
        }
    }

    /// Expert: constructs a query matching the named term's spans, using the
    /// provided [`TermStates`].
    ///
    /// Equivalent to `SpanTermQuery(Term, TermStates)`; Java's `null` states
    /// are `None`.
    pub fn with_term_states(term: Term, term_states: Option<Arc<TermStates>>) -> Self {
        Self { term, term_states }
    }

    /// Returns the term whose spans are matched.
    ///
    /// Equivalent to `SpanTermQuery.getTerm()`.
    pub fn get_term(&self) -> &Term {
        &self.term
    }

    /// Returns the [`TermStates`] passed to the constructor, or `None` if it
    /// was not passed.
    ///
    /// Equivalent to the experimental `SpanTermQuery.getTermStates()`.
    pub fn get_term_states(&self) -> Option<&Arc<TermStates>> {
        self.term_states.as_ref()
    }

    /// Returns an expected cost in simple operations of processing the
    /// occurrences of a term in a document that contains the term.
    ///
    /// Equivalent to the package-private static
    /// `SpanTermQuery.termPositionsCost(TermsEnum)`, a copy of
    /// `PhraseQuery.termPositionsCost`; the term is the one at which
    /// `terms_enum` is positioned.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub(crate) fn term_positions_cost(terms_enum: &mut dyn TermsEnum) -> Result<f32> {
        let doc_freq = terms_enum.doc_freq()?;
        debug_assert!(doc_freq > 0);
        let total_term_freq = terms_enum.total_term_freq()?;
        debug_assert!(total_term_freq > 0);
        let exp_occurrences_in_matching_doc = total_term_freq as f32 / doc_freq as f32;
        Ok(TERM_POSNS_SEEK_OPS_PER_DOC as f32
            + exp_occurrences_in_matching_doc * TERM_OPS_PER_POS as f32)
    }
}

impl Query for SpanTermQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        if self.term.field() == field {
            buffer.push_str(&self.term.text());
        } else {
            buffer.push_str(&self.term.to_string());
        }
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(self.term.field()) {
            visitor.consume_terms(self, std::slice::from_ref(&self.term));
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(self.create_span_weight(searcher, score_mode, boost)?)
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(self.rewrite_span(searcher)?.map(|q| q as Arc<dyn Query>))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<SpanTermQuery>()
                .is_some_and(|other| self.term == other.term)
    }

    /// `classHash() ^ term.hashCode()`.
    fn query_hash(&self) -> i32 {
        self.class_hash() ^ self.term.hash_code()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.SpanTermQuery"
    }
}

impl SpanQuery for SpanTermQuery {
    fn get_field(&self) -> Option<&str> {
        Some(self.term.field())
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        let top_context = searcher.get_top_reader_context();
        let context = match self.term_states.as_ref() {
            Some(states) if states.was_built_for(top_context) => Arc::clone(states),
            _ => Arc::new(TermStates::build(
                searcher,
                &self.term,
                score_mode.needs_scores(),
            )?),
        };
        let terms = if score_mode.needs_scores() {
            // `Collections.singletonMap(term, context)`.
            let mut terms = TermStatesMap::new();
            terms.insert(self.term.clone(), Arc::clone(&context));
            Some(terms)
        } else {
            None
        };
        Ok(SpanTermWeight::new(
            Arc::new(self.clone()),
            context,
            searcher,
            terms.as_ref(),
            boost,
        )?)
    }
}

/// Creates [`SpanTermQuery`] scorer instances.
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.spans.SpanTermQuery.SpanTermWeight`.
pub struct SpanTermWeight {
    base: SpanWeightBase,
    term: Term,
    term_states: Arc<TermStates>,
}

impl std::fmt::Debug for SpanTermWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanTermWeight")
            .field("term", &self.term)
            .finish_non_exhaustive()
    }
}

impl SpanTermWeight {
    /// Creates the weight of `query`.
    ///
    /// Equivalent to
    /// `SpanTermWeight(TermStates, IndexSearcher, Map<Term, TermStates>, float)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: Arc<SpanTermQuery>,
        term_states: Arc<TermStates>,
        searcher: &IndexSearcher,
        terms: Option<&TermStatesMap>,
        boost: f32,
    ) -> Result<Arc<Self>> {
        let term = query.term.clone();
        let base = SpanWeightBase::new(query, searcher, terms, boost)?;
        Ok(new_span_weight(move |this: Weak<dyn SpanWeight>| Self {
            base: base.bind(this),
            term,
            term_states,
        }))
    }
}

impl SegmentCacheable for SpanTermWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Weight for SpanTermWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        span_weight_explain(self, context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        span_weight_scorer_supplier(self, context)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        span_weight_matches(self, context, doc)
    }
}

impl SpanWeight for SpanTermWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.base
    }

    fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        contexts.insert(self.term.clone(), Arc::clone(&self.term_states));
    }

    fn get_spans(
        &self,
        context: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        debug_assert!(
            self.term_states.was_built_for_top_level_of(context),
            "The top-reader used to create Weight is not the same as the current reader's \
             top-reader ({context}"
        );

        let state = self.term_states.get(context)?;
        let Some(state) = state else {
            // term is not present in that reader
            debug_assert!(
                rucene::search::phrase_query::PhraseQuery::term_not_in_reader(
                    &context.leaf_reader(),
                    &self.term
                )?,
                "no termstate found but term exists in reader term={}",
                self.term
            );
            return Ok(None);
        };

        let Some(terms) = context.leaf_reader().terms(self.term.field())? else {
            return Ok(None);
        };
        if !terms.has_positions() {
            return Err(LuceneError::illegal_state(format!(
                "field \"{}\" was indexed without position data; cannot run SpanTermQuery \
                 (term={})",
                self.term.field(),
                self.term.text()
            )));
        }

        let mut terms_enum = terms.iterator()?;
        terms_enum.seek_term_state(self.term.bytes(), &*state)?;

        let postings = terms_enum
            .postings(None, required_postings.get_required_postings())?
            .ok_or_else(LuceneError::null_pointer)?;
        let positions_cost = SpanTermQuery::term_positions_cost(&mut *terms_enum)?
            * PHRASE_TO_SPAN_TERM_POSITIONS_COST;
        Ok(Some(Box::new(TermSpans::new(
            postings,
            self.term.clone(),
            positions_cost,
        ))))
    }
}
