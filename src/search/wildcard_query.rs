//! The wildcard search query, ported from
//! `org.apache.lucene.search.WildcardQuery`.

#![deny(unsafe_code)]

use std::any::Any;
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
use crate::util::automaton::{Automata, Automaton, Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
use crate::util::Accountable;

/// String equality with support for wildcards.
///
/// Equivalent to the `WildcardQuery.WILDCARD_STRING` constant.
pub const WILDCARD_STRING: char = '*';

/// Char equality with support for wildcards.
///
/// Equivalent to the `WildcardQuery.WILDCARD_CHAR` constant.
pub const WILDCARD_CHAR: char = '?';

/// The escape character.
///
/// Equivalent to the `WildcardQuery.WILDCARD_ESCAPE` constant.
pub const WILDCARD_ESCAPE: char = '\\';

/// Implements the wildcard search query.
///
/// Equivalent to `org.apache.lucene.search.WildcardQuery`. The supported
/// wildcards are `*`, which matches any character sequence including the empty
/// one, and `?`, which matches any single character; `\` is the escape
/// character.
///
/// Note that this query can be slow, as it needs to iterate over many terms: to
/// prevent extremely slow wildcard queries, a wildcard term should not start
/// with `*`. It uses the [`constant_score_blended_rewrite`] rewrite method.
#[derive(Debug, Clone)]
pub struct WildcardQuery {
    inner: AutomatonQuery,
}

impl WildcardQuery {
    /// Constructs a query for terms matching `term`.
    ///
    /// Equivalent to `WildcardQuery(Term)`, which uses
    /// [`DEFAULT_DETERMINIZE_WORK_LIMIT`].
    ///
    /// # Errors
    ///
    /// Propagates the determinization and compilation errors.
    pub fn new(term: Term) -> Result<Self> {
        Self::with_work_limit(term, DEFAULT_DETERMINIZE_WORK_LIMIT)
    }

    /// Constructs a query for terms matching `term`, bounding the effort spent
    /// compiling the automaton.
    ///
    /// Equivalent to `WildcardQuery(Term, int)`. Set `determinize_work_limit`
    /// higher to allow more complex queries and lower to prevent memory
    /// exhaustion; [`DEFAULT_DETERMINIZE_WORK_LIMIT`] is a decent default.
    ///
    /// # Errors
    ///
    /// Propagates the determinization and compilation errors.
    pub fn with_work_limit(term: Term, determinize_work_limit: i32) -> Result<Self> {
        Self::with_rewrite_method(
            term,
            determinize_work_limit,
            constant_score_blended_rewrite(),
        )
    }

    /// Constructs a query for terms matching `term`, with a rewrite method.
    ///
    /// Equivalent to `WildcardQuery(Term, int, RewriteMethod)`.
    ///
    /// # Errors
    ///
    /// Propagates the determinization and compilation errors.
    pub fn with_rewrite_method(
        term: Term,
        determinize_work_limit: i32,
        rewrite_method: Arc<dyn RewriteMethod>,
    ) -> Result<Self> {
        let automaton = Self::to_automaton(&term, determinize_work_limit)?;
        Ok(Self {
            inner: AutomatonQuery::with_rewrite_method(term, automaton, false, rewrite_method)?,
        })
    }

    /// Converts Lucene wildcard syntax into an automaton.
    ///
    /// Equivalent to the static
    /// `WildcardQuery.toAutomaton(Term, int)`.
    ///
    /// # Errors
    ///
    /// Returns the determinization error when the automaton is too complex.
    pub fn to_automaton(wildcardquery: &Term, determinize_work_limit: i32) -> Result<Automaton> {
        let mut automata: Vec<Automaton> = Vec::new();
        let wildcard_text = wildcardquery.text();
        let chars: Vec<char> = wildcard_text.chars().collect();

        let mut i = 0usize;
        while i < chars.len() {
            let c = chars[i];
            // Java advances by `Character.charCount(codePoint)`, which counts
            // UTF-16 code units; a `char` here is already a full code point, so
            // the step is one.
            let mut length = 1usize;
            match c {
                WILDCARD_STRING => automata.push(Automata::make_any_string()),
                WILDCARD_CHAR => automata.push(Automata::make_any_char()),
                WILDCARD_ESCAPE => {
                    // Add the next code point instead, if it exists; otherwise
                    // fall through, parsing a trailing `\` leniently.
                    if i + length < chars.len() {
                        let next_char = chars[i + length];
                        length += 1;
                        automata.push(Automata::make_char(next_char as i32));
                    } else {
                        automata.push(Automata::make_char(c as i32));
                    }
                }
                _ => automata.push(Automata::make_char(c as i32)),
            }
            i += length;
        }

        Ok(Operations::determinize(
            &Operations::concatenate(&automata),
            determinize_work_limit,
        )?)
    }

    /// Returns the pattern term.
    ///
    /// Equivalent to `WildcardQuery.getTerm()`.
    pub fn get_term(&self) -> &Term {
        self.inner.term()
    }

    /// Returns the underlying automaton query.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `WildcardQuery` *is* an
    /// `AutomatonQuery`; Rust has no inheritance, so it holds one and exposes
    /// it here.
    pub fn automaton_query(&self) -> &AutomatonQuery {
        &self.inner
    }
}

impl Accountable for WildcardQuery {
    fn ram_bytes_used(&self) -> i64 {
        self.inner.ram_bytes_used()
    }
}

impl Query for WildcardQuery {
    fn is_multi_term_query(&self) -> bool {
        true
    }

    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        if self.inner.field() != field {
            buffer.push_str(self.inner.field());
            buffer.push(':');
        }
        buffer.push_str(&self.inner.term().text());
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
        let Some(other) = other.as_any().downcast_ref::<WildcardQuery>() else {
            return false;
        };
        automaton_query_eq(self, &self.inner, other, &other.inner)
    }

    fn query_hash(&self) -> u64 {
        automaton_query_hash(self, &self.inner)
    }
}

impl MultiTermQuery for WildcardQuery {
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
