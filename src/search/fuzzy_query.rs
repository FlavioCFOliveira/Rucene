//! The fuzzy search query, ported from
//! `org.apache.lucene.search.FuzzyQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{SingleTermsEnum, Term, Terms, TermsEnum};
use crate::search::automaton_query::term_hash;
use crate::search::fuzzy_automaton_builder::FuzzyAutomatonBuilder;
use crate::search::fuzzy_terms_enum::FuzzyTermsEnum;
use crate::search::index_searcher::IndexSearcher;
use crate::search::multi_term_query::{
    multi_term_query_eq, multi_term_query_hash, multi_term_rewrite, MultiTermQuery, RewriteMethod,
    TopTermsBlendedFreqScoringRewrite,
};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::util::attribute::AttributeSource;
use crate::util::automaton::{
    Automata, ByteRunAutomaton, CompiledAutomaton, MAXIMUM_SUPPORTED_DISTANCE,
};

/// The default maximum edit distance.
///
/// Equivalent to the `FuzzyQuery.defaultMaxEdits` constant.
pub const DEFAULT_MAX_EDITS: i32 = MAXIMUM_SUPPORTED_DISTANCE;

/// The default length of the required common prefix.
///
/// Equivalent to the `FuzzyQuery.defaultPrefixLength` constant.
pub const DEFAULT_PREFIX_LENGTH: i32 = 0;

/// The default maximum number of terms to match.
///
/// Equivalent to the `FuzzyQuery.defaultMaxExpansions` constant.
pub const DEFAULT_MAX_EXPANSIONS: i32 = 50;

/// Whether transpositions count as a single edit by default.
///
/// Equivalent to the `FuzzyQuery.defaultTranspositions` constant.
pub const DEFAULT_TRANSPOSITIONS: bool = true;

/// Creates the default top-terms blended-frequency scoring rewrite with the
/// given maximum number of expansions.
///
/// Equivalent to the static
/// `FuzzyQuery.defaultRewriteMethod(int)`.
pub fn default_rewrite_method(max_expansions: i32) -> Arc<dyn RewriteMethod> {
    Arc::new(TopTermsBlendedFreqScoringRewrite::new(max_expansions))
}

/// Implements the fuzzy search query.
///
/// Equivalent to `org.apache.lucene.search.FuzzyQuery`. The similarity
/// measurement is based on the Damerau–Levenshtein (optimal string alignment)
/// algorithm, though classic Levenshtein can be chosen explicitly by passing
/// `false` for `transpositions`.
///
/// This query uses [`TopTermsBlendedFreqScoringRewrite`] by default, so terms
/// are collected and scored according to their edit distance, and only the top
/// terms are used to build the boolean query. Changing the rewrite mode of a
/// fuzzy query is not recommended.
///
/// At most, this query matches terms up to
/// [`MAXIMUM_SUPPORTED_DISTANCE`] edits. Higher distances — especially with
/// transpositions enabled — are generally not useful and match a significant
/// amount of the term dictionary; an n-gram indexing technique is a better
/// answer when that is really wanted.
///
/// **NOTE**: terms of length 1 or 2 sometimes do not match, because of how the
/// scaled distance between two terms is computed: for a term to match, the edit
/// distance must be less than the minimum length of the two terms. A fuzzy
/// query on `abcd` with `max_edits = 2` does not match the indexed term `ab`,
/// and one on `a` with `max_edits = 2` does not match `abc`.
#[derive(Debug, Clone)]
pub struct FuzzyQuery {
    field: String,
    rewrite_method: Arc<dyn RewriteMethod>,
    max_edits: i32,
    max_expansions: i32,
    transpositions: bool,
    prefix_length: i32,
    term: Term,
}

impl FuzzyQuery {
    /// Creates a query matching terms at an edit distance of at most
    /// `max_edits` from `term`, with a required common prefix of
    /// `prefix_length` characters, controlling every option.
    ///
    /// Equivalent to the widest
    /// `FuzzyQuery(Term, int, int, int, boolean, RewriteMethod)`.
    ///
    /// * `max_edits` must be in `0..=`[`MAXIMUM_SUPPORTED_DISTANCE`];
    /// * `prefix_length` is the length of the common, non-fuzzy prefix;
    /// * `max_expansions` is the maximum number of terms to match — when it
    ///   exceeds [`IndexSearcher::get_max_clause_count`] at rewrite time, the
    ///   maximum clause count is used instead;
    /// * `transpositions` treats a transposition as a primitive edit operation;
    ///   when `false`, comparisons implement the classic Levenshtein algorithm.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an out-of-range
    /// `max_edits`, a negative `prefix_length` or a non-positive
    /// `max_expansions`, which are the three `IllegalArgumentException`s Java
    /// throws.
    pub fn with_rewrite_method(
        term: Term,
        max_edits: i32,
        prefix_length: i32,
        max_expansions: i32,
        transpositions: bool,
        rewrite_method: Arc<dyn RewriteMethod>,
    ) -> Result<Self> {
        if !(0..=MAXIMUM_SUPPORTED_DISTANCE).contains(&max_edits) {
            return Err(LuceneError::IllegalArgument(format!(
                "maxEdits must be between 0 and {MAXIMUM_SUPPORTED_DISTANCE}"
            )));
        }
        if prefix_length < 0 {
            return Err(LuceneError::IllegalArgument(
                "prefixLength cannot be negative.".to_string(),
            ));
        }
        if max_expansions <= 0 {
            return Err(LuceneError::IllegalArgument(
                "maxExpansions must be positive.".to_string(),
            ));
        }
        Ok(Self {
            field: term.field().to_string(),
            rewrite_method,
            max_edits,
            max_expansions,
            transpositions,
            prefix_length,
            term,
        })
    }

    /// Creates a query with the default rewrite method.
    ///
    /// Equivalent to `FuzzyQuery(Term, int, int, int, boolean)`.
    ///
    /// # Errors
    ///
    /// As [`with_rewrite_method`](Self::with_rewrite_method).
    pub fn with_expansions(
        term: Term,
        max_edits: i32,
        prefix_length: i32,
        max_expansions: i32,
        transpositions: bool,
    ) -> Result<Self> {
        Self::with_rewrite_method(
            term,
            max_edits,
            prefix_length,
            max_expansions,
            transpositions,
            default_rewrite_method(max_expansions),
        )
    }

    /// Creates a query with the default number of expansions and
    /// transpositions.
    ///
    /// Equivalent to `FuzzyQuery(Term, int, int)`.
    ///
    /// # Errors
    ///
    /// As [`with_rewrite_method`](Self::with_rewrite_method).
    pub fn with_prefix_length(term: Term, max_edits: i32, prefix_length: i32) -> Result<Self> {
        Self::with_expansions(
            term,
            max_edits,
            prefix_length,
            DEFAULT_MAX_EXPANSIONS,
            DEFAULT_TRANSPOSITIONS,
        )
    }

    /// Creates a query with the default prefix length.
    ///
    /// Equivalent to `FuzzyQuery(Term, int)`.
    ///
    /// # Errors
    ///
    /// As [`with_rewrite_method`](Self::with_rewrite_method).
    pub fn with_max_edits(term: Term, max_edits: i32) -> Result<Self> {
        Self::with_prefix_length(term, max_edits, DEFAULT_PREFIX_LENGTH)
    }

    /// Creates a query with every default.
    ///
    /// Equivalent to `FuzzyQuery(Term)`.
    ///
    /// # Errors
    ///
    /// As [`with_rewrite_method`](Self::with_rewrite_method).
    pub fn new(term: Term) -> Result<Self> {
        Self::with_max_edits(term, DEFAULT_MAX_EDITS)
    }

    /// Returns the maximum edit distance allowed for this query to match.
    ///
    /// Equivalent to `FuzzyQuery.getMaxEdits()`.
    pub fn get_max_edits(&self) -> i32 {
        self.max_edits
    }

    /// Returns the non-fuzzy prefix length: the number of characters at the
    /// start of a term that must be identical to the query term for the query
    /// to match it.
    ///
    /// Equivalent to `FuzzyQuery.getPrefixLength()`.
    pub fn get_prefix_length(&self) -> i32 {
        self.prefix_length
    }

    /// Returns `true` if transpositions are treated as a primitive edit
    /// operation.
    ///
    /// Equivalent to `FuzzyQuery.getTranspositions()`.
    pub fn get_transpositions(&self) -> bool {
        self.transpositions
    }

    /// Returns the maximum number of terms to match.
    ///
    /// Equivalent to reading the `private final int maxExpansions` field.
    pub fn get_max_expansions(&self) -> i32 {
        self.max_expansions
    }

    /// Returns the compiled automaton used to match terms.
    ///
    /// Equivalent to `FuzzyQuery.getAutomata()`.
    ///
    /// # Errors
    ///
    /// Propagates the automaton construction error.
    pub fn get_automata(&self) -> Result<CompiledAutomaton> {
        Self::get_fuzzy_automaton(
            &self.term.text(),
            self.max_edits,
            self.prefix_length,
            self.transpositions,
        )
    }

    /// Returns the [`CompiledAutomaton`] a fuzzy query uses internally to match
    /// terms.
    ///
    /// Equivalent to the internal static
    /// `FuzzyQuery.getFuzzyAutomaton(String, int, int, boolean)`. This is a
    /// very low-level method and may no longer exist should the implementation
    /// of fuzzy matching change in the future.
    ///
    /// # Errors
    ///
    /// Propagates the automaton construction error.
    pub fn get_fuzzy_automaton(
        term: &str,
        max_edits: i32,
        prefix_length: i32,
        transpositions: bool,
    ) -> Result<CompiledAutomaton> {
        FuzzyAutomatonBuilder::new(term, max_edits, prefix_length, transpositions)?
            .build_max_edit_automaton()
    }

    /// Returns the pattern term.
    ///
    /// Equivalent to `FuzzyQuery.getTerm()`.
    pub fn get_term(&self) -> &Term {
        &self.term
    }

    /// Converts a "minimum similarity" fraction to a raw edit distance.
    ///
    /// Equivalent to the static
    /// `FuzzyQuery.floatToEdits(float, int)`, where `term_len` is the length of
    /// the term in Unicode code points.
    pub fn float_to_edits(minimum_similarity: f32, term_len: i32) -> i32 {
        if minimum_similarity >= 1.0 {
            // Java's `(int) Math.min(minimumSimilarity, MAXIMUM_SUPPORTED_DISTANCE)`
            // truncates the float towards zero.
            (minimum_similarity.min(MAXIMUM_SUPPORTED_DISTANCE as f32)) as i32
        } else if minimum_similarity == 0.0 {
            // `0` means exact, not an infinite number of edits.
            0
        } else {
            (((1.0f64 - f64::from(minimum_similarity)) * f64::from(term_len)) as i32)
                .min(MAXIMUM_SUPPORTED_DISTANCE)
        }
    }
}

impl Query for FuzzyQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        if self.term.field() != field {
            buffer.push_str(self.term.field());
            buffer.push(':');
        }
        buffer.push_str(&self.term.text());
        buffer.push('~');
        buffer.push_str(&self.max_edits.to_string());
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.consume_terms_matching(self, self.term.field(), &|| {
                // Java's supplier is `() -> getAutomata().runAutomaton`, which
                // cannot report an error because the automaton was already
                // built once; a term too complex to determinize would have been
                // rejected by the constructor of the terms enum. An automaton
                // accepting nothing stands in for the impossible case rather
                // than a panic.
                self.get_automata()
                    .ok()
                    .and_then(|compiled| compiled.run_automaton)
                    .unwrap_or_else(|| {
                        ByteRunAutomaton::new(Automata::make_empty(), true)
                            .expect("INVARIANT: the empty automaton is deterministic and binary")
                    })
            });
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        multi_term_rewrite(self, index_searcher)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<FuzzyQuery>() else {
            return false;
        };
        multi_term_query_eq(self, other)
            && self.max_edits == other.max_edits
            && self.prefix_length == other.prefix_length
            && self.max_expansions == other.max_expansions
            && self.transpositions == other.transpositions
            && self.term == other.term
    }

    fn query_hash(&self) -> u64 {
        let prime = 31u64;
        let mut result = multi_term_query_hash(self);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.max_edits as u64);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.prefix_length as u64);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.max_expansions as u64);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(if self.transpositions { 0 } else { 1 });
        let mut hasher = DefaultHasher::new();
        term_hash(&self.term).hash(&mut hasher);
        prime.wrapping_mul(result).wrapping_add(hasher.finish())
    }
}

impl MultiTermQuery for FuzzyQuery {
    fn get_field(&self) -> &str {
        &self.field
    }

    fn get_rewrite_method(&self) -> Arc<dyn RewriteMethod> {
        Arc::clone(&self.rewrite_method)
    }

    fn get_terms_enum_with_atts(
        &self,
        terms: &Arc<dyn Terms>,
        atts: &mut AttributeSource,
    ) -> Result<Box<dyn TermsEnum>> {
        if self.max_edits == 0 {
            // Can only match if it is exact.
            return Ok(SingleTermsEnum::new(
                terms.iterator()?,
                self.term.bytes().clone(),
            ));
        }
        Ok(Box::new(FuzzyTermsEnum::with_attributes(
            Arc::clone(terms),
            atts,
            self.term.clone(),
            self.max_edits,
            self.prefix_length,
            self.transpositions,
        )?))
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn to_multi_term_query_arc(&self) -> Arc<dyn MultiTermQuery> {
        Arc::new(self.clone())
    }

    fn to_query_arc(&self) -> Arc<dyn Query> {
        Arc::new(self.clone())
    }
}
