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

//! Port of `PayloadFilteredTermIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::terms::TermsEnum;
use rucene::index::{
    LeafReaderContext, PostingsEnum, Term, POSTINGS_ENUM_ALL, POSTINGS_ENUM_PAYLOADS,
};
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::BytesRef;

use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::interval_query::IntervalQuery;
use crate::intervals::intervals_source::{
    objects_hash, same_object, utf8_to_string, IntervalsSource,
};
use crate::intervals::term_intervals_source::{no_positions, TermIntervalsSource};

/// The `java.util.function.Predicate<BytesRef>` that filters the payload of
/// each position; it receives the bytes of the payload, `None` for a `null`
/// payload.
pub type PayloadFilter = Arc<dyn Fn(Option<&[u8]>) -> bool + Send + Sync>;

/// The intervals of a term, filtered by the value of the term's payload at
/// each position.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.PayloadFilteredTermIntervalsSource`.
/// As in Java, the filter takes no part in `equals` and `hashCode`.
///
/// **Divergence.** The Java predicate receives the payload as a `BytesRef`;
/// the [`PayloadFilter`] receives the same bytes as a slice.
#[derive(Clone)]
pub(crate) struct PayloadFilteredTermIntervalsSource {
    /// The package-private `term` field.
    term: BytesRef,
    /// The package-private `filter` field.
    filter: PayloadFilter,
}

impl std::fmt::Debug for PayloadFilteredTermIntervalsSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayloadFilteredTermIntervalsSource")
            .field("term", &self.term)
            .finish_non_exhaustive()
    }
}

impl PayloadFilteredTermIntervalsSource {
    /// Equivalent to `PayloadFilteredTermIntervalsSource(BytesRef,
    /// Predicate<BytesRef>)`.
    pub(crate) fn new(term: BytesRef, filter: PayloadFilter) -> Self {
        Self { term, filter }
    }

    /// The shared prologue of `intervals` and `matches`.
    fn seek(&self, field: &str, ctx: &LeafReaderContext) -> Result<Option<Box<dyn TermsEnum>>> {
        let Some(terms) = ctx.leaf_reader().terms(field)? else {
            return Ok(None);
        };
        if !terms.has_positions() {
            return Err(no_positions(field));
        }
        if !terms.has_payloads() {
            return Err(LuceneError::illegal_argument(format!(
                "Cannot create a payload-filtered iterator over field {field} because it has no indexed payloads"
            )));
        }
        let mut te = terms.iterator()?;
        if !te.seek_exact(&self.term)? {
            return Ok(None);
        }
        Ok(Some(te))
    }

    /// Equivalent to the private
    /// `PayloadFilteredTermIntervalsSource.intervals(TermsEnum)`.
    fn intervals_for(&self, te: &mut dyn TermsEnum) -> Result<Box<dyn IntervalIterator>> {
        let pe = te
            .postings(None, POSTINGS_ENUM_PAYLOADS)?
            .ok_or_else(LuceneError::null_pointer)?;
        let cost = TermIntervalsSource::term_positions_cost(te)?;
        Ok(Box::new(PayloadFilteredIntervalIterator {
            term: self.term.clone(),
            filter: Arc::clone(&self.filter),
            pe,
            cost,
            pos: -1,
            upto: 0,
        }))
    }

    /// Equivalent to the private
    /// `PayloadFilteredTermIntervalsSource.matches(TermsEnum, int)`.
    fn matches_for(
        &self,
        te: &mut dyn TermsEnum,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let mut pe = te
            .postings(None, POSTINGS_ENUM_ALL)?
            .ok_or_else(LuceneError::null_pointer)?;
        if pe.advance(doc)? != doc {
            return Ok(None);
        }
        let upto = pe.freq()?;
        Ok(Some(Box::new(PayloadFilteredMatchesIterator {
            filter: Arc::clone(&self.filter),
            pe,
            upto,
            pos: -1,
        })))
    }
}

/// `filter.test(pe.getPayload())`.
fn test_payload(filter: &PayloadFilter, pe: &dyn PostingsEnum) -> Result<bool> {
    let payload = pe.get_payload()?;
    Ok(filter(payload))
}

/// The anonymous `IntervalIterator` of
/// `PayloadFilteredTermIntervalsSource.intervals(TermsEnum)`.
struct PayloadFilteredIntervalIterator {
    term: BytesRef,
    filter: PayloadFilter,
    pe: Box<dyn PostingsEnum>,
    cost: f32,
    pos: i32,
    upto: i32,
}

impl PayloadFilteredIntervalIterator {
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

impl DocIdSetIterator for PayloadFilteredIntervalIterator {
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

impl IntervalIterator for PayloadFilteredIntervalIterator {
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
        loop {
            if self.upto <= 0 {
                self.pos = NO_MORE_INTERVALS;
                return Ok(self.pos);
            }
            self.upto -= 1;
            self.pos = self.pe.next_position()?;
            if test_payload(&self.filter, &*self.pe)? {
                break;
            }
        }
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

/// The anonymous `IntervalMatchesIterator` of
/// `PayloadFilteredTermIntervalsSource.matches(TermsEnum, int)`.
///
/// **Divergence.** Java's `getQuery()` throws
/// `UnsupportedOperationException`; the core
/// [`MatchesIterator::get_query`] cannot fail, so it panics with that
/// exception's name.
struct PayloadFilteredMatchesIterator {
    filter: PayloadFilter,
    pe: Box<dyn PostingsEnum>,
    upto: i32,
    pos: i32,
}

impl MatchesIterator for PayloadFilteredMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        loop {
            if self.upto <= 0 {
                self.pos = NO_MORE_INTERVALS;
                return Ok(false);
            }
            self.upto -= 1;
            self.pos = self.pe.next_position()?;
            if test_payload(&self.filter, &*self.pe)? {
                break;
            }
        }
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
        panic!("java.lang.UnsupportedOperationException")
    }
}

impl IntervalMatchesIterator for PayloadFilteredMatchesIterator {
    fn gaps(&self) -> i32 {
        0
    }

    fn width(&self) -> i32 {
        1
    }
}

impl IntervalsSource for PayloadFilteredTermIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let Some(mut te) = self.seek(field, ctx)? else {
            return Ok(None);
        };
        Ok(Some(self.intervals_for(&mut *te)?))
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
        self.matches_for(&mut *te, doc)
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
                .downcast_ref::<PayloadFilteredTermIntervalsSource>()
                .is_some_and(|that| self.term == that.term)
    }

    fn to_source_string(&self) -> String {
        format!("PAYLOAD_FILTERED({})", utf8_to_string(&self.term))
    }
}
