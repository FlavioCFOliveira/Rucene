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

//! Port of `SpanWeight.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::cell::RefCell;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::index::multi_reader::get_top_level_context;
use rucene::index::{
    IndexReaderContext, LeafReaderContext, PostingsEnum, Term, POSTINGS_ENUM_OFFSETS,
    POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::{owned_leaf_context, Matches, MatchesIterator, MatchesUtils};
use rucene::search::query::Query;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::sim_scorer_source::{similarity_simple_name, SharedSimScorer, SimScorerSource};
use rucene::search::similarities::{Explanation, Similarity, TermStatistics};
use rucene::search::term_query::TermQuery;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::util::java_fmt::float_to_string;

use crate::spans::span_collector::SpanCollector;
use crate::spans::span_query::{SpanQuery, TermStatesMap};
use crate::spans::span_scorer::SpanScorer;
use crate::spans::spans::{Spans, NO_MORE_POSITIONS};

/// Expert-only. Public for use by other weight implementations.
///
/// Equivalent to the abstract class `org.apache.lucene.queries.spans.SpanWeight`.
///
/// **Divergence from Lucene 10.5.0.** Java's abstract class holds the
/// `similarity`, `simScorer` and `field` fields (`SpanWeight.java:79-81`) and
/// makes `scorerSupplier`,
/// `explain` and `matches` concrete. A Rust trait cannot hold state, so the
/// fields live in a [`SpanWeightBase`] every implementation owns and lends
/// through [`span_weight_base`](Self::span_weight_base), and the concrete
/// methods become the free functions [`span_weight_scorer_supplier`],
/// [`span_weight_explain`] and [`span_weight_matches`] of this module, which
/// every implementation's [`Weight`] calls.
pub trait SpanWeight: Weight {
    /// Lends the state Java's abstract class holds.
    fn span_weight_base(&self) -> &SpanWeightBase;

    /// Collects all [`TermStates`](rucene::index::TermStates) used by this
    /// weight.
    ///
    /// Equivalent to `SpanWeight.extractTermStates(Map<Term, TermStates>)`.
    fn extract_term_states(&self, contexts: &mut TermStatesMap);

    /// Expert: returns a [`Spans`] iterating over matches from this weight, or
    /// `None` where Java returns `null`.
    ///
    /// Equivalent to `SpanWeight.getSpans(LeafReaderContext, Postings)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the postings.
    fn get_spans(
        &self,
        ctx: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>>;

    /// Returns the [`SimScorer`](rucene::search::similarities::SimScorer), or
    /// `None` where Java returns `null`.
    ///
    /// Equivalent to `SpanWeight.getSimScorer()`.
    fn get_sim_scorer(&self) -> Option<SharedSimScorer> {
        self.span_weight_base().sim_scorer.clone()
    }

    /// Builds the [`SpanScorer`] of a leaf, or `None` when no document matches.
    ///
    /// Equivalent to `(SpanScorer) scorer(context)` — the cast with which
    /// `SpanWeight.explain` reads the sloppy frequency — through the
    /// `scorerSupplier(LeafReaderContext)` of `SpanWeight`, which the weights
    /// that override it (`SpanPayloadCheckWeight`) override here too.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the spans or the norms.
    fn span_scorer(&self, context: &LeafReaderContext) -> Result<Option<SpanScorer>> {
        let Some(spans) = self.get_spans(context, Postings::Positions)? else {
            return Ok(None);
        };
        let norms = get_norm_values(context, self.span_weight_base().field())?;
        Ok(Some(SpanScorer::new(spans, self.get_sim_scorer(), norms)))
    }
}

/// `context.reader().getNormValues(field)` for a span weight's
/// possibly-`null` field.
///
/// A `null` field names no field, so Java's `FieldInfos.fieldInfo(null)`
/// finds nothing and no norms are returned; `None` answers the same.
pub(crate) fn get_norm_values(
    context: &LeafReaderContext,
    field: Option<&str>,
) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
    match field {
        Some(field) => context.leaf_reader().get_norm_values(field),
        None => Ok(None),
    }
}

/// The postings information to retrieve from the index for a given
/// [`Spans`].
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanWeight.Postings`. The
/// variants are declared in Java's order, which [`at_least`](Self::at_least)
/// compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Postings {
    /// Java constant `POSITIONS`.
    Positions,
    /// Java constant `PAYLOADS`.
    Payloads,
    /// Java constant `OFFSETS`.
    Offsets,
}

impl Postings {
    /// Returns the [`PostingsEnum`] flags these spans need.
    ///
    /// Equivalent to `Postings.getRequiredPostings()`, which each constant
    /// overrides.
    pub fn get_required_postings(self) -> i32 {
        match self {
            Postings::Positions => POSTINGS_ENUM_POSITIONS,
            Postings::Payloads => POSTINGS_ENUM_PAYLOADS,
            Postings::Offsets => POSTINGS_ENUM_PAYLOADS | POSTINGS_ENUM_OFFSETS,
        }
    }

    /// Returns the greater of `self` and `postings`.
    ///
    /// Equivalent to `Postings.atLeast(Postings)`, which compares the
    /// constants' ordinals.
    pub fn at_least(self, postings: Postings) -> Postings {
        if postings > self {
            return postings;
        }
        self
    }

    /// Java's `Postings.toString()`, the constant's name.
    pub fn name(self) -> &'static str {
        match self {
            Postings::Positions => "POSITIONS",
            Postings::Payloads => "PAYLOADS",
            Postings::Offsets => "OFFSETS",
        }
    }
}

/// The state of a [`SpanWeight`]: the fields of Java's abstract class.
///
/// Equivalent to the `protected final` fields `similarity`, `simScorer` and
/// `field` of `SpanWeight`, the `parentQuery` it inherits from `Weight`, and
/// the body of its constructor
/// `SpanWeight(SpanQuery, IndexSearcher, Map<Term, TermStates>, float)`.
///
/// It also keeps a weak handle on the weight that owns it, which
/// [`span_weight_matches`] needs: Java's `matches` hands the `Matches` a lambda
/// that captures `this`, and a Rust closure that outlives `&self` needs an
/// owning handle on the weight instead. Every weight of this crate is built
/// with [`Arc::new_cyclic`] and installs that handle with
/// [`bind`](Self::bind).
pub struct SpanWeightBase {
    parent_query: Arc<dyn Query>,
    similarity: Arc<dyn Similarity>,
    sim_scorer: Option<SharedSimScorer>,
    field: Option<String>,
    this: Option<Weak<dyn SpanWeight>>,
}

impl std::fmt::Debug for SpanWeightBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanWeightBase")
            .field("parent_query", &self.parent_query.to_query_string(""))
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl SpanWeightBase {
    /// Creates the state of a new span weight.
    ///
    /// Equivalent to
    /// `SpanWeight(SpanQuery, IndexSearcher, Map<Term, TermStates>, float)`.
    ///
    /// * `query` — the parent query;
    /// * `searcher` — the searcher to query against;
    /// * `term_states` — a map of terms to their states for use in building
    ///   the similarity; `None` if scores are not required.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics, and the
    /// `NullPointerException` Java raises when terms exist but the field has
    /// no collection statistics.
    pub fn new(
        query: Arc<dyn SpanQuery>,
        searcher: &IndexSearcher,
        term_states: Option<&TermStatesMap>,
        boost: f32,
    ) -> Result<Self> {
        let field = query.get_field().map(str::to_string);
        let similarity = Arc::clone(searcher.get_similarity());
        let sim_scorer =
            Self::build_sim_weight(&*query, searcher, &similarity, term_states, boost)?;
        let parent_query: Arc<dyn Query> = query;
        Ok(Self {
            parent_query,
            similarity,
            sim_scorer,
            field,
            this: None,
        })
    }

    /// Equivalent to the private
    /// `SpanWeight.buildSimWeight(SpanQuery, IndexSearcher, Map<Term, TermStates>, float)`.
    fn build_sim_weight(
        query: &dyn SpanQuery,
        searcher: &IndexSearcher,
        similarity: &Arc<dyn Similarity>,
        term_states: Option<&TermStatesMap>,
        boost: f32,
    ) -> Result<Option<SharedSimScorer>> {
        let Some(term_states) = term_states else {
            return Ok(None);
        };
        let Some(field) = query.get_field() else {
            return Ok(None);
        };
        if term_states.is_empty() {
            return Ok(None);
        }
        let mut term_stats: Vec<TermStatistics> = Vec::with_capacity(term_states.len());
        for (term, ts) in term_states {
            if ts.doc_freq()? > 0 {
                term_stats.push(searcher.term_statistics(
                    term,
                    ts.doc_freq()?,
                    ts.total_term_freq()?,
                )?);
            }
        }
        let collection_stats = searcher.collection_statistics(field)?;
        if !term_stats.is_empty() {
            // `similarity.scorer(boost, collectionStats, ...)` on a `null`
            // `collectionStats` dereferences it.
            let collection_stats = collection_stats.ok_or_else(LuceneError::null_pointer)?;
            Ok(Some(Arc::new(SimScorerSource::new(
                Arc::clone(similarity),
                boost,
                collection_stats,
                term_stats,
            ))))
        } else {
            Ok(None) // no terms at all exist, we won't use similarity
        }
    }

    /// Installs the weak handle on the weight that owns this state; see the
    /// type documentation.
    pub fn bind(mut self, this: Weak<dyn SpanWeight>) -> Self {
        self.this = Some(this);
        self
    }

    /// The parent query: `Weight.getQuery()`.
    pub fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.parent_query)
    }

    /// The protected `similarity` field.
    pub fn similarity(&self) -> &Arc<dyn Similarity> {
        &self.similarity
    }

    /// The protected `simScorer` field.
    pub fn sim_scorer(&self) -> Option<&SharedSimScorer> {
        self.sim_scorer.as_ref()
    }

    /// The protected `field` field, `None` where Java holds `null`.
    pub fn field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    /// Returns an owning handle on the weight that owns this state.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`](rucene::error::LuceneErrorKind::IllegalState)
    /// when the weight was not built through [`Arc::new_cyclic`] and
    /// [`bind`](Self::bind), which no weight of this crate does.
    pub fn this(&self) -> Result<Arc<dyn SpanWeight>> {
        self.this.as_ref().and_then(Weak::upgrade).ok_or_else(|| {
            LuceneError::illegal_state(
                "SpanWeight was not bound to the Arc that owns it".to_string(),
            )
        })
    }
}

/// Builds a span weight through [`Arc::new_cyclic`], handing `build` the weak
/// handle its [`SpanWeightBase`] keeps.
pub(crate) fn new_span_weight<W, F>(build: F) -> Arc<W>
where
    W: SpanWeight + 'static,
    F: FnOnce(Weak<dyn SpanWeight>) -> W,
{
    Arc::new_cyclic(|weak: &Weak<W>| {
        let this: Weak<dyn SpanWeight> = weak.clone();
        build(this)
    })
}

/// The body of `SpanWeight.scorerSupplier(LeafReaderContext)`.
///
/// Java's anonymous supplier hands out the scorer it built and reports its
/// iterator's cost; [`DefaultScorerSupplier`] is that supplier, which every
/// span weight overriding `scorerSupplier` returns (`SpanNearQuery.java:251`).
///
/// # Errors
///
/// Propagates any I/O error raised while building the scorer.
pub fn span_weight_scorer_supplier(
    weight: &dyn SpanWeight,
    context: &LeafReaderContext,
) -> Result<Option<Box<dyn ScorerSupplier>>> {
    Ok(weight.span_scorer(context)?.map(|scorer| {
        Box::new(DefaultScorerSupplier::new(Box::new(scorer))) as Box<dyn ScorerSupplier>
    }))
}

/// The body of `SpanWeight.explain(LeafReaderContext, int)`.
///
/// # Errors
///
/// Propagates any I/O error raised while scoring.
pub fn span_weight_explain(
    weight: &dyn SpanWeight,
    context: &LeafReaderContext,
    doc: i32,
) -> Result<Explanation> {
    if let Some(mut scorer) = weight.span_scorer(context)? {
        let new_doc = scorer.iterator().advance(doc)?;
        if new_doc == doc {
            let base = weight.span_weight_base();
            if let Some(sim_scorer) = base.sim_scorer.as_ref() {
                let freq = scorer.sloppy_freq()?;
                let freq_explanation = Explanation::matched(
                    freq,
                    format!("phraseFreq={}", float_to_string(freq)),
                    Vec::new(),
                );
                let norms = get_norm_values(context, base.field())?;
                let mut norm = 1i64;
                if let Some(mut norms) = norms {
                    if norms.advance_exact(doc)? {
                        norm = norms.long_value()?;
                    }
                }
                let score_explanation = sim_scorer.explain(&freq_explanation, norm);
                return Ok(Explanation::matched(
                    score_explanation.value(),
                    format!(
                        "weight({} in {doc}) [{}], result of:",
                        base.parent_query.to_query_string(""),
                        similarity_simple_name(&*base.similarity)
                    ),
                    vec![score_explanation],
                ));
            } else {
                // simScorer won't be set when scoring isn't needed
                return Ok(Explanation::matched(
                    0f32,
                    format!(
                        "match {} in {doc} without score",
                        base.parent_query.to_query_string("")
                    ),
                    Vec::new(),
                ));
            }
        }
    }

    Ok(Explanation::no_match("no matching term", Vec::new()))
}

/// The body of `SpanWeight.matches(LeafReaderContext, int)`.
///
/// # Errors
///
/// Propagates any I/O error raised while positioning the spans.
pub fn span_weight_matches(
    weight: &dyn SpanWeight,
    context: &LeafReaderContext,
    doc: i32,
) -> Result<Option<Arc<dyn Matches>>> {
    let base = weight.span_weight_base();
    let this = base.this()?;
    let query = base.get_query();
    let context = leaf_context_handle(context)?;
    // A weight with a `null` field opens no spans, so the supplier answers
    // `null` before `MatchesUtils.forField` would keep the field.
    let field = base.field.clone().unwrap_or_default();
    MatchesUtils::for_field(
        field,
        Arc::new(move || {
            let Some(mut spans) = this.get_spans(&context, Postings::Offsets)? else {
                return Ok(None);
            };
            if spans.advance(doc)? != doc {
                return Ok(None);
            }
            Ok(Some(Box::new(SpanMatchesIterator {
                spans: RefCell::new(spans),
                inner: RefCell::new(InnerTerms::default()),
                query: Arc::clone(&query),
            }) as Box<dyn MatchesIterator>))
        }),
    )
}

/// An owning handle on `context` itself, for the `Matches` supplier of
/// [`span_weight_matches`] to keep.
///
/// Java's anonymous supplier captures the `LeafReaderContext` object it is
/// handed (`SpanWeight.java:210-217`); the spans it opens later run
/// `SpanTermWeight.getSpans`, whose
/// `assert termStates.wasBuiltFor(ReaderUtil.getTopLevelContext(context))`
/// (`SpanTermQuery.java:135`) holds only for a context of the searcher's tree.
/// A borrow cannot be kept, so the context's own `Arc` is found again: the
/// leaf of the same ordinal of its top-level context (its parent's, or for a
/// top-level leaf its reader's cached context), which is `context` itself.
/// Only a context outside every reader's tree falls back to
/// [`owned_leaf_context`], a copy with a fresh identity.
///
/// # Errors
///
/// [`LuceneErrorKind::AlreadyClosed`](rucene::error::LuceneErrorKind::AlreadyClosed)
/// when the top-level leaf's reader is closed.
fn leaf_context_handle(context: &LeafReaderContext) -> Result<Arc<LeafReaderContext>> {
    let top: Arc<dyn IndexReaderContext> = match context.parent().and_then(|p| p.upgrade()) {
        Some(parent) => get_top_level_context(parent),
        None => IndexReaderContext::reader(context).get_context()?,
    };
    if top.is_top_level() {
        if let Some(leaf) = top.leaves().get(context.ord() as usize) {
            if std::ptr::eq(Arc::as_ptr(leaf), context) {
                return Ok(Arc::clone(leaf));
            }
        }
    }
    Ok(owned_leaf_context(context))
}

/// One leaf term collected from the current match.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.spans.SpanWeight.TermMatch`.
#[derive(Debug, Clone)]
pub(crate) struct TermMatch {
    term: Term,
    position: i32,
    start_offset: i32,
    end_offset: i32,
}

/// The inner terms of the current match and the collector that gathers them.
///
/// Equivalent to the `innerTermCount` and `innerTerms` fields of the anonymous
/// `MatchesIterator`, and to its anonymous `termCollector`: the array grows,
/// never shrinks, and is reused from one match to the next.
#[derive(Debug, Default)]
struct InnerTerms {
    count: usize,
    terms: Vec<TermMatch>,
}

impl SpanCollector for InnerTerms {
    fn collect_leaf(
        &mut self,
        postings: &dyn PostingsEnum,
        position: i32,
        term: &Term,
    ) -> Result<()> {
        self.count += 1;
        let entry = TermMatch {
            term: term.clone(),
            position,
            start_offset: postings.start_offset(),
            end_offset: postings.end_offset(),
        };
        if self.count > self.terms.len() {
            self.terms.push(entry);
        } else {
            self.terms[self.count - 1] = entry;
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.count = 0;
    }
}

/// `innerTerms[index]` on Java's array, which throws
/// `ArrayIndexOutOfBoundsException` past its end — and for index `-1`, which
/// `innerTerms[innerTermCount - 1]` reads when no term was collected.
fn term_at(terms: &[TermMatch], index: isize) -> &TermMatch {
    if index < 0 || index as usize >= terms.len() {
        rucene::store::java_nio_file::throw_unchecked(
            "java.lang.ArrayIndexOutOfBoundsException",
            Some(&format!(
                "Index {index} out of bounds for length {}",
                terms.len()
            )),
        );
    }
    &terms[index as usize]
}

/// The [`MatchesIterator`] a [`SpanWeight`] hands out.
///
/// Equivalent to the anonymous `MatchesIterator` of
/// `SpanWeight.matches(LeafReaderContext, int)`. Java's `startOffset()` and
/// `getSubMatches()` collect the inner terms lazily; their Rust signatures take
/// `&self`, so the spans and the inner terms are kept in [`RefCell`]s.
struct SpanMatchesIterator {
    spans: RefCell<Box<dyn Spans>>,
    inner: RefCell<InnerTerms>,
    query: Arc<dyn Query>,
}

impl SpanMatchesIterator {
    /// Equivalent to the anonymous iterator's `collectInnerTerms()`.
    fn collect_inner_terms(&self) -> Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.reset();
        self.spans.borrow_mut().collect(&mut *inner)?;
        let count = inner.count;
        // `Arrays.sort(innerTerms, 0, innerTermCount, comparing(position))`, a
        // stable sort.
        inner.terms[..count].sort_by_key(|t| t.position);
        Ok(())
    }
}

impl MatchesIterator for SpanMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.inner.borrow_mut().count = 0;
        Ok(self.spans.borrow_mut().next_start_position()? != NO_MORE_POSITIONS)
    }

    fn start_position(&self) -> i32 {
        self.spans.borrow().start_position()
    }

    fn end_position(&self) -> i32 {
        self.spans.borrow().end_position().wrapping_sub(1)
    }

    fn start_offset(&self) -> Result<i32> {
        if self.inner.borrow().count == 0 {
            self.collect_inner_terms()?;
        }
        Ok(term_at(&self.inner.borrow().terms, 0).start_offset)
    }

    fn end_offset(&self) -> Result<i32> {
        if self.inner.borrow().count == 0 {
            self.collect_inner_terms()?;
        }
        let inner = self.inner.borrow();
        Ok(term_at(&inner.terms, inner.count as isize - 1).end_offset)
    }

    /// **Divergence from Lucene 10.5.0.** Java's sub-iterator
    /// (`SpanWeight.java:280-323`) reads the outer iterator's `innerTerms` and
    /// `innerTermCount` live, so moving the outer
    /// iterator while the sub-iterator is in use changes what it returns. The
    /// sub-iterator cannot borrow its parent here, so it takes a copy of the
    /// current match's terms; within the contract — a sub-iterator is used for
    /// the current match only — the two agree.
    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        if self.inner.borrow().count == 0 {
            self.collect_inner_terms()?;
        }
        let inner = self.inner.borrow();
        Ok(Some(Box::new(SpanSubMatchesIterator {
            terms: inner.terms[..inner.count.min(inner.terms.len())].to_vec(),
            upto: -1,
        })))
    }

    fn get_query(&self) -> Option<Arc<dyn Query>> {
        Some(Arc::clone(&self.query))
    }
}

/// The leaf-level [`MatchesIterator`] over the inner terms of one match.
///
/// Equivalent to the anonymous `MatchesIterator` the anonymous iterator's
/// `getSubMatches()` returns.
struct SpanSubMatchesIterator {
    terms: Vec<TermMatch>,
    upto: isize,
}

impl MatchesIterator for SpanSubMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.upto += 1;
        Ok((self.upto as usize) < self.terms.len())
    }

    fn start_position(&self) -> i32 {
        term_at(&self.terms, self.upto).position
    }

    fn end_position(&self) -> i32 {
        term_at(&self.terms, self.upto).position
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(term_at(&self.terms, self.upto).start_offset)
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(term_at(&self.terms, self.upto).end_offset)
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        Ok(None)
    }

    fn get_query(&self) -> Option<Arc<dyn Query>> {
        Some(Arc::new(TermQuery::new(
            term_at(&self.terms, self.upto).term.clone(),
        )))
    }
}
