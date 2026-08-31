//! Matching terms with a given prefix, ported from
//! `org.apache.lucene.search.PrefixQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::{Term, Terms, TermsEnum};
use crate::search::automaton_query::{
    automaton_query_eq, automaton_query_hash, automaton_query_visit, term_hash, AutomatonQuery,
};
use crate::search::index_searcher::IndexSearcher;
use crate::search::multi_term_query::{
    constant_score_blended_rewrite, multi_term_rewrite, MultiTermQuery, RewriteMethod,
};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::util::attribute::AttributeSource;
use crate::util::automaton::Automaton;
use crate::util::{Accountable, BytesRef};

/// A [`Query`] that matches documents containing terms with a specified prefix.
///
/// Equivalent to `org.apache.lucene.search.PrefixQuery`, which a query parser
/// builds for input like `app*`. It uses the
/// [`constant_score_blended_rewrite`] rewrite method.
#[derive(Debug, Clone)]
pub struct PrefixQuery {
    inner: AutomatonQuery,
}

impl PrefixQuery {
    /// Constructs a query for terms starting with `prefix`.
    ///
    /// Equivalent to `PrefixQuery(Term)`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while compiling the automaton.
    pub fn new(prefix: Term) -> Result<Self> {
        Self::with_rewrite_method(prefix, constant_score_blended_rewrite())
    }

    /// Constructs a query for terms starting with `prefix`, using the given
    /// rewrite method.
    ///
    /// Equivalent to `PrefixQuery(Term, RewriteMethod)`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while compiling the automaton.
    pub fn with_rewrite_method(
        prefix: Term,
        rewrite_method: Arc<dyn RewriteMethod>,
    ) -> Result<Self> {
        let automaton = Self::to_automaton(prefix.bytes());
        Ok(Self {
            inner: AutomatonQuery::with_rewrite_method(prefix, automaton, true, rewrite_method)?,
        })
    }

    /// Builds an automaton accepting all terms with the specified prefix.
    ///
    /// Equivalent to the static `PrefixQuery.toAutomaton(BytesRef)`.
    pub fn to_automaton(prefix: &BytesRef) -> Automaton {
        let num_states_and_transitions = prefix.length + 1;
        let mut automaton =
            Automaton::with_capacity(num_states_and_transitions, num_states_and_transitions);
        let mut last_state = automaton.create_state();
        for i in 0..prefix.length {
            let state = automaton.create_state();
            automaton.add_transition(
                last_state,
                state,
                i32::from(prefix.bytes[prefix.offset + i]),
            );
            last_state = state;
        }
        automaton.set_accept(last_state, true);
        automaton.add_transition_range(last_state, last_state, 0, 255);
        automaton.finish_state();
        debug_assert!(automaton.is_deterministic());
        automaton
    }

    /// Returns the prefix of this query.
    ///
    /// Equivalent to `PrefixQuery.getPrefix()`.
    pub fn get_prefix(&self) -> &Term {
        self.inner.term()
    }

    /// Returns the underlying automaton query.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `PrefixQuery` *is* an
    /// `AutomatonQuery`; Rust has no inheritance, so it holds one and exposes
    /// it here for the accessors that `AutomatonQuery` declares.
    pub fn automaton_query(&self) -> &AutomatonQuery {
        &self.inner
    }
}

impl Accountable for PrefixQuery {
    fn ram_bytes_used(&self) -> i64 {
        self.inner.ram_bytes_used()
    }
}

impl Query for PrefixQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        if self.inner.field() != field {
            buffer.push_str(self.inner.field());
            buffer.push(':');
        }
        buffer.push_str(&self.inner.term().text());
        buffer.push('*');
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
        let Some(other) = other.as_any().downcast_ref::<PrefixQuery>() else {
            return false;
        };
        automaton_query_eq(self, &self.inner, other, &other.inner)
            && self.inner.term() == other.inner.term()
    }

    fn query_hash(&self) -> u64 {
        31u64
            .wrapping_mul(automaton_query_hash(self, &self.inner))
            .wrapping_add(term_hash(self.inner.term()))
    }
}

impl MultiTermQuery for PrefixQuery {
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
