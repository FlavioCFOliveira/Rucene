//! Matching a sequence of terms, ported from
//! `org.apache.lucene.search.PhraseQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    ImpactsEnum, LeafReaderContext, SlowImpactsEnum, Term, TermsEnum, POSTINGS_ENUM_OFFSETS,
    POSTINGS_ENUM_POSITIONS,
};
use crate::search::boolean_clause::Occur;
use crate::search::exact_phrase_matcher::ExactPhraseMatcher;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::phrase_matcher::{PhraseMatcher, SharedPostings};
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
use crate::util::BytesRef;

/// A guess of the average number of simple operations for the initial seek and
/// buffer refill per document, for the positions of a term.
///
/// Equivalent to the private
/// `PhraseQuery.TERM_POSNS_SEEK_OPS_PER_DOC`.
const TERM_POSNS_SEEK_OPS_PER_DOC: i32 = 256;

/// The number of simple operations in a postings enum's `nextPosition()` when
/// no seek or buffer refill is done.
///
/// Equivalent to the private `PhraseQuery.TERM_OPS_PER_POS`.
const TERM_OPS_PER_POS: i32 = 7;

/// Returns the expected cost, in simple operations, of processing the
/// occurrences of a term in a document that contains it.
///
/// Equivalent to the static
/// `PhraseQuery.termPositionsCost(TermsEnum)`, for use by
/// [`TwoPhaseIterator::match_cost`](crate::search::TwoPhaseIterator::match_cost)
/// implementations. The terms enum must be positioned on the term.
///
/// # Errors
///
/// Propagates any I/O error raised while reading the statistics.
pub fn term_positions_cost(terms_enum: &dyn TermsEnum) -> Result<f32> {
    let doc_freq = terms_enum.doc_freq()?;
    debug_assert!(doc_freq > 0);
    let total_term_freq = terms_enum.total_term_freq()?;
    let exp_occurrences_in_matching_doc = total_term_freq as f32 / doc_freq as f32;
    Ok(TERM_POSNS_SEEK_OPS_PER_DOC as f32
        + exp_occurrences_in_matching_doc * TERM_OPS_PER_POS as f32)
}

/// Term postings and position information for phrase matching.
///
/// Equivalent to the internal `PhraseQuery.PostingsAndFreq`.
///
/// **Divergence from Lucene 10.5.0.** Java keeps two fields, `postings` and
/// `impacts`, which hold the *same* enum under
/// [`ScoreMode::TOP_SCORES`] and a postings enum plus a `SlowImpactsEnum`
/// wrapping it otherwise. Rust cannot alias one object through two fields, so
/// this type keeps a single [`SharedPostings`] that answers both roles: the
/// non-`TOP_SCORES` case wraps the postings in a
/// [`SlowImpactsEnum`](crate::index::SlowImpactsEnum) first, which delegates
/// every postings method to it, so the enum the matcher reads positions from is
/// the one Java reads them from.
#[derive(Debug)]
pub struct PostingsAndFreq {
    postings: SharedPostings,
    position: i32,
    terms: Vec<Term>,
    /// For faster comparisons.
    n_terms: usize,
}

impl PostingsAndFreq {
    /// Creates a `PostingsAndFreq` instance.
    ///
    /// Equivalent to
    /// `PostingsAndFreq(PostingsEnum, ImpactsEnum, int, Term...)` and to its
    /// `List<Term>` overload, which differ only in the shape of the term
    /// argument.
    pub fn new(postings: SharedPostings, position: i32, terms: Vec<Term>) -> Self {
        let n_terms = terms.len();
        let mut terms = terms;
        if n_terms > 1 {
            terms.sort();
        }
        Self {
            postings,
            position,
            terms,
            n_terms,
        }
    }

    /// Returns the shared postings list.
    pub fn postings(&self) -> &SharedPostings {
        &self.postings
    }

    /// Unwraps this value, returning the shared postings list.
    pub fn into_postings(self) -> SharedPostings {
        self.postings
    }

    /// Returns the position of the term within the phrase.
    pub fn position(&self) -> i32 {
        self.position
    }

    /// Returns the terms, sorted, or an empty slice when there are none.
    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    /// Returns the number of terms.
    pub fn n_terms(&self) -> usize {
        self.n_terms
    }
}

impl PartialEq for PostingsAndFreq {
    /// Equivalent to `PostingsAndFreq.equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.position == other.position && self.terms == other.terms
    }
}

impl Eq for PostingsAndFreq {}

impl PartialOrd for PostingsAndFreq {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PostingsAndFreq {
    /// Equivalent to `PostingsAndFreq.compareTo(PostingsAndFreq)`.
    fn cmp(&self, other: &Self) -> Ordering {
        if self.position != other.position {
            return self.position.cmp(&other.position);
        }
        if self.n_terms != other.n_terms {
            return self.n_terms.cmp(&other.n_terms);
        }
        if self.n_terms == 0 {
            return Ordering::Equal;
        }
        for (a, b) in self.terms.iter().zip(&other.terms) {
            let res = a.cmp(b);
            if res != Ordering::Equal {
                return res;
            }
        }
        Ordering::Equal
    }
}

/// A builder for phrase queries.
///
/// Equivalent to `PhraseQuery.Builder`.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    slop: i32,
    max_terms: i32,
    terms: Vec<Term>,
    positions: Vec<i32>,
}

impl Builder {
    /// Creates an empty builder.
    ///
    /// Equivalent to the sole `Builder()` constructor, whose slop is `0` and
    /// whose maximum number of terms is `-1`, that is unbounded.
    pub fn new() -> Self {
        Self {
            slop: 0,
            max_terms: -1,
            terms: Vec::new(),
            positions: Vec::new(),
        }
    }

    /// Sets the slop.
    ///
    /// Equivalent to `Builder.setSlop(int)`; see
    /// [`PhraseQuery::get_slop`].
    pub fn set_slop(&mut self, slop: i32) -> &mut Self {
        self.slop = slop;
        self
    }

    /// Sets the maximum number of terms allowed in the phrase query, which
    /// helps prevent excessive memory usage for very long phrases.
    ///
    /// Equivalent to `Builder.setMaxTerms(int)`. Adding more terms than this
    /// threshold is reported as an error.
    pub fn set_max_terms(&mut self, max_terms: i32) -> &mut Self {
        self.max_terms = max_terms;
        self
    }

    /// Adds a term to the end of the query phrase, at the position immediately
    /// after the last term added.
    ///
    /// Equivalent to `Builder.add(Term)`.
    ///
    /// # Errors
    ///
    /// As [`add_at`](Self::add_at).
    pub fn add(&mut self, term: Term) -> Result<&mut Self> {
        let position = match self.positions.last() {
            None => 0,
            Some(last) => 1 + last,
        };
        self.add_at(term, position)
    }

    /// Adds a term to the end of the query phrase at an explicit position,
    /// which must be greater than or equal to that of the previously added
    /// term.
    ///
    /// Equivalent to `Builder.add(Term, int)`. A greater position allows
    /// phrases with gaps, in connection with stop words for instance. If the
    /// position is equal, [`MultiPhraseQuery`](crate::search::MultiPhraseQuery)
    /// is most likely what is wanted, since it only requires one term at each
    /// position to match while this class requires all of them.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the position is negative,
    /// when it goes backwards, when the term is on another field than the first
    /// one, or when the maximum number of terms is exceeded — the four
    /// `IllegalArgumentException`s Java throws.
    pub fn add_at(&mut self, term: Term, position: i32) -> Result<&mut Self> {
        if position < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "Positions must be >= 0, got {position}"
            )));
        }
        if let Some(&last_position) = self.positions.last() {
            if position < last_position {
                return Err(LuceneError::IllegalArgument(format!(
                    "Positions must be added in order, got {position} after {last_position}"
                )));
            }
        }
        if let Some(first) = self.terms.first() {
            if term.field() != first.field() {
                return Err(LuceneError::IllegalArgument(format!(
                    "All terms must be on the same field, got {} and {}",
                    term.field(),
                    first.field()
                )));
            }
        }
        if self.max_terms > 0 && self.terms.len() >= self.max_terms as usize {
            return Err(LuceneError::IllegalArgument(format!(
                "The current number of terms is {}, which exceeds the limit of {}",
                self.terms.len(),
                self.max_terms
            )));
        }
        self.terms.push(term);
        self.positions.push(position);
        Ok(self)
    }

    /// Builds a phrase query from the terms that have been added.
    ///
    /// Equivalent to `Builder.build()`.
    ///
    /// # Errors
    ///
    /// As [`PhraseQuery::new`].
    pub fn build(&self) -> Result<PhraseQuery> {
        PhraseQuery::new(self.slop, self.terms.clone(), self.positions.clone())
    }
}

/// A [`Query`] that matches documents containing a particular sequence of
/// terms.
///
/// Equivalent to `org.apache.lucene.search.PhraseQuery`, which a query parser
/// builds for input like `"new york"`. It may be combined with other terms or
/// queries with a [`BooleanQuery`](crate::search::BooleanQuery).
///
/// **NOTE**: all terms in the phrase must match, even those at the same
/// position. For terms at the same position — synonyms, perhaps —
/// [`MultiPhraseQuery`](crate::search::MultiPhraseQuery) is what is wanted,
/// since it only requires one term at a position to match. Leading holes have
/// no particular meaning for this query and are ignored: a phrase built from
/// `(body:one, 4)` and `(body:two, 5)` is equivalent to one built from
/// `(body:one, 0)` and `(body:two, 1)`.
#[derive(Debug, Clone)]
pub struct PhraseQuery {
    slop: i32,
    field: Option<String>,
    terms: Vec<Term>,
    positions: Vec<i32>,
}

impl PhraseQuery {
    /// Builds a phrase query from its terms and their positions.
    ///
    /// Equivalent to the private `PhraseQuery(int, Term[], int[])`, which
    /// [`Builder::build`] calls. It is public here because Rust has no
    /// package-private visibility.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when there are not as many
    /// terms as positions, when the slop is negative, when the terms are not
    /// all on the same field, when a position is negative, or when the
    /// positions go backwards — the five `IllegalArgumentException`s Java
    /// throws.
    pub fn new(slop: i32, terms: Vec<Term>, positions: Vec<i32>) -> Result<Self> {
        if terms.len() != positions.len() {
            return Err(LuceneError::IllegalArgument(
                "Must have as many terms as positions".to_string(),
            ));
        }
        if slop < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "Slop must be >= 0, got {slop}"
            )));
        }
        for i in 1..terms.len() {
            if terms[i - 1].field() != terms[i].field() {
                return Err(LuceneError::IllegalArgument(
                    "All terms should have the same field".to_string(),
                ));
            }
        }
        for &position in &positions {
            if position < 0 {
                return Err(LuceneError::IllegalArgument(format!(
                    "Positions must be >= 0, got {position}"
                )));
            }
        }
        for i in 1..positions.len() {
            if positions[i] < positions[i - 1] {
                return Err(LuceneError::IllegalArgument(format!(
                    "Positions should not go backwards, got {} before {}",
                    positions[i - 1],
                    positions[i]
                )));
            }
        }
        let field = terms.first().map(|t| t.field().to_string());
        Ok(Self {
            slop,
            field,
            terms,
            positions,
        })
    }

    /// Creates a phrase query matching documents that contain the given terms
    /// at consecutive positions in `field`, at a maximum edit distance of
    /// `slop`.
    ///
    /// Equivalent to `PhraseQuery(int, String, String...)`; for more
    /// complicated use cases, use [`Builder`].
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_slop(slop: i32, field: &str, terms: &[&str]) -> Result<Self> {
        let terms: Vec<Term> = terms.iter().map(|t| Term::from_text(field, t)).collect();
        let positions = incremental_positions(terms.len());
        Self::new(slop, terms, positions)
    }

    /// Creates a phrase query matching documents that contain the given terms
    /// at consecutive positions in `field`.
    ///
    /// Equivalent to `PhraseQuery(String, String...)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_strings(field: &str, terms: &[&str]) -> Result<Self> {
        Self::with_slop(0, field, terms)
    }

    /// Creates a phrase query from term bytes, at a maximum edit distance of
    /// `slop`.
    ///
    /// Equivalent to `PhraseQuery(int, String, BytesRef...)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_slop_bytes(slop: i32, field: &str, terms: &[BytesRef]) -> Result<Self> {
        let terms: Vec<Term> = terms.iter().map(|t| Term::new(field, t.clone())).collect();
        let positions = incremental_positions(terms.len());
        Self::new(slop, terms, positions)
    }

    /// Creates a phrase query from term bytes.
    ///
    /// Equivalent to `PhraseQuery(String, BytesRef...)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_bytes(field: &str, terms: &[BytesRef]) -> Result<Self> {
        Self::with_slop_bytes(0, field, terms)
    }

    /// Returns the slop of this phrase query.
    ///
    /// Equivalent to `PhraseQuery.getSlop()`. The slop is an edit distance
    /// between the respective positions of the terms as defined in this query
    /// and the positions of the terms in a document.
    ///
    /// For instance, when searching for `"quick fox"`, the difference between
    /// the positions of `fox` and `quick` is expected to be `1`, so
    /// `a quick brown fox` is at an edit distance of `1`, since the difference
    /// of the positions is `2`. Similarly, `the fox is quick` is at an edit
    /// distance of `3`, since the difference of the positions is `-2`. The slop
    /// defines the maximum edit distance for a document to match, and more
    /// exact matches score higher than sloppier ones.
    pub fn get_slop(&self) -> i32 {
        self.slop
    }

    /// Returns the field this query applies to, or `None` when it has no term.
    ///
    /// Equivalent to `PhraseQuery.getField()`, which answers `null` for an
    /// empty phrase.
    pub fn get_field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    /// Returns the terms of this phrase.
    ///
    /// Equivalent to `PhraseQuery.getTerms()`.
    pub fn get_terms(&self) -> &[Term] {
        &self.terms
    }

    /// Returns the relative positions of the terms in this phrase.
    ///
    /// Equivalent to `PhraseQuery.getPositions()`.
    pub fn get_positions(&self) -> &[i32] {
        &self.positions
    }
}

/// The positions `0, 1, .., length - 1`.
///
/// Equivalent to the private static
/// `PhraseQuery.incrementalPositions(int)`.
fn incremental_positions(length: usize) -> Vec<i32> {
    (0..length as i32).collect()
}

impl Query for PhraseQuery {
    fn to_query_string(&self, f: &str) -> String {
        let mut buffer = String::new();
        if let Some(field) = self.field.as_deref() {
            if field != f {
                buffer.push_str(field);
                buffer.push(':');
            }
        }

        buffer.push('"');
        let max_position = match self.positions.last() {
            None => -1,
            Some(&last) => last,
        };
        let mut pieces: Vec<Option<String>> = vec![None; (max_position + 1).max(0) as usize];
        for i in 0..self.terms.len() {
            let pos = self.positions[i] as usize;
            let text = self.terms[i].text();
            pieces[pos] = Some(match pieces[pos].take() {
                None => text,
                Some(s) => format!("{s}|{text}"),
            });
        }
        for (i, piece) in pieces.iter().enumerate() {
            if i > 0 {
                buffer.push(' ');
            }
            match piece {
                None => buffer.push('?'),
                Some(s) => buffer.push_str(s),
            }
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
        v.consume_terms(self, &self.terms);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, _index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.terms.is_empty() {
            Ok(Some(Arc::new(MatchNoDocsQuery::new("empty PhraseQuery"))))
        } else if self.terms.len() == 1 {
            Ok(Some(Arc::new(TermQuery::new(self.terms[0].clone()))))
        } else if self.positions[0] != 0 {
            let new_positions: Vec<i32> = self
                .positions
                .iter()
                .map(|p| p - self.positions[0])
                .collect();
            Ok(Some(Arc::new(PhraseQuery::new(
                self.slop,
                self.terms.clone(),
                new_positions,
            )?)))
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
                "PhraseWeight does not support less than 2 terms, call rewrite first".to_string(),
            )
        })?;
        let inner = Arc::new(PhraseQueryWeightImpl {
            query: self.clone(),
            field: field.clone(),
            score_mode,
            boost,
            similarity: Arc::clone(searcher.get_similarity()),
            states: std::sync::OnceLock::new(),
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
        let Some(other) = other.as_any().downcast_ref::<PhraseQuery>() else {
            return false;
        };
        self.slop == other.slop && self.terms == other.terms && self.positions == other.positions
    }

    fn query_hash(&self) -> u64 {
        let mut h = self.class_hash();
        h = h.wrapping_mul(31).wrapping_add(self.slop as u64);
        let mut hasher = DefaultHasher::new();
        for term in &self.terms {
            term.field().hash(&mut hasher);
            term.bytes().slice().hash(&mut hasher);
        }
        h = h.wrapping_mul(31).wrapping_add(hasher.finish());
        let mut hasher = DefaultHasher::new();
        self.positions.hash(&mut hasher);
        h.wrapping_mul(31).wrapping_add(hasher.finish())
    }
}

/// The [`PhraseWeightImpl`] of a [`PhraseQuery`].
///
/// Equivalent to the anonymous `PhraseWeight` subclass
/// `PhraseQuery.createWeight` returns; it holds what that subclass reads from
/// its enclosing scope.
#[derive(Debug)]
struct PhraseQueryWeightImpl {
    query: PhraseQuery,
    field: String,
    score_mode: ScoreMode,
    boost: f32,
    similarity: Arc<dyn Similarity>,
    /// The per-term states, built by `get_stats` and read by
    /// `get_phrase_matcher`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java stores them in a `transient`
    /// field it assigns from `getStats`, which the constructor calls before any
    /// leaf is visited. A `Weight` is `Sync` here and `get_phrase_matcher`
    /// takes `&self`, so the field is a write-once cell.
    states: std::sync::OnceLock<Vec<Arc<TermStates>>>,
}

impl PhraseWeightImpl for PhraseQueryWeightImpl {
    fn get_stats(&self, searcher: &IndexSearcher) -> Result<Option<SharedSimScorer>> {
        let positions = self.query.get_positions();
        if positions.len() < 2 {
            return Err(LuceneError::IllegalState(
                "PhraseWeight does not support less than 2 terms, call rewrite first".to_string(),
            ));
        }
        if positions[0] != 0 {
            return Err(LuceneError::IllegalState(
                "PhraseWeight requires that the first position is 0, call rewrite first"
                    .to_string(),
            ));
        }
        let terms = self.query.get_terms();
        let mut states = Vec::with_capacity(terms.len());
        let mut term_stats: Vec<TermStatistics> = Vec::with_capacity(terms.len());
        for term in terms {
            let ts = Arc::new(TermStates::build(
                searcher,
                term,
                self.score_mode.needs_scores(),
            )?);
            if self.score_mode.needs_scores() && ts.doc_freq()? > 0 {
                term_stats.push(searcher.term_statistics(
                    term,
                    ts.doc_freq()?,
                    ts.total_term_freq()?,
                )?);
            }
            states.push(ts);
        }
        let _ = self.states.set(states);
        if term_stats.is_empty() {
            // No terms at all, so the similarity is not used.
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
            term_stats,
        ))))
    }

    fn get_phrase_matcher(
        &self,
        context: &LeafReaderContext,
        scorer: &SharedSimScorer,
        expose_offsets: bool,
    ) -> Result<Option<Box<dyn PhraseMatcher>>> {
        let terms = self.query.get_terms();
        debug_assert!(!terms.is_empty());
        let reader = context.leaf_reader();

        let Some(field_terms) = reader.terms(&self.field)? else {
            return Ok(None);
        };

        if !field_terms.has_positions() {
            return Err(LuceneError::IllegalState(format!(
                "field \"{}\" was indexed without position data; cannot run PhraseQuery (phrase={})",
                self.field,
                self.query.to_query_string("")
            )));
        }

        let states = self.states.get().ok_or_else(|| {
            LuceneError::IllegalState(
                "PhraseWeight.getStats must run before getPhraseMatcher".to_string(),
            )
        })?;

        // Reuse a single terms enum below.
        let mut te = field_terms.iterator()?;
        let mut total_match_cost = 0f32;
        let mut postings_freqs = Vec::with_capacity(terms.len());

        for (i, t) in terms.iter().enumerate() {
            let Some(state) = states[i].get(context)? else {
                // The term does not exist in this segment.
                return Ok(None);
            };
            te.seek_term_state(t.bytes(), &*state)?;
            let flags = if expose_offsets {
                POSTINGS_ENUM_OFFSETS
            } else {
                POSTINGS_ENUM_POSITIONS
            };
            let postings: Box<dyn ImpactsEnum> = if self.score_mode == ScoreMode::TOP_SCORES {
                te.impacts(flags)?
            } else {
                Box::new(SlowImpactsEnum::new(te.postings(None, flags)?))
            };
            postings_freqs.push(PostingsAndFreq::new(
                SharedPostings::new(postings),
                self.query.get_positions()[i],
                vec![t.clone()],
            ));
            total_match_cost += term_positions_cost(&*te)?;
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
