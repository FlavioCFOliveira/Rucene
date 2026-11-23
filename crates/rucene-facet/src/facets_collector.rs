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

//! Port of `FacetsCollector.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{IndexReaderContext, LeafReaderContext};
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::{CollectionResult, Collector, LeafCollector, Scorable, ScoreMode};
use rucene::util::doc_id_set::DocIdSetBuilder;
use rucene::util::ArrayUtil;

/// A `DocIdSet` shared by the `MatchingDocs` that hold it.
pub type SharedDocIdSet = Arc<dyn DocIdSet + Send + Sync>;

/// Recovers the owning handle on the leaf context a collector is handed.
///
/// Java's collectors keep the `LeafReaderContext` they receive; Rucene's
/// receive a borrow. The context is found again among its parent's leaves by
/// its identity; a top-level leaf, which has no parent, is rebuilt with
/// `owned_leaf_context` (same reader, ordinal and doc base). The original
/// identity is kept in [`MatchingDocs::context_identity`], which is what Java
/// compares contexts by.
pub(crate) fn retain_leaf_context(context: &LeafReaderContext) -> Arc<LeafReaderContext> {
    if let Some(parent) = IndexReaderContext::parent(context).and_then(|p| p.upgrade()) {
        if let Some(found) = parent
            .leaves()
            .into_iter()
            .find(|leaf| leaf.id() == context.id())
        {
            return found;
        }
    }
    rucene::search::matches::owned_leaf_context(context)
}

/// Holds the documents that were matched in the `LeafReaderContext`. If
/// scores were required, then `scores` is not `None`.
///
/// Equivalent to the record
/// `org.apache.lucene.facet.FacetsCollector.MatchingDocs`.
#[derive(Clone)]
pub struct MatchingDocs {
    /// Context for this segment.
    pub context: Arc<LeafReaderContext>,
    /// Which documents were seen.
    pub bits: SharedDocIdSet,
    /// Total number of hits.
    pub total_hits: i32,
    /// Non-sparse scores array.
    pub scores: Option<Vec<f32>>,
    /// The identity of the context the collector saw, which Java's `==` and
    /// `HashMap` compare.
    pub context_identity: usize,
}

impl MatchingDocs {
    /// The record's canonical constructor.
    pub fn new(
        context: Arc<LeafReaderContext>,
        bits: SharedDocIdSet,
        total_hits: i32,
        scores: Option<Vec<f32>>,
    ) -> Self {
        let context_identity = context.id();
        Self {
            context,
            bits,
            total_hits,
            scores,
            context_identity,
        }
    }

    /// Equivalent to `context()`.
    pub fn context(&self) -> &Arc<LeafReaderContext> {
        &self.context
    }

    /// Equivalent to `bits()`.
    pub fn bits(&self) -> &SharedDocIdSet {
        &self.bits
    }

    /// Equivalent to `totalHits()`.
    pub fn total_hits(&self) -> i32 {
        self.total_hits
    }

    /// Equivalent to `scores()`.
    pub fn scores(&self) -> Option<&[f32]> {
        self.scores.as_deref()
    }
}

impl std::fmt::Debug for MatchingDocs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatchingDocs")
            .field("context", &self.context.to_string())
            .field("total_hits", &self.total_hits)
            .finish_non_exhaustive()
    }
}

/// The methods of `FacetsCollector` a subclass overrides: `doSetNextReader`
/// and `finish`.
///
/// Rust has no implementation inheritance; a subclass's overrides (and its
/// own fields) are this strategy object, which receives the collector in
/// place of `this`, and its defaults are the base class's bodies
/// ([`FacetsCollector::super_do_set_next_reader`],
/// [`FacetsCollector::super_finish`]). [`Self::as_any`] answers Java's
/// `instanceof` tests and casts on the subclass.
pub trait FacetsCollectorOverrides: Send + Sync {
    /// Equivalent to the protected `doSetNextReader(LeafReaderContext)`.
    fn do_set_next_reader(
        &self,
        collector: &mut FacetsCollector,
        context: &LeafReaderContext,
    ) -> Result<()> {
        collector.super_do_set_next_reader(context)
    }

    /// Equivalent to `finish()`.
    fn finish(&self, collector: &mut FacetsCollector) -> Result<()> {
        collector.super_finish()
    }

    /// The subclass instance, for `instanceof` tests and casts.
    fn as_any(&self) -> &dyn Any;
}

/// Collects hits for subsequent faceting.
///
/// Equivalent to `org.apache.lucene.facet.FacetsCollector`, a
/// `SimpleCollector`.
pub struct FacetsCollector {
    context: Option<Arc<LeafReaderContext>>,
    context_identity: usize,
    total_hits: i32,
    scores: Option<Vec<f32>>,
    keep_scores: bool,
    matching_docs: Vec<MatchingDocs>,
    docs_builder: Option<DocIdSetBuilder>,
    /// The `RandomSamplingFacetsCollector` state, whose `getMatchingDocs()`
    /// override samples the collected docs.
    pub(crate) sampling: Option<Box<crate::random_sampling_facets_collector::SamplingState>>,
    /// A subclass's overrides of `doSetNextReader` and `finish`; `None` is
    /// the base class.
    overrides: Option<Arc<dyn FacetsCollectorOverrides>>,
}

impl Default for FacetsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl FacetsCollector {
    /// Default constructor.
    ///
    /// Equivalent to `FacetsCollector()`.
    pub fn new() -> Self {
        Self::with_keep_scores(false)
    }

    /// Create this; if `keep_scores` is true then a float[] is allocated to
    /// hold score of all hits.
    ///
    /// Equivalent to `FacetsCollector(boolean)`.
    pub fn with_keep_scores(keep_scores: bool) -> Self {
        Self {
            context: None,
            context_identity: 0,
            total_hits: 0,
            scores: None,
            keep_scores,
            matching_docs: Vec::new(),
            docs_builder: None,
            sampling: None,
            overrides: None,
        }
    }

    /// The `super(keepScores)` call of a subclass that overrides the methods
    /// listed by [`FacetsCollectorOverrides`].
    pub fn with_overrides(keep_scores: bool, overrides: Arc<dyn FacetsCollectorOverrides>) -> Self {
        let mut this = Self::with_keep_scores(keep_scores);
        this.overrides = Some(overrides);
        this
    }

    /// The subclass overrides, `None` for a plain `FacetsCollector`: Java's
    /// `instanceof` test on a subclass reads them through
    /// [`FacetsCollectorOverrides::as_any`].
    pub fn overrides(&self) -> Option<&Arc<dyn FacetsCollectorOverrides>> {
        self.overrides.as_ref()
    }

    /// True if scores were saved.
    ///
    /// Equivalent to `getKeepScores()`.
    pub fn get_keep_scores(&self) -> bool {
        self.keep_scores
    }

    /// Returns the documents matched by the query, one `MatchingDocs` per
    /// visited segment.
    ///
    /// Equivalent to `getMatchingDocs()`.
    /// A `RandomSamplingFacetsCollector` returns its sample once more docs
    /// than its sample size were collected (its override of this method).
    ///
    /// # Errors
    ///
    /// For a `RandomSamplingFacetsCollector`, Java's `RuntimeException`
    /// wrapping an `IOException` raised while sampling.
    pub fn get_matching_docs(&self) -> Result<&Vec<MatchingDocs>> {
        match &self.sampling {
            None => Ok(&self.matching_docs),
            Some(sampling) => sampling.get_matching_docs(&self.matching_docs),
        }
    }

    /// The docs collected, before any sampling: `super.getMatchingDocs()`.
    pub fn get_original_matching_docs(&self) -> &Vec<MatchingDocs> {
        &self.matching_docs
    }

    /// The mutable list `getMatchingDocs()` returns.
    pub fn get_matching_docs_mut(&mut self) -> &mut Vec<MatchingDocs> {
        &mut self.matching_docs
    }

    /// Equivalent to `doSetNextReader(LeafReaderContext)`: the virtual call,
    /// dispatched to the [`FacetsCollectorOverrides`] when present.
    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()> {
        match self.overrides.clone() {
            Some(overrides) => overrides.do_set_next_reader(self, context),
            None => self.super_do_set_next_reader(context),
        }
    }

    /// The base class's `doSetNextReader(LeafReaderContext)`, reached by an
    /// override as `super.doSetNextReader(context)`.
    pub fn super_do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()> {
        debug_assert!(self.docs_builder.is_none());
        self.docs_builder = Some(DocIdSetBuilder::new(context.leaf_reader().max_doc()));
        self.total_hits = 0;
        if self.keep_scores {
            self.scores = Some(vec![0.0; 64]); // some initial size
        }
        self.context = Some(retain_leaf_context(context));
        self.context_identity = context.id();
        Ok(())
    }
}

impl LeafCollector for FacetsCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.docs_builder
            .as_mut()
            .expect("INVARIANT: doSetNextReader ran")
            .grow(1)
            .add(doc);
        if self.keep_scores {
            let scores = self
                .scores
                .as_mut()
                .expect("INVARIANT: allocated with keepScores");
            if doc as usize >= scores.len() {
                scores.resize(ArrayUtil::oversize(doc as usize + 1, 4), 0.0);
            }
            scores[doc as usize] = scorer.score()?;
        }
        self.total_hits += 1;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        match self.overrides.clone() {
            Some(overrides) => overrides.finish(self),
            None => self.super_finish(),
        }
    }
}

impl FacetsCollector {
    /// The base class's `finish()`, reached by an override as
    /// `super.finish()`.
    pub fn super_finish(&mut self) -> Result<()> {
        let bits: SharedDocIdSet = match self.docs_builder.take() {
            Some(builder) => Arc::from(builder.build()?),
            None => Arc::new(rucene::search::doc_id_set::empty()),
        };
        let context = self
            .context
            .take()
            .expect("INVARIANT: finish follows doSetNextReader");
        let mut matching = MatchingDocs::new(context, bits, self.total_hits, self.scores.take());
        matching.context_identity = self.context_identity;
        self.matching_docs.push(matching);
        Ok(())
    }
}

impl Collector for FacetsCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        self.do_set_next_reader(context)?;
        Ok(Box::new(self))
    }

    fn score_mode(&self) -> ScoreMode {
        if self.keep_scores {
            ScoreMode::COMPLETE
        } else {
            ScoreMode::COMPLETE_NO_SCORES
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}
