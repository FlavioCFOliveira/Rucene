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

//! Port of `OneTimeIntervalSource.java`: a mock interval source that will
//! only return a constant position for all documents.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene_queries::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use rucene_queries::intervals::interval_matches_iterator::IntervalMatchesIterator;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use crate::queries_support::assertion_error;

/// `OneTimeIntervalSource`.
#[derive(Debug, Default)]
pub struct OneTimeIntervalSource;

/// The anonymous `IntervalIterator` of `intervals`.
struct OneTimeIntervalIterator {
    doc: i32,
    flag: bool,
    max_doc: i32,
}

impl DocIdSetIterator for OneTimeIntervalIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc += 1;
        if self.doc >= self.max_doc {
            self.doc = NO_MORE_DOCS;
        }
        self.flag = true;
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = target;
        if self.doc >= self.max_doc {
            self.doc = NO_MORE_DOCS;
        }
        self.flag = true;
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        0
    }
}

impl IntervalIterator for OneTimeIntervalIterator {
    fn start(&self) -> i32 {
        0
    }

    fn end(&self) -> i32 {
        0
    }

    fn gaps(&self) -> i32 {
        0
    }

    /// Only returns valid position every first time called per doc.
    fn next_interval(&mut self) -> Result<i32> {
        if self.doc != NO_MORE_DOCS {
            if self.flag {
                self.flag = false;
                return Ok(self.start());
            } else {
                return Ok(NO_MORE_INTERVALS);
            }
        }
        Err(assertion_error("Called with docId == NO_MORE_DOCS"))
    }

    fn match_cost(&self) -> f32 {
        0.0
    }
}

/// The anonymous `IntervalMatchesIterator` of `matches`.
struct OneTimeMatchesIterator {
    next: bool,
}

impl MatchesIterator for OneTimeMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        if self.next {
            self.next = false;
            return Ok(true);
        }
        Ok(false)
    }

    fn start_position(&self) -> i32 {
        0
    }

    fn end_position(&self) -> i32 {
        0
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(0)
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(0)
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        Ok(None)
    }

    /// Java returns `null`, which its caller dereferences into a
    /// `NullPointerException`; the infallible core signature throws that
    /// exception here (the class of divergence the intervals package
    /// declares for `getQuery`).
    fn get_query(&self) -> Arc<dyn Query> {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    }
}

impl IntervalMatchesIterator for OneTimeMatchesIterator {
    fn gaps(&self) -> i32 {
        0
    }

    fn width(&self) -> i32 {
        1
    }
}

impl IntervalsSource for OneTimeIntervalSource {
    fn intervals(
        &self,
        _field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        Ok(Some(Box::new(OneTimeIntervalIterator {
            doc: -1,
            flag: false,
            max_doc: ctx.leaf_reader().max_doc(),
        })))
    }

    fn matches(
        &self,
        _field: &str,
        _ctx: &LeafReaderContext,
        _doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        Ok(Some(Box::new(OneTimeMatchesIterator { next: true })))
    }

    fn visit(self: Arc<Self>, _field: &str, _visitor: &mut dyn QueryVisitor) {}

    fn min_extent(&self) -> i32 {
        0
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    fn source_hash(&self) -> i32 {
        0
    }

    fn source_eq(&self, _other: &dyn IntervalsSource) -> bool {
        false
    }

    fn to_source_string(&self) -> String {
        String::new()
    }
}
