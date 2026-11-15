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

//! Port of `TermIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::terms::TermsEnum;
use rucene::index::{
    LeafReaderContext, PostingsEnum, Term, POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::term_query::TermQuery;
use rucene::util::BytesRef;

use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::interval_query::IntervalQuery;
use crate::intervals::intervals_source::{
    objects_hash, same_object, utf8_to_string, IntervalsSource,
};

/// A guess of the average number of simple operations for the initial seek
/// and buffer refill per document for the positions of a term.
///
/// Equivalent to the private `TermIntervalsSource.TERM_POSNS_SEEK_OPS_PER_DOC`.
const TERM_POSNS_SEEK_OPS_PER_DOC: i32 = 256;

/// Number of simple operations in a postings `nextPosition()` when no seek or
/// buffer refill is done.
///
/// Equivalent to the private `TermIntervalsSource.TERM_OPS_PER_POS`.
const TERM_OPS_PER_POS: i32 = 7;

/// The message of a field indexed without positions.
pub(crate) fn no_positions(field: &str) -> LuceneError {
    LuceneError::illegal_argument(format!(
        "Cannot create an IntervalIterator over field {field} because it has no indexed positions"
    ))
}

/// The intervals of a single term: one interval per position.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.TermIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct TermIntervalsSource {
    /// The package-private `term` field.
    pub(crate) term: BytesRef,
}

impl TermIntervalsSource {
    /// Equivalent to `TermIntervalsSource(BytesRef)`.
    pub(crate) fn new(term: BytesRef) -> Self {
        Self { term }
    }

    /// The intervals of the term at which `te` is positioned.
    ///
    /// Equivalent to the package-private static
    /// `TermIntervalsSource.intervals(BytesRef, TermsEnum)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the postings.
    pub(crate) fn intervals_for_term(
        term: BytesRef,
        te: &mut dyn TermsEnum,
    ) -> Result<Box<dyn IntervalIterator>> {
        let pe = te
            .postings(None, POSTINGS_ENUM_POSITIONS)?
            .ok_or_else(LuceneError::null_pointer)?;
        let cost = Self::term_positions_cost(te)?;
        Ok(Box::new(TermIntervalIterator {
            term,
            pe,
            cost,
            pos: -1,
            upto: 0,
        }))
    }

    /// The matches of the term at which `te` is positioned in `doc`, or
    /// `None` when the term does not occur in it.
    ///
    /// Equivalent to the package-private static
    /// `TermIntervalsSource.matches(TermsEnum, int, String)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the postings.
    pub(crate) fn matches_for_term(
        te: &mut dyn TermsEnum,
        doc: i32,
        field: &str,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let term = te.term()?.ok_or_else(LuceneError::null_pointer)?;
        let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new(field.to_string(), term)));
        let mut pe = te
            .postings(None, POSTINGS_ENUM_OFFSETS)?
            .ok_or_else(LuceneError::null_pointer)?;
        if pe.advance(doc)? != doc {
            return Ok(None);
        }
        let upto = pe.freq()?;
        Ok(Some(Box::new(TermMatchesIterator {
            pe,
            query,
            upto,
            pos: -1,
        })))
    }

    /// Returns an expected cost in simple operations of processing the
    /// occurrences of the term at which `terms_enum` is positioned in a
    /// document that contains it.
    ///
    /// Equivalent to the package-private static
    /// `TermIntervalsSource.termPositionsCost(TermsEnum)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub(crate) fn term_positions_cost(terms_enum: &mut dyn TermsEnum) -> Result<f32> {
        // TODO: When intervals move to core, refactor to use the copy of this
        // in PhraseQuery
        let doc_freq = terms_enum.doc_freq()?;
        debug_assert!(doc_freq > 0);
        let total_term_freq = terms_enum.total_term_freq()?;
        let exp_occurrences_in_matching_doc = total_term_freq as f32 / doc_freq as f32;
        Ok(TERM_POSNS_SEEK_OPS_PER_DOC as f32
            + exp_occurrences_in_matching_doc * TERM_OPS_PER_POS as f32)
    }

    /// The terms of `field` positioned on the term, or `None` when the field
    /// or the term is absent; the shared prologue of `intervals` and `matches`.
    fn seek(&self, field: &str, ctx: &LeafReaderContext) -> Result<Option<Box<dyn TermsEnum>>> {
        let Some(terms) = ctx.leaf_reader().terms(field)? else {
            return Ok(None);
        };
        if !terms.has_positions() {
            return Err(no_positions(field));
        }
        let mut te = terms.iterator()?;
        if !te.seek_exact(&self.term)? {
            return Ok(None);
        }
        Ok(Some(te))
    }
}

/// The anonymous `IntervalIterator` of `TermIntervalsSource.intervals`.
struct TermIntervalIterator {
    term: BytesRef,
    pe: Box<dyn PostingsEnum>,
    cost: f32,
    pos: i32,
    upto: i32,
}

impl TermIntervalIterator {
    /// The private `reset()` of the anonymous class.
    fn reset(&mut self) -> Result<()> {
        if self.pe.doc_id() == NO_MORE_DOCS {
            self.upto = -1;
            self.pos = NO_MORE_INTERVALS;
        } else {
            self.upto = self.pe.freq()?;
            self.pos = -1;
        }
        Ok(())
    }
}

impl DocIdSetIterator for TermIntervalIterator {
    fn doc_id(&self) -> i32 {
        self.pe.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.pe.next_doc()?;
        self.reset()?;
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.pe.advance(target)?;
        self.reset()?;
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.pe.cost()
    }
}

impl IntervalIterator for TermIntervalIterator {
    fn start(&self) -> i32 {
        self.pos
    }

    fn end(&self) -> i32 {
        self.pos
    }

    fn gaps(&self) -> i32 {
        0
    }

    fn next_interval(&mut self) -> Result<i32> {
        if self.upto <= 0 {
            self.pos = NO_MORE_INTERVALS;
            return Ok(self.pos);
        }
        self.upto -= 1;
        self.pos = self.pe.next_position()?;
        Ok(self.pos)
    }

    fn match_cost(&self) -> f32 {
        self.cost
    }

    fn interval_to_string(&self) -> String {
        format!(
            "{}:{}:[{}->{}]",
            utf8_to_string(&self.term),
            self.doc_id(),
            self.start(),
            self.end()
        )
    }
}

/// The anonymous `IntervalMatchesIterator` of `TermIntervalsSource.matches`.
struct TermMatchesIterator {
    pe: Box<dyn PostingsEnum>,
    query: Arc<dyn Query>,
    upto: i32,
    pos: i32,
}

impl MatchesIterator for TermMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        if self.upto <= 0 {
            self.pos = NO_MORE_INTERVALS;
            return Ok(false);
        }
        self.upto -= 1;
        self.pos = self.pe.next_position()?;
        Ok(true)
    }

    fn start_position(&self) -> i32 {
        self.pos
    }

    fn end_position(&self) -> i32 {
        self.pos
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(self.pe.start_offset())
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(self.pe.end_offset())
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        Ok(None)
    }

    fn get_query(&self) -> Option<Arc<dyn Query>> {
        Some(Arc::clone(&self.query))
    }
}

impl IntervalMatchesIterator for TermMatchesIterator {
    fn gaps(&self) -> i32 {
        0
    }

    fn width(&self) -> i32 {
        1
    }
}

impl IntervalsSource for TermIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let Some(mut te) = self.seek(field, ctx)? else {
            return Ok(None);
        };
        Ok(Some(Self::intervals_for_term(self.term.clone(), &mut *te)?))
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let Some(mut te) = self.seek(field, ctx)? else {
            return Ok(None);
        };
        Self::matches_for_term(&mut *te, doc, field)
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        let term = Term::new(field.to_string(), self.term.clone());
        let query = IntervalQuery::new(field, self);
        visitor.consume_terms(&query, std::slice::from_ref(&term));
    }

    fn min_extent(&self) -> i32 {
        1
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    /// `Objects.hash(term)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.term.hash_code()])
    }

    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<TermIntervalsSource>()
                .is_some_and(|that| self.term == that.term)
    }

    fn to_source_string(&self) -> String {
        utf8_to_string(&self.term)
    }
}
