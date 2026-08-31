//! A fast regular-expression query, ported from
//! `org.apache.lucene.search.RegexpQuery`.

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
use crate::util::automaton::{
    Automaton, AutomatonProvider, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use crate::util::Accountable;

/// A provider that provides no named automata.
///
/// Equivalent to the `RegexpQuery.DEFAULT_PROVIDER` constant, the lambda
/// `name -> null`.
///
/// **Divergence from Lucene 10.5.0.** Java's provider answers `null` for every
/// name, and `RegExp.toAutomaton` then reports the missing identifier; this
/// port's [`AutomatonProvider`] returns a [`Result`], so the same condition is
/// reported as an error here.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultProvider;

impl AutomatonProvider for DefaultProvider {
    fn get_automaton(&self, name: &str) -> Result<Automaton> {
        Err(crate::error::LuceneError::IllegalArgument(format!(
            "'{name}' not found"
        )))
    }
}

/// A fast regular-expression query based on the [`crate::util::automaton`]
/// package.
///
/// Equivalent to `org.apache.lucene.search.RegexpQuery`. Comparisons are fast
/// and the term dictionary is enumerated in an intelligent way to avoid them;
/// see [`AutomatonQuery`] for details. The supported syntax is documented in
/// [`RegExp`], and may differ from other regular-expression implementations.
///
/// Note that this query can be slow, as it needs to iterate over many terms: to
/// prevent extremely slow regexp queries, a regexp term should not start with
/// `.*`.
#[derive(Debug, Clone)]
pub struct RegexpQuery {
    inner: AutomatonQuery,
}

impl RegexpQuery {
    /// Constructs a query for terms matching `term`, with all regular
    /// expression features enabled.
    ///
    /// Equivalent to `RegexpQuery(Term)`.
    ///
    /// # Errors
    ///
    /// Propagates the parsing, determinization and compilation errors.
    pub fn new(term: Term) -> Result<Self> {
        Self::with_syntax_flags(term, RegExp::ALL)
    }

    /// Constructs a query for terms matching `term`, with the given optional
    /// [`RegExp`] features.
    ///
    /// Equivalent to `RegexpQuery(Term, int)`.
    ///
    /// # Errors
    ///
    /// Propagates the parsing, determinization and compilation errors.
    pub fn with_syntax_flags(term: Term, flags: i32) -> Result<Self> {
        Self::with_provider(
            term,
            flags,
            &DefaultProvider,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        )
    }

    /// Constructs a query for terms matching `term`, bounding the effort spent
    /// compiling the automaton.
    ///
    /// Equivalent to `RegexpQuery(Term, int, int)`.
    ///
    /// # Errors
    ///
    /// Propagates the parsing, determinization and compilation errors.
    pub fn with_work_limit(term: Term, flags: i32, determinize_work_limit: i32) -> Result<Self> {
        Self::with_provider(term, flags, &DefaultProvider, determinize_work_limit)
    }

    /// Constructs a query for terms matching `term`, with match flags.
    ///
    /// Equivalent to `RegexpQuery(Term, int, int, int)`, whose third argument
    /// is the boolean "or" of match-behaviour options such as case
    /// insensitivity.
    ///
    /// # Errors
    ///
    /// Propagates the parsing, determinization and compilation errors.
    pub fn with_match_flags(
        term: Term,
        syntax_flags: i32,
        match_flags: i32,
        determinize_work_limit: i32,
    ) -> Result<Self> {
        Self::with_everything(
            term,
            syntax_flags,
            match_flags,
            &DefaultProvider,
            determinize_work_limit,
            constant_score_blended_rewrite(),
            true,
        )
    }

    /// Constructs a query for terms matching `term`, with a custom
    /// [`AutomatonProvider`] for named automata.
    ///
    /// Equivalent to
    /// `RegexpQuery(Term, int, AutomatonProvider, int)`.
    ///
    /// # Errors
    ///
    /// Propagates the parsing, determinization and compilation errors.
    pub fn with_provider(
        term: Term,
        syntax_flags: i32,
        provider: &dyn AutomatonProvider,
        determinize_work_limit: i32,
    ) -> Result<Self> {
        Self::with_everything(
            term,
            syntax_flags,
            0,
            provider,
            determinize_work_limit,
            constant_score_blended_rewrite(),
            true,
        )
    }

    /// Constructs a query for terms matching `term`, controlling every option.
    ///
    /// Equivalent to the widest
    /// `RegexpQuery(Term, int, int, AutomatonProvider, int, RewriteMethod, boolean)`.
    /// When `do_determinization` is `false` the query does not force the
    /// generated automaton to be a DFA, so it might be an NFA and be executed
    /// with [`NFARunAutomaton`](crate::util::automaton::NFARunAutomaton), which
    /// is not thread-safe — in that case a rewrite method other than
    /// [`constant_score_blended_rewrite`] is preferable when the searcher is
    /// configured with an executor.
    ///
    /// # Errors
    ///
    /// Propagates the parsing, determinization and compilation errors.
    pub fn with_everything(
        term: Term,
        syntax_flags: i32,
        match_flags: i32,
        provider: &dyn AutomatonProvider,
        determinize_work_limit: i32,
        rewrite_method: Arc<dyn RewriteMethod>,
        do_determinization: bool,
    ) -> Result<Self> {
        let regexp = RegExp::with_match_flags(&term.text(), syntax_flags, match_flags)?;
        let automaton = Self::to_automaton(
            &regexp,
            determinize_work_limit,
            provider,
            do_determinization,
        )?;
        Ok(Self {
            inner: AutomatonQuery::with_rewrite_method(term, automaton, false, rewrite_method)?,
        })
    }

    /// Equivalent to the private static
    /// `RegexpQuery.toAutomaton(RegExp, int, AutomatonProvider, boolean)`.
    fn to_automaton(
        regexp: &RegExp,
        determinize_work_limit: i32,
        provider: &dyn AutomatonProvider,
        do_determinization: bool,
    ) -> Result<Automaton> {
        let automaton = regexp.to_automaton_with(None, Some(provider))?;
        if do_determinization {
            Ok(Operations::determinize(&automaton, determinize_work_limit)?)
        } else {
            Ok(automaton)
        }
    }

    /// Returns the regexp of this query wrapped in a [`Term`].
    ///
    /// Equivalent to `RegexpQuery.getRegexp()`.
    pub fn get_regexp(&self) -> &Term {
        self.inner.term()
    }

    /// Returns the underlying automaton query.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `RegexpQuery` *is* an
    /// `AutomatonQuery`; Rust has no inheritance, so it holds one and exposes
    /// it here.
    pub fn automaton_query(&self) -> &AutomatonQuery {
        &self.inner
    }
}

impl Accountable for RegexpQuery {
    fn ram_bytes_used(&self) -> i64 {
        self.inner.ram_bytes_used()
    }
}

impl Query for RegexpQuery {
    fn is_multi_term_query(&self) -> bool {
        true
    }

    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        if self.inner.term().field() != field {
            buffer.push_str(self.inner.term().field());
            buffer.push(':');
        }
        buffer.push('/');
        buffer.push_str(&self.inner.term().text());
        buffer.push('/');
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
        let Some(other) = other.as_any().downcast_ref::<RegexpQuery>() else {
            return false;
        };
        automaton_query_eq(self, &self.inner, other, &other.inner)
    }

    fn query_hash(&self) -> u64 {
        automaton_query_hash(self, &self.inner)
    }
}

impl MultiTermQuery for RegexpQuery {
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
