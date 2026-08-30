//! Query-tree traversal, ported from `org.apache.lucene.search.QueryVisitor`.

#![deny(unsafe_code)]

use std::collections::BTreeSet;

use crate::index::Term;
use crate::search::boolean_clause::Occur;
use crate::search::query::Query;
use crate::util::ByteRunAutomaton;

/// Allows recursion through a query tree.
///
/// Equivalent to the abstract class `org.apache.lucene.search.QueryVisitor`,
/// passed to [`Query::visit`].
pub trait QueryVisitor {
    /// Called by leaf queries that match on specific terms.
    ///
    /// Equivalent to `QueryVisitor.consumeTerms(Query, Term...)`; the Java
    /// varargs become a slice.
    fn consume_terms(&mut self, _query: &dyn Query, _terms: &[Term]) {}

    /// Called by leaf queries that match on a class of terms.
    ///
    /// Equivalent to
    /// `QueryVisitor.consumeTermsMatching(Query, String, Supplier<ByteRunAutomaton>)`,
    /// whose default delegates to [`visit_leaf`](Self::visit_leaf) for backward
    /// compatibility. The Java `Supplier` becomes a closure so that building
    /// the automaton stays lazy.
    fn consume_terms_matching(
        &mut self,
        query: &dyn Query,
        _field: &str,
        _automaton: &dyn Fn() -> ByteRunAutomaton,
    ) {
        self.visit_leaf(query); // default impl for backward compatibility
    }

    /// Called by leaf queries that do not match on terms.
    ///
    /// Equivalent to `QueryVisitor.visitLeaf(Query)`.
    fn visit_leaf(&mut self, _query: &dyn Query) {}

    /// Whether this field is of interest to the visitor.
    ///
    /// Equivalent to `QueryVisitor.acceptField(String)`, which returns `true`
    /// by default. Implement it to avoid collecting terms from heavy queries
    /// that are not running on fields of interest.
    fn accept_field(&self, _field: &str) -> bool {
        true
    }

    /// Pulls a visitor instance for visiting the child clauses of a query.
    ///
    /// Equivalent to `QueryVisitor.getSubVisitor(BooleanClause.Occur, Query)`,
    /// whose default returns `this` unless `occur` is
    /// [`Occur::MUST_NOT`], in which case it returns the empty visitor.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a bare reference, which
    /// is either the same object or a shared immutable singleton. Rust cannot
    /// return "either a borrow of self or a static", so the result is boxed;
    /// the identity of the visitor — and therefore the state it accumulates —
    /// is preserved by boxing a borrow of `self`.
    fn get_sub_visitor<'a>(
        &'a mut self,
        occur: Occur,
        _parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        if occur == Occur::MUST_NOT {
            return Box::new(EmptyQueryVisitor);
        }
        Box::new(self)
    }
}

impl<T: QueryVisitor + ?Sized> QueryVisitor for &mut T {
    fn consume_terms(&mut self, query: &dyn Query, terms: &[Term]) {
        (**self).consume_terms(query, terms);
    }

    fn consume_terms_matching(
        &mut self,
        query: &dyn Query,
        field: &str,
        automaton: &dyn Fn() -> ByteRunAutomaton,
    ) {
        (**self).consume_terms_matching(query, field, automaton);
    }

    fn visit_leaf(&mut self, query: &dyn Query) {
        (**self).visit_leaf(query);
    }

    fn accept_field(&self, field: &str) -> bool {
        (**self).accept_field(field)
    }
}

/// A [`QueryVisitor`] implementation that does nothing.
///
/// Equivalent to the `QueryVisitor.EMPTY_VISITOR` constant, whose
/// `acceptField` returns `false` so that a subtree is skipped entirely.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyQueryVisitor;

impl QueryVisitor for EmptyQueryVisitor {
    fn accept_field(&self, _field: &str) -> bool {
        false
    }
}

/// A [`QueryVisitor`] that collects every term that may match a query.
///
/// Equivalent to the visitor returned by
/// `QueryVisitor.termCollector(Set<Term>)`.
///
/// **Divergence from Lucene 10.5.0.** Java collects into a `java.util.Set`,
/// usually a `HashSet`. [`Term`] implements `Ord` but not `Hash` in this port,
/// so the collector uses a [`BTreeSet`]. The set of collected terms is the
/// same; only the iteration order differs, and Java's was unspecified.
#[derive(Debug)]
pub struct TermCollectorVisitor<'a> {
    term_set: &'a mut BTreeSet<Term>,
}

impl<'a> TermCollectorVisitor<'a> {
    /// Builds a visitor that adds every collected term to `term_set`.
    ///
    /// Equivalent to `QueryVisitor.termCollector(Set<Term>)`.
    pub fn new(term_set: &'a mut BTreeSet<Term>) -> Self {
        Self { term_set }
    }
}

impl QueryVisitor for TermCollectorVisitor<'_> {
    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        self.term_set.extend(terms.iter().cloned());
    }
}
