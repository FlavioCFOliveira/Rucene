//! MAXSCORE bulk scoring, ported from
//! `org.apache.lucene.search.MaxScoreBulkScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::DocAndFloatFeatureBuffer;
use crate::search::bit_set_util;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::disi_priority_queue::DisiPriorityQueue;
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::scorable::SimpleScorable;
use crate::search::scorer::Scorer;
use crate::search::scorer_util::{DocAndScoreAccBuffer, ScorerUtil};
use crate::util::{Bits, FixedBitSet, MathUtil};

/// The size of the inner windows matches are collected into.
///
/// Equivalent to the package-private `MaxScoreBulkScorer.INNER_WINDOW_SIZE`
/// constant.
pub const INNER_WINDOW_SIZE: i32 = 1 << 12;

/// A [`BulkScorer`] implementing the MAXSCORE algorithm over impact-based score
/// upper bounds.
///
/// Equivalent to the `final class
/// org.apache.lucene.search.MaxScoreBulkScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java's `allScorers` array holds
/// `DisiWrapper` references that the essential-clause heap also holds, and
/// `partitionScorers` permutes that array in place. Rust forbids the aliasing,
/// so the wrappers live in one array in construction order and both
/// `all_scorers` and the heap hold **positions** into it; the permutation and
/// the heap are unchanged.
pub struct MaxScoreBulkScorer {
    max_doc: i32,
    /// The clauses, in construction order.
    wrappers: Vec<DisiWrapper>,
    /// Positions of the clauses, sorted by increasing max score.
    all_scorers: Vec<usize>,
    scratch: Vec<usize>,
    /// The last clauses of `all_scorers` that are "essential", that is,
    /// required for a match to have a competitive score.
    essential_queue: DisiPriorityQueue,
    /// Index of the first essential scorer: `essential_queue` contains all the
    /// clauses from `all_scorers[first_essential_scorer..]`. All the clauses
    /// below this index are non-essential.
    first_essential_scorer: usize,
    /// Index of the first clause that is required: this clause and all the
    /// following ones are required for a document to match.
    first_required_scorer: usize,
    /// The minimum value of the minimum competitive score that would produce a
    /// more favorable partitioning.
    next_min_competitive_score: f32,
    cost: i64,
    scorable: SimpleScorable,
    max_score_sums: Vec<f64>,
    filter: Option<DisiWrapper>,

    window_matches: FixedBitSet,
    window_scores: Vec<f64>,
    filter_matches: Option<FixedBitSet>,

    doc_and_score_buffer: DocAndFloatFeatureBuffer,
    doc_and_score_acc_buffer: DocAndScoreAccBuffer,

    /// Number of outer windows that have been evaluated.
    num_outer_windows: i32,
    /// Number of candidate matches so far.
    num_candidates: i32,
    /// Minimum window size; see [`compute_outer_window_max`](Self::compute_outer_window_max),
    /// which adjusts it based on the average number of candidate matches per
    /// outer window, to keep the per-window overhead under control.
    min_window_size: i32,
}

impl std::fmt::Debug for MaxScoreBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaxScoreBulkScorer")
            .field("clauses", &self.wrappers.len())
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl MaxScoreBulkScorer {
    /// Builds a MAXSCORE bulk scorer.
    ///
    /// Equivalent to `MaxScoreBulkScorer(int, List<Scorer>, Scorer)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the clause costs.
    pub fn new(
        max_doc: i32,
        scorers: Vec<Box<dyn Scorer>>,
        filter: Option<Box<dyn Scorer>>,
    ) -> Result<Self> {
        let filter = filter.map(|scorer| DisiWrapper::new(scorer, false));
        let num_scorers = scorers.len();
        let mut wrappers = Vec::with_capacity(num_scorers);
        let mut cost: i64 = 0;
        for scorer in scorers {
            let w = DisiWrapper::new(scorer, true);
            cost = cost.wrapping_add(w.cost);
            wrappers.push(w);
        }
        let all_scorers: Vec<usize> = (0..num_scorers).collect();
        let scratch = vec![0usize; num_scorers];
        let essential_queue = DisiPriorityQueue::of_max_size(num_scorers);
        let max_score_sums = vec![0.0f64; num_scorers];
        let mut doc_and_score_acc_buffer = DocAndScoreAccBuffer::new();
        doc_and_score_acc_buffer.grow_no_copy(INNER_WINDOW_SIZE as usize);

        let mut filter_matches = None;
        if let Some(filter) = filter.as_ref() {
            if !filter.has_two_phase() && max_doc >= INNER_WINDOW_SIZE {
                let mut min_scorer_cost = wrappers[0].cost;
                for w in wrappers.iter().skip(1) {
                    min_scorer_cost = min_scorer_cost.min(w.cost);
                }
                // Use the bitset filter path if either:
                //  - the sparsest disjunction scorer is denser than the filter,
                //    OR
                //  - there are many scorers and their combined cost is denser
                //    than the filter, so the candidate stream is dense enough to
                //    favor bulk bit-set gating over per-candidate filter
                //    advance()
                if min_scorer_cost >= filter.cost || (num_scorers > 4 && cost >= filter.cost) {
                    filter_matches = Some(FixedBitSet::new(INNER_WINDOW_SIZE as usize));
                }
            }
        }

        Ok(Self {
            max_doc,
            wrappers,
            all_scorers,
            scratch,
            essential_queue,
            first_essential_scorer: 0,
            first_required_scorer: 0,
            next_min_competitive_score: 0.0,
            cost,
            scorable: SimpleScorable::new(),
            max_score_sums,
            filter,
            window_matches: FixedBitSet::new(INNER_WINDOW_SIZE as usize),
            window_scores: vec![0.0; INNER_WINDOW_SIZE as usize],
            filter_matches,
            doc_and_score_buffer: DocAndFloatFeatureBuffer::new(),
            doc_and_score_acc_buffer,
            num_outer_windows: 0,
            num_candidates: 0,
            min_window_size: 1,
        })
    }

    /// Equivalent to the private
    /// `MaxScoreBulkScorer.scoreInnerWindow(LeafCollector, Bits, int,
    /// DisiWrapper)`.
    fn score_inner_window(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        max: i32,
    ) -> CollectionResult<()> {
        if self.filter.is_some() {
            return self.score_inner_window_with_filter(collector, accept_docs, max);
        }
        let top = self
            .essential_queue
            .top()
            .expect("INVARIANT: the essential queue is non-empty while scoring");
        let top2 = self.essential_queue.top2(&self.wrappers);
        match top2 {
            None => self.score_inner_window_single_essential_clause(collector, accept_docs, max),
            Some(top2) => {
                let top2_doc = self.wrappers[top2].doc;
                if top2_doc - INNER_WINDOW_SIZE / 2 >= self.wrappers[top].doc {
                    // The first half of the window would match a single clause.
                    // Let's collect this single clause until the next doc ID of
                    // the next clause.
                    self.score_inner_window_single_essential_clause(
                        collector,
                        accept_docs,
                        max.min(top2_doc),
                    )
                } else {
                    self.score_inner_window_multiple_essential_clauses(collector, accept_docs, max)
                }
            }
        }
    }

    /// Equivalent to the private
    /// `MaxScoreBulkScorer.scoreInnerWindowWithFilter(LeafCollector, Bits, int,
    /// DisiWrapper)`.
    fn score_inner_window_with_filter(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        max: i32,
    ) -> CollectionResult<()> {
        let mut top = self
            .essential_queue
            .top()
            .expect("INVARIANT: the essential queue is non-empty while scoring");
        debug_assert!(self.wrappers[top].doc < max);
        let filter_doc = self
            .filter
            .as_ref()
            .expect("INVARIANT: this path only runs with a filter")
            .doc;
        while self.wrappers[top].doc < filter_doc {
            // Must use the iterator as `top` might be a two-phase iterator
            let next = self.wrappers[top].iterator().advance(filter_doc)?;
            self.wrappers[top].doc = next;
            top = self
                .essential_queue
                .update_top(&self.wrappers)
                .expect("INVARIANT: the essential queue is non-empty");
        }

        if self.wrappers[top].doc >= max {
            return Ok(());
        }

        // Only score an inner window, after that we'll check if the min
        // competitive score has increased enough for a more favorable
        // partitioning to be used.
        let inner_window_min = self.wrappers[top].doc;
        let inner_window_max =
            MathUtil::unsigned_min(max, inner_window_min.wrapping_add(INNER_WINDOW_SIZE));

        self.doc_and_score_acc_buffer.size = 0;
        if self.filter_matches.is_none() {
            self.fill_score_buffer_via_leap_frog(top, accept_docs, inner_window_max)?;
        } else {
            self.fill_score_buffer_via_bit_set(top, accept_docs, inner_window_max)?;
        }

        let first_essential_scorer = self.first_essential_scorer;
        self.score_non_essential_clauses(collector, first_essential_scorer)
    }

    /// Equivalent to the private
    /// `MaxScoreBulkScorer.fillScoreBufferViaBitSet(DisiWrapper, Bits, int)`.
    fn fill_score_buffer_via_bit_set(
        &mut self,
        top: usize,
        accept_docs: Option<&dyn Bits>,
        inner_window_max: i32,
    ) -> Result<()> {
        let inner_window_min = self.wrappers[top].doc;
        {
            let filter_matches = self
                .filter_matches
                .as_mut()
                .expect("INVARIANT: this path only runs with a filter bit set");
            filter_matches.clear_all();
            let filter = self
                .filter
                .as_mut()
                .expect("INVARIANT: this path only runs with a filter");
            if filter.doc < inner_window_max {
                if filter.doc < inner_window_min {
                    filter.doc = filter.approximation().advance(inner_window_min)?;
                }
                if filter.doc < inner_window_max {
                    filter.approximation().into_bit_set(
                        inner_window_max,
                        filter_matches,
                        inner_window_min,
                    )?;
                    filter.doc = filter.approximation_doc_id();
                }
            }
            if let Some(accept_docs) = accept_docs {
                bit_set_util::apply_mask(accept_docs, filter_matches, inner_window_min);
            }
        }

        let inner_window_size = inner_window_max - inner_window_min;
        // Collect matches of essential clauses into a bitset, checking the
        // filter via a bitset lookup.
        self.collect_essential_scores_into_window(
            top,
            inner_window_max,
            inner_window_min,
            None,
            true,
        )?;
        self.flush_window_to_doc_and_score_acc_buffer(inner_window_min, inner_window_size);
        Ok(())
    }

    /// Equivalent to the private
    /// `MaxScoreBulkScorer.fillScoreBufferViaLeapFrog(DisiWrapper, Bits, int)`.
    fn fill_score_buffer_via_leap_frog(
        &mut self,
        mut top: usize,
        accept_docs: Option<&dyn Bits>,
        inner_window_max: i32,
    ) -> Result<()> {
        while self.wrappers[top].doc < inner_window_max {
            let filter_doc = self
                .filter
                .as_ref()
                .expect("INVARIANT: this path only runs with a filter")
                .doc;
            debug_assert!(filter_doc <= self.wrappers[top].doc); // invariant
            let filter_doc = if filter_doc < self.wrappers[top].doc {
                let target = self.wrappers[top].doc;
                let filter = self
                    .filter
                    .as_mut()
                    .expect("INVARIANT: this path only runs with a filter");
                filter.doc = filter.approximation().advance(target)?;
                filter.doc
            } else {
                filter_doc
            };

            if filter_doc != self.wrappers[top].doc {
                loop {
                    let next = self.wrappers[top].iterator().advance(filter_doc)?;
                    self.wrappers[top].doc = next;
                    top = self
                        .essential_queue
                        .update_top(&self.wrappers)
                        .expect("INVARIANT: the essential queue is non-empty");
                    if self.wrappers[top].doc >= filter_doc {
                        break;
                    }
                }
            } else {
                let doc = self.wrappers[top].doc;
                let accepted = accept_docs.map_or(true, |bits| bits.get(doc as usize));
                let confirmed = if accepted {
                    self.filter
                        .as_mut()
                        .expect("INVARIANT: this path only runs with a filter")
                        .matches()?
                } else {
                    false
                };
                let matched = accepted && confirmed;
                let mut score = 0.0f64;
                loop {
                    if matched {
                        score += f64::from(self.wrappers[top].scorer().score()?);
                    }
                    let next = self.wrappers[top].iterator().next_doc()?;
                    self.wrappers[top].doc = next;
                    top = self
                        .essential_queue
                        .update_top(&self.wrappers)
                        .expect("INVARIANT: the essential queue is non-empty");
                    if self.wrappers[top].doc != doc {
                        break;
                    }
                }

                if matched {
                    let size = self.doc_and_score_acc_buffer.size;
                    self.doc_and_score_acc_buffer.grow(size + 1);
                    self.doc_and_score_acc_buffer.docs[size] = doc;
                    self.doc_and_score_acc_buffer.scores[size] = score;
                    self.doc_and_score_acc_buffer.size += 1;
                }
            }
        }
        Ok(())
    }

    /// Collects matches of the essential clauses into `window_matches` and
    /// `window_scores`.
    ///
    /// Equivalent to the private
    /// `MaxScoreBulkScorer.collectEssentialScoresIntoWindow(DisiWrapper, int,
    /// int, Bits, FixedBitSet)`; `gate_on_filter_matches` stands for Java's
    /// non-`null` `filterMatches` argument.
    fn collect_essential_scores_into_window(
        &mut self,
        mut top: usize,
        inner_window_max: i32,
        inner_window_min: i32,
        accept_docs: Option<&dyn Bits>,
        gate_on_filter_matches: bool,
    ) -> Result<()> {
        loop {
            loop {
                {
                    let Self {
                        wrappers,
                        doc_and_score_buffer,
                        ..
                    } = self;
                    wrappers[top].scorer().next_docs_and_scores(
                        inner_window_max,
                        accept_docs,
                        doc_and_score_buffer,
                    )?;
                }
                if self.doc_and_score_buffer.size == 0 {
                    break;
                }
                for index in 0..self.doc_and_score_buffer.size {
                    let doc = self.doc_and_score_buffer.docs[index];
                    if gate_on_filter_matches {
                        let filter_matches = self
                            .filter_matches
                            .as_ref()
                            .expect("INVARIANT: gating implies a filter bit set");
                        if !filter_matches.get((doc - inner_window_min) as usize) {
                            continue;
                        }
                    }
                    let score = self.doc_and_score_buffer.features[index];
                    let i = (doc - inner_window_min) as usize;
                    self.window_matches.set(i);
                    self.window_scores[i] += f64::from(score);
                }
            }

            self.wrappers[top].doc = self.wrappers[top].iterator().doc_id();
            top = self
                .essential_queue
                .update_top(&self.wrappers)
                .expect("INVARIANT: the essential queue is non-empty");
            if self.wrappers[top].doc >= inner_window_max {
                break;
            }
        }
        Ok(())
    }

    /// Flushes `window_matches` and `window_scores` into the accumulation
    /// buffer.
    ///
    /// Equivalent to the private
    /// `MaxScoreBulkScorer.flushWindowToDocAndScoreAccBuffer(int, int)`.
    fn flush_window_to_doc_and_score_acc_buffer(
        &mut self,
        inner_window_min: i32,
        inner_window_size: i32,
    ) {
        self.doc_and_score_acc_buffer.size = 0;
        let mut index = bit_set_util::next_set_bit(&self.window_matches, 0);
        while index != NO_MORE_DOCS && index < inner_window_size {
            let i = index as usize;
            let size = self.doc_and_score_acc_buffer.size;
            self.doc_and_score_acc_buffer.docs[size] = inner_window_min + index;
            self.doc_and_score_acc_buffer.scores[size] = self.window_scores[i];
            self.doc_and_score_acc_buffer.size += 1;
            self.window_scores[i] = 0.0;
            if i + 1 >= self.window_matches.length() {
                break;
            }
            index = bit_set_util::next_set_bit(&self.window_matches, i + 1);
        }
        // Only bits below `inner_window_size` can have been set.
        self.window_matches.clear_all();
    }

    /// Equivalent to the private
    /// `MaxScoreBulkScorer.scoreInnerWindowSingleEssentialClause(LeafCollector,
    /// Bits, int)`.
    fn score_inner_window_single_essential_clause(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        up_to: i32,
    ) -> CollectionResult<()> {
        let top = self
            .essential_queue
            .top()
            .expect("INVARIANT: the essential queue is non-empty while scoring");

        // Single essential clause in this window: we can iterate it directly and
        // skip the bitset. This is a common case for 2-clause queries.
        loop {
            {
                let Self {
                    wrappers,
                    doc_and_score_buffer,
                    ..
                } = self;
                wrappers[top].scorer().next_docs_and_scores(
                    up_to,
                    accept_docs,
                    doc_and_score_buffer,
                )?;
            }
            if self.doc_and_score_buffer.size == 0 {
                break;
            }
            self.doc_and_score_acc_buffer
                .copy_from(&self.doc_and_score_buffer);
            let first_essential_scorer = self.first_essential_scorer;
            self.score_non_essential_clauses(collector, first_essential_scorer)?;
        }

        self.wrappers[top].doc = self.wrappers[top].iterator().doc_id();
        self.essential_queue.update_top(&self.wrappers);
        Ok(())
    }

    /// Equivalent to the private
    /// `MaxScoreBulkScorer.scoreInnerWindowMultipleEssentialClauses(
    /// LeafCollector, Bits, int)`.
    fn score_inner_window_multiple_essential_clauses(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        max: i32,
    ) -> CollectionResult<()> {
        let top = self
            .essential_queue
            .top()
            .expect("INVARIANT: the essential queue is non-empty while scoring");

        let inner_window_min = self.wrappers[top].doc;
        let inner_window_max =
            MathUtil::unsigned_min(max, inner_window_min.wrapping_add(INNER_WINDOW_SIZE));
        let inner_window_size = inner_window_max - inner_window_min;

        // Collect matches of essential clauses into a bitset
        self.collect_essential_scores_into_window(
            top,
            inner_window_max,
            inner_window_min,
            accept_docs,
            false,
        )?;
        self.flush_window_to_doc_and_score_acc_buffer(inner_window_min, inner_window_size);

        let first_essential_scorer = self.first_essential_scorer;
        self.score_non_essential_clauses(collector, first_essential_scorer)
    }

    /// Equivalent to the private
    /// `MaxScoreBulkScorer.computeOuterWindowMax(int)`.
    fn compute_outer_window_max(&mut self, window_min: i32) -> Result<i32> {
        // Only use essential scorers to compute the window's max doc ID, in
        // order to avoid constantly recomputing max scores over small windows
        let len = self.all_scorers.len();
        let first_window_lead = self.first_essential_scorer.min(len - 1);
        let mut window_max = NO_MORE_DOCS;
        let filter_cost = self.filter.as_ref().map(|filter| filter.cost);
        for i in first_window_lead..len {
            let position = self.all_scorers[i];
            let cost = self.wrappers[position].cost;
            if filter_cost.map_or(true, |filter_cost| cost >= filter_cost) {
                let doc = self.wrappers[position].doc;
                let up_to = self.wrappers[position]
                    .scorer()
                    .advance_shallow(doc.max(window_min))?;
                // up_to is inclusive
                window_max = MathUtil::unsigned_min(window_max, up_to.wrapping_add(1));
            }
        }

        if len - first_window_lead > 1 {
            // The more clauses we consider to compute outer windows, the higher
            // the chances that one of these clauses has a block boundary in the
            // next few doc IDs. This situation can result in more time spent
            // computing maximum scores per outer window than evaluating hits. To
            // avoid such situations, we target at least 32 candidate matches per
            // clause per outer window on average, to make sure we amortize the
            // cost of computing maximum scores.
            let threshold = i64::from(self.num_outer_windows) * 32i64 * len as i64;
            if i64::from(self.num_candidates) < threshold {
                self.min_window_size = (self.min_window_size << 1).min(INNER_WINDOW_SIZE);
            } else {
                self.min_window_size = 1;
            }

            let min_window_max =
                MathUtil::unsigned_min(i32::MAX, window_min.wrapping_add(self.min_window_size));
            window_max = window_max.max(min_window_max);
        }

        Ok(window_max)
    }

    /// Equivalent to the package-private
    /// `MaxScoreBulkScorer.updateMaxWindowScores(int, int)`.
    fn update_max_window_scores(&mut self, window_min: i32, window_max: i32) -> Result<()> {
        for position in 0..self.wrappers.len() {
            if self.wrappers[position].doc < window_max {
                if self.wrappers[position].doc < window_min {
                    // Make sure to advance shallow if necessary to get as good
                    // score upper bounds as possible.
                    self.wrappers[position]
                        .scorer()
                        .advance_shallow(window_min)?;
                }
                self.wrappers[position].max_window_score = self.wrappers[position]
                    .scorer()
                    .get_max_score(window_max - 1)?;
            } else {
                // This scorer has no documents in the considered window.
                self.wrappers[position].max_window_score = 0.0;
            }
        }
        Ok(())
    }

    /// Equivalent to the private
    /// `MaxScoreBulkScorer.scoreNonEssentialClauses(LeafCollector,
    /// DocAndScoreAccBuffer, int)`.
    fn score_non_essential_clauses(
        &mut self,
        collector: &mut dyn LeafCollector,
        num_non_essential_clauses: usize,
    ) -> CollectionResult<()> {
        self.num_candidates = self
            .num_candidates
            .wrapping_add(self.doc_and_score_acc_buffer.size as i32);

        let num_scorers = self.all_scorers.len() as i32;
        for i in (0..num_non_essential_clauses).rev() {
            let position = self.all_scorers[i];
            debug_assert!(
                self.scorable.min_competitive_score() > 0.0,
                "All clauses are essential if minCompetitiveScore is equal to zero"
            );

            ScorerUtil::filter_competitive_hits(
                &mut self.doc_and_score_acc_buffer,
                self.max_score_sums[i],
                self.scorable.min_competitive_score(),
                num_scorers,
            );

            {
                let Self {
                    wrappers,
                    doc_and_score_acc_buffer,
                    ..
                } = self;
                let wrapper = &mut wrappers[position];
                // The iterator and the scorable are two views of the same
                // clause; Java holds them in two fields, this port re-borrows
                // each in turn.
                if i >= self.first_required_scorer {
                    apply_required_clause(doc_and_score_acc_buffer, wrapper)?;
                } else {
                    apply_optional_clause(doc_and_score_acc_buffer, wrapper)?;
                }
            }
            self.wrappers[position].doc = self.wrappers[position].iterator().doc_id();
        }

        {
            let Self {
                scorable,
                doc_and_score_acc_buffer,
                ..
            } = self;
            for i in 0..doc_and_score_acc_buffer.size {
                scorable.set_score(doc_and_score_acc_buffer.scores[i] as f32);
                collector.collect(doc_and_score_acc_buffer.docs[i], scorable)?;
            }
        }
        Ok(())
    }

    /// Partitions the clauses into essential and non-essential ones.
    ///
    /// Equivalent to the package-private
    /// `MaxScoreBulkScorer.partitionScorers()`.
    fn partition_scorers(&mut self) -> bool {
        // Partitioning scorers is an optimization problem: the optimal set of
        // non-essential scorers is the subset of scorers whose sum of max window
        // scores is less than the minimum competitive score that maximizes the
        // sum of costs. Computing the optimal solution would take
        // O(2^num_clauses). As a first approximation, we take the first scorers
        // sorted by max_window_score / cost whose sum of max scores is less than
        // the minimum competitive score.
        let len = self.all_scorers.len();
        self.scratch[..len].copy_from_slice(&self.all_scorers[..len]);
        let wrappers = &self.wrappers;
        self.scratch[..len].sort_by(|a, b| {
            let ratio_a =
                f64::from(wrappers[*a].max_window_score) / (wrappers[*a].cost.max(1) as f64);
            let ratio_b =
                f64::from(wrappers[*b].max_window_score) / (wrappers[*b].cost.max(1) as f64);
            ratio_a.total_cmp(&ratio_b)
        });

        let mut max_score_sum = 0.0f64;
        self.first_essential_scorer = 0;
        self.next_min_competitive_score = f32::INFINITY;
        for i in 0..len {
            let w = self.scratch[i];
            let new_max_score_sum = max_score_sum + f64::from(self.wrappers[w].max_window_score);
            let max_score_sum_float = MathUtil::sum_upper_bound(
                new_max_score_sum,
                self.first_essential_scorer as i32 + 1,
            ) as f32;
            if max_score_sum_float < self.scorable.min_competitive_score() {
                max_score_sum = new_max_score_sum;
                self.all_scorers[self.first_essential_scorer] = w;
                self.max_score_sums[self.first_essential_scorer] = max_score_sum;
                self.first_essential_scorer += 1;
            } else {
                self.all_scorers[len - 1 - (i - self.first_essential_scorer)] = w;
                self.next_min_competitive_score =
                    max_score_sum_float.min(self.next_min_competitive_score);
            }
        }

        self.first_required_scorer = len;

        if self.first_essential_scorer == len {
            return false;
        }

        self.essential_queue.clear();
        for i in self.first_essential_scorer..len {
            self.essential_queue
                .add(&self.wrappers, self.all_scorers[i]);
        }

        if self.first_essential_scorer == len - 1 {
            // single essential clause
            //
            // If there is a single essential clause and matching it plus all
            // non-essential clauses but the best one is not enough to yield a
            // competitive match, then we know that hits must match both the
            // essential clause and the best non-essential clause.
            self.first_required_scorer = len - 1;
            let mut max_required_score = f64::from(
                self.wrappers[self.all_scorers[self.first_essential_scorer]].max_window_score,
            );

            while self.first_required_scorer > 0 {
                let mut max_possible_score_without_previous_clause = max_required_score;
                if self.first_required_scorer > 1 {
                    max_possible_score_without_previous_clause +=
                        self.max_score_sums[self.first_required_scorer - 2];
                }
                if (max_possible_score_without_previous_clause as f32)
                    >= self.scorable.min_competitive_score()
                {
                    break;
                }
                // The sum of maximum scores ignoring the previous clause is less
                // than the minimum competitive score.
                self.first_required_scorer -= 1;
                max_required_score += f64::from(
                    self.wrappers[self.all_scorers[self.first_required_scorer]].max_window_score,
                );
            }
        }

        true
    }

    /// Returns the next candidate on or after `range_end`.
    ///
    /// Equivalent to the private `MaxScoreBulkScorer.nextCandidate(int)`.
    fn next_candidate(&self, range_end: i32) -> i32 {
        if range_end >= self.max_doc {
            return NO_MORE_DOCS;
        }

        let mut next = NO_MORE_DOCS;
        for position in &self.all_scorers {
            let doc = self.wrappers[*position].doc;
            if doc < range_end {
                return range_end;
            }
            next = next.min(doc);
        }
        next
    }
}

/// Applies a clause as required, re-borrowing the iterator and the scorable of
/// the same wrapper in turn.
///
/// Equivalent to `ScorerUtil.applyRequiredClause(DocAndScoreAccBuffer,
/// DocIdSetIterator, Scorable)`, which Java can call with two aliases of the
/// same clause.
fn apply_required_clause(
    buffer: &mut DocAndScoreAccBuffer,
    wrapper: &mut DisiWrapper,
) -> Result<()> {
    let mut intersection_size = 0;
    let mut cur_doc = wrapper.iterator().doc_id();
    for i in 0..buffer.size {
        let target_doc = buffer.docs[i];
        if cur_doc < target_doc {
            cur_doc = wrapper.iterator().advance(target_doc)?;
        }
        if cur_doc == target_doc {
            buffer.docs[intersection_size] = target_doc;
            buffer.scores[intersection_size] =
                buffer.scores[i] + f64::from(wrapper.scorable().score()?);
            intersection_size += 1;
        }
    }
    buffer.size = intersection_size;
    Ok(())
}

/// Applies a clause as optional; see [`apply_required_clause`].
///
/// Equivalent to `ScorerUtil.applyOptionalClause(DocAndScoreAccBuffer,
/// DocIdSetIterator, Scorable)`.
fn apply_optional_clause(
    buffer: &mut DocAndScoreAccBuffer,
    wrapper: &mut DisiWrapper,
) -> Result<()> {
    let mut cur_doc = wrapper.iterator().doc_id();
    for i in 0..buffer.size {
        let target_doc = buffer.docs[i];
        if cur_doc < target_doc {
            cur_doc = wrapper.iterator().advance(target_doc)?;
        }
        if cur_doc == target_doc {
            buffer.scores[i] += f64::from(wrapper.scorable().score()?);
        }
    }
    Ok(())
}

impl BulkScorer for MaxScoreBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        collector.set_scorer(&mut self.scorable)?;

        // This scorer computes outer windows based on impacts that are stored in
        // the index. These outer windows should be small enough to provide good
        // upper bounds of scores, and big enough to make sure we spend more time
        // collecting docs than recomputing windows. Then within these outer
        // windows, it creates inner windows of size INNER_WINDOW_SIZE that help
        // collect matches into a bitset and save the overhead of rebalancing the
        // priority queue on every match.
        let mut outer_window_min = min;
        'outer: while outer_window_min < max {
            let mut outer_window_max = self.compute_outer_window_max(outer_window_min)?;
            outer_window_max = outer_window_max.min(max);

            loop {
                self.update_max_window_scores(outer_window_min, outer_window_max)?;
                if !self.partition_scorers() {
                    // No matches in this window
                    outer_window_min = outer_window_max;
                    continue 'outer;
                }

                // There is a dependency between windows and maximum scores, as
                // we compute windows based on maximum scores and maximum scores
                // based on windows. So the approach consists of starting by
                // computing a window based on the set of essential scorers from
                // the _previous_ window and then iteratively recompute maximum
                // scores and windows as long as the window size decreases.
                let new_outer_window_max = self.compute_outer_window_max(outer_window_min)?;
                if new_outer_window_max >= outer_window_max {
                    break;
                }
                outer_window_max = new_outer_window_max;
            }

            let mut top = self
                .essential_queue
                .top()
                .expect("INVARIANT: partitionScorers left the essential queue non-empty");
            while self.wrappers[top].doc < outer_window_min {
                let next = self.wrappers[top].iterator().advance(outer_window_min)?;
                self.wrappers[top].doc = next;
                top = self
                    .essential_queue
                    .update_top(&self.wrappers)
                    .expect("INVARIANT: the essential queue is non-empty");
            }

            while self.wrappers[top].doc < outer_window_max {
                self.score_inner_window(collector, accept_docs, outer_window_max)?;
                top = self
                    .essential_queue
                    .top()
                    .expect("INVARIANT: the essential queue is non-empty");
                if self.scorable.min_competitive_score() >= self.next_min_competitive_score {
                    // The minimum competitive score increased substantially, so
                    // we can now partition scorers in a more favorable way.
                    break;
                }
            }

            outer_window_min = self.wrappers[top].doc.min(outer_window_max);
            self.num_outer_windows += 1;
        }

        Ok(self.next_candidate(max))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}
