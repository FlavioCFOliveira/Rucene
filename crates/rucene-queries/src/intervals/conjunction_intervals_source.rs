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

//! Port of `ConjunctionIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::conjunction_matches_iterator::ConjunctionMatchesIterator;
use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches::IntervalMatches;
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::interval_query::IntervalQuery;
use crate::intervals::intervals_source::IntervalsSource;

/// A source whose intervals combine the intervals of all its sub-sources.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.ConjunctionIntervalsSource`, whose
/// final `intervals`, `matches` and overridable `visit` are [`intervals`],
/// [`matches`] and [`visit`], to which an implementor's [`IntervalsSource`]
/// implementation delegates.
pub(crate) trait ConjunctionIntervalsSource: IntervalsSource {
    /// The protected `subSources` field.
    fn sub_sources(&self) -> &[Arc<dyn IntervalsSource>];

    /// Equivalent to the abstract protected
    /// `ConjunctionIntervalsSource.combine(List<IntervalIterator>)`.
    ///
    /// # Errors
    ///
    /// The error of building the combined iterator.
    fn combine(
        &self,
        iterators: Vec<Box<dyn IntervalIterator>>,
    ) -> Result<Box<dyn IntervalIterator>>;

    /// Creates a matches iterator from an advanced and validated interval
    /// iterator and the matches iterators of all the sub-sources.
    ///
    /// Equivalent to the protected
    /// `ConjunctionIntervalsSource.createMatchesIterator(IntervalIterator,
    /// List<IntervalMatchesIterator>)`.
    fn create_matches_iterator(
        &self,
        it: Box<dyn IntervalIterator>,
        subs: Vec<SharedIntervalMatchesIterator>,
    ) -> Box<dyn IntervalMatchesIterator> {
        Box::new(ConjunctionMatchesIterator::new(it, subs))
    }
}

/// Equivalent to `ConjunctionIntervalsSource.visit(String, QueryVisitor)`.
pub(crate) fn visit<T: ConjunctionIntervalsSource>(
    this: Arc<T>,
    field: &str,
    visitor: &mut dyn QueryVisitor,
) {
    let sources: Vec<Arc<dyn IntervalsSource>> = this.sub_sources().to_vec();
    let parent = IntervalQuery::new(field, this);
    let mut v = visitor.get_sub_visitor(Occur::MUST, &parent);
    for source in sources {
        source.visit(field, &mut *v);
    }
}

/// Equivalent to the final `ConjunctionIntervalsSource.intervals(String,
/// LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the errors of the sub-sources and of `combine`.
pub(crate) fn intervals<T: ConjunctionIntervalsSource + ?Sized>(
    this: &T,
    field: &str,
    ctx: &LeafReaderContext,
) -> Result<Option<Box<dyn IntervalIterator>>> {
    let mut sub_intervals = Vec::with_capacity(this.sub_sources().len());
    for source in this.sub_sources() {
        let Some(it) = source.intervals(field, ctx)? else {
            return Ok(None);
        };
        sub_intervals.push(it);
    }
    Ok(Some(this.combine(sub_intervals)?))
}

/// Equivalent to the final `ConjunctionIntervalsSource.matches(String,
/// LeafReaderContext, int)`.
///
/// # Errors
///
/// Propagates the errors of the sub-sources and of `combine`.
pub(crate) fn matches<T: ConjunctionIntervalsSource + ?Sized>(
    this: &T,
    field: &str,
    ctx: &LeafReaderContext,
    doc: i32,
) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
    let mut subs = Vec::with_capacity(this.sub_sources().len());
    for source in this.sub_sources() {
        let Some(mi) = source.matches(field, ctx, doc)? else {
            return Ok(None);
        };
        subs.push(SharedIntervalMatchesIterator::new(mi));
    }
    let mut it = this.combine(
        subs.iter()
            .map(|m| IntervalMatches::wrap_matches(Box::new(m.clone()), doc))
            .collect(),
    )?;
    if it.advance(doc)? != doc {
        return Ok(None);
    }
    if it.next_interval()? == NO_MORE_INTERVALS {
        return Ok(None);
    }
    Ok(Some(this.create_matches_iterator(it, subs)))
}
