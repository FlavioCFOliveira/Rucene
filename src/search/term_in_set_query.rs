//! A disjunction over many terms of one field, ported from
//! `org.apache.lucene.search.TermInSetQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::Result;
use crate::index::{
    AcceptStatus, FilteredTermsEnum, FilteredTermsEnumFilter, PrefixCodedTerms,
    PrefixCodedTermsBuilder, PrefixCodedTermsIterator, Term, Terms, TermsEnum,
};
use crate::search::index_or_doc_values_query::IndexOrDocValuesQuery;
use crate::search::index_searcher::IndexSearcher;
use crate::search::multi_term_query::{
    constant_score_blended_rewrite, doc_values_rewrite, multi_term_rewrite, MultiTermQuery,
    RewriteMethod,
};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::term_range_query::term_bytes_to_string;
use crate::util::attribute::AttributeSource;
use crate::util::automaton::{Automata, ByteRunAutomaton};
use crate::util::{Accountable, BytesRef, BytesRefIterator, RamUsageEstimator};

/// The shallow size of a [`TermInSetQuery`], standing in for Java's
/// `RamUsageEstimator.shallowSizeOfInstance(TermInSetQuery.class)`.
const BASE_RAM_BYTES_USED: i64 =
    3 * RamUsageEstimator::NUM_BYTES_OBJECT_REF + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + 4;

/// A specialisation for a disjunction over many terms which, by default,
/// behaves like a [`ConstantScoreQuery`](crate::search::ConstantScoreQuery)
/// over a [`BooleanQuery`](crate::search::BooleanQuery) containing only
/// [`Occur::SHOULD`](crate::search::Occur) clauses.
///
/// Equivalent to `org.apache.lucene.search.TermInSetQuery`, which implements
/// [`Accountable`]. Unless a custom [`RewriteMethod`] is provided, this query
/// executes like a regular disjunction where there are few terms; when there
/// are many, instead of merging iterators on the fly, it populates a bit set
/// with the matching docs of the least costly terms and maintains a
/// size-limited set of more costly iterators that are merged on the fly — see
/// [`constant_score_blended_rewrite`].
///
/// A custom [`RewriteMethod`] may define a different execution behaviour, such
/// as relying on doc values
/// ([`doc_values_rewrite`](crate::search::doc_values_rewrite)) or computing
/// scores ([`scoring_boolean_rewrite`](crate::search::scoring_boolean_rewrite));
/// see [`MultiTermQuery`] for the other options.
///
/// **NOTE**: this query produces scores that are equal to its boost.
///
/// **Scope note.** Java also offers the static factory
/// `TermInSetQuery.newIndexOrDocValuesQuery(RewriteMethod, String, Collection<BytesRef>)`,
/// which packs the terms once and pairs an indexed query with a doc-values one
/// inside an `IndexOrDocValuesQuery`. That query type belongs to
/// `org.apache.lucene.search` but is not part of this port yet, so the factory
/// is absent; building the two `TermInSetQuery` instances by hand produces the
/// same pair, only packing the terms twice.
#[derive(Debug, Clone)]
pub struct TermInSetQuery {
    field: String,
    rewrite_method: Arc<dyn RewriteMethod>,
    term_data: Arc<PrefixCodedTerms>,
    /// Cached hash code of `term_data`, as in Java.
    term_data_hash_code: u64,
    /// The encoded size of `term_data`, computed once; see
    /// [`ram_bytes_used`](Accountable::ram_bytes_used).
    term_data_bytes: i64,
}

impl TermInSetQuery {
    /// Creates a query from the given collection of terms, using
    /// [`constant_score_blended_rewrite`].
    ///
    /// Equivalent to `TermInSetQuery(String, Collection<BytesRef>)`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while packing the terms.
    pub fn new(field: impl Into<String>, terms: Vec<BytesRef>) -> Result<Self> {
        Self::with_rewrite_method(constant_score_blended_rewrite(), field, terms)
    }

    /// Creates a query from the given collection of terms, with a rewrite
    /// method.
    ///
    /// Equivalent to
    /// `TermInSetQuery(RewriteMethod, String, Collection<BytesRef>)`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while packing the terms.
    pub fn with_rewrite_method(
        rewrite_method: Arc<dyn RewriteMethod>,
        field: impl Into<String>,
        terms: Vec<BytesRef>,
    ) -> Result<Self> {
        let field = field.into();
        let term_data = Self::pack_terms(&field, terms)?;
        Self::from_packed(rewrite_method, field, Arc::new(term_data))
    }

    /// Creates a query from already-packed terms.
    ///
    /// Equivalent to the private
    /// `TermInSetQuery(RewriteMethod, String, PrefixCodedTerms)`, which the
    /// `newIndexOrDocValuesQuery` factory uses so that the terms are packed
    /// once; it is public here because Rust has no package visibility.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while reading the packed terms back.
    pub fn from_packed(
        rewrite_method: Arc<dyn RewriteMethod>,
        field: impl Into<String>,
        term_data: Arc<PrefixCodedTerms>,
    ) -> Result<Self> {
        let (term_data_hash_code, term_data_bytes) = Self::digest(&term_data)?;
        Ok(Self {
            field: field.into(),
            rewrite_method,
            term_data,
            term_data_hash_code,
            term_data_bytes,
        })
    }

    /// Creates an [`IndexOrDocValuesQuery`] combining two
    /// [`TermInSetQuery`]s over the same terms, which are packed only once —
    /// which is faster.
    ///
    /// Equivalent to the static
    /// `TermInSetQuery.newIndexOrDocValuesQuery(RewriteMethod, String, Collection<BytesRef>)`.
    /// The doc-values query always uses
    /// [`doc_values_rewrite`](crate::search::doc_values_rewrite).
    ///
    /// * `index_rewrite_method` — the rewrite method the indexed query uses;
    /// * `field` — the field name of both the indexed and the doc-values query;
    /// * `terms` — the query terms.
    ///
    /// # Errors
    ///
    /// Propagates the error the term encoder raises.
    pub fn new_index_or_doc_values_query(
        index_rewrite_method: Arc<dyn RewriteMethod>,
        field: &str,
        terms: Vec<BytesRef>,
    ) -> Result<IndexOrDocValuesQuery> {
        let packed = Arc::new(Self::pack_terms(field, terms)?);
        let index_query: Arc<dyn Query> = Arc::new(Self::from_packed(
            index_rewrite_method,
            field,
            Arc::clone(&packed),
        )?);
        let dv_query: Arc<dyn Query> =
            Arc::new(Self::from_packed(doc_values_rewrite(), field, packed)?);
        Ok(IndexOrDocValuesQuery::new(index_query, dv_query))
    }

    /// Sorts, deduplicates and prefix-encodes the query terms.
    ///
    /// Equivalent to the private static
    /// `TermInSetQuery.packTerms(String, Collection<BytesRef>)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java skips sorting when the
    /// collection is a `SortedSet` with the natural comparator, which Rust
    /// cannot detect on a `Vec`; the terms are therefore always sorted. Sorting
    /// an already-sorted input changes nothing but the time spent.
    ///
    /// # Errors
    ///
    /// Propagates the error the encoder raises.
    pub fn pack_terms(field: &str, mut terms: Vec<BytesRef>) -> Result<PrefixCodedTerms> {
        terms.sort();
        let mut builder = PrefixCodedTermsBuilder::new();
        let mut previous: Option<BytesRef> = None;
        for term in terms {
            if let Some(previous) = previous.as_ref() {
                if previous == &term {
                    // Deduplicate.
                    continue;
                }
            }
            builder.add_bytes(field, &term)?;
            previous = Some(term);
        }
        Ok(builder.finish())
    }

    /// Computes the cached hash code and encoded size of the packed terms.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `PrefixCodedTerms.hashCode()`
    /// and `equals` compare the encoded byte buffer, and `ramBytesUsed()`
    /// reports its length; this port's [`PrefixCodedTerms`] keeps that buffer
    /// private and exposes only its iterator, so both are derived from the term
    /// sequence the iterator yields. The encoding is canonical, so two packed
    /// term sets have the same bytes exactly when they have the same term
    /// sequence, and the derived hash agrees with the derived equality.
    fn digest(term_data: &PrefixCodedTerms) -> Result<(u64, i64)> {
        let mut hasher = DefaultHasher::new();
        let mut bytes = 0i64;
        let mut iterator = term_data.iterator();
        while let Some(term) = iterator.next()? {
            iterator.field().hash(&mut hasher);
            term.slice().hash(&mut hasher);
            // The encoder writes the shared-prefix length, the suffix length
            // and the suffix bytes.
            bytes += term.length as i64 + 8;
        }
        term_data.size().hash(&mut hasher);
        Ok((hasher.finish(), bytes))
    }

    /// Returns an iterator over the encoded terms, for query inspection.
    ///
    /// Equivalent to the experimental
    /// `TermInSetQuery.getBytesRefIterator()`.
    pub fn get_bytes_ref_iterator(&self) -> impl BytesRefIterator {
        PackedTermsIterator {
            iterator: self.term_data.iterator(),
        }
    }

    /// Returns the packed terms.
    ///
    /// Equivalent to reading the `private final PrefixCodedTerms termData`
    /// field, which Java's `newIndexOrDocValuesQuery` needs.
    pub fn term_data(&self) -> &Arc<PrefixCodedTerms> {
        &self.term_data
    }

    /// Builds the byte automaton accepting exactly the query terms.
    ///
    /// Equivalent to the private
    /// `TermInSetQuery.asByteRunAutomaton()`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while building the union automaton.
    pub fn as_byte_run_automaton(&self) -> Result<ByteRunAutomaton> {
        let mut terms = Vec::new();
        let mut iterator = self.term_data.iterator();
        while let Some(term) = iterator.next()? {
            terms.push(term);
        }
        let a = Automata::make_binary_string_union(terms)?;
        ByteRunAutomaton::new(a, true)
    }
}

/// A [`BytesRefIterator`] over the packed terms of a [`TermInSetQuery`].
///
/// Equivalent to the lambda `() -> iterator.next()` that
/// `TermInSetQuery.getBytesRefIterator()` returns.
struct PackedTermsIterator {
    iterator: PrefixCodedTermsIterator,
}

impl BytesRefIterator for PackedTermsIterator {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.iterator.next()
    }
}

impl Accountable for TermInSetQuery {
    fn ram_bytes_used(&self) -> i64 {
        BASE_RAM_BYTES_USED + self.term_data_bytes
    }

    fn child_resources(&self) -> Vec<&dyn Accountable> {
        Vec::new()
    }
}

impl Query for TermInSetQuery {
    fn to_query_string(&self, _default_field: &str) -> String {
        let mut builder = String::new();
        builder.push_str(&self.field);
        builder.push_str(":(");

        let mut iterator = self.term_data.iterator();
        let mut first = true;
        // The iterator reads an in-memory buffer, so it cannot fail; a failure
        // would truncate the rendering rather than panic.
        while let Ok(Some(term)) = iterator.next() {
            if !first {
                builder.push(' ');
            }
            first = false;
            builder.push_str(&term_bytes_to_string(&term));
        }
        builder.push(')');
        builder
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if !visitor.accept_field(&self.field) {
            return;
        }
        if self.term_data.size() == 1 {
            let mut iterator = self.term_data.iterator();
            if let Ok(Some(term)) = iterator.next() {
                let term = Term::new(self.field.clone(), term);
                visitor.consume_terms(self, std::slice::from_ref(&term));
            }
        }
        if self.term_data.size() > 1 {
            visitor.consume_terms_matching(self, &self.field, &|| {
                // `QueryVisitor.consumeTermsMatching` takes a supplier that
                // cannot report an error, and Java wraps the impossible
                // `IOException` in an `UncheckedIOException`; the union of a
                // fixed, sorted term list cannot fail either, so an automaton
                // accepting nothing stands in for the impossible case.
                self.as_byte_run_automaton().unwrap_or_else(|_| {
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
        let Some(other) = other.as_any().downcast_ref::<TermInSetQuery>() else {
            return false;
        };
        // There is no need to check `field` explicitly, since it is encoded in
        // `term_data`; comparing the packed terms may be heavy, so the cached
        // hash code is checked first.
        if self.term_data_hash_code != other.term_data_hash_code {
            return false;
        }
        if Arc::ptr_eq(&self.term_data, &other.term_data) {
            return true;
        }
        let mut a = self.term_data.iterator();
        let mut b = other.term_data.iterator();
        loop {
            match (a.next(), b.next()) {
                (Ok(None), Ok(None)) => return true,
                (Ok(Some(x)), Ok(Some(y))) => {
                    if x != y || a.field() != b.field() {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }

    fn query_hash(&self) -> u64 {
        31u64
            .wrapping_mul(self.class_hash())
            .wrapping_add(self.term_data_hash_code)
    }
}

impl MultiTermQuery for TermInSetQuery {
    fn get_field(&self) -> &str {
        &self.field
    }

    fn get_rewrite_method(&self) -> Arc<dyn RewriteMethod> {
        Arc::clone(&self.rewrite_method)
    }

    fn get_terms_enum_with_atts(
        &self,
        terms: &Arc<dyn Terms>,
        _atts: &mut AttributeSource,
    ) -> Result<Box<dyn TermsEnum>> {
        let mut iterator = self.term_data.iterator();
        let seek_term = iterator.next()?;
        let filter = Box::new(SetEnumFilter {
            iterator,
            seek_term: seek_term.clone(),
        });
        let tenum = terms.iterator()?;
        Ok(Box::new(match seek_term {
            // Java's `FilteredTermsEnum(TermsEnum)` starts with a seek, whose
            // target is the first query term.
            Some(seek_term) => FilteredTermsEnum::new_with_seek(tenum, filter, seek_term),
            // With no query term at all, the first `accept` ends the
            // enumeration, exactly as Java's `nextSeekTerm(null)` returning
            // `null` does.
            None => FilteredTermsEnum::new(tenum, filter),
        }))
    }

    fn get_terms_count(&self) -> i64 {
        self.term_data.size()
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

    fn accountable_ram_bytes_used(&self) -> Option<i64> {
        Some(self.ram_bytes_used())
    }
}

/// Ping-pong intersects the terms dictionary against the encoded query terms.
///
/// Equivalent to the private inner class `TermInSetQuery.SetEnum`, which Java
/// describes as a baby
/// [`AutomatonTermsEnum`](crate::index::AutomatonTermsEnum). In this port the
/// filtering half of a `FilteredTermsEnum` is a separate object, so the class
/// becomes a [`FilteredTermsEnumFilter`].
struct SetEnumFilter {
    iterator: PrefixCodedTermsIterator,
    seek_term: Option<BytesRef>,
}

impl FilteredTermsEnumFilter for SetEnumFilter {
    fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus> {
        // Advance the iterator until it is `>=` the incoming term: an exact
        // match is a hit, anything else is a miss.
        let mut cmp = std::cmp::Ordering::Equal;
        while let Some(seek_term) = self.seek_term.as_ref() {
            cmp = seek_term.cmp(term);
            if cmp != std::cmp::Ordering::Less {
                break;
            }
            self.seek_term = self.iterator.next()?;
        }
        if self.seek_term.is_none() {
            Ok(AcceptStatus::End)
        } else if cmp == std::cmp::Ordering::Equal {
            Ok(AcceptStatus::YesAndSeek)
        } else {
            Ok(AcceptStatus::NoAndSeek)
        }
    }

    fn next_seek_term(&mut self, current_term: Option<&BytesRef>) -> Result<Option<BytesRef>> {
        // Advance the iterator until it is `>` the current term; it must always
        // make progress.
        let Some(current_term) = current_term else {
            return Ok(self.seek_term.clone());
        };
        while let Some(seek_term) = self.seek_term.as_ref() {
            if seek_term.cmp(current_term) == std::cmp::Ordering::Greater {
                break;
            }
            self.seek_term = self.iterator.next()?;
        }
        Ok(self.seek_term.clone())
    }
}
