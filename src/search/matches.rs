//! Match positions, ported from `org.apache.lucene.search.Matches`,
//! `MatchesIterator` and the part of `MatchesUtils` that the query-execution
//! spine needs.
//!
//! The two interfaces and the whole of `MatchesUtils` are ported here:
//! [`Weight::matches`](crate::search::Weight::matches) is defined in terms of
//! the interfaces, the boolean and disjunction-max weights amalgamate their
//! clauses with `fromSubMatches`, and the term, phrase and multi-term queries
//! build their own matches with `forField` and `disjunction`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::{Arc, LazyLock};

use crate::error::Result;
use crate::index::{IndexReaderContext, LeafReaderContext};
use crate::search::disjunction_matches_iterator::{
    from_sub_iterators, from_terms_enum as disjunction_from_terms_enum,
};
use crate::search::query::Query;
use crate::util::BytesRefIterator;

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

    /// Returns this instance as [`Any`], so that a caller can recover the
    /// concrete type.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reaches the concrete type with
    /// `instanceof` and a cast, which
    /// [`NamedMatches::find_named_matches`](crate::search::NamedMatches::find_named_matches)
    /// needs in order to pick the named nodes out of a `Matches` tree. Rust
    /// needs the escape hatch to be declared; every implementation writes
    /// `self`. It mirrors [`Query::as_any`], which exists for the same reason.
    fn as_any(&self) -> &dyn Any;
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

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Builds a [`MatchesIterator`] on demand.
///
/// Equivalent to the `IOSupplier<MatchesIterator>` parameter of
/// `MatchesUtils.forField(String, IOSupplier<MatchesIterator>)`. It is `Send`
/// and `Sync` because [`Matches`] is, and it is an [`Arc`] rather than a
/// [`Box`] so that the produced [`Matches`] can be shared.
pub type MatchesIteratorSupplier =
    Arc<dyn Fn() -> Result<Option<Box<dyn MatchesIterator>>> + Send + Sync>;

/// Static helpers that aid the implementation of [`Matches`] and
/// [`MatchesIterator`].
///
/// Equivalent to `org.apache.lucene.search.MatchesUtils`.
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

    /// Creates a [`Matches`] for a single field, or `None` when the supplier
    /// reports no match.
    ///
    /// Equivalent to
    /// `MatchesUtils.forField(String, IOSupplier<MatchesIterator>)`. The
    /// indirection through a supplier, rather than a [`MatchesIterator`]
    /// directly, is what lets several
    /// [`Matches::get_matches`] calls return new iterators; the supplier is
    /// still called eagerly, to work out whether there is a hit at all.
    ///
    /// **Divergence from Lucene 10.5.0.** Java hands the eagerly built iterator
    /// to the *first* `getMatches` call and only calls the supplier again
    /// afterwards. Caching it here would mean storing a `dyn MatchesIterator`
    /// inside a `Send + Sync` [`Matches`], which the [`MatchesIterator`] trait
    /// is not, so the eager iterator is dropped and every call builds a fresh
    /// one. Callers see the same matches — a freshly positioned iterator is
    /// exactly what the contract promises — at the cost of building one extra
    /// iterator.
    ///
    /// # Errors
    ///
    /// Propagates any error the supplier raises.
    pub fn for_field(
        field: impl Into<String>,
        mis: MatchesIteratorSupplier,
    ) -> Result<Option<Arc<dyn Matches>>> {
        if mis()?.is_none() {
            return Ok(None);
        }
        Ok(Some(Arc::new(FieldMatches {
            field: field.into(),
            mis,
        })))
    }

    /// Creates a [`MatchesIterator`] that iterates in order over all matches in
    /// a set of sub-iterators.
    ///
    /// Equivalent to `MatchesUtils.disjunction(List<MatchesIterator>)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while priming the sub-iterators.
    pub fn disjunction(
        sub_matches: Vec<Box<dyn MatchesIterator>>,
    ) -> Result<Option<Box<dyn MatchesIterator>>> {
        from_sub_iterators(sub_matches)
    }

    /// Creates a [`MatchesIterator`] that is a disjunction over a list of terms
    /// extracted from a [`BytesRefIterator`].
    ///
    /// Equivalent to
    /// `MatchesUtils.disjunction(LeafReaderContext, int, Query, String, BytesRefIterator)`.
    /// Only terms that have at least one match in the given document are
    /// included.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while seeking terms or reading postings.
    pub fn disjunction_from_terms(
        context: &LeafReaderContext,
        doc: i32,
        query: Arc<dyn Query>,
        field: &str,
        terms: Box<dyn BytesRefIterator>,
    ) -> Result<Option<Box<dyn MatchesIterator>>> {
        disjunction_from_terms_enum(context, doc, query, field, terms)
    }
}

/// Rebuilds an owning handle on a leaf context.
///
/// **This has no counterpart in Lucene 10.5.0.** Java's
/// [`Weight::matches`](crate::search::Weight::matches) captures the
/// `LeafReaderContext` it is handed, and the `Matches` it returns keeps it
/// alive. This port receives only a borrow and the returned [`Matches`] is
/// `'static`, so a handle is rebuilt from the same leaf reader, ordinal and doc
/// base. The rebuilt context has a fresh identity, which nothing that reads a
/// `Matches` looks at — only the reader, the ordinal and the doc base are used.
pub fn owned_leaf_context(context: &LeafReaderContext) -> Arc<LeafReaderContext> {
    Arc::new(LeafReaderContext::new(
        IndexReaderContext::reader(context),
        context.leaf_reader(),
        None,
        IndexReaderContext::ord_in_parent(context),
        IndexReaderContext::doc_base_in_parent(context),
        context.ord(),
        context.doc_base(),
    ))
}

/// The [`Matches`] of a single field.
///
/// Equivalent to the anonymous class `MatchesUtils.forField` returns.
struct FieldMatches {
    field: String,
    mis: MatchesIteratorSupplier,
}

impl Matches for FieldMatches {
    fn get_matches(&self, field: &str) -> Result<Option<Box<dyn MatchesIterator>>> {
        if self.field != field {
            return Ok(None);
        }
        (self.mis)()
    }

    fn get_sub_matches(&self) -> Vec<Arc<dyn Matches>> {
        Vec::new()
    }

    fn fields(&self) -> Vec<String> {
        vec![self.field.clone()]
    }

    fn as_any(&self) -> &dyn Any {
        self
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
        from_sub_iterators(sub_iterators)
    }

    fn get_sub_matches(&self) -> Vec<Arc<dyn Matches>> {
        self.sub_matches.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
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
