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

//! Port of `DrillSidewaysScorer.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::{
    BulkScorer, CollectionResult, Collector, DocIdSetIterator, LeafCollector, Scorable, Scorer,
    ScoreCachingWrappingScorer, NO_MORE_DOCS,
};
use rucene::util::{BitSet, Bits, FixedBitSet};

/// A sideways collector of one dimension for one leaf, shared with the
/// `DrillSidewaysQuery` that keeps it for the reduction.
pub(crate) type CollectorSlot<K> = Arc<Mutex<K>>;

const CHUNK: i32 = 2048;
const MASK: i32 = CHUNK - 1;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Equivalent to `org.apache.lucene.facet.DrillSidewaysScorer.DocsAndCost`.
///
/// Java keeps the dimension's leaf collector, obtained when the scorer
/// supplier is built; a Rucene leaf collector borrows its collector, so the
/// dimension keeps the collector and its leaf collector is obtained when
/// scoring starts.
pub(crate) struct DocsAndCost<K> {
    scorer: Box<dyn Scorer>,
    has_two_phase: bool,
    /// Equivalent to `cost`.
    pub(crate) cost: i64,
    /// Equivalent to `matchCost`.
    match_cost: f32,
    collector: CollectorSlot<K>,
}

impl<K> DocsAndCost<K> {
    /// Equivalent to `DocsAndCost(Scorer, LeafCollector)`.
    pub(crate) fn new(mut scorer: Box<dyn Scorer>, collector: CollectorSlot<K>) -> Self {
        let (has_two_phase, match_cost) = match scorer.two_phase_iterator() {
            None => (false, 0f32),
            Some(two_phase) => (true, two_phase.match_cost()),
        };
        let mut this = Self {
            scorer,
            has_two_phase,
            cost: 0,
            match_cost,
            collector,
        };
        this.cost = this.approximation().cost();
        this
    }

    /// Equivalent to `approximation`.
    pub(crate) fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.has_two_phase {
            self.scorer
                .two_phase_iterator()
                .expect("INVARIANT: two-phase")
                .approximation()
        } else {
            self.scorer.iterator()
        }
    }

    fn approximation_doc_id(&mut self) -> i32 {
        self.approximation().doc_id()
    }

    /// `twoPhase.matches()`; only called when there is a two-phase view.
    fn two_phase_matches(&mut self) -> Result<bool> {
        self.scorer
            .two_phase_iterator()
            .expect("INVARIANT: two-phase")
            .matches()
    }

    /// `dc.twoPhase == null || dc.twoPhase.matches()`.
    fn matches_or_no_two_phase(&mut self) -> Result<bool> {
        if self.has_two_phase {
            self.two_phase_matches()
        } else {
            Ok(true)
        }
    }

    /// The collector slot.
    pub(crate) fn collector(&self) -> &CollectorSlot<K> {
        &self.collector
    }
}

/// The `Score` scorable: the score of the doc being collected.
///
/// Equivalent to `org.apache.lucene.facet.DrillSidewaysScorer.Score`.
pub(crate) struct Score {
    collect_score: f32,
}

impl Scorable for Score {
    fn score(&mut self) -> Result<f32> {
        Ok(self.collect_score)
    }
}

/// Bulk scorer for `DrillSidewaysQuery`.
///
/// Equivalent to `org.apache.lucene.facet.DrillSidewaysScorer`.
pub(crate) struct DrillSidewaysScorer<K: Collector> {
    dims: Vec<DocsAndCost<K>>,
    base_scorer: Box<dyn Scorer>,
    base_has_two_phase: bool,
    context: Arc<LeafReaderContext>,
    score_sub_docs_at_once: bool,
    collect_doc_id: i32,
    collect_score: f32,
}

/// Advances `iterator` to `doc_id` when it is behind.
///
/// Equivalent to `advanceIfBehind(int, DocIdSetIterator)`.
fn advance_if_behind(doc_id: i32, iterator: &mut dyn DocIdSetIterator) -> Result<i32> {
    if iterator.doc_id() < doc_id {
        iterator.advance(doc_id)
    } else {
        Ok(iterator.doc_id())
    }
}

impl<K: Collector> DrillSidewaysScorer<K> {
    /// Equivalent to `DrillSidewaysScorer(LeafReaderContext, Scorer,
    /// DocsAndCost[], boolean)`.
    pub(crate) fn new(
        context: Arc<LeafReaderContext>,
        mut base_scorer: Box<dyn Scorer>,
        dims: Vec<DocsAndCost<K>>,
        score_sub_docs_at_once: bool,
    ) -> Self {
        let base_has_two_phase = base_scorer.two_phase_iterator().is_some();
        Self {
            dims,
            base_scorer,
            base_has_two_phase,
            context,
            score_sub_docs_at_once,
            collect_doc_id: -1,
            collect_score: 0.0,
        }
    }

    fn base_iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.base_scorer.iterator()
    }

    fn base_approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.base_has_two_phase {
            self.base_scorer
                .two_phase_iterator()
                .expect("INVARIANT: two-phase")
                .approximation()
        } else {
            self.base_scorer.iterator()
        }
    }

    fn base_two_phase_matches(&mut self) -> Result<bool> {
        self.base_scorer
            .two_phase_iterator()
            .expect("INVARIANT: two-phase")
            .matches()
    }

    /// The scoring loops, given the leaf collectors of the dimensions.
    fn score_with(
        &mut self,
        collector: &mut dyn LeafCollector,
        leaves: &mut [Box<dyn LeafCollector + '_>],
        accept_docs: Option<&dyn Bits>,
    ) -> CollectionResult<()> {
        let base_query_cost = self.base_iterator().cost();

        let num_dims = self.dims.len();

        let mut drill_down_cost = 0i64;
        for dim in &mut self.dims {
            drill_down_cost += dim.approximation().cost();
        }

        let mut drill_down_advanced_cost = 0i64;
        if num_dims > 1 {
            drill_down_advanced_cost = self.dims[1].approximation().cost();
        }

        // Position all scorers to their first matching doc:
        for dim in &mut self.dims {
            dim.approximation().next_doc()?;
        }

        if self.score_sub_docs_at_once || base_query_cost < drill_down_cost / 10 {
            self.base_approximation().next_doc()?;
            self.do_query_first_scoring(accept_docs, collector, leaves)?;
        } else if num_dims > 1 && drill_down_advanced_cost < base_query_cost / 10 {
            self.base_iterator().next_doc()?;
            self.do_drill_down_advance_scoring(accept_docs, collector, leaves)?;
        } else {
            self.base_iterator().next_doc()?;
            self.do_union_scoring(accept_docs, collector, leaves)?;
        }
        Ok(())
    }

    /// Query-first scoring specialized for the case where there is only one
    /// drill-sideways dimension.
    ///
    /// Equivalent to `doQueryFirstScoringSingleDim(Bits, LeafCollector,
    /// DocsAndCost)`.
    fn do_query_first_scoring_single_dim(
        &mut self,
        accept_docs: Option<&dyn Bits>,
        collector: &mut dyn LeafCollector,
        leaves: &mut [Box<dyn LeafCollector + '_>],
    ) -> CollectionResult<()> {
        let mut doc_id = self.base_approximation().doc_id();
        while doc_id != NO_MORE_DOCS {
            debug_assert!(doc_id == self.base_approximation().doc_id());
            if accept_docs.is_some_and(|a| !a.get(doc_id as usize)) {
                doc_id = self.base_approximation().next_doc()?;
                continue;
            }

            // If baseApproximation has a twoPhase iterator, we need to
            // confirm it matches before collecting
            if self.base_has_two_phase && !self.base_two_phase_matches()? {
                doc_id = self.base_approximation().next_doc()?;
                continue;
            }

            // We have either a near-miss or full match. Check the sideways dim
            // to see which it is:
            self.collect_doc_id = doc_id;
            let dim = &mut self.dims[0];
            if advance_if_behind(doc_id, dim.approximation())? != doc_id
                || (dim.has_two_phase && !dim.two_phase_matches()?)
            {
                // The sideways dim missed, so we have a "near miss":
                leaves[0].collect(self.collect_doc_id, &mut *self.base_scorer)?;
            } else {
                // Hit passed all filters, so it's "real":
                collector.collect(self.collect_doc_id, &mut *self.base_scorer)?;
                leaves[0].collect(self.collect_doc_id, &mut *self.base_scorer)?;
            }

            doc_id = self.base_approximation().next_doc()?;
        }
        Ok(())
    }

    /// Used when base query is highly constraining vs the drilldowns, or
    /// when the docs must be scored at once (i.e., like `BooleanScorer2`, not
    /// `BooleanScorer`).
    ///
    /// Equivalent to `doQueryFirstScoring(Bits, LeafCollector,
    /// DocsAndCost[])`: as Java (`DrillSidewaysScorer.java:199`), the main
    /// collector is wrapped by `ScoreCachingWrappingScorer.wrap`, so the
    /// base scorer is asked for each collected document's score at most once
    /// through it; the sideways collectors get the base scorer directly.
    fn do_query_first_scoring(
        &mut self,
        accept_docs: Option<&dyn Bits>,
        collector: &mut dyn LeafCollector,
        leaves: &mut [Box<dyn LeafCollector + '_>],
    ) -> CollectionResult<()> {
        let mut wrapped = ScoreCachingWrappingScorer::wrap(collector);
        let collector: &mut dyn LeafCollector = &mut wrapped;
        collector.set_scorer(&mut *self.base_scorer)?;
        for leaf in leaves.iter_mut() {
            leaf.set_scorer(&mut *self.base_scorer)?;
        }
        if self.dims.len() == 1 {
            return self.do_query_first_scoring_single_dim(accept_docs, collector, leaves);
        }

        // Sort our sideways dims by approximation cost so we can advance the
        // lower cost ones first:
        let mut sideways_dims: Vec<usize> = (0..self.dims.len()).collect();
        sideways_dims.sort_by_key(|&i| self.dims[i].cost); // stable, as timSort

        // Maintain a list of dims that have two-phase iterators so we can
        // confirm matches in cost order:
        let mut sideways_two_phase_dims: Option<Vec<usize>> = None;
        for (i, dim) in self.dims.iter().enumerate() {
            if dim.has_two_phase {
                sideways_two_phase_dims.get_or_insert_with(Vec::new).push(i);
            }
        }
        if let Some(two_phase_dims) = &mut sideways_two_phase_dims {
            // stable, as timSort; `Float.compare` of the match costs
            two_phase_dims.sort_by(|&a, &b| {
                crate::taxonomy::taxonomy_facets::java_float_compare(
                    self.dims[a].match_cost,
                    self.dims[b].match_cost,
                )
            });
        }

        // We keep track of a "runaway" dimension, which is a previously
        // "near-miss" dim; a `PriorityQueue` of size 1 ordered by the
        // approximation's docID.
        let mut runaway_dim: Option<usize> = None;

        let mut doc_id = self.base_approximation().doc_id();

        'next_doc: while doc_id != NO_MORE_DOCS {
            debug_assert!(doc_id == self.base_approximation().doc_id());
            if accept_docs.is_some_and(|a| !a.get(doc_id as usize)) {
                doc_id = self.base_approximation().next_doc()?;
                continue;
            }

            // If the runaway dim has advanced past the current doc, it's no
            // longer a runaway:
            let mut runaway = runaway_dim;
            if let Some(r) = runaway {
                if self.dims[r].approximation_doc_id() <= doc_id {
                    runaway_dim = None;
                    runaway = None;
                }
            }

            // Check the sideways dims, advancing them as necessary:
            for &dim in &sideways_dims {
                let dim_doc_id = advance_if_behind(doc_id, self.dims[dim].approximation())?;
                if dim_doc_id != doc_id && Some(dim) != runaway {
                    // `runawayDim.insertWithOverflow(dim)`:
                    let evicted = match runaway_dim {
                        None => {
                            runaway_dim = Some(dim);
                            None
                        }
                        Some(top) => {
                            // `lessThan(a, b)`: `a.approximation.docID() <
                            // b.approximation.docID()`.
                            let dim_doc = self.dims[dim].approximation_doc_id();
                            let top_doc = self.dims[top].approximation_doc_id();
                            if !(dim_doc < top_doc) {
                                runaway_dim = Some(dim);
                                Some(top)
                            } else {
                                Some(dim)
                            }
                        }
                    };
                    if let Some(evicted) = evicted {
                        // We have two "near-misses", so this doc is not a
                        // match; advance the base to the evicted dim's doc:
                        let next = self.dims[evicted].approximation_doc_id();
                        doc_id = self.base_approximation().advance(next)?;
                        continue 'next_doc;
                    }
                }
            }

            // If the base query has a two-phase iterator, confirm it now:
            if self.base_has_two_phase && !self.base_two_phase_matches()? {
                doc_id = self.base_approximation().next_doc()?;
                continue;
            }

            // If we have two-phase iterators for our sideways dims, check them
            // now:
            let mut failed_dim = runaway_dim;
            if let Some(two_phase_dims) = &sideways_two_phase_dims {
                if failed_dim.is_none() {
                    for &dim in two_phase_dims {
                        debug_assert!(self.dims[dim].approximation_doc_id() == doc_id);
                        if !self.dims[dim].two_phase_matches()? {
                            if failed_dim.is_some() {
                                // Two failed dims means this isn't a hit:
                                doc_id = self.base_approximation().next_doc()?;
                                continue 'next_doc;
                            } else {
                                failed_dim = Some(dim);
                            }
                        }
                    }
                } else {
                    for &dim in two_phase_dims {
                        if failed_dim == Some(dim) {
                            continue;
                        }
                        debug_assert!(self.dims[dim].approximation_doc_id() == doc_id);
                        if !self.dims[dim].two_phase_matches()? {
                            doc_id = self.base_approximation().next_doc()?;
                            continue 'next_doc;
                        }
                    }
                }
            }

            self.collect_doc_id = doc_id;
            match failed_dim {
                None => {
                    // Hit passed all filters, so it's "real":
                    collector.collect(self.collect_doc_id, &mut *self.base_scorer)?;
                    for &dim in &sideways_dims {
                        leaves[dim].collect(self.collect_doc_id, &mut *self.base_scorer)?;
                    }
                }
                Some(failed) => {
                    // Hit missed exactly one dim:
                    leaves[failed].collect(self.collect_doc_id, &mut *self.base_scorer)?;
                }
            }

            doc_id = self.base_approximation().next_doc()?;
        }
        Ok(())
    }

    /// Used when drill downs are highly constraining vs baseQuery.
    ///
    /// Equivalent to `doDrillDownAdvanceScoring(Bits, LeafCollector,
    /// DocsAndCost[])`.
    fn do_drill_down_advance_scoring(
        &mut self,
        accept_docs: Option<&dyn Bits>,
        collector: &mut dyn LeafCollector,
        leaves: &mut [Box<dyn LeafCollector + '_>],
    ) -> CollectionResult<()> {
        let mut score = Score { collect_score: 0.0 };
        collector.set_scorer(&mut score)?;
        for leaf in leaves.iter_mut() {
            leaf.set_scorer(&mut score)?;
        }

        let max_doc = self.context.leaf_reader().max_doc();
        let num_dims = self.dims.len() as i32;

        let mut filled_slots = vec![0i32; CHUNK as usize];
        let mut doc_ids = vec![0i32; CHUNK as usize];
        let mut scores = vec![0f32; CHUNK as usize];
        let mut missing_dims = vec![0i32; CHUNK as usize];
        let mut counts = vec![0i32; CHUNK as usize];

        doc_ids[0] = -1;
        let mut next_chunk_start = CHUNK;

        let mut seen = FixedBitSet::new(CHUNK as usize);

        loop {
            // First dim:
            {
                let dc = &mut self.dims[0];
                let mut doc_id = dc.approximation_doc_id();
                while doc_id < next_chunk_start {
                    if accept_docs.is_none_or(|a| a.get(doc_id as usize)) {
                        let slot = (doc_id & MASK) as usize;
                        if doc_ids[slot] != doc_id && dc.matches_or_no_two_phase()? {
                            seen.set(slot);
                            // Mark slot as valid:
                            doc_ids[slot] = doc_id;
                            missing_dims[slot] = 1;
                            counts[slot] = 1;
                        }
                    }
                    doc_id = dc.approximation().next_doc()?;
                }
            }

            // Second dim:
            {
                let dc = &mut self.dims[1];
                let mut doc_id = dc.approximation_doc_id();
                while doc_id < next_chunk_start {
                    // Java: `acceptDocs == null || acceptDocs.get(docID) &&
                    // (dc.twoPhase == null || dc.twoPhase.matches())`.
                    let accepted = match accept_docs {
                        None => true,
                        Some(a) => a.get(doc_id as usize) && dc.matches_or_no_two_phase()?,
                    };
                    if accepted {
                        let slot = (doc_id & MASK) as usize;
                        if doc_ids[slot] != doc_id {
                            // Mark slot as valid:
                            seen.set(slot);
                            doc_ids[slot] = doc_id;
                            missing_dims[slot] = 0;
                            counts[slot] = 1;
                        } else {
                            // TODO: single-valued dims will always be true
                            // below; we could somehow specialize
                            if missing_dims[slot] >= 1 {
                                missing_dims[slot] = 2;
                                counts[slot] = 2;
                            } else {
                                counts[slot] = 1;
                            }
                        }
                    }
                    doc_id = dc.approximation().next_doc()?;
                }
            }

            // After this we can "upgrade" to conjunction, because any doc not
            // seen by either dim 0 or dim 1 cannot be a hit or a near miss:

            // Now we score the base query against the candidates:
            let mut filled_count = 0usize;
            let mut slot0 = 0i32;
            while slot0 < CHUNK {
                slot0 = seen.next_set_bit(slot0);
                if slot0 == NO_MORE_DOCS {
                    break;
                }
                let dd_doc_id = doc_ids[slot0 as usize];
                debug_assert!(dd_doc_id != -1);

                let mut base_doc_id = self.base_iterator().doc_id();
                if base_doc_id < dd_doc_id {
                    base_doc_id = self.base_iterator().advance(dd_doc_id)?;
                }
                if base_doc_id == dd_doc_id {
                    scores[slot0 as usize] = self.base_scorer.score()?;
                    filled_slots[filled_count] = slot0;
                    filled_count += 1;
                    counts[slot0 as usize] += 1;
                } else {
                    doc_ids[slot0 as usize] = -1;

                    // TODO: we could jump slot0 forward to the base doc's
                    // slot ... but we'd need to first advance base
                }
                slot0 += 1;
            }
            seen.clear_range(0, CHUNK as usize);

            if filled_count == 0 {
                if next_chunk_start >= max_doc {
                    break;
                }
                next_chunk_start += CHUNK;
                continue;
            }

            // TODO: factor this out & share w/ union scorer, except we start
            // from dim=2 instead:
            for dim in 2..num_dims {
                let dc = &mut self.dims[dim as usize];
                let mut doc_id = dc.approximation_doc_id();
                while doc_id < next_chunk_start {
                    let slot = (doc_id & MASK) as usize;
                    if doc_ids[slot] == doc_id && counts[slot] >= dim && dc.matches_or_no_two_phase()? {
                        // TODO: single-valued dims will always be true below;
                        // we could somehow specialize
                        if missing_dims[slot] >= dim {
                            // Count the doc as a near miss only if it's
                            // missing the previous dim(s):
                            missing_dims[slot] = dim + 1;
                            counts[slot] = dim + 2;
                        } else {
                            counts[slot] = dim + 1;
                        }
                    }
                    doc_id = dc.approximation().next_doc()?;
                }
            }

            // Collect:
            for &slot in &filled_slots[..filled_count] {
                let slot = slot as usize;
                self.collect_doc_id = doc_ids[slot];
                self.collect_score = scores[slot];
                score.collect_score = self.collect_score;
                if counts[slot] == 1 + num_dims {
                    collector.collect(self.collect_doc_id, &mut score)?;
                    for leaf in leaves.iter_mut() {
                        leaf.collect(self.collect_doc_id, &mut score)?;
                    }
                } else if counts[slot] == num_dims {
                    leaves[missing_dims[slot] as usize].collect(self.collect_doc_id, &mut score)?;
                }
            }

            if next_chunk_start >= max_doc {
                break;
            }

            next_chunk_start += CHUNK;
        }
        Ok(())
    }

    /// Equivalent to `doUnionScoring(Bits, LeafCollector, DocsAndCost[])`.
    fn do_union_scoring(
        &mut self,
        accept_docs: Option<&dyn Bits>,
        collector: &mut dyn LeafCollector,
        leaves: &mut [Box<dyn LeafCollector + '_>],
    ) -> CollectionResult<()> {
        let mut score = Score { collect_score: 0.0 };
        collector.set_scorer(&mut score)?;
        for leaf in leaves.iter_mut() {
            leaf.set_scorer(&mut score)?;
        }

        let max_doc = self.context.leaf_reader().max_doc();
        let num_dims = self.dims.len() as i32;

        // TODO: maybe a class like BS, instead of parallel arrays
        let mut filled_slots = vec![0i32; CHUNK as usize];
        let mut doc_ids = vec![0i32; CHUNK as usize];
        let mut scores = vec![0f32; CHUNK as usize];
        let mut missing_dims = vec![0i32; CHUNK as usize];
        let mut counts = vec![0i32; CHUNK as usize];

        doc_ids[0] = -1;

        // NOTE: this is basically a specialized version of BooleanScorer, to
        // the minShouldMatch=N-1 case, but we also track which clause
        // (dimension) hasn't matched for every doc, so we can "collect
        // sideways" for that dim (i.e. treat it as a drill-down).
        let mut next_chunk_start = CHUNK;

        loop {
            // First, fill the doc ids and scores, using the base iterator:
            let mut filled_count = 0usize;
            let mut doc_id = self.base_iterator().doc_id();
            while doc_id < next_chunk_start {
                if accept_docs.is_none_or(|a| a.get(doc_id as usize)) {
                    let slot = (doc_id & MASK) as usize;

                    // Mark slot as valid:
                    debug_assert!(doc_ids[slot] != doc_id, "slot={slot} docID={doc_id}");
                    doc_ids[slot] = doc_id;
                    scores[slot] = self.base_scorer.score()?;
                    filled_slots[filled_count] = slot as i32;
                    filled_count += 1;
                    missing_dims[slot] = 0;
                    counts[slot] = 1;
                }
                doc_id = self.base_iterator().next_doc()?;
            }

            if filled_count == 0 {
                if next_chunk_start >= max_doc {
                    break;
                }
                next_chunk_start += CHUNK;
                continue;
            }

            // Then drill down dims:

            // First drill-down dim, basically adds SHOULD onto the baseQuery:
            {
                let dc = &mut self.dims[0];
                let mut doc_id = dc.approximation_doc_id();
                while doc_id < next_chunk_start {
                    let slot = (doc_id & MASK) as usize;
                    // this also checks that the doc is not deleted
                    if doc_ids[slot] == doc_id && dc.matches_or_no_two_phase()? {
                        missing_dims[slot] = 1;
                        counts[slot] = 2;
                    }
                    doc_id = dc.approximation().next_doc()?;
                }
            }

            for dim in 1..num_dims {
                let dc = &mut self.dims[dim as usize];
                let mut doc_id = dc.approximation_doc_id();
                while doc_id < next_chunk_start {
                    let slot = (doc_id & MASK) as usize;

                    // also means that the doc is not deleted
                    if doc_ids[slot] == doc_id && counts[slot] >= dim && dc.matches_or_no_two_phase()? {
                        // This doc is still in the running...
                        // TODO: single-valued dims will always be true below;
                        // we could somehow specialize
                        if missing_dims[slot] >= dim {
                            missing_dims[slot] = dim + 1;
                            counts[slot] = dim + 2;
                        } else {
                            counts[slot] = dim + 1;
                        }
                    }
                    doc_id = dc.approximation().next_doc()?;
                }
            }

            // Collect:
            for &slot in &filled_slots[..filled_count] {
                let slot = slot as usize;
                self.collect_doc_id = doc_ids[slot];
                self.collect_score = scores[slot];
                score.collect_score = self.collect_score;
                if counts[slot] == 1 + num_dims {
                    collector.collect(self.collect_doc_id, &mut score)?;
                    for leaf in leaves.iter_mut() {
                        leaf.collect(self.collect_doc_id, &mut score)?;
                    }
                } else if counts[slot] == num_dims {
                    leaves[missing_dims[slot] as usize].collect(self.collect_doc_id, &mut score)?;
                }
            }

            if next_chunk_start >= max_doc {
                break;
            }

            next_chunk_start += CHUNK;
        }
        Ok(())
    }
}

impl<K: Collector> BulkScorer for DrillSidewaysScorer<K> {
    fn cost(&self) -> i64 {
        self.base_scorer.iterator_ref().map_or(0, |it| it.cost())
    }

    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max_doc: i32,
    ) -> CollectionResult<i32> {
        if min != 0 {
            return Err(LuceneError::illegal_argument(format!("min must be 0, got {min}")).into());
        }
        if max_doc != i32::MAX {
            return Err(LuceneError::illegal_argument("maxDoc must be Integer.MAX_VALUE").into());
        }

        // The dimensions' leaf collectors (see `DocsAndCost`).
        let slots: Vec<CollectorSlot<K>> = self.dims.iter().map(|d| Arc::clone(&d.collector)).collect();
        let mut guards: Vec<MutexGuard<'_, K>> = slots.iter().map(|s| lock(s)).collect();
        let context = Arc::clone(&self.context);
        let mut leaves: Vec<Box<dyn LeafCollector + '_>> = Vec::with_capacity(guards.len());
        for guard in guards.iter_mut() {
            leaves.push(guard.get_leaf_collector(&context)?);
        }

        let result = self.score_with(collector, &mut leaves, accept_docs);
        // `finally { finish(dims); }`
        let mut finished: Result<()> = Ok(());
        for leaf in leaves.iter_mut() {
            if let Err(e) = leaf.finish() {
                if finished.is_ok() {
                    finished = Err(e);
                }
            }
        }
        result?;
        finished?;

        Ok(i32::MAX)
    }
}
