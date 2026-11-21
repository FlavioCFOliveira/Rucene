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

//! Port of `org.apache.lucene.tests.search.AssertingCollector`.

use std::sync::Arc;

use rucene::index::LeafReaderContext;
use rucene::search::{CollectionResult, Collector, LeafCollector, ScoreMode, Weight};

use crate::search::asserting_leaf_collector::{AssertingLeafCollector, GlobalOrder};

/// The mutable fields of [`AssertingCollector`], which its leaf collectors
/// update.
#[derive(Debug, Clone)]
pub struct AssertingCollectorState {
    /// `weightSet`.
    pub(crate) weight_set: bool,
    /// `maxDoc`: the last document collected, in the top-level doc ID space.
    pub(crate) max_doc: i32,
    /// `previousLeafMaxDoc`.
    pub(crate) previous_leaf_max_doc: i32,
    /// `hasFinishedCollectingPreviousLeaf`: public in Java for drill-sideways
    /// testing.
    pub has_finished_collecting_previous_leaf: bool,
}

impl Default for AssertingCollectorState {
    fn default() -> Self {
        AssertingCollectorState {
            weight_set: false,
            max_doc: -1,
            previous_leaf_max_doc: 0,
            has_finished_collecting_previous_leaf: true,
        }
    }
}

/// A collector that asserts that the weight is set once and first, that leaves
/// are visited in order and finished before the next one starts, and that
/// documents are collected in order globally.
///
/// Equivalent to `org.apache.lucene.tests.search.AssertingCollector`, a
/// `FilterCollector`.
///
/// **Divergence.** Java's `wrap` returns its argument when it already is an
/// `AssertingCollector`; a collector cannot be tested for its type in this
/// port, so [`wrap`](Self::wrap) always wraps. A second wrapper keeps its own
/// state and repeats the same checks, so a correct collection passes both.
#[derive(Debug)]
pub struct AssertingCollector<C: Collector> {
    /// `in`.
    pub inner: C,
    /// The collector's state.
    pub state: AssertingCollectorState,
}

impl<C: Collector> AssertingCollector<C> {
    /// `AssertingCollector.wrap(in)` (see the divergence).
    pub fn wrap(inner: C) -> AssertingCollector<C> {
        AssertingCollector {
            inner,
            state: AssertingCollectorState::default(),
        }
    }

    /// `hasFinishedCollectingPreviousLeaf`.
    pub fn has_finished_collecting_previous_leaf(&self) -> bool {
        self.state.has_finished_collecting_previous_leaf
    }
}

impl<C: Collector> Collector for AssertingCollector<C> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        java_assert!(self.state.weight_set, "Set the weight first");
        java_assert!(context.doc_base() >= self.state.previous_leaf_max_doc);
        self.state.previous_leaf_max_doc = context.doc_base() + context.leaf_reader().max_doc();
        java_assert!(self.state.has_finished_collecting_previous_leaf);
        let inner = self.inner.get_leaf_collector(context)?;
        self.state.has_finished_collecting_previous_leaf = false;
        let doc_base = context.doc_base();
        Ok(Box::new(AssertingLeafCollector::new_for_collector(
            inner,
            GlobalOrder {
                doc_base,
                state: &mut self.state,
            },
        )))
    }

    fn score_mode(&self) -> ScoreMode {
        self.inner.score_mode()
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        java_assert_unchecked!(!self.state.weight_set, "Weight set twice");
        self.state.weight_set = true;
        self.inner.set_weight(weight);
    }
}

/// A [`Collector`] over a borrowed `dyn Collector`, so that
/// [`AssertingCollector`] can wrap the collector `IndexSearcher.search`
/// receives by reference.
pub struct CollectorRef<'c>(pub &'c mut dyn Collector);

impl Collector for CollectorRef<'_> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        self.0.get_leaf_collector(context)
    }

    fn score_mode(&self) -> ScoreMode {
        self.0.score_mode()
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        self.0.set_weight(weight);
    }
}
