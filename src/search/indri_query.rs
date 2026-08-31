//! The Indri query base, ported from
//! `org.apache.lucene.search.IndriQuery`.

#![deny(unsafe_code)]

use crate::search::boolean_clause::BooleanClause;
use crate::search::boolean_query::BooleanQuery;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;

/// The state and the concrete behaviour of `IndriQuery`.
///
/// Equivalent to the abstract class `org.apache.lucene.search.IndriQuery`,
/// which holds a `List<BooleanClause>` and implements `toString`, `equals`,
/// `hashCode`, `visit` and `iterator` on top of it, leaving only
/// `createWeight` abstract. Rust has no implementation inheritance, so the
/// class becomes this struct and the concrete queries hold one, forwarding the
/// concrete methods to it.
#[derive(Debug, Clone)]
pub struct IndriQuery {
    clauses: Vec<BooleanClause>,
}

impl IndriQuery {
    /// Creates the base state over the given clauses.
    ///
    /// Equivalent to `IndriQuery(List<BooleanClause>)`.
    pub fn new(clauses: Vec<BooleanClause>) -> Self {
        Self { clauses }
    }

    /// Returns the clauses of this query.
    ///
    /// Equivalent to `IndriQuery.getClauses()`.
    pub fn get_clauses(&self) -> &[BooleanClause] {
        &self.clauses
    }

    /// Iterates the clauses of this query.
    ///
    /// Equivalent to `IndriQuery.iterator()`, the `Iterable<BooleanClause>`
    /// implementation.
    pub fn iter(&self) -> std::slice::Iter<'_, BooleanClause> {
        self.clauses.iter()
    }

    /// Renders the query.
    ///
    /// Equivalent to `IndriQuery.toString(String)`, which wraps a nested
    /// boolean query in parentheses.
    pub fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        for (i, clause) in self.clauses.iter().enumerate() {
            buffer.push_str(&clause.occur().to_string());

            let sub_query = clause.query();
            if sub_query.as_any().is::<BooleanQuery>() {
                // Wrap sub-bools in parens.
                buffer.push('(');
                buffer.push_str(&sub_query.to_query_string(field));
                buffer.push(')');
            } else {
                buffer.push_str(&sub_query.to_query_string(field));
            }

            if i != self.clauses.len() - 1 {
                buffer.push(' ');
            }
        }
        buffer
    }

    /// Visits this query as a leaf.
    ///
    /// Equivalent to `IndriQuery.visit(QueryVisitor)`, which does *not* recurse
    /// into the clauses.
    pub fn visit(&self, query: &dyn Query, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(query);
    }

    /// Clause-list equivalence.
    ///
    /// Equivalent to the private `IndriQuery.equalsTo(IndriQuery)`, which
    /// compares the clause lists.
    pub fn base_eq(&self, other: &IndriQuery) -> bool {
        self.clauses.len() == other.clauses.len()
            && self
                .clauses
                .iter()
                .zip(other.clauses.iter())
                .all(|(a, b)| a.occur() == b.occur() && a.query().query_eq(b.query().as_ref()))
    }

    /// Clause-list hash code.
    ///
    /// Equivalent to `IndriQuery.hashCode()`, which replaces a hash of `0` with
    /// `1`.
    pub fn base_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        for clause in &self.clauses {
            clause.occur().hash(&mut hasher);
            clause.query().query_hash().hash(&mut hasher);
        }
        let hash_code = hasher.finish();
        if hash_code == 0 {
            1
        } else {
            hash_code
        }
    }
}
