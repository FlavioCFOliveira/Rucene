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

//! Port of `PhraseHelper.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::any::Any;
use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::error::{LuceneError, Result};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::{
    FieldInfos, LeafReader, NumericDocValues, PostingsEnum, Term, Terms, TermsEnum,
    POSTINGS_ENUM_OFFSETS,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::Scorer;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};
use rucene::util::BytesRef;
use rucene_queries::spans::span_collector::SpanCollector;
use rucene_queries::spans::span_query::{to_span_query_arc, SpanQuery};
use rucene_queries::spans::span_scorer::SpanScorer;
use rucene_queries::spans::spans::{Spans, NO_MORE_POSITIONS};

use crate::highlight::weighted_span_term_extractor::{
    PositionCheckingMap, WeightedSpanTermExtractor, WeightedSpanTermExtractorOverrides,
};
use crate::uhighlight::offsets_enum::OffsetsEnum;
use crate::uhighlight::uh_components::FieldMatcher;
use crate::wrappers::{forward_terms, forward_terms_enum};

/// `Function<SpanQuery, Boolean>`: whether a span query must be rewritten,
/// `None` (Java's `null`) to defer to the default.
pub type RewriteQueryPred = Arc<dyn Fn(&dyn SpanQuery) -> Option<bool> + Send + Sync>;

/// `Function<Query, Collection<Query>>`: the queries to extract in place of
/// a query, `None` (Java's `null`) to extract the query itself.
pub type PreExtractRewriteFunction =
    Arc<dyn Fn(&Arc<dyn Query>) -> Option<Vec<Arc<dyn Query>>> + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The state the extractor fills.
#[derive(Default)]
struct Extracted {
    position_insensitive_terms: HashSet<BytesRef>, // (TermQuery terms)
    span_queries: Vec<Arc<dyn SpanQuery>>,
    must_rewrite: bool,
}

/// Helps the `FieldOffsetStrategy` with position sensitive queries (e.g.
/// highlight phrases correctly). This is a stateful class holding
/// information about the query, but it can (and is) re-used across
/// highlighting documents. Despite this state, it's immutable after
/// construction.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.PhraseHelper`.
pub struct PhraseHelper {
    field_name: String,
    position_insensitive_terms: HashSet<BytesRef>, // (TermQuery terms)
    span_queries: Vec<Arc<dyn SpanQuery>>,
    will_rewrite: bool,
    field_matcher: FieldMatcher,
}

impl fmt::Debug for PhraseHelper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhraseHelper")
            .field("field_name", &self.field_name)
            .field("span_queries", &self.span_queries.len())
            .field("will_rewrite", &self.will_rewrite)
            .finish_non_exhaustive()
    }
}

/// The anonymous `WeightedSpanTermExtractor` subclass of the constructor.
struct ExtractorOverrides {
    field_matcher: FieldMatcher,
    rewrite_query_pred: RewriteQueryPred,
    pre_extract_rewrite_function: PreExtractRewriteFunction,
    ignore_queries_needing_rewrite: bool,
    extracted: Arc<Mutex<Extracted>>,
}

impl fmt::Debug for ExtractorOverrides {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhraseHelper$1").finish_non_exhaustive()
    }
}

/// The anonymous visitor of `extractWeightedTerms`.
struct InsensitiveTermsVisitor<'a> {
    field_matcher: &'a FieldMatcher,
    terms: &'a mut HashSet<BytesRef>,
}

impl QueryVisitor for InsensitiveTermsVisitor<'_> {
    fn accept_field(&self, field: &str) -> bool {
        (self.field_matcher)(field)
    }

    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        for term in terms {
            self.terms.insert(term.bytes().clone());
        }
    }
}

impl WeightedSpanTermExtractorOverrides for ExtractorOverrides {
    fn extract(
        &self,
        this: &mut WeightedSpanTermExtractor,
        query: &Arc<dyn Query>,
        boost: f32,
        terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        match (self.pre_extract_rewrite_function)(query) {
            Some(new_queries_to_extract) => {
                for new_query in &new_queries_to_extract {
                    this.extract(new_query, boost, terms)?;
                }
                Ok(())
            }
            None => this.super_extract(query, boost, terms),
        }
    }

    fn is_query_unsupported(&self, _this: &WeightedSpanTermExtractor, _query: &dyn Query) -> bool {
        // We do MTQ processing separately in MultiTermHighlighting.java
        true // TODO set to false and provide a hook to customize certain queries.
    }

    // called on Query types that are NOT position sensitive, e.g. TermQuery
    fn extract_weighted_terms(
        &self,
        _this: &mut WeightedSpanTermExtractor,
        _terms: &mut PositionCheckingMap,
        query: &Arc<dyn Query>,
        _boost: f32,
    ) -> Result<()> {
        let mut extracted = lock(&self.extracted);
        query.visit(&mut InsensitiveTermsVisitor {
            field_matcher: &self.field_matcher,
            terms: &mut extracted.position_insensitive_terms,
        });
        Ok(())
    }

    // called on SpanQueries. Some other position-sensitive queries like
    // PhraseQuery are converted beforehand
    fn extract_weighted_span_terms(
        &self,
        this: &mut WeightedSpanTermExtractor,
        _terms: &mut PositionCheckingMap,
        span_query: Arc<dyn SpanQuery>,
        _boost: f32,
    ) -> Result<()> {
        // if this span query isn't for this field, skip it.
        let mut field_name_set: HashSet<Option<String>> = HashSet::new(); // TODO reuse.  note: almost always size 1
        this.collect_span_query_fields(&*span_query, &mut field_name_set);
        for span_field in &field_name_set {
            let matched = match span_field {
                Some(span_field) => (self.field_matcher)(span_field),
                None => false,
            };
            if !matched {
                return Ok(());
            }
        }

        let must_rewrite_query = this.must_rewrite_query(&*span_query);
        if self.ignore_queries_needing_rewrite && must_rewrite_query {
            return Ok(()); // ignore this query
        }
        let mut extracted = lock(&self.extracted);
        extracted.must_rewrite |= must_rewrite_query;

        if !extracted
            .span_queries
            .iter()
            .any(|q| q.query_eq(&*span_query as &dyn Query))
        {
            extracted.span_queries.push(span_query);
        }
        Ok(())
    }

    fn must_rewrite_query(&self, this: &WeightedSpanTermExtractor, span_query: &dyn SpanQuery) -> bool {
        match (self.rewrite_query_pred)(span_query) {
            // allow to override
            Some(rewrite_q) => rewrite_q,
            None => this.super_must_rewrite_query(span_query),
        }
    }
}

impl PhraseHelper {
    /// Equivalent to `PhraseHelper.NONE`.
    pub fn none() -> Arc<PhraseHelper> {
        Arc::new(
            Self::new(
                Arc::new(MatchAllDocsQuery),
                "_ignored_",
                Arc::new(|_| false),
                Arc::new(|_| None),
                Arc::new(|_| None),
                true,
            )
            .expect("INVARIANT: MatchAllDocsQuery extracts nothing"),
        )
    }

    /// Constructor. `rewrite_query_pred` is an extension hook to override
    /// the default choice of `WeightedSpanTermExtractor.mustRewriteQuery`;
    /// `pre_extract_rewrite_function` is an extension hook to allow
    /// different queries to be extracted; `ignore_queries_needing_rewrite`
    /// ignores queries that need to be rewritten.
    ///
    /// Equivalent to `new PhraseHelper(Query, String, Predicate, Function,
    /// Function, boolean)`.
    ///
    /// # Errors
    ///
    /// What the extraction raises (Java wraps it in a `RuntimeException`).
    pub fn new(
        query: Arc<dyn Query>,
        field: &str,
        field_matcher: FieldMatcher,
        rewrite_query_pred: RewriteQueryPred,
        pre_extract_rewrite_function: PreExtractRewriteFunction,
        ignore_queries_needing_rewrite: bool,
    ) -> Result<Self> {
        let extracted = Arc::new(Mutex::new(Extracted::default()));

        // Uses WeightedSpanTermExtractor to find the span queries and the
        // position-insensitive terms.
        let mut extractor = WeightedSpanTermExtractor::with_default_field(Some(field));
        extractor.set_overrides(Arc::new(ExtractorOverrides {
            field_matcher: Arc::clone(&field_matcher),
            rewrite_query_pred,
            pre_extract_rewrite_function,
            ignore_queries_needing_rewrite,
            extracted: Arc::clone(&extracted),
        }));
        extractor.set_expand_multi_term_query(true); // necessary for mustRewriteQuery(spanQuery) to work.
        extractor
            .extract(&query, 1.0, &mut PositionCheckingMap::new())
            .map_err(LuceneError::runtime_exception)?;
        drop(extractor);

        let extracted = std::mem::take(&mut *lock(&extracted));
        Ok(Self {
            field_name: field.to_string(),
            position_insensitive_terms: extracted.position_insensitive_terms,
            span_queries: extracted.span_queries,
            will_rewrite: extracted.must_rewrite,
            field_matcher,
        })
    }

    /// Equivalent to `getSpanQueries()`.
    pub fn get_span_queries(&self) -> &[Arc<dyn SpanQuery>] {
        &self.span_queries
    }

    /// If there is no position sensitivity then use of the instance of this
    /// class can be ignored.
    ///
    /// Equivalent to `hasPositionSensitivity()`.
    pub fn has_position_sensitivity(&self) -> bool {
        !self.span_queries.is_empty()
    }

    /// Rewrite is needed for handling a `SpanMultiTermQueryWrapper` (MTQ /
    /// wildcards) or some custom things. When true, the resulting term list
    /// will probably be different than what it was known to be initially.
    ///
    /// Equivalent to `willRewrite()`.
    pub fn will_rewrite(&self) -> bool {
        self.will_rewrite
    }

    /// Returns the terms that are position-insensitive (sorted).
    ///
    /// Equivalent to `getAllPositionInsensitiveTerms()`.
    pub fn get_all_position_insensitive_terms(&self) -> Vec<BytesRef> {
        let mut result: Vec<BytesRef> = self.position_insensitive_terms.iter().cloned().collect();
        result.sort();
        result
    }

    /// Given the internal SpanQueries, produce a number of OffsetsEnum into
    /// the `results` param.
    ///
    /// Equivalent to `createOffsetsEnumsForSpans(LeafReader, int, List)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn create_offsets_enums_for_spans(
        &self,
        leaf_reader: &Arc<dyn LeafReader>,
        doc_id: i32,
        results: &mut Vec<Box<dyn OffsetsEnum>>,
    ) -> Result<()> {
        let leaf_reader: Arc<dyn LeafReader> = Arc::new(SingleFieldWithOffsetsFilterLeafReader::new(
            Arc::clone(leaf_reader),
            &self.field_name,
        )?);
        // TODO avoid searcher and do what it does to rewrite & get weight?
        let mut searcher = IndexSearcher::new(Arc::clone(&leaf_reader).as_index_reader())?;
        searcher.set_query_cache(None);

        // for each SpanQuery, grab it's Spans and put it into a
        // PriorityQueue
        let mut spans_priority_queue: PriorityQueue<SpanScorer, SpansStartLessThan> =
            PriorityQueue::new(self.span_queries.len(), SpansStartLessThan)?;
        let context = Arc::clone(&leaf_reader)
            .as_index_reader()
            .get_context()?
            .as_leaf()
            .ok_or_else(LuceneError::null_pointer)?;
        for query in &self.span_queries {
            let rewritten = searcher.rewrite(Arc::clone(query) as Arc<dyn Query>)?;
            let span_query = to_span_query_arc(&*rewritten).ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    format!(
                        "class {} cannot be cast to class org.apache.lucene.queries.spans.SpanQuery",
                        rewritten.java_class_name()
                    ),
                )
            })?;
            let weight = span_query.create_span_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
            let Some(mut scorer) = weight.span_scorer(&context)? else {
                continue;
            };
            match scorer.two_phase_iterator() {
                Some(two_phase_iterator) => {
                    if two_phase_iterator.approximation().advance(doc_id)? != doc_id
                        || !two_phase_iterator.matches()?
                    {
                        continue;
                    }
                }
                None => {
                    if scorer.iterator().advance(doc_id)? != doc_id {
                        // preposition, and return doing nothing if find none
                        continue;
                    }
                }
            }

            let spans = scorer.get_spans_mut();
            debug_assert!(spans.doc_id() == doc_id);
            if spans.next_start_position()? != NO_MORE_POSITIONS {
                spans_priority_queue.add(scorer);
            }
        }

        // Iterate the Spans in the PriorityQueue, collecting as we go. By
        // using a PriorityQueue ordered by position, the collector can
        // accumulate start/end offsets in sorted order.
        let mut span_collector = OffsetSpanCollector {
            field_matcher: Arc::clone(&self.field_matcher),
            position_insensitive_terms: &self.position_insensitive_terms,
            term_to_offsets_enums: Vec::new(),
        };
        while spans_priority_queue.size() > 0 {
            let scorer = spans_priority_queue
                .top_mut()
                .expect("INVARIANT: the queue is not empty");
            let spans = scorer.get_spans_mut();
            spans.collect(&mut span_collector)?;
            if spans.next_start_position()? == NO_MORE_POSITIONS {
                spans_priority_queue.pop();
            } else {
                spans_priority_queue.update_top();
            }
        }
        for (_, offsets_enum) in span_collector.term_to_offsets_enums {
            results.push(Box::new(offsets_enum));
        }
        Ok(())
    }
}

/// The anonymous `PriorityQueue<Spans>` order of
/// `createOffsetsEnumsForSpans`: `a.startPosition() <= b.startPosition()`.
#[derive(Debug, Clone, Copy)]
struct SpansStartLessThan;

impl PriorityQueueComparator<SpanScorer> for SpansStartLessThan {
    fn less_than(&self, a: &SpanScorer, b: &SpanScorer) -> bool {
        a.get_spans().start_position() <= b.get_spans().start_position()
    }
}

/// Needed to support the ability to highlight a query irrespective of the
/// field a query refers to (aka requireFieldMatch=false). This reader will
/// just delegate every call to a single field in the wrapped LeafReader.
/// This way we ensure that all queries going through this reader target the
/// same field.
///
/// Equivalent to the private nested class
/// `PhraseHelper.SingleFieldWithOffsetsFilterLeafReader`.
pub(crate) struct SingleFieldWithOffsetsFilterLeafReader {
    base: FilterLeafReader,
    field_name: String,
}

impl SingleFieldWithOffsetsFilterLeafReader {
    fn new(input: Arc<dyn LeafReader>, field_name: &str) -> Result<Self> {
        Ok(Self {
            base: FilterLeafReader::new(input)?,
            field_name: field_name.to_string(),
        })
    }

    fn input(&self) -> &Arc<dyn LeafReader> {
        self.base.get_delegate()
    }
}

impl fmt::Debug for SingleFieldWithOffsetsFilterLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SingleFieldWithOffsetsFilterLeafReader")
            .field("field_name", &self.field_name)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for SingleFieldWithOffsetsFilterLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl LeafReader for SingleFieldWithOffsetsFilterLeafReader {
    crate::forward_leaf_reader!(filter base;
        core,
        filter_leaf_reader_delegate,
        do_close,
        term_vectors,
        num_docs,
        max_doc,
        stored_fields,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
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

    /// **Divergence.** Java throws `UnsupportedOperationException`
    /// (`PhraseHelper.java:309-312`),
    /// an unchecked exception the caller receives. Core
    /// `LeafReader::get_field_infos` returns `FieldInfos` without a
    /// `Result` (the shape of all 35 readers and their callers, including
    /// crates outside this one), so the exception is a panic naming the same
    /// class; returning it as an `Err` needs that core signature changed.
    ///
    /// # Panics
    ///
    /// Always, as Java throws `UnsupportedOperationException`.
    fn get_field_infos(&self) -> FieldInfos {
        panic!("java.lang.UnsupportedOperationException"); // TODO merge them
    }

    /// # Errors
    ///
    /// `NullPointerException("incoming Terms must not be null")` when the
    /// wrapped reader has no such field, as `new FilterTerms(null)` throws.
    fn terms(&self, _field: &str) -> Result<Option<Arc<dyn Terms>>> {
        let inner = self.input().terms(&self.field_name)?.ok_or_else(|| {
            LuceneError::null_pointer_with_message("incoming Terms must not be null")
        })?;
        Ok(Some(Arc::new(OffsetsTerms { inner })))
    }

    fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.input().get_norm_values(&self.field_name)
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// The anonymous `FilterTerms` of `SingleFieldWithOffsetsFilterLeafReader.terms`.
struct OffsetsTerms {
    inner: Arc<dyn Terms>,
}

impl Terms for OffsetsTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(OffsetsTermsEnum {
            inner: self.inner.iterator()?,
        }))
    }

    forward_terms!(inner);
}

/// The anonymous `FilterTermsEnum`: postings always with offsets.
struct OffsetsTermsEnum {
    inner: Box<dyn TermsEnum>,
}

impl fmt::Display for OffsetsTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.inner, f)
    }
}

impl TermsEnum for OffsetsTermsEnum {
    forward_terms_enum!(inner);

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        self.inner.postings(reuse, flags | POSTINGS_ENUM_OFFSETS)
    }
}

/// Simple SpanCollector that accumulates offsets per term in an
/// OffsetsEnum.
///
/// Equivalent to the private inner class `PhraseHelper.OffsetSpanCollector`;
/// the map's values are kept in insertion order (Java iterates its
/// `HashMap`, whose order only feeds a priority queue ordered by offsets and
/// term, the keys being distinct terms).
pub(crate) struct OffsetSpanCollector<'a> {
    field_matcher: FieldMatcher,
    position_insensitive_terms: &'a HashSet<BytesRef>,
    term_to_offsets_enums: Vec<(BytesRef, SpanCollectedOffsetsEnum)>,
}

impl SpanCollector for OffsetSpanCollector<'_> {
    fn collect_leaf(&mut self, postings: &dyn PostingsEnum, _position: i32, term: &Term) -> Result<()> {
        if !term.field_opt().is_some_and(|f| (self.field_matcher)(f)) {
            return Ok(());
        }

        let bytes = term.bytes();
        let index = match self
            .term_to_offsets_enums
            .iter()
            .position(|(t, _)| t == bytes)
        {
            Some(index) => index,
            None => {
                // If it's pos insensitive we handle it outside of
                // PhraseHelper. term.field() is from the Query.
                if self.position_insensitive_terms.contains(bytes) {
                    return Ok(());
                }
                self.term_to_offsets_enums.push((
                    bytes.clone(),
                    SpanCollectedOffsetsEnum::new(bytes.clone(), postings.freq()?),
                ));
                self.term_to_offsets_enums.len() - 1
            }
        };
        self.term_to_offsets_enums[index]
            .1
            .add(postings.start_offset(), postings.end_offset());
        Ok(())
    }

    fn reset(&mut self) {
        // called when at a new position.  We don't care.
    }
}

/// Equivalent to the private nested class `PhraseHelper.SpanCollectedOffsetsEnum`.
pub(crate) struct SpanCollectedOffsetsEnum {
    // TODO perhaps optionally collect (and expose) payloads?
    term: BytesRef,
    start_offsets: Vec<i32>,
    end_offsets: Vec<i32>,
    num_pairs: usize,
    enum_idx: i32,
}

impl SpanCollectedOffsetsEnum {
    fn new(term: BytesRef, postings_freq: i32) -> Self {
        Self {
            term,
            start_offsets: vec![0; postings_freq as usize], // hopefully not wasteful?  At least we needn't resize it.
            end_offsets: vec![0; postings_freq as usize],
            num_pairs: 0,
            enum_idx: -1,
        }
    }

    // called from collector before it's navigated
    fn add(&mut self, start_offset: i32, end_offset: i32) {
        debug_assert!(self.enum_idx == -1, "bad state");

        // loop backwards since we expect a match at the end or close to it.
        // We expect O(1) not O(N).
        let mut pair_idx: i64 = self.num_pairs as i64 - 1;
        while pair_idx >= 0 {
            let i_start_offset = self.start_offsets[pair_idx as usize];
            let i_end_offset = self.end_offsets[pair_idx as usize];
            let mut cmp = i_start_offset.cmp(&start_offset);
            if cmp == std::cmp::Ordering::Equal {
                cmp = i_end_offset.cmp(&end_offset);
            }
            if cmp == std::cmp::Ordering::Equal {
                return; // we already have this offset-pair for this term
            } else if cmp == std::cmp::Ordering::Less {
                break; // we will insert offsetPair to the right of pairIdx
            }
            pair_idx -= 1;
        }
        // pairIdx is now one position to the left of where we insert the
        // new pair
        let insert = (pair_idx + 1) as usize;
        // shift right any pairs by one to make room
        let shift_len = self.num_pairs - insert;
        if shift_len > 0 {
            self.start_offsets
                .copy_within(insert..insert + shift_len, insert + 1);
            self.end_offsets
                .copy_within(insert..insert + shift_len, insert + 1);
        }
        // now we can place the offset pair
        self.start_offsets[insert] = start_offset;
        self.end_offsets[insert] = end_offset;
        self.num_pairs += 1;
    }
}

impl OffsetsEnum for SpanCollectedOffsetsEnum {
    fn next_position(&mut self) -> Result<bool> {
        self.enum_idx += 1;
        Ok((self.enum_idx as usize) < self.num_pairs)
    }

    fn freq(&self) -> Result<i32> {
        Ok(self.num_pairs as i32) // not exactly the same as the postings freq
    }

    fn get_term(&self) -> Result<Option<BytesRef>> {
        Ok(Some(self.term.clone()))
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(self.start_offsets[self.enum_idx as usize])
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(self.end_offsets[self.enum_idx as usize])
    }

    fn simple_name(&self) -> &'static str {
        "SpanCollectedOffsetsEnum"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
