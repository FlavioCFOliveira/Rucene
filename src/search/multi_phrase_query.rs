//! A phrase query with alternatives at a position, ported from
//! `org.apache.lucene.search.MultiPhraseQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    LeafReaderContext, PostingsEnum, SlowImpactsEnum, Term, POSTINGS_ENUM_ALL,
    POSTINGS_ENUM_POSITIONS,
};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::exact_phrase_matcher::ExactPhraseMatcher;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::phrase_matcher::{PhraseMatcher, SharedPostings};
use crate::search::phrase_query::{term_positions_cost, PostingsAndFreq};
use crate::search::phrase_weight::{PhraseWeight, PhraseWeightImpl};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::sim_scorer_source::{SharedSimScorer, SimScorerSource};
use crate::search::similarities::{Similarity, TermStatistics};
use crate::search::sloppy_phrase_matcher::SloppyPhraseMatcher;
use crate::search::term_query::TermQuery;
use crate::search::term_states::TermStates;
use crate::search::weight::Weight;

/// A builder for multi-phrase queries.
///
/// Equivalent to `MultiPhraseQuery.Builder`.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    /// Becomes `Some` on the first `add` and is then unmodified.
    field: Option<String>,
    term_arrays: Vec<Vec<Term>>,
    positions: Vec<i32>,
    slop: i32,
}

impl Builder {
    /// Creates an empty builder.
    ///
    /// Equivalent to the default `Builder()` constructor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a builder with the same configuration as `multi_phrase_query`.
    ///
    /// Equivalent to the copy constructor `Builder(MultiPhraseQuery)`.
    pub fn from_query(multi_phrase_query: &MultiPhraseQuery) -> Self {
        Self {
            field: multi_phrase_query.field.clone(),
            term_arrays: multi_phrase_query.term_arrays.clone(),
            positions: multi_phrase_query.positions.clone(),
            slop: multi_phrase_query.slop,
        }
    }

    /// Sets the phrase slop for this query.
    ///
    /// Equivalent to `Builder.setSlop(int)`; see
    /// [`PhraseQuery::get_slop`](crate::search::PhraseQuery::get_slop).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a negative slop, which is
    /// the `IllegalArgumentException` Java throws.
    pub fn set_slop(&mut self, s: i32) -> Result<&mut Self> {
        if s < 0 {
            return Err(LuceneError::IllegalArgument(
                "slop value cannot be negative".to_string(),
            ));
        }
        self.slop = s;
        Ok(self)
    }

    /// Adds a single term at the next position in the phrase.
    ///
    /// Equivalent to `Builder.add(Term)`.
    ///
    /// # Errors
    ///
    /// As [`add_at`](Self::add_at).
    pub fn add(&mut self, term: Term) -> Result<&mut Self> {
        self.add_terms(vec![term])
    }

    /// Adds several terms at the next position in the phrase; any of them may
    /// match, as a disjunction.
    ///
    /// Equivalent to `Builder.add(Term[])`.
    ///
    /// # Errors
    ///
    /// As [`add_at`](Self::add_at).
    pub fn add_terms(&mut self, terms: Vec<Term>) -> Result<&mut Self> {
        let position = match self.positions.last() {
            None => 0,
            Some(last) => last + 1,
        };
        self.add_at(terms, position)
    }

    /// Adds several terms at an explicit relative position within the phrase.
    ///
    /// Equivalent to `Builder.add(Term[], int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a term is not on the
    /// phrase's field, which is the `IllegalArgumentException` Java throws, and
    /// when the term list is empty, which Java reports as an
    /// `ArrayIndexOutOfBoundsException` while reading `terms[0].field()`.
    pub fn add_at(&mut self, terms: Vec<Term>, position: i32) -> Result<&mut Self> {
        if self.term_arrays.is_empty() {
            let first = terms.first().ok_or_else(|| {
                LuceneError::IllegalArgument(
                    "the first term array of a MultiPhraseQuery must not be empty".to_string(),
                )
            })?;
            self.field = Some(first.field().to_string());
        }

        for term in &terms {
            if Some(term.field()) != self.field.as_deref() {
                return Err(LuceneError::IllegalArgument(format!(
                    "All phrase terms must be in the same field ({}): {term}",
                    self.field.as_deref().unwrap_or("null")
                )));
            }
        }

        self.term_arrays.push(terms);
        self.positions.push(position);
        Ok(self)
    }

    /// Builds the [`MultiPhraseQuery`].
    ///
    /// Equivalent to `Builder.build()`.
    pub fn build(&self) -> MultiPhraseQuery {
        MultiPhraseQuery::new(
            self.field.clone(),
            self.term_arrays.clone(),
            self.positions.clone(),
            self.slop,
        )
    }
}

/// A generalised [`PhraseQuery`](crate::search::PhraseQuery), which may hold
/// more than one term at the same position, treated as a disjunction.
///
/// Equivalent to `org.apache.lucene.search.MultiPhraseQuery`. To search for the
/// phrase `Microsoft app*`, first create a [`Builder`] and
/// [`add`](Builder::add) the term `microsoft` — assuming lower-case analysis —
/// then find all the terms that have `app` as a prefix by seeking a
/// [`TermsEnum`](crate::index::TermsEnum) to `app` and iterating while the
/// prefix holds, and finally [`add_terms`](Builder::add_terms) them.
#[derive(Debug, Clone)]
pub struct MultiPhraseQuery {
    field: Option<String>,
    term_arrays: Vec<Vec<Term>>,
    positions: Vec<i32>,
    slop: i32,
}

impl MultiPhraseQuery {
    /// Builds the query from its term arrays and positions.
    ///
    /// Equivalent to the private
    /// `MultiPhraseQuery(String, Term[][], int[], int)`, which [`Builder`]
    /// calls. There is no argument checking here, because the builder has
    /// already done it.
    pub fn new(
        field: Option<String>,
        term_arrays: Vec<Vec<Term>>,
        positions: Vec<i32>,
        slop: i32,
    ) -> Self {
        Self {
            field,
            term_arrays,
            positions,
            slop,
        }
    }

    /// Returns the phrase slop.
    ///
    /// Equivalent to `MultiPhraseQuery.getSlop()`.
    pub fn get_slop(&self) -> i32 {
        self.slop
    }

    /// Returns the arrays of terms of the multi-phrase.
    ///
    /// Equivalent to `MultiPhraseQuery.getTermArrays()`, which Java documents
    /// as "do not modify"; Rust's borrow makes that a rule the compiler keeps.
    pub fn get_term_arrays(&self) -> &[Vec<Term>] {
        &self.term_arrays
    }

    /// Returns the relative positions of the terms in this phrase.
    ///
    /// Equivalent to `MultiPhraseQuery.getPositions()`.
    pub fn get_positions(&self) -> &[i32] {
        &self.positions
    }

    /// Returns the field this query applies to, or `None` when it has no term.
    ///
    /// Equivalent to reading the `private final String field` field.
    pub fn get_field(&self) -> Option<&str> {
        self.field.as_deref()
    }
}

impl Query for MultiPhraseQuery {
    fn to_query_string(&self, f: &str) -> String {
        let mut buffer = String::new();
        match self.field.as_deref() {
            None => buffer.push_str("null:"),
            Some(field) => {
                if field != f {
                    buffer.push_str(field);
                    buffer.push(':');
                }
            }
        }

        buffer.push('"');
        let mut last_pos: i32 = -1;

        for (i, terms) in self.term_arrays.iter().enumerate() {
            let position = self.positions[i];
            if i != 0 {
                buffer.push(' ');
                for _ in 1..(position - last_pos) {
                    buffer.push_str("? ");
                }
            }
            if terms.len() > 1 {
                buffer.push('(');
                for (j, term) in terms.iter().enumerate() {
                    buffer.push_str(&term.text());
                    if j < terms.len() - 1 {
                        buffer.push(' ');
                    }
                }
                buffer.push(')');
            } else if let Some(term) = terms.first() {
                buffer.push_str(&term.text());
            }
            last_pos = position;
        }
        buffer.push('"');

        if self.slop != 0 {
            buffer.push('~');
            buffer.push_str(&self.slop.to_string());
        }
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let Some(field) = self.field.as_deref() else {
            return;
        };
        if !visitor.accept_field(field) {
            return;
        }
        let mut v = visitor.get_sub_visitor(Occur::MUST, self);
        for terms in &self.term_arrays {
            let mut sv = v.get_sub_visitor(Occur::SHOULD, self);
            sv.consume_terms(self, terms);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, _index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.term_arrays.is_empty() {
            Ok(Some(Arc::new(MatchNoDocsQuery::new(
                "empty MultiPhraseQuery",
            ))))
        } else if self.term_arrays.len() == 1 {
            // Optimise the one-term case.
            let mut builder = BooleanQueryBuilder::new();
            for term in &self.term_arrays[0] {
                builder.add(Arc::new(TermQuery::new(term.clone())), Occur::SHOULD)?;
            }
            Ok(Some(Arc::new(builder.build())))
        } else {
            Ok(None)
        }
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let field = self.field.clone().ok_or_else(|| {
            LuceneError::IllegalState(
                "MultiPhraseQuery has no term; call rewrite first".to_string(),
            )
        })?;
        let inner = Arc::new(MultiPhraseWeightImpl {
            query: self.clone(),
            field: field.clone(),
            score_mode,
            boost,
            similarity: Arc::clone(searcher.get_similarity()),
            term_states: std::sync::OnceLock::new(),
        });
        Ok(Arc::new(PhraseWeight::new(
            Arc::new(self.clone()),
            field,
            searcher,
            score_mode,
            inner,
        )?))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<MultiPhraseQuery>() else {
            return false;
        };
        // Equal terms imply an equal field.
        self.slop == other.slop
            && self.term_arrays == other.term_arrays
            && self.positions == other.positions
    }

    fn query_hash(&self) -> u64 {
        // Equal terms imply an equal field.
        let mut term_arrays_hash = 1u64;
        for term_array in &self.term_arrays {
            let mut hasher = DefaultHasher::new();
            for term in term_array {
                term.field().hash(&mut hasher);
                term.bytes().slice().hash(&mut hasher);
            }
            term_arrays_hash = term_arrays_hash
                .wrapping_mul(31)
                .wrapping_add(hasher.finish());
        }
        let mut hasher = DefaultHasher::new();
        self.positions.hash(&mut hasher);
        self.class_hash() ^ (self.slop as u64) ^ term_arrays_hash ^ hasher.finish()
    }
}

/// The [`PhraseWeightImpl`] of a [`MultiPhraseQuery`].
///
/// Equivalent to the anonymous `PhraseWeight` subclass
/// `MultiPhraseQuery.createWeight` returns, together with the `termStates` map
/// it closes over.
#[derive(Debug)]
struct MultiPhraseWeightImpl {
    query: MultiPhraseQuery,
    field: String,
    score_mode: ScoreMode,
    boost: f32,
    similarity: Arc<dyn Similarity>,
    /// **Divergence from Lucene 10.5.0.** Java's `createWeight` allocates a
    /// `HashMap<Term, TermStates>` outside the anonymous weight and both
    /// `getStats` and `getPhraseMatcher` read it. A `Weight` is `Sync` here and
    /// `get_phrase_matcher` takes `&self`, so the map is a write-once cell,
    /// filled by `get_stats` — which the weight's constructor always runs
    /// first, exactly as in Java.
    term_states: std::sync::OnceLock<BTreeMap<Term, Arc<TermStates>>>,
}

impl PhraseWeightImpl for MultiPhraseWeightImpl {
    fn get_stats(&self, searcher: &IndexSearcher) -> Result<Option<SharedSimScorer>> {
        // Compute the IDF.
        let mut all_term_stats: Vec<TermStatistics> = Vec::new();
        let mut term_states: BTreeMap<Term, Arc<TermStates>> = BTreeMap::new();
        for terms in self.query.get_term_arrays() {
            for term in terms {
                if !term_states.contains_key(term) {
                    let ts = Arc::new(TermStates::build(
                        searcher,
                        term,
                        self.score_mode.needs_scores(),
                    )?);
                    term_states.insert(term.clone(), ts);
                }
                let ts = &term_states[term];
                if self.score_mode.needs_scores() && ts.doc_freq()? > 0 {
                    all_term_stats.push(searcher.term_statistics(
                        term,
                        ts.doc_freq()?,
                        ts.total_term_freq()?,
                    )?);
                }
            }
        }
        let _ = self.term_states.set(term_states);
        if all_term_stats.is_empty() {
            // None of the terms was found, so the similarity is not used.
            return Ok(None);
        }
        let collection_stats = searcher
            .collection_statistics(&self.field)?
            .ok_or_else(|| {
                LuceneError::IllegalState(format!(
                    "field {} has term statistics but no collection statistics",
                    self.field
                ))
            })?;
        Ok(Some(Arc::new(SimScorerSource::new(
            Arc::clone(&self.similarity),
            self.boost,
            collection_stats,
            all_term_stats,
        ))))
    }

    fn get_phrase_matcher(
        &self,
        context: &LeafReaderContext,
        scorer: &SharedSimScorer,
        expose_offsets: bool,
    ) -> Result<Option<Box<dyn PhraseMatcher>>> {
        let term_arrays = self.query.get_term_arrays();
        debug_assert!(!term_arrays.is_empty());
        let reader = context.leaf_reader();

        let Some(field_terms) = reader.terms(&self.field)? else {
            return Ok(None);
        };

        if !field_terms.has_positions() {
            return Err(LuceneError::IllegalState(format!(
                "field \"{}\" was indexed without position data; cannot run MultiPhraseQuery (phrase={})",
                self.field,
                self.query.to_query_string("")
            )));
        }

        let term_states = self.term_states.get().ok_or_else(|| {
            LuceneError::IllegalState(
                "PhraseWeight.getStats must run before getPhraseMatcher".to_string(),
            )
        })?;

        // Reuse a single terms enum below.
        let mut terms_enum = field_terms.iterator()?;
        let mut total_match_cost = 0f32;
        let mut postings_freqs = Vec::with_capacity(term_arrays.len());

        for (pos, terms) in term_arrays.iter().enumerate() {
            let mut postings: Vec<Box<dyn PostingsEnum>> = Vec::new();

            for term in terms {
                let states = term_states.get(term).ok_or_else(|| {
                    LuceneError::IllegalState(format!("no TermStates were built for {term}"))
                })?;
                if let Some(term_state) = states.get(context)? {
                    terms_enum.seek_term_state(term.bytes(), &*term_state)?;
                    let flags = if expose_offsets {
                        POSTINGS_ENUM_ALL
                    } else {
                        POSTINGS_ENUM_POSITIONS
                    };
                    postings.push(terms_enum.postings(None, flags)?);
                    total_match_cost += term_positions_cost(&*terms_enum)?;
                }
            }

            if postings.is_empty() {
                return Ok(None);
            }

            let postings_enum: Box<dyn PostingsEnum> = if postings.len() == 1 {
                postings.pop().expect("INVARIANT: the list holds one entry")
            } else if expose_offsets {
                Box::new(UnionFullPostingsEnum::new(postings))
            } else {
                Box::new(UnionPostingsEnum::new(postings))
            };

            postings_freqs.push(PostingsAndFreq::new(
                SharedPostings::new(Box::new(SlowImpactsEnum::new(postings_enum))),
                self.query.get_positions()[pos],
                terms.clone(),
            ));
        }

        if self.query.get_slop() == 0 {
            // Sort by increasing doc-freq order.
            postings_freqs.sort();
            Ok(Some(Box::new(ExactPhraseMatcher::new(
                postings_freqs,
                self.score_mode,
                Arc::clone(scorer),
                total_match_cost,
            )?)))
        } else {
            Ok(Some(Box::new(SloppyPhraseMatcher::new(
                postings_freqs,
                self.query.get_slop(),
                self.score_mode,
                Arc::clone(scorer),
                total_match_cost,
                expose_offsets,
            )?)))
        }
    }
}

/// A disjunction of postings ordered by doc ID.
///
/// Equivalent to the static nested
/// `MultiPhraseQuery.UnionPostingsEnum.DocsQueue`.
///
/// **Divergence from Lucene 10.5.0.** Java's queue holds the `PostingsEnum`
/// objects, which are also reachable through the `subs` array; Rust forbids
/// that aliasing, so the queue holds the *indices* into `subs` and every method
/// takes that slice, which is what `lessThan` reads. `up_heap` and `down_heap`
/// reproduce `org.apache.lucene.util.PriorityQueue` exactly.
#[derive(Debug)]
struct DocsQueue {
    heap: Vec<Option<usize>>,
    size: usize,
}

impl DocsQueue {
    fn new(size: usize) -> Self {
        let heap_size = if size == 0 { 2 } else { size + 1 };
        Self {
            heap: (0..heap_size).map(|_| None).collect(),
            size: 0,
        }
    }

    fn less_than(subs: &[Box<dyn PostingsEnum>], a: usize, b: usize) -> bool {
        subs[a].doc_id() < subs[b].doc_id()
    }

    fn top(&self) -> usize {
        self.heap[1].expect("INVARIANT: a DocsQueue always holds every sub")
    }

    fn add(&mut self, subs: &[Box<dyn PostingsEnum>], element: usize) {
        let index = self.size + 1;
        self.heap[index] = Some(element);
        self.size = index;
        self.up_heap(subs, index);
    }

    fn update_top(&mut self, subs: &[Box<dyn PostingsEnum>]) -> usize {
        self.down_heap(subs, 1);
        self.top()
    }

    fn up_heap(&mut self, subs: &[Box<dyn PostingsEnum>], orig_pos: usize) {
        let mut i = orig_pos;
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: up_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i >> 1;
        while j > 0 && Self::less_than(subs, node, self.heap[j].expect(occupied)) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j >>= 1;
        }
        self.heap[i] = Some(node);
    }

    fn down_heap(&mut self, subs: &[Box<dyn PostingsEnum>], mut i: usize) {
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: down_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size
            && Self::less_than(
                subs,
                self.heap[k].expect(occupied),
                self.heap[j].expect(occupied),
            )
        {
            j = k;
        }
        while j <= self.size && Self::less_than(subs, self.heap[j].expect(occupied), node) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size
                && Self::less_than(
                    subs,
                    self.heap[k].expect(occupied),
                    self.heap[j].expect(occupied),
                )
            {
                j = k;
            }
        }
        self.heap[i] = Some(node);
    }
}

/// A sorted array of all the positions of a single document.
///
/// Equivalent to the static nested
/// `MultiPhraseQuery.UnionPostingsEnum.PositionsQueue`.
#[derive(Debug, Default)]
struct PositionsQueue {
    index: usize,
    size: usize,
    array: Vec<i32>,
}

impl PositionsQueue {
    fn new() -> Self {
        Self {
            index: 0,
            size: 0,
            array: vec![0; 16],
        }
    }

    fn add(&mut self, i: i32) {
        if self.size == self.array.len() {
            self.array.resize(self.array.len() * 2, 0);
        }
        self.array[self.size] = i;
        self.size += 1;
    }

    fn next(&mut self) -> i32 {
        let value = self.array[self.index];
        self.index += 1;
        value
    }

    fn sort(&mut self) {
        self.array[self.index..self.size].sort_unstable();
    }

    fn clear(&mut self) {
        self.index = 0;
        self.size = 0;
    }

    fn size(&self) -> usize {
        self.size
    }
}

/// Takes the logical union of several [`PostingsEnum`] iterators.
///
/// Equivalent to `MultiPhraseQuery.UnionPostingsEnum`. Note that positions are
/// merged during [`PostingsEnum::freq`].
pub struct UnionPostingsEnum {
    /// The queue ordered by doc ID.
    docs_queue: DocsQueue,
    /// The cost of this enum: the sum of the costs of its subs.
    cost: i64,
    /// The queue ordered by position for the current document.
    pos_queue: PositionsQueue,
    /// The document the position queue is working on.
    pos_queue_doc: i32,
    /// The subs, unordered.
    subs: Vec<Box<dyn PostingsEnum>>,
}

impl std::fmt::Debug for UnionPostingsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnionPostingsEnum")
            .field("subs", &self.subs.len())
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl UnionPostingsEnum {
    /// Unions the given postings.
    ///
    /// Equivalent to `UnionPostingsEnum(Collection<PostingsEnum>)`.
    pub fn new(subs: Vec<Box<dyn PostingsEnum>>) -> Self {
        let mut docs_queue = DocsQueue::new(subs.len());
        let mut cost = 0i64;
        for (i, sub) in subs.iter().enumerate() {
            cost += sub.cost();
            docs_queue.add(&subs, i);
        }
        Self {
            docs_queue,
            cost,
            pos_queue: PositionsQueue::new(),
            pos_queue_doc: -2,
            subs,
        }
    }

    /// Merges the positions of every sub positioned on the current document.
    ///
    /// Equivalent to the body of `UnionPostingsEnum.freq()` that runs when the
    /// document has changed.
    ///
    /// **Divergence from Lucene 10.5.0.** Java merges lazily, on the first
    /// `freq()` call for a document, because `freq()` may mutate the enum
    /// there. [`PostingsEnum::freq`] takes `&self` in this port, so the merge
    /// runs when the union *reaches* the document instead. The merged
    /// positions, the frequency and the order in which they are handed out are
    /// identical; only documents whose frequency is never asked for pay for a
    /// merge they did not need.
    fn merge_positions(&mut self) -> Result<()> {
        let doc = self.subs[self.docs_queue.top()].doc_id();
        if doc == NO_MORE_DOCS || doc == self.pos_queue_doc {
            return Ok(());
        }
        self.pos_queue.clear();
        for sub in &mut self.subs {
            if sub.doc_id() == doc {
                let freq = sub.freq()?;
                for _ in 0..freq {
                    let position = sub.next_position()?;
                    self.pos_queue.add(position);
                }
            }
        }
        self.pos_queue.sort();
        self.pos_queue_doc = doc;
        Ok(())
    }

    /// Returns the subs, unordered.
    ///
    /// Equivalent to reading the `final PostingsEnum[] subs` field, which
    /// `UnionFullPostingsEnum` also uses.
    pub fn subs(&self) -> &[Box<dyn PostingsEnum>] {
        &self.subs
    }

    /// Returns the subs for mutation.
    pub fn subs_mut(&mut self) -> &mut [Box<dyn PostingsEnum>] {
        &mut self.subs
    }

    /// Returns the document the position state was built for.
    ///
    /// Equivalent to reading the `int posQueueDoc` field.
    pub fn pos_queue_doc(&self) -> i32 {
        self.pos_queue_doc
    }

    /// Records the document the position state was built for.
    pub fn set_pos_queue_doc(&mut self, doc: i32) {
        self.pos_queue_doc = doc;
    }
}

impl DocIdSetIterator for UnionPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.subs[self.docs_queue.top()].doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let mut top = self.docs_queue.top();
        let doc = self.subs[top].doc_id();

        loop {
            self.subs[top].next_doc()?;
            top = self.docs_queue.update_top(&self.subs);
            if self.subs[top].doc_id() != doc {
                break;
            }
        }

        self.merge_positions()?;
        Ok(self.subs[self.docs_queue.top()].doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let mut top = self.docs_queue.top();

        loop {
            self.subs[top].advance(target)?;
            top = self.docs_queue.update_top(&self.subs);
            if self.subs[top].doc_id() >= target {
                break;
            }
        }

        self.merge_positions()?;
        Ok(self.subs[self.docs_queue.top()].doc_id())
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl PostingsEnum for UnionPostingsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.pos_queue.size() as i32)
    }

    fn next_position(&mut self) -> Result<i32> {
        Ok(self.pos_queue.next())
    }

    fn start_offset(&self) -> i32 {
        // Offsets are unsupported.
        -1
    }

    fn end_offset(&self) -> i32 {
        // Offsets are unsupported.
        -1
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        // Payloads are unsupported.
        Ok(None)
    }
}

/// One sub of a [`UnionFullPostingsEnum`], with its current position.
///
/// Equivalent to the package-private
/// `MultiPhraseQuery.PostingsAndPosition`; the postings enum lives in the
/// union's `subs` and is reached by index, because Rust forbids holding it here
/// and in `subs` at once.
#[derive(Debug, Clone, Copy)]
struct PostingsAndPosition {
    pe: usize,
    pos: i32,
    upto: i32,
}

/// A heap of [`PostingsAndPosition`] ordered by position.
///
/// Equivalent to the anonymous `PriorityQueue<PostingsAndPosition>` of
/// `UnionFullPostingsEnum`. `up_heap` and `down_heap` reproduce
/// `org.apache.lucene.util.PriorityQueue` exactly; the queue owns its elements,
/// so it can hand out the mutable top that `nextPosition` needs.
#[derive(Debug)]
struct PosQueue {
    heap: Vec<Option<PostingsAndPosition>>,
    size: usize,
}

impl PosQueue {
    fn new(size: usize) -> Self {
        let heap_size = if size == 0 { 2 } else { size + 1 };
        Self {
            heap: (0..heap_size).map(|_| None).collect(),
            size: 0,
        }
    }

    fn less_than(a: &PostingsAndPosition, b: &PostingsAndPosition) -> bool {
        a.pos < b.pos
    }

    fn top(&self) -> PostingsAndPosition {
        self.heap[1].expect("INVARIANT: the position queue is not empty while iterating")
    }

    fn top_mut(&mut self) -> &mut PostingsAndPosition {
        self.heap[1]
            .as_mut()
            .expect("INVARIANT: the position queue is not empty while iterating")
    }

    fn clear(&mut self) {
        for slot in self.heap.iter_mut() {
            *slot = None;
        }
        self.size = 0;
    }

    fn add(&mut self, element: PostingsAndPosition) {
        let index = self.size + 1;
        self.heap[index] = Some(element);
        self.size = index;
        self.up_heap(index);
    }

    fn pop(&mut self) -> Option<PostingsAndPosition> {
        if self.size == 0 {
            return None;
        }
        let result = self.heap[1].take();
        if self.size > 1 {
            self.heap[1] = self.heap[self.size].take();
            self.size -= 1;
            self.down_heap(1);
        } else {
            self.size = 0;
        }
        result
    }

    fn update_top(&mut self) {
        self.down_heap(1);
    }

    fn up_heap(&mut self, orig_pos: usize) {
        let mut i = orig_pos;
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: up_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i >> 1;
        while j > 0 && Self::less_than(&node, self.heap[j].as_ref().expect(occupied)) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j >>= 1;
        }
        self.heap[i] = Some(node);
    }

    fn down_heap(&mut self, mut i: usize) {
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: down_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size
            && Self::less_than(
                self.heap[k].as_ref().expect(occupied),
                self.heap[j].as_ref().expect(occupied),
            )
        {
            j = k;
        }
        while j <= self.size && Self::less_than(self.heap[j].as_ref().expect(occupied), &node) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size
                && Self::less_than(
                    self.heap[k].as_ref().expect(occupied),
                    self.heap[j].as_ref().expect(occupied),
                )
            {
                j = k;
            }
        }
        self.heap[i] = Some(node);
    }
}

/// A slower [`UnionPostingsEnum`] that delegates offsets and positions, for use
/// by a [`MatchesIterator`](crate::search::MatchesIterator).
///
/// Equivalent to `MultiPhraseQuery.UnionFullPostingsEnum`.
pub struct UnionFullPostingsEnum {
    base: UnionPostingsEnum,
    freq: i32,
    started: bool,
    pos_queue: PosQueue,
}

impl std::fmt::Debug for UnionFullPostingsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnionFullPostingsEnum")
            .field("freq", &self.freq)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

impl UnionFullPostingsEnum {
    /// Unions the given postings, keeping their positions and offsets.
    ///
    /// Equivalent to `UnionFullPostingsEnum(List<PostingsEnum>)`.
    pub fn new(subs: Vec<Box<dyn PostingsEnum>>) -> Self {
        let len = subs.len();
        Self {
            base: UnionPostingsEnum::new(subs),
            freq: -1,
            started: false,
            pos_queue: PosQueue::new(len),
        }
    }

    /// Rebuilds the position queue for the current document.
    ///
    /// Equivalent to the body of `UnionFullPostingsEnum.freq()`.
    ///
    /// **Divergence from Lucene 10.5.0.** As with
    /// [`UnionPostingsEnum::merge_positions`], the work moves from the first
    /// `freq()` call for a document to the moment the union reaches it, because
    /// [`PostingsEnum::freq`] takes `&self` here. Java's own cache check —
    /// `if (doc == posQueueDoc) return freq;` — never fires, because
    /// `UnionFullPostingsEnum` overrides `freq()` and therefore never assigns
    /// `posQueueDoc`; the rebuild is unconditional in both.
    fn rebuild_positions(&mut self) -> Result<()> {
        let doc = DocIdSetIterator::doc_id(&self.base);
        if doc == NO_MORE_DOCS {
            return Ok(());
        }
        self.freq = 0;
        self.started = false;
        self.pos_queue.clear();
        for i in 0..self.base.subs().len() {
            if self.base.subs()[i].doc_id() == doc {
                let subs = self.base.subs_mut();
                let pos = subs[i].next_position()?;
                let upto = subs[i].freq()?;
                self.pos_queue.add(PostingsAndPosition { pe: i, pos, upto });
                self.freq += upto;
            }
        }
        Ok(())
    }
}

impl DocIdSetIterator for UnionFullPostingsEnum {
    fn doc_id(&self) -> i32 {
        DocIdSetIterator::doc_id(&self.base)
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.base.next_doc()?;
        self.rebuild_positions()?;
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.base.advance(target)?;
        self.rebuild_positions()?;
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        DocIdSetIterator::cost(&self.base)
    }
}

impl PostingsEnum for UnionFullPostingsEnum {
    fn freq(&self) -> Result<i32> {
        Ok(self.freq)
    }

    fn next_position(&mut self) -> Result<i32> {
        if !self.started {
            self.started = true;
            return Ok(self.pos_queue.top().pos);
        }
        if self.pos_queue.top().upto == 1 {
            self.pos_queue.pop();
            return Ok(self.pos_queue.top().pos);
        }
        let pe = self.pos_queue.top().pe;
        let pos = self.base.subs_mut()[pe].next_position()?;
        {
            let top = self.pos_queue.top_mut();
            top.pos = pos;
            top.upto -= 1;
        }
        self.pos_queue.update_top();
        Ok(self.pos_queue.top().pos)
    }

    fn start_offset(&self) -> i32 {
        self.base.subs()[self.pos_queue.top().pe].start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.base.subs()[self.pos_queue.top().pe].end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.base.subs()[self.pos_queue.top().pe].get_payload()
    }
}
