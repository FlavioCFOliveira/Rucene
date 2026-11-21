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

//! Port of `FilteredIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::fmt::Debug;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::java_collections::java_string_hash_code;

use crate::intervals::disjunctions::Disjunctions;
use crate::intervals::interval_filter::IntervalFilter;
use crate::intervals::interval_iterator::IntervalIterator;
use crate::intervals::interval_matches::IntervalMatches;
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::intervals::Intervals;
use crate::intervals::intervals_source::{
    objects_equals, objects_hash, same_object, IntervalsSource,
};

/// The members a subclass of [`FilteredIntervalsSource`] overrides: its
/// abstract `accept`, and optionally `pullUpDisjunctions`.
pub trait FilteredIntervalsSourceFilter: Debug + Send + Sync {
    /// Returns `false` if the current interval should be filtered out.
    ///
    /// Equivalent to the abstract protected
    /// `FilteredIntervalsSource.accept(IntervalIterator)`.
    fn accept(&self, it: &dyn IntervalIterator) -> bool;

    /// Equivalent to `FilteredIntervalsSource.pullUpDisjunctions()`, which
    /// returns `Collections.singletonList(this)` unless overridden; `this` is
    /// the filtered source.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    fn pull_up_disjunctions(
        &self,
        this: Arc<FilteredIntervalsSource>,
    ) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        Ok(vec![this])
    }
}

/// An intervals source that filters the intervals from another source.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.FilteredIntervalsSource`.
///
/// **Divergence.** Java's subclasses override `accept(IntervalIterator)`
/// (and `MaxWidth` `pullUpDisjunctions()`); here a subclass is the
/// [`FilteredIntervalsSourceFilter`] the source is built with. Equality is
/// Java's: any two filtered sources with the same name and filtered source
/// are equal, whatever their filters.
#[derive(Debug, Clone)]
pub struct FilteredIntervalsSource {
    /// The private `name` field.
    name: String,
    /// The protected `in` field.
    in_: Arc<dyn IntervalsSource>,
    filter: Arc<dyn FilteredIntervalsSourceFilter>,
}

impl FilteredIntervalsSource {
    /// Creates a new filtered source.
    ///
    /// Equivalent to `FilteredIntervalsSource(String, IntervalsSource)`, with
    /// the subclass's `accept`.
    pub fn new(
        name: impl Into<String>,
        in_: Arc<dyn IntervalsSource>,
        filter: Arc<dyn FilteredIntervalsSourceFilter>,
    ) -> Self {
        Self {
            name: name.into(),
            in_,
            filter,
        }
    }

    /// Filters a source by the gaps of its intervals.
    ///
    /// Equivalent to the static
    /// `FilteredIntervalsSource.maxGaps(IntervalsSource, int)`: a `MaxGaps`
    /// over each disjunction the source pulls up, or-ed together.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn max_gaps(
        in_: Arc<dyn IntervalsSource>,
        max_gaps: i32,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let sources: Vec<Arc<dyn IntervalsSource>> = in_
            .pull_up_disjunctions()?
            .into_iter()
            .map(|s| Arc::new(MaxGaps::source(s, max_gaps)) as Arc<dyn IntervalsSource>)
            .collect();
        Intervals::or(sources)
    }

    /// Filters a source by the width of its intervals.
    ///
    /// Equivalent to the static
    /// `FilteredIntervalsSource.maxWidth(IntervalsSource, int)`.
    pub fn max_width(in_: Arc<dyn IntervalsSource>, max_width: i32) -> Arc<dyn IntervalsSource> {
        Arc::new(MaxWidth::source(in_, max_width))
    }
}

impl IntervalsSource for FilteredIntervalsSource {
    fn intervals(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
    ) -> Result<Option<Box<dyn IntervalIterator>>> {
        let Some(i) = self.in_.intervals(field, ctx)? else {
            return Ok(None);
        };
        let filter = Arc::clone(&self.filter);
        Ok(Some(Box::new(IntervalFilter::new(
            i,
            Box::new(move |in_: &dyn IntervalIterator| filter.accept(in_)),
        ))))
    }

    fn matches(
        &self,
        field: &str,
        ctx: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
        let Some(mi) = self.in_.matches(field, ctx, doc)? else {
            return Ok(None);
        };
        let mi = SharedIntervalMatchesIterator::new(mi);
        let filter = Arc::clone(&self.filter);
        let filtered = IntervalFilter::new(
            IntervalMatches::wrap_matches(Box::new(mi.clone()), doc),
            Box::new(move |in_: &dyn IntervalIterator| filter.accept(in_)),
        );
        IntervalMatches::as_matches(Box::new(filtered), Some(Box::new(mi)), doc)
    }

    fn min_extent(&self) -> i32 {
        self.in_.min_extent()
    }

    fn pull_up_disjunctions(self: Arc<Self>) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let filter = Arc::clone(&self.filter);
        filter.pull_up_disjunctions(self)
    }

    fn visit(self: Arc<Self>, field: &str, visitor: &mut dyn QueryVisitor) {
        Arc::clone(&self.in_).visit(field, visitor);
    }

    /// `o instanceof FilteredIntervalsSource` with an equal name and source.
    fn source_eq(&self, other: &dyn IntervalsSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<FilteredIntervalsSource>()
                .is_some_and(|that| self.name == that.name && objects_equals(&self.in_, &that.in_))
    }

    /// `Objects.hash(name, in)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[java_string_hash_code(&self.name), self.in_.source_hash()])
    }

    fn to_source_string(&self) -> String {
        format!("{}({})", self.name, self.in_)
    }
}

/// The filter that keeps intervals with at most `max_gaps` gaps.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.FilteredIntervalsSource.MaxGaps`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MaxGaps {
    max_gaps: i32,
}

impl MaxGaps {
    /// Equivalent to `MaxGaps(IntervalsSource, int)`:
    /// `super("MAXGAPS/" + maxGaps, in)`.
    fn source(in_: Arc<dyn IntervalsSource>, max_gaps: i32) -> FilteredIntervalsSource {
        FilteredIntervalsSource::new(
            format!("MAXGAPS/{max_gaps}"),
            in_,
            Arc::new(MaxGaps { max_gaps }),
        )
    }
}

impl FilteredIntervalsSourceFilter for MaxGaps {
    fn accept(&self, it: &dyn IntervalIterator) -> bool {
        it.gaps() <= self.max_gaps
    }
}

/// The filter that keeps intervals at most `max_width` wide.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.intervals.FilteredIntervalsSource.MaxWidth`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MaxWidth {
    max_width: i32,
}

impl MaxWidth {
    /// Equivalent to `MaxWidth(IntervalsSource, int)`:
    /// `super("MAXWIDTH/" + maxWidth, in)`.
    fn source(in_: Arc<dyn IntervalsSource>, max_width: i32) -> FilteredIntervalsSource {
        FilteredIntervalsSource::new(
            format!("MAXWIDTH/{max_width}"),
            in_,
            Arc::new(MaxWidth { max_width }),
        )
    }
}

impl FilteredIntervalsSourceFilter for MaxWidth {
    fn accept(&self, it: &dyn IntervalIterator) -> bool {
        it.end().wrapping_sub(it.start()).wrapping_add(1) <= self.max_width
    }

    /// `Disjunctions.pullUp(in, s -> new MaxWidth(s, maxWidth))`.
    fn pull_up_disjunctions(
        &self,
        this: Arc<FilteredIntervalsSource>,
    ) -> Result<Vec<Arc<dyn IntervalsSource>>> {
        let max_width = self.max_width;
        Disjunctions::pull_up_one(Arc::clone(&this.in_), |s| {
            Arc::new(MaxWidth::source(s, max_width)) as Arc<dyn IntervalsSource>
        })
    }
}
