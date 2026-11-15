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

//! Port of `FixedFieldIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::java_collections::java_string_hash_code;

use crate::intervals::interval_iterator::IntervalIterator;
use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::intervals_source::{
    objects_equals, objects_hash, same_object, to_java_hash_set, IntervalsSource,
};

/// A source that always reads its intervals from a specific field.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.FixedFieldIntervalsSource`.
#[derive(Debug, Clone)]
pub(crate) struct FixedFieldIntervalsSource {
    field: String,
    source: Arc<dyn IntervalsSource>,
}

impl FixedFieldIntervalsSource {
    /// Equivalent to `FixedFieldIntervalsSource(String, IntervalsSource)`.
    pub(crate) fn new(field: String, source: Arc<dyn IntervalsSource>) -> Self {
        Self { field, source }
    }
}

impl IntervalsSource for FixedFieldIntervalsSource {
    fn intervals(
        &self,
        _field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        self.source.intervals(&self.field, ctx)
    }

    fn matches(
        &self,
        _field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        self.source.matches(&self.field, ctx, doc)
    }

    fn visit(self: Arc<Self>, _field: &str, visitor: &mut dyn QueryVisitor) {
        Arc::clone(&self.source).visit(&self.field, visitor);
    }

    fn min_extent(&self) -> i32 {
        self.source.min_extent()
    }

    /// `Collections.singleton(this)` for a single disjunct, a `HashSet` of
    /// fixed-field sources over the disjuncts otherwise.
    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let inner = Arc::clone(&self.source).pull_up_disjunctions()?;
        if inner.len() == 1 {
            return Ok(vec![self]);
        }
        let field = self.field.clone();
        Ok(to_java_hash_set(inner.into_iter().map(|s| {
            Arc::new(FixedFieldIntervalsSource::new(field.clone(), s)) as Arc<dyn IntervalsSource>
        })))
    }

    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<FixedFieldIntervalsSource>()
                .is_some_and(|that| {
                    self.field == that.field && objects_equals(&self.source, &that.source)
                })
    }

    /// `Objects.hash(field, source)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[
            java_string_hash_code(&self.field),
            self.source.source_hash(),
        ])
    }

    fn to_source_string(&self) -> String {
        format!("FIELD({},{})", self.field, self.source)
    }
}
