//! A phrase query optimised for n-grams, ported from
//! `org.apache.lucene.search.NGramPhraseQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::Term;
use crate::search::boolean_clause::Occur;
use crate::search::index_searcher::IndexSearcher;
use crate::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;

/// A [`PhraseQuery`] optimised for n-gram phrase queries.
///
/// Equivalent to `org.apache.lucene.search.NGramPhraseQuery`. Querying `ABCD`
/// on a 2-gram field is better served by this query than by a plain
/// [`PhraseQuery`], because it rewrites to `AB/0 CD/2` where the phrase query
/// would ask for `AB/0 BC/1 CD/2` — writing `term/position`.
#[derive(Debug, Clone)]
pub struct NGramPhraseQuery {
    n: i32,
    phrase_query: PhraseQuery,
}

impl NGramPhraseQuery {
    /// Creates an n-gram phrase query of gram size `n`.
    ///
    /// Equivalent to `NGramPhraseQuery(int, PhraseQuery)`; Java's
    /// `Objects.requireNonNull` is unnecessary because a [`PhraseQuery`] cannot
    /// be null.
    pub fn new(n: i32, query: PhraseQuery) -> Self {
        Self {
            n,
            phrase_query: query,
        }
    }

    /// Returns the `n` in n-gram.
    ///
    /// Equivalent to `NGramPhraseQuery.getN()`.
    pub fn get_n(&self) -> i32 {
        self.n
    }

    /// Returns the list of terms.
    ///
    /// Equivalent to `NGramPhraseQuery.getTerms()`.
    pub fn get_terms(&self) -> &[Term] {
        self.phrase_query.get_terms()
    }

    /// Returns the list of relative positions each term should appear at.
    ///
    /// Equivalent to `NGramPhraseQuery.getPositions()`.
    pub fn get_positions(&self) -> &[i32] {
        self.phrase_query.get_positions()
    }

    /// Returns the wrapped phrase query.
    ///
    /// Equivalent to reading the `private final PhraseQuery phraseQuery` field.
    pub fn get_phrase_query(&self) -> &PhraseQuery {
        &self.phrase_query
    }
}

impl Query for NGramPhraseQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.phrase_query.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub = visitor.get_sub_visitor(Occur::MUST, self);
        self.phrase_query.visit(&mut *sub);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let terms = self.phrase_query.get_terms();
        let positions = self.phrase_query.get_positions();

        let mut is_optimizable = self.phrase_query.get_slop() == 0
            // A non-overlapping n-gram cannot be optimised.
            && self.n >= 2
            // Short ones cannot be optimised.
            && terms.len() >= 3;

        if is_optimizable {
            for i in 1..positions.len() {
                if positions[i] != positions[i - 1] + 1 {
                    is_optimizable = false;
                    break;
                }
            }
        }

        if !is_optimizable {
            // Java returns `phraseQuery.rewrite(indexSearcher)`, which is the
            // phrase query itself when it does not rewrite; this port's `None`
            // would mean that the *n-gram* query rewrites to itself, so the
            // wrapped query is returned explicitly in that case.
            return match self.phrase_query.rewrite(index_searcher)? {
                Some(rewritten) => Ok(Some(rewritten)),
                None => Ok(Some(Arc::new(self.phrase_query.clone()))),
            };
        }

        let mut builder = PhraseQueryBuilder::new();
        for (i, term) in terms.iter().enumerate() {
            if i % self.n as usize == 0 || i == terms.len() - 1 {
                builder.add_at(term.clone(), i as i32)?;
            }
        }
        Ok(Some(Arc::new(builder.build()?)))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<NGramPhraseQuery>() else {
            return false;
        };
        self.n == other.n && self.phrase_query.query_eq(&other.phrase_query)
    }

    fn query_hash(&self) -> u64 {
        let mut h = self.class_hash();
        h = h
            .wrapping_mul(31)
            .wrapping_add(self.phrase_query.query_hash());
        h.wrapping_mul(31).wrapping_add(self.n as u64)
    }
}
