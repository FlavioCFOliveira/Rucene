//! Match positions, ported from `org.apache.lucene.search.Matches`,
//! `MatchesIterator` and the part of `MatchesUtils` that the query-execution
//! spine needs.
//!
//! The two interfaces, the `MATCH_WITH_NO_TERMS` singleton and
//! `MatchesUtils.fromSubMatches` are ported here: the first two are what
//! [`Weight::matches`](crate::search::Weight::matches) is defined in terms of,
//! and the third is what the boolean and disjunction-max weights amalgamate
//! their clauses with. The rest of `MatchesUtils` belongs with the queries that
//! build composite matches.

#![deny(unsafe_code)]

use std::sync::{Arc, LazyLock};

use crate::error::{LuceneError, Result};
use crate::search::query::Query;

/// Reports the positions, and optionally the offsets, of all the matching terms
/// of a query for a single document.
///
/// Equivalent to `org.apache.lucene.search.Matches`. To obtain a
/// [`MatchesIterator`] for a particular field, call
/// [`get_matches`](Self::get_matches); it may be called several times to
/// retrieve new iterators, but it is not thread-safe.
///
/// Java's interface extends `Iterable<String>` over the fields that have
/// matches; that iteration is [`fields`](Self::fields) here, because Rust
/// cannot make a trait object iterable by inheritance.
pub trait Matches: Send + Sync {
    /// Returns an iterator over the matches for a single field, or `None` if
    /// there are no matches in that field.
    ///
    /// Equivalent to `Matches.getMatches(String)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the iterator.
    fn get_matches(&self, field: &str) -> Result<Option<Box<dyn MatchesIterator>>>;

    /// Returns the collection of [`Matches`] that make up this instance; if it
    /// is not a composite, this returns an empty list.
    ///
    /// Equivalent to `Matches.getSubMatches()`.
    fn get_sub_matches(&self) -> Vec<Arc<dyn Matches>>;

    /// Returns the names of the fields that have matches.
    ///
    /// Equivalent to iterating the Java `Matches`, which is an
    /// `Iterable<String>` over exactly those field names.
    fn fields(&self) -> Vec<String>;
}

/// An iterator over match positions, and optionally offsets, for a single
/// document and field.
///
/// Equivalent to `org.apache.lucene.search.MatchesIterator`. Call
/// [`next`](Self::next) until it returns `false`, retrieving positions and/or
/// offsets after each call. The position and offset methods must not be called
/// before [`next`](Self::next) has returned `true`, nor after it has returned
/// `false`. Matches are ordered by start position and then by end position, and
/// match intervals may overlap.
pub trait MatchesIterator {
    /// Advances the iterator to the next match position, returning `true` if
    /// matches have not been exhausted.
    ///
    /// Equivalent to `MatchesIterator.next()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing.
    fn next(&mut self) -> Result<bool>;

    /// The start position of the current match, or `-1` if positions are not
    /// available.
    ///
    /// Equivalent to `MatchesIterator.startPosition()`.
    fn start_position(&self) -> i32;

    /// The end position of the current match, or `-1` if positions are not
    /// available.
    ///
    /// Equivalent to `MatchesIterator.endPosition()`.
    fn end_position(&self) -> i32;

    /// The starting offset of the current match, or `-1` if offsets are not
    /// available.
    ///
    /// Equivalent to `MatchesIterator.startOffset()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the offset.
    fn start_offset(&self) -> Result<i32>;

    /// The ending offset of the current match, or `-1` if offsets are not
    /// available.
    ///
    /// Equivalent to `MatchesIterator.endOffset()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the offset.
    fn end_offset(&self) -> Result<i32>;

    /// Returns an iterator over the positions and offsets of the individual
    /// terms within the current match, or `None` when the current iterator is
    /// already at the leaf level.
    ///
    /// Equivalent to `MatchesIterator.getSubMatches()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the iterator.
    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>>;

    /// Returns the query causing the current match.
    ///
    /// Equivalent to `MatchesIterator.getQuery()`.
    fn get_query(&self) -> Arc<dyn Query>;
}

/// Indicates a match with no term positions — for example on a point or
/// doc-values field, or a field indexed as docs and freqs only.
///
/// Equivalent to the anonymous class behind
/// `MatchesUtils.MATCH_WITH_NO_TERMS`.
#[derive(Debug, Default, Clone, Copy)]
pub struct MatchWithNoTerms;

impl Matches for MatchWithNoTerms {
    fn get_matches(&self, _field: &str) -> Result<Option<Box<dyn MatchesIterator>>> {
        Ok(None)
    }

    fn get_sub_matches(&self) -> Vec<Arc<dyn Matches>> {
        Vec::new()
    }

    fn fields(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Static helpers that aid the implementation of [`Matches`] and
/// [`MatchesIterator`].
///
/// Equivalent to `org.apache.lucene.search.MatchesUtils`, restricted to the
/// singleton that the execution spine needs; see the module documentation.
#[derive(Debug, Clone, Copy)]
pub struct MatchesUtils;

static MATCH_WITH_NO_TERMS: LazyLock<Arc<dyn Matches>> =
    LazyLock::new(|| Arc::new(MatchWithNoTerms));

impl MatchesUtils {
    /// Returns the shared [`MatchWithNoTerms`] instance.
    ///
    /// Equivalent to the `MatchesUtils.MATCH_WITH_NO_TERMS` constant. Java can
    /// expose it as a `static final` field; Rust needs a function because the
    /// value is built behind a [`LazyLock`].
    pub fn match_with_no_terms() -> Arc<dyn Matches> {
        Arc::clone(&MATCH_WITH_NO_TERMS)
    }

    /// Amalgamates a collection of [`Matches`] into a single object, or returns
    /// `None` when the collection is empty.
    ///
    /// Equivalent to `MatchesUtils.fromSubMatches(List<Matches>)`, which
    /// `BooleanWeight.matches` and `DisjunctionMaxQuery`'s weight call. Java
    /// filters the shared `MATCH_WITH_NO_TERMS` singleton out by reference
    /// identity; [`Arc::ptr_eq`] against
    /// [`match_with_no_terms`](Self::match_with_no_terms) is exactly that test.
    pub fn from_sub_matches(sub_matches: Vec<Arc<dyn Matches>>) -> Option<Arc<dyn Matches>> {
        if sub_matches.is_empty() {
            return None;
        }
        let no_terms = Self::match_with_no_terms();
        let sm: Vec<Arc<dyn Matches>> = sub_matches
            .iter()
            .filter(|m| !Arc::ptr_eq(m, &no_terms))
            .map(Arc::clone)
            .collect();
        if sm.is_empty() {
            return Some(no_terms);
        }
        if sm.len() == 1 {
            return Some(Arc::clone(&sm[0]));
        }
        Some(Arc::new(CompositeMatches { sm, sub_matches }))
    }
}

/// The amalgamation of several [`Matches`].
///
/// Equivalent to the anonymous class `MatchesUtils.fromSubMatches` returns when
/// more than one sub-match survives the filtering.
struct CompositeMatches {
    /// The sub-matches that carry terms.
    sm: Vec<Arc<dyn Matches>>,
    /// Every sub-match, including the term-less ones; this is what
    /// `getSubMatches()` returns.
    sub_matches: Vec<Arc<dyn Matches>>,
}

impl Matches for CompositeMatches {
    fn get_matches(&self, field: &str) -> Result<Option<Box<dyn MatchesIterator>>> {
        let mut sub_iterators = Vec::with_capacity(self.sm.len());
        for matches in &self.sm {
            if let Some(iterator) = matches.get_matches(field)? {
                sub_iterators.push(iterator);
            }
        }
        // Equivalent to `DisjunctionMatchesIterator.fromSubIterators`, whose
        // empty and single-iterator cases are these two.
        match sub_iterators.len() {
            0 => Ok(None),
            1 => Ok(sub_iterators.pop()),
            _ => Err(LuceneError::UnsupportedOperation(
                "combining the match positions of several clauses needs \
                 org.apache.lucene.search.DisjunctionMatchesIterator, which is not ported yet"
                    .to_string(),
            )),
        }
    }

    fn get_sub_matches(&self) -> Vec<Arc<dyn Matches>> {
        self.sub_matches.clone()
    }

    fn fields(&self) -> Vec<String> {
        // For each sub-match, iterate its fields and return the distinct set,
        // in first-seen order.
        let mut fields: Vec<String> = Vec::new();
        for matches in &self.sm {
            for field in matches.fields() {
                if !fields.contains(&field) {
                    fields.push(field);
                }
            }
        }
        fields
    }
}
