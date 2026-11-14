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

//! Port of `DifferenceIntervalsSource.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::query_visitor::QueryVisitor;

use crate::intervals::interval_iterator::IntervalIterator;
use crate::intervals::interval_matches::IntervalMatches;
use crate::intervals::interval_matches_iterator::{
    IntervalMatchesIterator, SharedIntervalMatchesIterator,
};
use crate::intervals::interval_query::IntervalQuery;
use crate::intervals::intervals_source::IntervalsSource;

/// A source whose intervals are those of a minuend, filtered by their
/// relation to those of a subtrahend.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.intervals.DifferenceIntervalsSource`, whose
/// final `intervals` and `matches`, `visit` and `minExtent` are
/// [`intervals`], [`matches`], [`visit`] and [`min_extent`].
pub(crate) trait DifferenceIntervalsSource: IntervalsSource {
    /// The package-private `minuend` field.
    fn minuend(&self) -> &Arc<dyn IntervalsSource>;

    /// The package-private `subtrahend` field.
    fn subtrahend(&self) -> &Arc<dyn IntervalsSource>;

    /// Equivalent to the abstract protected
    /// `DifferenceIntervalsSource.combine(IntervalIterator,
    /// IntervalIterator)`.
    fn combine(
        &self,
        minuend: Box<dyn IntervalIterator>,
        subtrahend: Box<dyn IntervalIterator>,
    ) -> Box<dyn IntervalIterator>;
}

/// Equivalent to the final `DifferenceIntervalsSource.intervals(String,
/// LeafReaderContext)`: the minuend's intervals unchanged when the
/// subtrahend has none in the segment.
///
/// # Errors
///
/// Propagates the errors of the sources.
pub(crate) fn intervals<T: DifferenceIntervalsSource + ?Sized>(
    this: &T,
    field: &str,
    ctx: &LeafReaderContext,
) -> Result<Option<Box<dyn IntervalIterator>>> {
    let Some(min_it) = this.minuend().intervals(field, ctx)? else {
        return Ok(None);
    };
    let Some(sub_it) = this.subtrahend().intervals(field, ctx)? else {
        return Ok(Some(min_it));
    };
    Ok(Some(this.combine(min_it, sub_it)))
}

/// Equivalent to the final `DifferenceIntervalsSource.matches(String,
/// LeafReaderContext, int)`.
///
/// # Errors
///
/// Propagates the errors of the sources.
pub(crate) fn matches<T: DifferenceIntervalsSource + ?Sized>(
    this: &T,
    field: &str,
    ctx: &LeafReaderContext,
    doc: i32,
) -> Result<Option<Box<dyn IntervalMatchesIterator>>> {
    let Some(min_it) = this.minuend().matches(field, ctx, doc)? else {
        return Ok(None);
    };
    let Some(sub_it) = this.subtrahend().matches(field, ctx, doc)? else {
        return Ok(Some(min_it));
    };
    let min_it = SharedIntervalMatchesIterator::new(min_it);
    let difference = this.combine(
        IntervalMatches::wrap_matches(Box::new(min_it.clone()), doc),
        IntervalMatches::wrap_matches(sub_it, doc),
    );
    IntervalMatches::as_matches(difference, Some(Box::new(min_it)), doc)
}

/// Equivalent to `DifferenceIntervalsSource.visit(String, QueryVisitor)`.
pub(crate) fn visit<T: DifferenceIntervalsSource>(
    this: Arc<T>,
    field: &str,
    visitor: &mut dyn QueryVisitor,
) {
    let minuend = Arc::clone(this.minuend());
    let subtrahend = Arc::clone(this.subtrahend());
    let q = IntervalQuery::new(field, this);
    {
        let mut v = visitor.get_sub_visitor(Occur::MUST, &q);
        minuend.visit(field, &mut *v);
    }
    let mut v = visitor.get_sub_visitor(Occur::MUST_NOT, &q);
    subtrahend.visit(field, &mut *v);
}

/// Equivalent to `DifferenceIntervalsSource.minExtent()`.
pub(crate) fn min_extent<T: DifferenceIntervalsSource + ?Sized>(this: &T) -> i32 {
    this.minuend().min_extent()
}
