/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `SpanContainQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;

use crate::spans::span_query::{accept_field, SpanQuery, TermStatesMap};
use crate::spans::span_weight::{Postings, SpanWeight, SpanWeightBase};
use crate::spans::spans::Spans;

/// The common state of [`SpanContainingQuery`] and [`SpanWithinQuery`]: a big
/// and a little span query on the same field.
///
/// Equivalent to the package-private abstract class
/// `org.apache.lucene.queries.spans.SpanContainQuery`. Its two subclasses hold
/// one and forward to its members; Java's `clone()` in `rewrite` is
/// [`with_clauses`](Self::with_clauses).
///
/// [`SpanContainingQuery`]: crate::spans::span_containing_query::SpanContainingQuery
/// [`SpanWithinQuery`]: crate::spans::span_within_query::SpanWithinQuery
#[derive(Debug, Clone)]
pub(crate) struct SpanContainQuery {
    /// The package-private `big` field.
    pub(crate) big: Arc<dyn SpanQuery>,
    /// The package-private `little` field.
    pub(crate) little: Arc<dyn SpanQuery>,
}

impl SpanContainQuery {
    /// Equivalent to the package-private `SpanContainQuery(SpanQuery, SpanQuery)`.
    ///
    /// # Errors
    ///
    /// Returns the `NullPointerException` of `Objects.requireNonNull` when
    /// either field is `None`, and
    /// [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when the fields differ.
    pub(crate) fn new(big: Arc<dyn SpanQuery>, little: Arc<dyn SpanQuery>) -> Result<Self> {
        let big_field = big
            .get_field()
            .ok_or_else(LuceneError::null_pointer)?
            .to_string();
        let little_field = little.get_field().ok_or_else(LuceneError::null_pointer)?;
        if big_field != little_field {
            return Err(LuceneError::illegal_argument(
                "big and little not same field".to_string(),
            ));
        }
        Ok(Self { big, little })
    }

    /// Equivalent to `SpanContainQuery.getField()`.
    pub(crate) fn get_field(&self) -> Option<&str> {
        self.big.get_field()
    }

    /// Equivalent to `SpanContainQuery.getBig()`.
    pub(crate) fn get_big(&self) -> &Arc<dyn SpanQuery> {
        &self.big
    }

    /// Equivalent to `SpanContainQuery.getLittle()`.
    pub(crate) fn get_little(&self) -> &Arc<dyn SpanQuery> {
        &self.little
    }

    /// Equivalent to the package-private
    /// `SpanContainQuery.toString(String, String)`.
    pub(crate) fn to_string_with_name(&self, field: &str, name: &str) -> String {
        let mut buffer = String::new();
        buffer.push_str(name);
        buffer.push('(');
        buffer.push_str(&self.big.to_query_string(field));
        buffer.push_str(", ");
        buffer.push_str(&self.little.to_query_string(field));
        buffer.push(')');
        buffer
    }

    /// A copy of this state with other clauses: Java's `super.clone()`
    /// followed by the assignment of `big` and `little`.
    fn with_clauses(&self, big: Arc<dyn SpanQuery>, little: Arc<dyn SpanQuery>) -> Self {
        Self { big, little }
    }

    /// Equivalent to `SpanContainQuery.rewrite(IndexSearcher)`: the rewritten
    /// state, or `None` when neither clause rewrites.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting the clauses.
    pub(crate) fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Self>> {
        let rewritten_big = self.big.rewrite_span(searcher)?;
        let rewritten_little = self.little.rewrite_span(searcher)?;
        if rewritten_big.is_some() || rewritten_little.is_some() {
            return Ok(Some(self.with_clauses(
                rewritten_big.unwrap_or_else(|| Arc::clone(&self.big)),
                rewritten_little.unwrap_or_else(|| Arc::clone(&self.little)),
            )));
        }
        Ok(None)
    }

    /// Equivalent to `SpanContainQuery.visit(QueryVisitor)`; `this` is the
    /// subclass instance.
    pub(crate) fn visit(&self, this: &dyn Query, visitor: &mut dyn QueryVisitor) {
        if accept_field(visitor, self.get_field()) {
            let mut v = visitor.get_sub_visitor(Occur::MUST, this);
            self.big.visit(&mut *v);
            self.little.visit(&mut *v);
        }
    }

    /// Equivalent to the private `SpanContainQuery.equalsTo(SpanContainQuery)`.
    pub(crate) fn equals_to(&self, other: &SpanContainQuery) -> bool {
        self.big.query_eq(&*other.big as &dyn Query)
            && self.little.query_eq(&*other.little as &dyn Query)
    }

    /// Equivalent to `SpanContainQuery.hashCode()`, given the subclass's
    /// `classHash()`.
    pub(crate) fn hash_code(&self, class_hash: i32) -> i32 {
        let mut h = class_hash.rotate_left(1);
        h ^= self.big.query_hash();
        h = h.rotate_left(1);
        h ^= self.little.query_hash();
        h
    }
}

/// The big and the little spans of a leaf, in that order: the two-element
/// list `SpanContainWeight.prepareConjunction` returns.
pub(crate) type BigAndLittle = (Box<dyn Spans>, Box<dyn Spans>);

/// The common state of the weights of
/// [`SpanContainingQuery`](crate::spans::span_containing_query::SpanContainingQuery)
/// and [`SpanWithinQuery`](crate::spans::span_within_query::SpanWithinQuery).
///
/// Equivalent to the abstract inner class
/// `org.apache.lucene.queries.spans.SpanContainQuery.SpanContainWeight`; its
/// two subclasses hold one.
#[derive(Debug)]
pub struct SpanContainWeight {
    pub(crate) base: SpanWeightBase,
    /// The package-private `bigWeight` field.
    pub(crate) big_weight: Arc<dyn SpanWeight>,
    /// The package-private `littleWeight` field.
    pub(crate) little_weight: Arc<dyn SpanWeight>,
}

impl SpanContainWeight {
    /// Equivalent to
    /// `SpanContainWeight(IndexSearcher, Map<Term, TermStates>, SpanWeight, SpanWeight, float)`;
    /// `query` is the enclosing query.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: Arc<dyn SpanQuery>,
        searcher: &IndexSearcher,
        terms: Option<&TermStatesMap>,
        big_weight: Arc<dyn SpanWeight>,
        little_weight: Arc<dyn SpanWeight>,
        boost: f32,
    ) -> Result<Self> {
        Ok(Self {
            base: SpanWeightBase::new(query, searcher, terms, boost)?,
            big_weight,
            little_weight,
        })
    }

    /// Installs the weak handle on the subclass weight; see
    /// [`SpanWeightBase::bind`].
    pub(crate) fn bind(mut self, this: Weak<dyn SpanWeight>) -> Self {
        self.base = self.base.bind(this);
        self
    }

    /// Opens the big and the little spans of a leaf, or `None` when either is
    /// absent.
    ///
    /// Equivalent to the package-private
    /// `SpanContainWeight.prepareConjunction(LeafReaderContext, Postings)`,
    /// whose two-element list is a pair here.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the spans.
    pub(crate) fn prepare_conjunction(
        &self,
        context: &LeafReaderContext,
        postings: Postings,
    ) -> Result<Option<BigAndLittle>> {
        let Some(big_spans) = self.big_weight.get_spans(context, postings)? else {
            return Ok(None);
        };
        let Some(little_spans) = self.little_weight.get_spans(context, postings)? else {
            return Ok(None);
        };
        Ok(Some((big_spans, little_spans)))
    }

    /// Equivalent to `SpanContainWeight.extractTermStates(Map<Term, TermStates>)`.
    pub(crate) fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        self.big_weight.extract_term_states(contexts);
        self.little_weight.extract_term_states(contexts);
    }
}
