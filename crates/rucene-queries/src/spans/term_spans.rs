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

//! Port of `TermSpans.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::error::Result;
use rucene::index::{PostingsEnum, Term};
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};

use crate::spans::span_collector::SpanCollector;
use crate::spans::spans::{Spans, NO_MORE_POSITIONS};

/// Expert: public for extension only.
///
/// Equivalent to `org.apache.lucene.queries.spans.TermSpans`. This does not
/// work correctly for terms that indexed at position [`i32::MAX`].
pub struct TermSpans {
    /// The protected `postings` field.
    postings: Box<dyn PostingsEnum>,
    /// The protected `term` field.
    term: Term,
    /// The protected `doc` field.
    doc: i32,
    /// The protected `freq` field.
    freq: i32,
    /// The protected `count` field.
    count: i32,
    /// The protected `position` field.
    position: i32,
    /// The protected `readPayload` field.
    read_payload: bool,
    positions_cost: f32,
}

impl TermSpans {
    /// Creates the spans of one term.
    ///
    /// Equivalent to `TermSpans(PostingsEnum, Term, float)`.
    pub fn new(postings: Box<dyn PostingsEnum>, term: Term, positions_cost: f32) -> Self {
        debug_assert!(positions_cost > 0.0); // otherwise the TermSpans should not be created.
        Self {
            postings,
            term,
            doc: -1,
            freq: 0,
            count: 0,
            position: -1,
            read_payload: false,
            positions_cost,
        }
    }

    /// Returns the postings of this term.
    ///
    /// Equivalent to `TermSpans.getPostings()`.
    pub fn get_postings(&self) -> &dyn PostingsEnum {
        &*self.postings
    }
}

impl DocIdSetIterator for TermSpans {
    /// Equivalent to `TermSpans.toString()`.
    fn java_to_string(&self) -> String {
        format!(
            "spans({})@{}",
            self.term,
            if self.doc == -1 {
                "START".to_string()
            } else if self.doc == NO_MORE_DOCS {
                "ENDDOC".to_string()
            } else {
                format!(
                    "{} - {}",
                    self.doc,
                    if self.position == NO_MORE_POSITIONS {
                        "ENDPOS".to_string()
                    } else {
                        self.position.to_string()
                    }
                )
            }
        )
    }

    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc = self.postings.next_doc()?;
        if self.doc != NO_MORE_DOCS {
            self.freq = self.postings.freq()?;
            debug_assert!(self.freq >= 1);
            self.count = 0;
        }
        self.position = -1;
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        debug_assert!(target > self.doc);
        self.doc = self.postings.advance(target)?;
        if self.doc != NO_MORE_DOCS {
            self.freq = self.postings.freq()?;
            debug_assert!(self.freq >= 1);
            self.count = 0;
        }
        self.position = -1;
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.postings.cost()
    }
}

impl Spans for TermSpans {
    fn next_start_position(&mut self) -> Result<i32> {
        if self.count == self.freq {
            debug_assert!(self.position != NO_MORE_POSITIONS);
            self.position = NO_MORE_POSITIONS;
            return Ok(self.position);
        }
        let prev_position = self.position;
        self.position = self.postings.next_position()?;
        debug_assert!(
            self.position >= prev_position,
            "prevPosition={prev_position} > position={}",
            self.position
        );
        debug_assert!(self.position != NO_MORE_POSITIONS); // int endPosition not possible
        self.count += 1;
        self.read_payload = false;
        Ok(self.position)
    }

    fn start_position(&self) -> i32 {
        self.position
    }

    fn end_position(&self) -> i32 {
        if self.position == -1 {
            -1
        } else if self.position != NO_MORE_POSITIONS {
            self.position + 1
        } else {
            NO_MORE_POSITIONS
        }
    }

    fn width(&self) -> i32 {
        0
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        collector.collect_leaf(&*self.postings, self.position, &self.term)
    }

    fn positions_cost(&self) -> f32 {
        self.positions_cost
    }
}
