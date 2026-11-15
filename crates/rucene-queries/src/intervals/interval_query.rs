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

//! Port of `IntervalQuery.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::{Matches, MatchesIterator, MatchesUtils};
use rucene::search::query::{query_to_string, Query};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::util::java_collections::java_string_hash_code;

use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;
use crate::intervals::interval_score_function::{self, IntervalScoreFunction, SaturationFunction};
use crate::intervals::interval_scorer::IntervalScorer;
use crate::intervals::intervals_source::{objects_equals, objects_hash, IntervalsSource};

/// A query that retrieves documents containing intervals returned from an
/// [`IntervalsSource`].
///
/// Equivalent to the final class
/// `org.apache.lucene.queries.intervals.IntervalQuery`. Static constructor
/// functions for the different sources are in
/// [`Intervals`](crate::intervals::intervals::Intervals).
///
/// Scores are computed as a function of the sloppy frequency of intervals
/// appearing in a document: the number of matching intervals and their
/// width, wider intervals contributing lower values. The `pivot` is the
/// sloppy frequency at which the score of the document equals `0.5` (`1` by
/// default); higher values of `exp` make the function grow more slowly below
/// the pivot and faster above it (`1` by default). Scores are bounded to
/// between `0` and `1`; for higher contributions, wrap the query in a
/// `BoostQuery`.
#[derive(Debug, Clone)]
pub struct IntervalQuery {
    field: String,
    intervals_source: Arc<dyn IntervalsSource>,
    score_function: Arc<dyn IntervalScoreFunction>,
}

impl IntervalQuery {
    /// Creates a new interval query.
    ///
    /// Equivalent to `IntervalQuery(String, IntervalsSource)`, with
    /// `IntervalScoreFunction.saturationFunction(1)`.
    pub fn new(field: impl Into<String>, intervals_source: Arc<dyn IntervalsSource>) -> Self {
        Self::with_score_function(
            field.into(),
            intervals_source,
            Arc::new(SaturationFunction::new(1.0)),
        )
    }

    /// Creates a new interval query with a scoring pivot, the sloppy
    /// frequency at which the score is `0.5`.
    ///
    /// Equivalent to `IntervalQuery(String, IntervalsSource, float)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` unless `pivot` is finite and positive.
    pub fn with_pivot(
        field: impl Into<String>,
        intervals_source: Arc<dyn IntervalsSource>,
        pivot: f32,
    ) -> Result<Self> {
        Ok(Self::with_score_function(
            field.into(),
            intervals_source,
            interval_score_function::saturation_function(pivot)?,
        ))
    }

    /// Creates a new interval query with a scoring pivot and exponent.
    ///
    /// Equivalent to `IntervalQuery(String, IntervalsSource, float, float)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` unless `pivot` and `exp` are finite and
    /// positive.
    pub fn with_pivot_and_exp(
        field: impl Into<String>,
        intervals_source: Arc<dyn IntervalsSource>,
        pivot: f32,
        exp: f32,
    ) -> Result<Self> {
        Ok(Self::with_score_function(
            field.into(),
            intervals_source,
            interval_score_function::sigmoid_function(pivot, exp)?,
        ))
    }

    /// Equivalent to the private `IntervalQuery(String, IntervalsSource,
    /// IntervalScoreFunction)`; Java's `requireNonNull` checks are the types
    /// themselves.
    fn with_score_function(
        field: String,
        intervals_source: Arc<dyn IntervalsSource>,
        score_function: Arc<dyn IntervalScoreFunction>,
    ) -> Self {
        Self {
            field,
            intervals_source,
            score_function,
        }
    }

    /// The field to query.
    ///
    /// Equivalent to `IntervalQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }
}

impl Query for IntervalQuery {
    fn to_query_string(&self, field: &str) -> String {
        let prefix = if self.get_field() != field {
            format!("{}:", self.get_field())
        } else {
            String::new()
        };
        format!("{prefix}{}", self.intervals_source)
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(IntervalWeight {
            query: Arc::new(self.clone()),
            boost,
        }))
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            Arc::clone(&self.intervals_source).visit(&self.field, visitor);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// The same class, an equal field and an equal source; the score function
    /// takes no part.
    fn query_eq(&self, other: &dyn Query) -> bool {
        // `if (this == o) return true;`
        std::ptr::addr_eq(self as *const Self, other as *const dyn Query)
            || other
                .as_any()
                .downcast_ref::<IntervalQuery>()
                .is_some_and(|that| {
                    self.field == that.field
                        && objects_equals(&self.intervals_source, &that.intervals_source)
                })
    }

    /// `Objects.hash(field, intervalsSource)`.
    fn query_hash(&self) -> i32 {
        objects_hash(&[
            java_string_hash_code(&self.field),
            self.intervals_source.source_hash(),
        ])
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.intervals.IntervalQuery"
    }
}

/// The weight of an [`IntervalQuery`].
///
/// Equivalent to the private inner class
/// `org.apache.lucene.queries.intervals.IntervalQuery.IntervalWeight`.
#[derive(Debug)]
pub(crate) struct IntervalWeight {
    /// `IntervalQuery.this`, also the weight's query.
    query: Arc<IntervalQuery>,
    boost: f32,
}

impl IntervalWeight {
    /// `(IntervalScorer) scorer(context)`: the scorer `scorerSupplier`
    /// supplies, or `None` without intervals in the segment.
    fn interval_scorer(&self, context: &LeafReaderContext) -> Result<Option<IntervalScorer>> {
        let query = &self.query;
        let Some(intervals) = query.intervals_source.intervals(&query.field, context)? else {
            return Ok(None);
        };
        Ok(Some(IntervalScorer::new(
            intervals,
            query.intervals_source.min_extent(),
            self.boost,
            &*query.score_function,
        )))
    }
}

impl SegmentCacheable for IntervalWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

/// The anonymous `FilterMatchesIterator` of `IntervalWeight.matches`, whose
/// query is a new `IntervalQuery` over the field and source.
struct IntervalQueryMatchesIterator {
    inner: Box<dyn IntervalMatchesIterator>,
    field: String,
    intervals_source: Arc<dyn IntervalsSource>,
}

impl MatchesIterator for IntervalQueryMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.inner.next()
    }

    fn start_position(&self) -> i32 {
        self.inner.start_position()
    }

    fn end_position(&self) -> i32 {
        self.inner.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.inner.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.inner.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.inner.get_sub_matches()
    }

    fn get_query(&self) -> Option<Arc<dyn Query>> {
        Some(Arc::new(IntervalQuery::new(
            self.field.clone(),
            Arc::clone(&self.intervals_source),
        )))
    }
}

impl Weight for IntervalWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query) as Arc<dyn Query>
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        if let Some(mut scorer) = self.interval_scorer(context)? {
            let new_doc = DocIdSetIterator::advance(&mut scorer, doc)?;
            if new_doc == doc {
                let freq = scorer.freq()?;
                return Ok(self.query.score_function.explain(
                    &query_to_string(&*self.query),
                    self.boost,
                    freq,
                ));
            }
        }
        Ok(Explanation::no_match("no matching intervals", Vec::new()))
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let field = self.query.field.clone();
        let intervals_source = Arc::clone(&self.query.intervals_source);
        let context = context.clone();
        let supplier_field = field.clone();
        MatchesUtils::for_field(
            field,
            Arc::new(move || -> Result<Option<Box<dyn MatchesIterator>>> {
                let Some(mi) = intervals_source.matches(&supplier_field, &context, doc)? else {
                    return Ok(None);
                };
                Ok(Some(Box::new(IntervalQueryMatchesIterator {
                    inner: mi,
                    field: supplier_field.clone(),
                    intervals_source: Arc::clone(&intervals_source),
                })))
            }),
        )
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(scorer) = self.interval_scorer(context)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }
}
