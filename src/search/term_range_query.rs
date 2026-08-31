//! Matching documents within a range of terms, ported from
//! `org.apache.lucene.search.TermRangeQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::Result;
use crate::index::{Term, Terms, TermsEnum};
use crate::search::automaton_query::{
    automaton_query_eq, automaton_query_hash, automaton_query_visit, AutomatonQuery,
};
use crate::search::index_searcher::IndexSearcher;
use crate::search::multi_term_query::{
    constant_score_blended_rewrite, multi_term_rewrite, MultiTermQuery, RewriteMethod,
};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::util::attribute::AttributeSource;
use crate::util::automaton::{Automata, Automaton};
use crate::util::{Accountable, BytesRef};

/// Renders term bytes the way `Term.toString(BytesRef)` does.
///
/// The term might not be text, but usually is, so a best effort is made: it is
/// decoded as UTF-8 when that succeeds, and rendered as
/// [`BytesRef::to_hex_string`] — which is `BytesRef.toString()` — otherwise.
pub fn term_bytes_to_string(bytes: &BytesRef) -> String {
    bytes
        .utf8_to_string()
        .unwrap_or_else(|_| bytes.to_hex_string())
}

/// A [`Query`] that matches documents within a range of terms.
///
/// Equivalent to `org.apache.lucene.search.TermRangeQuery`. It looks for the
/// terms that fall into the supplied range according to the byte ordering of
/// [`BytesRef`].
///
/// **NOTE**: a term range query performs significantly slower than a
/// point-based range, as it needs to visit all the terms that match the range
/// and merge their matches. It uses the [`constant_score_blended_rewrite`]
/// rewrite method.
#[derive(Debug, Clone)]
pub struct TermRangeQuery {
    inner: AutomatonQuery,
    lower_term: Option<BytesRef>,
    upper_term: Option<BytesRef>,
    include_lower: bool,
    include_upper: bool,
}

impl TermRangeQuery {
    /// Constructs a query selecting all terms greater than or equal to
    /// `lower_term` but less than or equal to `upper_term`.
    ///
    /// Equivalent to
    /// `TermRangeQuery(String, BytesRef, BytesRef, boolean, boolean)`. A `None`
    /// endpoint is "open"; either or both endpoints may be open, and an open
    /// endpoint may not be exclusive — one cannot select all but the first or
    /// last term without naming the term to exclude.
    ///
    /// # Errors
    ///
    /// Propagates the automaton construction and compilation errors.
    pub fn new(
        field: impl Into<String>,
        lower_term: Option<BytesRef>,
        upper_term: Option<BytesRef>,
        include_lower: bool,
        include_upper: bool,
    ) -> Result<Self> {
        Self::with_rewrite_method(
            field,
            lower_term,
            upper_term,
            include_lower,
            include_upper,
            constant_score_blended_rewrite(),
        )
    }

    /// Constructs a range query with the given rewrite method.
    ///
    /// Equivalent to
    /// `TermRangeQuery(String, BytesRef, BytesRef, boolean, boolean, RewriteMethod)`.
    ///
    /// # Errors
    ///
    /// Propagates the automaton construction and compilation errors.
    pub fn with_rewrite_method(
        field: impl Into<String>,
        lower_term: Option<BytesRef>,
        upper_term: Option<BytesRef>,
        include_lower: bool,
        include_upper: bool,
        rewrite_method: Arc<dyn RewriteMethod>,
    ) -> Result<Self> {
        let automaton = Self::to_automaton(
            lower_term.as_ref(),
            upper_term.as_ref(),
            include_lower,
            include_upper,
        )?;
        // Java builds `new Term(field, lowerTerm)`, whose bytes are `null` when
        // the lower endpoint is open; this port's `Term` always holds a
        // `BytesRef`, so an open endpoint becomes the empty one. The term only
        // carries the field for `AutomatonQuery`, and the endpoints are
        // compared explicitly by `query_eq` below, so nothing observes the
        // difference.
        let term = Term::new(field, lower_term.clone().unwrap_or_default());
        Ok(Self {
            inner: AutomatonQuery::with_rewrite_method(term, automaton, true, rewrite_method)?,
            lower_term,
            upper_term,
            include_lower,
            include_upper,
        })
    }

    /// Builds the automaton accepting every term in the range.
    ///
    /// Equivalent to the static
    /// `TermRangeQuery.toAutomaton(BytesRef, BytesRef, boolean, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates the error `Automata.makeBinaryInterval` raises.
    pub fn to_automaton(
        lower_term: Option<&BytesRef>,
        upper_term: Option<&BytesRef>,
        mut include_lower: bool,
        mut include_upper: bool,
    ) -> Result<Automaton> {
        if lower_term.is_none() {
            // `makeBinaryInterval` is more picky than we are.
            include_lower = true;
        }
        if upper_term.is_none() {
            // `makeBinaryInterval` is more picky than we are.
            include_upper = true;
        }
        Automata::make_binary_interval(lower_term, include_lower, upper_term, include_upper)
    }

    /// Creates a range query from string term text.
    ///
    /// Equivalent to the static
    /// `TermRangeQuery.newStringRange(String, String, String, boolean, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates the automaton construction and compilation errors.
    pub fn new_string_range(
        field: impl Into<String>,
        lower_term: Option<&str>,
        upper_term: Option<&str>,
        include_lower: bool,
        include_upper: bool,
    ) -> Result<Self> {
        Self::new_string_range_with_rewrite_method(
            field,
            lower_term,
            upper_term,
            include_lower,
            include_upper,
            constant_score_blended_rewrite(),
        )
    }

    /// Creates a range query from string term text, with a rewrite method.
    ///
    /// Equivalent to the static
    /// `TermRangeQuery.newStringRange(String, String, String, boolean, boolean, RewriteMethod)`.
    ///
    /// # Errors
    ///
    /// Propagates the automaton construction and compilation errors.
    pub fn new_string_range_with_rewrite_method(
        field: impl Into<String>,
        lower_term: Option<&str>,
        upper_term: Option<&str>,
        include_lower: bool,
        include_upper: bool,
        rewrite_method: Arc<dyn RewriteMethod>,
    ) -> Result<Self> {
        let lower = lower_term.map(|t| BytesRef::new(t.as_bytes().to_vec()));
        let upper = upper_term.map(|t| BytesRef::new(t.as_bytes().to_vec()));
        Self::with_rewrite_method(
            field,
            lower,
            upper,
            include_lower,
            include_upper,
            rewrite_method,
        )
    }

    /// Returns the lower value of this range query.
    ///
    /// Equivalent to `TermRangeQuery.getLowerTerm()`.
    pub fn get_lower_term(&self) -> Option<&BytesRef> {
        self.lower_term.as_ref()
    }

    /// Returns the upper value of this range query.
    ///
    /// Equivalent to `TermRangeQuery.getUpperTerm()`.
    pub fn get_upper_term(&self) -> Option<&BytesRef> {
        self.upper_term.as_ref()
    }

    /// Returns `true` if the lower endpoint is inclusive.
    ///
    /// Equivalent to `TermRangeQuery.includesLower()`.
    pub fn includes_lower(&self) -> bool {
        self.include_lower
    }

    /// Returns `true` if the upper endpoint is inclusive.
    ///
    /// Equivalent to `TermRangeQuery.includesUpper()`.
    pub fn includes_upper(&self) -> bool {
        self.include_upper
    }

    /// Returns the underlying automaton query.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `TermRangeQuery` *is* an
    /// `AutomatonQuery`; Rust has no inheritance, so it holds one and exposes
    /// it here.
    pub fn automaton_query(&self) -> &AutomatonQuery {
        &self.inner
    }

    /// Renders an endpoint the way Java's `toString` does, escaping a term whose
    /// text is exactly `*` so that it is not read back as an open endpoint.
    fn endpoint_to_string(term: Option<&BytesRef>) -> String {
        match term {
            None => "*".to_string(),
            Some(term) => {
                let text = term_bytes_to_string(term);
                if text == "*" {
                    "\\*".to_string()
                } else {
                    text
                }
            }
        }
    }
}

impl Accountable for TermRangeQuery {
    fn ram_bytes_used(&self) -> i64 {
        self.inner.ram_bytes_used()
    }
}

impl Query for TermRangeQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        if self.inner.field() != field {
            buffer.push_str(self.inner.field());
            buffer.push(':');
        }
        buffer.push(if self.include_lower { '[' } else { '{' });
        buffer.push_str(&Self::endpoint_to_string(self.lower_term.as_ref()));
        buffer.push_str(" TO ");
        buffer.push_str(&Self::endpoint_to_string(self.upper_term.as_ref()));
        buffer.push(if self.include_upper { ']' } else { '}' });
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        automaton_query_visit(self, &self.inner, visitor);
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
        let Some(other) = other.as_any().downcast_ref::<TermRangeQuery>() else {
            return false;
        };
        automaton_query_eq(self, &self.inner, other, &other.inner)
            && self.include_lower == other.include_lower
            && self.include_upper == other.include_upper
            && self.lower_term == other.lower_term
            && self.upper_term == other.upper_term
    }

    fn query_hash(&self) -> u64 {
        let prime = 31u64;
        let mut result = automaton_query_hash(self, &self.inner);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(if self.include_lower { 1231 } else { 1237 });
        result = prime
            .wrapping_mul(result)
            .wrapping_add(if self.include_upper { 1231 } else { 1237 });
        let mut hasher = DefaultHasher::new();
        match self.lower_term.as_ref() {
            None => 0usize.hash(&mut hasher),
            Some(term) => term.slice().hash(&mut hasher),
        }
        result = prime.wrapping_mul(result).wrapping_add(hasher.finish());
        let mut hasher = DefaultHasher::new();
        match self.upper_term.as_ref() {
            None => 0usize.hash(&mut hasher),
            Some(term) => term.slice().hash(&mut hasher),
        }
        prime.wrapping_mul(result).wrapping_add(hasher.finish())
    }
}

impl MultiTermQuery for TermRangeQuery {
    fn get_field(&self) -> &str {
        self.inner.field()
    }

    fn get_rewrite_method(&self) -> Arc<dyn RewriteMethod> {
        Arc::clone(self.inner.rewrite_method())
    }

    fn get_terms_enum_with_atts(
        &self,
        terms: &Arc<dyn Terms>,
        _atts: &mut AttributeSource,
    ) -> Result<Box<dyn TermsEnum>> {
        self.inner.terms_enum(&**terms)
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
        Some(self.inner.ram_bytes_used())
    }
}
