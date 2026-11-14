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

//! Port of `NoMatchIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::java_collections::java_string_hash_code;

use crate::intervals::interval_iterator::IntervalIterator;
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::intervals_source::{objects_hash, IntervalsSource};

/// A source returning no matches.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.NoMatchIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct NoMatchIntervalsSource {
    /// The package-private `reason` field.
    reason: String,
}

impl NoMatchIntervalsSource {
    /// Equivalent to `NoMatchIntervalsSource(String)`.
    pub(crate) fn new(reason: String) -> Self {
        Self { reason }
    }
}

impl IntervalsSource for NoMatchIntervalsSource {
    fn intervals(
        &self,
        _field: &str,
        _ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        Ok(None)
    }

    fn matches(
        &self,
        _field: &str,
        _ctx: &LeafReaderContext,
        _doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        Ok(None)
    }

    fn visit(self: Arc<Self>, _field: &str, _visitor: &mut dyn QueryVisitor) {}

    fn min_extent(&self) -> i32 {
        0
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![self])
    }

    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        other
            .as_any()
            .downcast_ref::<NoMatchIntervalsSource>()
            .is_some_and(|that| self.reason == that.reason)
    }

    /// `Objects.hash(reason)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[java_string_hash_code(&self.reason)])
    }

    fn to_source_string(&self) -> String {
        format!("NOMATCH({})", self.reason)
    }
}
