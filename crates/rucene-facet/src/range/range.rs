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

//! Port of `Range.java`, of the Java package `org.apache.lucene.facet.range`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::doc_id_set_iterator;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::weight::DefaultScorerSupplier;
use rucene::search::{DocIdSetIterator, IndexSearcher, Query, QueryVisitor, ScoreMode, Weight};

use crate::facet_counts_with_filter_query::ScorerIterator;
use crate::label_and_value::objects_hash;

/// Base class for a single labeled range.
///
/// Equivalent to `org.apache.lucene.facet.range.Range`; its subclasses
/// `LongRange` and `DoubleRange` hold one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Range {
    /// Label that identifies this range.
    pub label: String,
}

impl Range {
    /// Sole constructor; Java's `null` label check is carried by the type.
    ///
    /// Equivalent to `Range(String)`.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }

    /// Invoke this for a useless range.
    ///
    /// Equivalent to `failNoMatch()`.
    pub fn fail_no_match(&self) -> LuceneError {
        LuceneError::illegal_argument(format!("range \"{}\" matches nothing", self.label))
    }
}

/// The per-segment test of a range filter query: Java's
/// `TwoPhaseIterator.matches()` of the `ValueSourceQuery` and
/// `MultiValueSourceQuery` nested classes of `LongRange` and `DoubleRange`.
pub(crate) trait RangeMatcher {
    fn matches(&mut self, doc: i32) -> Result<bool>;
}

/// The range and values source of a range filter query.
pub(crate) trait RangeFilterSource: fmt::Debug + Send + Sync + 'static {
    /// `getClass().getName()` of the Java query class.
    fn java_class_name(&self) -> &'static str;
    /// `range.toString()`.
    fn range_to_string(&self) -> String;
    /// `range.equals(other.range) && valueSource.equals(other.valueSource)`.
    fn source_eq(&self, other: &Self) -> bool;
    /// `range.hashCode()` and `valueSource.hashCode()`.
    fn hash_parts(&self) -> (i32, i32);
    /// `valueSource.isCacheable(ctx)`.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool;
    /// The values of `context`, with the range test.
    fn matcher(&self, context: &LeafReaderContext) -> Result<Box<dyn RangeMatcher>>;
}

/// A `Query` that matches the documents whose values fall in a range: the
/// shared body of the `ValueSourceQuery`/`MultiValueSourceQuery` nested
/// classes of `LongRange` and `DoubleRange`.
pub(crate) struct RangeFilterQuery<S: RangeFilterSource> {
    pub(crate) source: Arc<S>,
    pub(crate) fast_match_query: Option<Arc<dyn Query>>,
}

impl<S: RangeFilterSource> Clone for RangeFilterQuery<S> {
    fn clone(&self) -> Self {
        Self {
            source: Arc::clone(&self.source),
            fast_match_query: self.fast_match_query.clone(),
        }
    }
}

impl<S: RangeFilterSource> fmt::Debug for RangeFilterQuery<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RangeFilterQuery")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl<S: RangeFilterSource> Query for RangeFilterQuery<S> {
    fn to_query_string(&self, _field: &str) -> String {
        format!("Filter({})", self.source.range_to_string())
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if let Some(fast_match_query) = &self.fast_match_query {
            if let Some(fast_match_rewritten) = fast_match_query.rewrite(index_searcher)? {
                if !Arc::ptr_eq(&fast_match_rewritten, fast_match_query) {
                    return Ok(Some(Arc::new(RangeFilterQuery {
                        source: Arc::clone(&self.source),
                        fast_match_query: Some(fast_match_rewritten),
                    })));
                }
            }
        }
        Ok(None)
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let fast_match_weight = match &self.fast_match_query {
            None => None,
            Some(q) => Some(searcher.create_weight(Arc::clone(q), ScoreMode::COMPLETE_NO_SCORES, 1.0)?),
        };
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            RangeFilterWeight {
                source: Arc::clone(&self.source),
                fast_match_weight,
                score_mode,
            },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other.as_any().downcast_ref::<RangeFilterQuery<S>>().is_some_and(|o| {
                let fmq_eq = match (&self.fast_match_query, &o.fast_match_query) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.query_eq(&**b),
                    _ => false,
                };
                self.source.source_eq(&o.source) && fmq_eq
            })
    }

    fn query_hash(&self) -> i32 {
        // `classHash() + 31 * Objects.hash(range, fastMatchQuery, valueSource)`.
        let (range_hash, source_hash) = self.source.hash_parts();
        let fmq_hash = self.fast_match_query.as_ref().map_or(0, |q| q.query_hash());
        self.class_hash()
            .wrapping_add(31i32.wrapping_mul(objects_hash(&[range_hash, fmq_hash, source_hash])))
    }

    fn java_class_name(&self) -> &'static str {
        self.source.java_class_name()
    }
}

/// The anonymous `ConstantScoreWeight` of `createWeight`.
struct RangeFilterWeight<S: RangeFilterSource> {
    source: Arc<S>,
    fast_match_weight: Option<Arc<dyn Weight>>,
    score_mode: ScoreMode,
}

impl<S: RangeFilterSource> fmt::Debug for RangeFilterWeight<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RangeFilterWeight")
    }
}

impl<S: RangeFilterSource> ConstantScoreWeightImpl for RangeFilterWeight<S> {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let max_doc = context.leaf_reader().max_doc();

        let approximation: Box<dyn DocIdSetIterator> = match &self.fast_match_weight {
            None => Box::new(doc_id_set_iterator::all(max_doc)?),
            Some(fast_match_weight) => match fast_match_weight.scorer(context)? {
                None => return Ok(None),
                Some(s) => Box::new(ScorerIterator { scorer: s }),
            },
        };

        let matcher = self.source.matcher(context)?;
        let two_phase = RangeTwoPhase {
            approximation,
            matcher,
        };
        let scorer = ConstantScoreScorer::from_two_phase(score, self.score_mode, Box::new(two_phase));
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.source.is_cacheable(ctx)
    }
}

/// The anonymous `TwoPhaseIterator` of `scorerSupplier`.
struct RangeTwoPhase {
    approximation: Box<dyn DocIdSetIterator>,
    matcher: Box<dyn RangeMatcher>,
}

impl TwoPhaseIterator for RangeTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &*self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.approximation.doc_id();
        self.matcher.matches(doc)
    }

    fn match_cost(&self) -> f32 {
        100.0 // TODO: use cost of range.accept()
    }
}
