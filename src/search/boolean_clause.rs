//! Boolean clauses, ported from `org.apache.lucene.search.BooleanClause`.
//!
//! Only the clause record and its [`Occur`] operator live here. `BooleanQuery`
//! itself belongs to the query package and is not part of the query-execution
//! spine; [`Occur`] is needed because
//! [`QueryVisitor::get_sub_visitor`](crate::search::QueryVisitor::get_sub_visitor)
//! is keyed by it.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::Arc;

use crate::search::Query;

/// Specifies how clauses are to occur in matching documents.
///
/// Equivalent to `org.apache.lucene.search.BooleanClause.Occur`. The `Display`
/// rendering reproduces Java's per-constant `toString()` overrides, which are
/// what `BooleanQuery.toString` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum Occur {
    /// Use this operator for clauses that *must* appear in the matching
    /// documents.
    MUST,

    /// Like [`Occur::MUST`] except that these clauses do not participate in
    /// scoring.
    FILTER,

    /// Use this operator for clauses that *should* appear in the matching
    /// documents. For a boolean query with no `MUST` clauses, one or more
    /// `SHOULD` clauses must match a document for the query to match.
    SHOULD,

    /// Use this operator for clauses that *must not* appear in the matching
    /// documents. Note that it is not possible to search for queries that
    /// consist only of a `MUST_NOT` clause. These clauses do not contribute to
    /// the score of documents.
    MUST_NOT,
}

impl fmt::Display for Occur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::MUST => "+",
            Self::FILTER => "#",
            Self::SHOULD => "",
            Self::MUST_NOT => "-",
        };
        f.write_str(s)
    }
}

/// A clause in a boolean query.
///
/// Equivalent to the `org.apache.lucene.search.BooleanClause` record.
#[derive(Debug, Clone)]
pub struct BooleanClause {
    query: Arc<dyn Query>,
    occur: Occur,
}

impl BooleanClause {
    /// Creates a clause pairing a query with the way it must occur.
    ///
    /// Equivalent to the canonical constructor of the Java record.
    pub fn new(query: Arc<dyn Query>, occur: Occur) -> Self {
        Self { query, occur }
    }

    /// Returns the clause's query.
    ///
    /// Equivalent to `BooleanClause.query()`.
    pub fn query(&self) -> &Arc<dyn Query> {
        &self.query
    }

    /// Returns the clause's operator.
    ///
    /// Equivalent to `BooleanClause.occur()`.
    pub fn occur(&self) -> Occur {
        self.occur
    }

    /// Returns `true` if this clause must appear in matching documents.
    ///
    /// Equivalent to `BooleanClause.isRequired()`.
    pub fn is_required(&self) -> bool {
        matches!(self.occur, Occur::MUST | Occur::FILTER)
    }

    /// Returns `true` if this clause must not appear in matching documents.
    ///
    /// Equivalent to `BooleanClause.isProhibited()`.
    pub fn is_prohibited(&self) -> bool {
        self.occur == Occur::MUST_NOT
    }

    /// Returns `true` if this clause contributes to the score.
    ///
    /// Equivalent to `BooleanClause.isScoring()`.
    pub fn is_scoring(&self) -> bool {
        matches!(self.occur, Occur::MUST | Occur::SHOULD)
    }
}

impl fmt::Display for BooleanClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.occur, self.query.to_query_string(""))
    }
}
