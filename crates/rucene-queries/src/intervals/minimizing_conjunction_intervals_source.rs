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

//! Port of `MinimizingConjunctionIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::caching_matches_iterator::CachingMatchesIterator;
use crate::intervals::conjunction_matches_iterator::ConjunctionMatchesIterator;
use crate::intervals::interval_iterator::{IntervalIterator, NO_MORE_INTERVALS};
use crate::intervals::interval_matches::IntervalMatches;
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::interval_query::IntervalQuery;
use crate::intervals::intervals_source::IntervalsSource;

/// A conjunction source that minimises its intervals, and that may need to
/// cache the matches of its sub-sources at each match.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.MinimizingConjunctionIntervalsSource`,
/// whose `intervals`, `matches` and `visit` are [`intervals`], [`matches`]
/// and [`visit`].
pub(crate) trait MinimizingConjunctionIntervalsSource: IntervalsSource {
    /// The protected `subSources` field.
    fn sub_sources(&self) -> &[Arc<dyn IntervalsSource>];

    /// Equivalent to the abstract protected
    /// `MinimizingConjunctionIntervalsSource.combine(List<IntervalIterator>,
    /// MatchCallback)`.
    ///
    /// # Errors
    ///
    /// The error of building the combined iterator.
    fn combine(
        &self,
        iterators: Vec<Box<dyn IntervalIterator>>,
        on_match: Box<dyn MatchCallback>,
    ) -> Result<Box<dyn IntervalIterator>>;
}

/// Equivalent to `MinimizingConjunctionIntervalsSource.intervals(String,
/// LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the errors of the sub-sources and of `combine`.
pub(crate) fn intervals<T: MinimizingConjunctionIntervalsSource + ?Sized>(
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
    Ok(Some(
        this.combine(sub_intervals, Box::new(NoOpMatchCallback))?,
    ))
}

/// Equivalent to `MinimizingConjunctionIntervalsSource.matches(String,
/// LeafReaderContext, int)`: the sub-matches are cached at every match of
/// the combined iterator, so that the positions of a minimised match can be
/// reported.
///
/// # Errors
///
/// Propagates the errors of the sub-sources and of `combine`.
pub(crate) fn matches<T: MinimizingConjunctionIntervalsSource + ?Sized>(
    this: &T,
    field: &str,
    ctx: &LeafReaderContext,
    doc: i32,
) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
    let mut subs: Vec<Rc<RefCell<CachingMatchesIterator>>> =
        Vec::with_capacity(this.sub_sources().len());
    for source in this.sub_sources() {
        let Some(mi) = source.matches(field, ctx, doc)? else {
            return Ok(None);
        };
        subs.push(Rc::new(RefCell::new(CachingMatchesIterator::new(mi))));
    }
    let shared: Vec<SharedIntervalMatchesIterator> = subs
        .iter()
        .map(|m| {
            SharedIntervalMatchesIterator::from_rc(
                Rc::clone(m) as Rc<RefCell<dyn IntervalMatchesIterator>>
            )
        })
        .collect();
    let mut it = this.combine(
        shared
            .iter()
            .map(|m| IntervalMatches::wrap_matches(Box::new(m.clone()), doc))
            .collect(),
        cache_iterators(subs),
    )?;
    if it.advance(doc)? != doc {
        return Ok(None);
    }
    if it.next_interval()? == NO_MORE_INTERVALS {
        return Ok(None);
    }
    Ok(Some(Box::new(ConjunctionMatchesIterator::new(it, shared))))
}

/// Equivalent to `MinimizingConjunctionIntervalsSource.visit(String,
/// QueryVisitor)`.
pub(crate) fn visit<T: MinimizingConjunctionIntervalsSource>(
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

/// Called when the parent iterator has found a match.
///
/// Equivalent to the package-private interface
/// `org.apache.lucene.queries.intervals.MinimizingConjunctionIntervalsSource.MatchCallback`.
pub(crate) trait MatchCallback {
    /// Equivalent to `MatchCallback.onMatch()`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reacting to the match.
    fn on_match(&mut self) -> Result<()>;
}

/// The callback that does nothing.
///
/// Equivalent to the constant `MatchCallback.NO_OP`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NoOpMatchCallback;

impl MatchCallback for NoOpMatchCallback {
    fn on_match(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The callback that caches every one of `its` at each match.
///
/// Equivalent to the package-private static
/// `MinimizingConjunctionIntervalsSource.cacheIterators(Collection<CachingMatchesIterator>)`.
pub(crate) fn cache_iterators(
    its: Vec<Rc<RefCell<CachingMatchesIterator>>>,
) -> Box<dyn MatchCallback> {
    Box::new(CacheIterators { its })
}

/// The lambda `cacheIterators` returns.
struct CacheIterators {
    its: Vec<Rc<RefCell<CachingMatchesIterator>>>,
}

impl MatchCallback for CacheIterators {
    fn on_match(&mut self) -> Result<()> {
        for it in &self.its {
            it.borrow_mut().cache()?;
        }
        Ok(())
    }
}
