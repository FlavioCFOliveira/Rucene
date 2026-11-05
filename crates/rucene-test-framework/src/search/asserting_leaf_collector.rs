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

//! Port of `org.apache.lucene.tests.search.AssertingLeafCollector`.

use rucene::error::Result;
use rucene::search::{
    CheckedIntConsumer, CollectionError, CollectionResult, DocIdSetIterator, DocIdStream,
    LeafCollector, Scorable, Scorer, TwoPhaseIterator, NO_MORE_DOCS,
};
use rucene::util::FixedBitSet;

use crate::search::asserting_collector::AssertingCollectorState;
use crate::search::asserting_scorable::AssertingScorable;
use crate::search::asserting_scorer::AssertingScorerState;
use crate::util::lucene_test_case::LuceneTestCase;

/// The global-order checks of the anonymous subclass
/// `AssertingCollector.getLeafCollector` returns: the leaf's doc base and the
/// collector's state.
pub(crate) struct GlobalOrder<'a> {
    pub(crate) doc_base: i32,
    pub(crate) state: &'a mut AssertingCollectorState,
}

/// Wraps a [`LeafCollector`] and checks that the documents it collects are in
/// order and within the scored window `[min, max)`, that `finish` is called
/// once, and that the batch collection calls keep their contracts.
///
/// Equivalent to the package-private
/// `org.apache.lucene.tests.search.AssertingLeafCollector`, a
/// `FilterLeafCollector`. The anonymous subclass that
/// `AssertingCollector.getLeafCollector` returns — global order across leaves,
/// and the end-of-leaf flag set by `finish` — is the same type built with
/// [`new_for_collector`](Self::new_for_collector).
///
/// `setScorer` wraps the scorable with [`AssertingScorable::wrap`]; when the
/// result is a `Scorer`, the wrapped collector receives Java's anonymous
/// `FilterScorer` over it, which fails a `score()` read during
/// `collect(DocIdStream)` or `collectRange`.
///
/// This port lends the scorable to every collection call instead of letting
/// the wrapped collector keep the one `setScorer` received (see
/// [`LeafCollector`]), so each call re-applies the same wrapping to the
/// scorable it receives, and the state machine of the `AssertingScorer` that
/// `setScorer` built is carried from call to call.
pub struct AssertingLeafCollector<'a> {
    /// `in`.
    inner: Box<dyn LeafCollector + 'a>,
    /// `min`.
    min: i32,
    /// `max`.
    max: i32,
    /// `lastCollected`.
    last_collected: i32,
    /// `finishCalled`.
    finish_called: bool,
    /// `docBuffer`.
    doc_buffer: Option<Vec<i32>>,
    /// The anonymous subclass of `AssertingCollector`, when this is one.
    global: Option<GlobalOrder<'a>>,
    /// `batchCollectionDepth`.
    batch_collection_depth: i32,
    /// The state of the `AssertingScorer` that `setScorer` built, when it
    /// built one.
    scorer_state: Option<AssertingScorerState>,
}

impl<'a> AssertingLeafCollector<'a> {
    /// `new AssertingLeafCollector(collector, min, max)`.
    pub fn new(collector: Box<dyn LeafCollector + 'a>, min: i32, max: i32) -> Self {
        AssertingLeafCollector {
            inner: collector,
            min,
            max,
            last_collected: -1,
            finish_called: false,
            doc_buffer: None,
            global: None,
            batch_collection_depth: 0,
            scorer_state: None,
        }
    }

    /// The anonymous subclass `AssertingCollector.getLeafCollector` builds,
    /// over `[0, NO_MORE_DOCS)`.
    pub(crate) fn new_for_collector(
        collector: Box<dyn LeafCollector + 'a>,
        global: GlobalOrder<'a>,
    ) -> Self {
        let mut leaf = Self::new(collector, 0, NO_MORE_DOCS);
        leaf.global = Some(global);
        leaf
    }

    /// `AssertingLeafCollector.collect(int)`, the inherited body.
    fn super_collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        java_assert!(
            doc > self.last_collected,
            "Out of order : {} {}",
            self.last_collected,
            doc
        );
        java_assert!(doc >= self.min, "Out of range: {} < {}", doc, self.min);
        java_assert!(doc < self.max, "Out of range: {} >= {}", doc, self.max);
        let depth = self.batch_collection_depth;
        let inner = &mut self.inner;
        with_wrapped(&mut self.scorer_state, scorer, depth, |s| {
            inner.collect(doc, s)
        })?;
        self.last_collected = doc;
        Ok(())
    }
}

impl LeafCollector for AssertingLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.scorer_state = None;
        let depth = self.batch_collection_depth;
        let inner = &mut self.inner;
        with_wrapped(&mut self.scorer_state, scorer, depth, |s| {
            inner.set_scorer(s)
        })
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        match self.global.as_ref() {
            None => self.super_collect(doc, scorer),
            Some(global) => {
                let doc_base = global.doc_base;
                let max_doc = global.state.max_doc;
                // check that documents are scored in order globally, not only
                // per segment
                java_assert!(
                    doc_base + doc >= max_doc,
                    "collection is not in order: current doc={} while {} has already been collected",
                    doc_base + doc,
                    max_doc
                );
                self.super_collect(doc, scorer)?;
                if let Some(global) = self.global.as_mut() {
                    global.state.max_doc = doc_base + doc;
                }
                Ok(())
            }
        }
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        if LuceneTestCase::rarely() {
            let mut doc_buffer = self.doc_buffer.take().unwrap_or_else(|| vec![0; 32]);
            let result = (|| {
                let mut count = stream.into_array(&mut doc_buffer);
                while count != 0 {
                    for &doc in &doc_buffer[..count] {
                        self.collect(doc, scorer)?;
                    }
                    count = stream.into_array(&mut doc_buffer);
                }
                Ok(())
            })();
            self.doc_buffer = Some(doc_buffer);
            result
        } else {
            self.batch_collection_depth += 1;
            let mut asserting = AssertingDocIdStream {
                stream,
                last_up_to: -1,
                last_collected: &mut self.last_collected,
                min: self.min,
                max: self.max,
            };
            let depth = self.batch_collection_depth;
            let inner = &mut self.inner;
            let result = with_wrapped(&mut self.scorer_state, scorer, depth, |s| {
                inner.collect_stream(&mut asserting, s)
            });
            self.batch_collection_depth -= 1;
            result
        }
    }

    fn collect_range(
        &mut self,
        min: i32,
        max: i32,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        java_assert!(min > self.last_collected);
        java_assert!(max > min);
        java_assert!(min >= self.min, "Out of range: {} < {}", min, self.min);
        java_assert!(max <= self.max, "Out of range: {} >= {}", max - 1, self.max);
        self.batch_collection_depth += 1;
        let depth = self.batch_collection_depth;
        let inner = &mut self.inner;
        let result = with_wrapped(&mut self.scorer_state, scorer, depth, |s| {
            inner.collect_range(min, max, s)
        });
        self.batch_collection_depth -= 1;
        result?;
        self.last_collected = max - 1;
        Ok(())
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        Ok(self
            .inner
            .competitive_iterator()?
            .map(|inner| -> Box<dyn DocIdSetIterator> {
                Box::new(AssertingCompetitiveIterator {
                    inner,
                    max: self.max,
                })
            }))
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(global) = self.global.as_mut() {
            global.state.has_finished_collecting_previous_leaf = true;
        }
        java_assert!(!self.finish_called);
        self.finish_called = true;
        self.inner.finish()
    }
}

/// Applies `setScorer`'s wrapping to the scorable lent to one call and hands
/// the result to `call`: [`AssertingScorable::wrap`], then, when the wrapped
/// scorable is a `Scorer`, the batch-checking [`BatchCheckingScorer`].
fn with_wrapped<R>(
    scorer_state: &mut Option<AssertingScorerState>,
    scorer: &mut dyn Scorable,
    batch_collection_depth: i32,
    call: impl FnOnce(&mut dyn Scorable) -> R,
) -> R {
    let mut wrapped = AssertingScorable::wrap_with_state(scorer, *scorer_state);
    let result = if wrapped.as_scorer().is_some() {
        let wrapped_scorer = wrapped
            .as_scorer()
            .expect("INVARIANT: as_scorer answered Some on the line above");
        call(&mut BatchCheckingScorer {
            inner: wrapped_scorer,
            batch_collection_depth,
        })
    } else {
        // Some bulk scorers install synthetic Scorables that are deliberately safe to score during
        // batching, so only iterator-backed Scorers get the batch-position assertion above.
        call(wrapped.as_scorable())
    };
    if let Some(state) = wrapped.scorer_state() {
        *scorer_state = Some(state);
    }
    result
}

/// The anonymous `FilterScorer` of `setScorer`, over a wrapped scorable that
/// is a `Scorer`: `score()` fails during a batch collection.
struct BatchCheckingScorer<'s> {
    inner: &'s mut dyn Scorer,
    /// The enclosing collector's `batchCollectionDepth` during this call.
    batch_collection_depth: i32,
}

impl Scorable for BatchCheckingScorer<'_> {
    fn score(&mut self) -> Result<f32> {
        java_assert!(
            self.batch_collection_depth == 0,
            "Scorable.score() must not be called inside collect(DocIdStream) or \
             collectRange() since the scorer is not positioned on individual \
             documents during batch collection"
        );
        self.inner.score()
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.inner.set_min_competitive_score(min_score)
    }

    /// A `FilterScorer` is a `Scorer`.
    fn as_scorer(&mut self) -> Option<&mut dyn Scorer> {
        Some(self)
    }
}

impl Scorer for BatchCheckingScorer<'_> {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    /// `FilterScorer.docID()`.
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    /// `FilterScorer.iterator()`.
    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.inner.iterator()
    }

    /// `FilterScorer.twoPhaseIterator()`.
    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.inner.two_phase_iterator()
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.inner.advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.inner.get_max_score(up_to)
    }
}

/// The anonymous `FilterDocIdSetIterator` of `competitiveIterator()`.
struct AssertingCompetitiveIterator {
    inner: Box<dyn DocIdSetIterator>,
    max: i32,
}

impl DocIdSetIterator for AssertingCompetitiveIterator {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        java_assert!(
            self.inner.doc_id() < self.max,
            "advancing beyond the end of the scored window: docID={}, max={}",
            self.inner.doc_id(),
            self.max
        );
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        java_assert!(
            target <= self.max,
            "advancing beyond the end of the scored window: target={}, max={}",
            target,
            self.max
        );
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        java_assert!(
            up_to <= self.max,
            "advancing beyond the end of the scored window: upTo={}, max={}",
            up_to,
            self.max
        );
        self.inner.into_bit_set(up_to, bit_set, offset)?;
        java_assert!(self.inner.doc_id() >= up_to);
        Ok(())
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        java_assert!(self.doc_id() != -1);
        java_assert!(self.doc_id() != NO_MORE_DOCS);
        let next_non_matching_doc_id = self.inner.doc_id_run_end()?;
        java_assert!(
            next_non_matching_doc_id > self.doc_id(),
            "docIDRunEnd={}, docID={}",
            next_non_matching_doc_id,
            self.doc_id()
        );
        Ok(next_non_matching_doc_id)
    }
}

/// `AssertingLeafCollector.AssertingDocIdStream`.
struct AssertingDocIdStream<'s> {
    stream: &'s mut dyn DocIdStream,
    /// `lastUpTo`.
    last_up_to: i32,
    /// The enclosing collector's `lastCollected`.
    last_collected: &'s mut i32,
    min: i32,
    max: i32,
}

impl AssertingDocIdStream<'_> {
    /// The checking consumer both `forEach` overloads pass to the stream.
    fn checked_for_each(
        &mut self,
        consumer: &mut dyn CheckedIntConsumer<CollectionError>,
    ) -> CollectionResult<()> {
        let last_collected = &mut *self.last_collected;
        let (min, max) = (self.min, self.max);
        let mut checking = |doc: i32| -> CollectionResult<()> {
            java_assert!(
                doc > *last_collected,
                "Out of order : {} {}",
                *last_collected,
                doc
            );
            java_assert!(doc >= min, "Out of range: {} < {}", doc, min);
            java_assert!(doc < max, "Out of range: {} >= {}", doc, max);
            consumer.accept(doc)?;
            *last_collected = doc;
            Ok(())
        };
        // Java's `forEach(upTo, consumer)` also calls the stream's
        // unbounded `forEach` (`AssertingLeafCollector.java`, `forEach(int,
        // CheckedIntConsumer)`); reproduced.
        self.stream.for_each(&mut checking)
    }
}

impl DocIdStream for AssertingDocIdStream<'_> {
    fn for_each(
        &mut self,
        consumer: &mut dyn CheckedIntConsumer<CollectionError>,
    ) -> CollectionResult<()> {
        java_assert!(self.last_up_to != NO_MORE_DOCS, "exhausted");
        self.checked_for_each(consumer)?;
        self.last_up_to = NO_MORE_DOCS;
        java_assert!(!self.stream.may_have_remaining());
        Ok(())
    }

    fn for_each_up_to(
        &mut self,
        up_to: i32,
        consumer: &mut dyn CheckedIntConsumer<CollectionError>,
    ) -> CollectionResult<()> {
        java_assert!(
            self.last_up_to < up_to,
            "upTo={} but previous upTo={}",
            up_to,
            self.last_up_to
        );
        self.checked_for_each(consumer)?;
        self.last_up_to = up_to;
        if up_to == NO_MORE_DOCS {
            java_assert!(!self.stream.may_have_remaining());
        }
        Ok(())
    }

    fn count(&mut self) -> Result<i32> {
        java_assert!(self.last_up_to != NO_MORE_DOCS, "exhausted");
        let count = self.stream.count()?;
        self.last_up_to = NO_MORE_DOCS;
        java_assert!(!self.stream.may_have_remaining());
        Ok(count)
    }

    fn count_up_to(&mut self, up_to: i32) -> Result<i32> {
        java_assert!(
            self.last_up_to < up_to,
            "upTo={} but previous upTo={}",
            up_to,
            self.last_up_to
        );
        let count = self.stream.count_up_to(up_to)?;
        self.last_up_to = up_to;
        if up_to == NO_MORE_DOCS {
            java_assert!(!self.stream.may_have_remaining());
        }
        Ok(count)
    }

    fn into_array(&mut self, array: &mut [i32]) -> usize {
        java_assert_unchecked!(!array.is_empty());
        let count = self.stream.into_array(array);
        java_assert_unchecked!(self.last_up_to != NO_MORE_DOCS || count == 0);
        if count < array.len() {
            self.last_up_to = NO_MORE_DOCS;
            java_assert_unchecked!(!self.stream.may_have_remaining());
        } else {
            self.last_up_to = array[array.len() - 1] + 1;
        }
        count
    }

    fn into_array_up_to(&mut self, up_to: i32, array: &mut [i32]) -> usize {
        java_assert_unchecked!(!array.is_empty());
        java_assert_unchecked!(
            self.last_up_to <= up_to,
            "upTo={} but previous upTo={}",
            up_to,
            self.last_up_to
        );
        let count = self.stream.into_array_up_to(up_to, array);
        java_assert_unchecked!(self.last_up_to != up_to || count == 0);
        if count != 0 {
            java_assert_unchecked!(array[count - 1] < up_to);
        }
        if count < array.len() {
            self.last_up_to = up_to;
        } else {
            self.last_up_to = array[array.len() - 1] + 1;
        }
        if up_to == NO_MORE_DOCS && count < array.len() {
            java_assert_unchecked!(!self.stream.may_have_remaining());
        }
        count
    }

    fn may_have_remaining(&self) -> bool {
        let may_have_remaining = self.stream.may_have_remaining();
        if self.last_up_to == NO_MORE_DOCS {
            java_assert_unchecked!(!may_have_remaining);
        }
        may_have_remaining
    }
}
