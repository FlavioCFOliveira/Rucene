//! Block-max WAND scoring, ported from
//! `org.apache.lucene.search.WANDScorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::disi_priority_queue::{parent_node, DisiPriorityQueue};
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_util::ScorerUtil;
use crate::search::two_phase_iterator::TwoPhaseIterator;
use crate::util::MathUtil;

/// The number of mantissa bits of a single-precision floating-point number.
///
/// Equivalent to the package-private `WANDScorer.FLOAT_MANTISSA_BITS` constant.
pub const FLOAT_MANTISSA_BITS: i32 = 24;

/// The greatest scaled score a clause may contribute.
///
/// Equivalent to the private `WANDScorer.MAX_SCALED_SCORE` constant.
const MAX_SCALED_SCORE: i64 = (1i64 << 24) - 1;

/// Reproduces `java.lang.Math.getExponent(double)`: the unbiased exponent, or
/// `Double.MIN_EXPONENT - 1` for zero and subnormal values.
fn get_exponent(value: f64) -> i32 {
    (((value.to_bits() >> 52) & 0x7FF) as i32) - 1023
}

/// Reproduces `java.lang.Math.scalb(double, int)`: `value * 2^scale_factor`,
/// computed without an intermediate overflow.
fn scalb(mut value: f64, mut scale_factor: i32) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    while scale_factor > 1023 {
        value *= two_pow(1023);
        scale_factor -= 1023;
    }
    while scale_factor < -1022 {
        value *= two_pow(-1022);
        scale_factor += 1022;
    }
    value * two_pow(scale_factor)
}

/// Returns `2^exponent` for `-1022 <= exponent <= 1023`.
fn two_pow(exponent: i32) -> f64 {
    f64::from_bits((((exponent + 1023) as u64) & 0x7FF) << 52)
}

/// Returns a scaling factor for `value` so that `value * 2^scalingFactor` falls
/// in `[2^23, 2^24)`.
///
/// Equivalent to the package-private `WANDScorer.scalingFactor(float)`. The
/// special cases are `scalingFactor(0) = scalingFactor(MIN_VALUE) + 1` and
/// `scalingFactor(+Infty) = scalingFactor(MAX_VALUE) - 1`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] for a negative value, matching
/// Java's `IllegalArgumentException`.
pub fn scaling_factor(value: f32) -> Result<i32> {
    if value < 0.0 {
        Err(LuceneError::IllegalArgument(
            "Scores must be positive or null".to_string(),
        ))
    } else if value == 0.0 {
        Ok(scaling_factor(f32::from_bits(1))? + 1)
    } else if value.is_infinite() {
        Ok(scaling_factor(f32::MAX)? - 1)
    } else {
        let d = f64::from(value);
        // Since doubles have more amplitude than floats for the exponent, the
        // cast produces a normal value.
        debug_assert!(d == 0.0 || get_exponent(d) >= -1022);
        Ok(FLOAT_MANTISSA_BITS - 1 - get_exponent(d))
    }
}

/// Scales a maximum score into an unsigned integer, rounding up so that no
/// match is missed.
///
/// Equivalent to the package-private `WANDScorer.scaleMaxScore(float, int)`.
pub fn scale_max_score(max_score: f32, scaling_factor: i32) -> i64 {
    debug_assert!(!max_score.is_nan());
    debug_assert!(max_score >= 0.0);

    // NOTE: because doubles have more amplitude than floats for the exponent,
    // the scalb call produces an accurate value.
    let scaled = scalb(f64::from(max_score), scaling_factor);

    if scaled > MAX_SCALED_SCORE as f64 {
        // This happens if one scorer returns +Infty as a max score, or if the
        // scorer returns greater max scores locally than globally - which
        // shouldn't happen with well-behaved scorers.
        return MAX_SCALED_SCORE;
    }

    // round up, cast is accurate since value is < 2^24
    scaled.ceil() as i64
}

/// Scales a minimum competitive score, rounding down so that no match is
/// missed.
///
/// Equivalent to the private `WANDScorer.scaleMinScore(float, int)`.
fn scale_min_score(min_score: f32, scaling_factor: i32) -> i64 {
    debug_assert!(min_score.is_finite());
    debug_assert!(min_score >= 0.0);

    let scaled = scalb(f64::from(min_score), scaling_factor);
    // round down; the cast saturates when the value exceeds `i64::MAX`, which
    // is what Java's narrowing cast does too.
    scaled.floor() as i64
}

/// In the tail, we want to get first the entries that produce the maximum
/// scores and, in case of ties (for instance constant-score queries), those
/// that have the least cost so that they are likely to advance further.
///
/// Equivalent to the private static
/// `WANDScorer.greaterMaxScore(DisiWrapper, DisiWrapper)`.
fn greater_max_score(w1: &DisiWrapper, w2: &DisiWrapper) -> bool {
    if w1.scaled_max_score > w2.scaled_max_score {
        true
    } else if w1.scaled_max_score < w2.scaled_max_score {
        false
    } else {
        w1.cost < w2.cost
    }
}

/// The state shared between the approximation and the confirmation of a
/// [`WANDScorer`].
///
/// Equivalent to the fields of `org.apache.lucene.search.WANDScorer` together
/// with its two anonymous inner classes — the approximation and the
/// `TwoPhaseIterator` built around it — which in Java read and write those very
/// fields.
///
/// **Divergence from Lucene 10.5.0.** Java keeps its sub-scorers in three
/// places at once: the `lead` linked list, the `head` heap and the `tail` heap,
/// all holding references to the same `DisiWrapper` objects that `allScorers`
/// also names. Rust forbids that aliasing, so the wrappers live in one array in
/// construction order and the three structures hold **positions** into it. The
/// linked list becomes a vector in Java's prepend order, so that the traversal
/// order — which decides the order in which the lead scores are summed — is
/// preserved exactly.
struct WANDCore {
    /// All the clauses, in the order the caller supplied them.
    ///
    /// Equivalent to the `Scorer[] allScorers` field, whose entries are the
    /// scorers the `DisiWrapper`s wrap.
    wrappers: Vec<DisiWrapper>,

    scaling_factor: i32,
    /// Scaled minimum competitive score.
    min_competitive_score: i64,

    /// Clauses which 'lead' the iteration and are currently positioned on
    /// `doc`. This is sometimes called the 'pivot' in some descriptions of WAND.
    /// Positions are pushed in Java's prepend order, so the traversal order is
    /// the reverse.
    lead: Vec<usize>,
    /// Current doc ID of the leads.
    doc: i32,
    /// Score of the leads.
    lead_score: f64,

    /// Clauses that are too advanced compared to the current doc, ordered by
    /// doc ID.
    head: DisiPriorityQueue,

    /// Clauses which are behind the current doc, ordered by maximum score. The
    /// vector is a 0-based heap of `tail_size` entries, as Java's array is.
    tail: Vec<usize>,
    /// Sum of the max scores of the clauses in `tail`.
    tail_max_score: i64,
    tail_size: usize,

    cost: i64,

    /// Upper bound for which max scores are valid.
    up_to: i32,

    min_should_match: usize,
    freq: usize,

    score_mode: ScoreMode,
    lead_cost: i64,
}

impl WANDCore {
    /// Equivalent to the private `WANDScorer.addLead(DisiWrapper)`.
    fn add_lead(&mut self, lead: usize) -> Result<()> {
        self.lead.push(lead);
        self.freq += 1;
        if self.score_mode == ScoreMode::TOP_SCORES {
            self.lead_score += f64::from(self.wrappers[lead].scorable().score()?);
        }
        Ok(())
    }

    /// Equivalent to the private
    /// `WANDScorer.addUnpositionedLead(DisiWrapper)`.
    fn add_unpositioned_lead(&mut self, lead: usize) {
        debug_assert_eq!(self.wrappers[lead].doc, -1);
        self.lead.push(lead);
        self.freq += 1;
    }

    /// Equivalent to the private `WANDScorer.pushBackLeads(int)`.
    fn push_back_leads(&mut self, target: i32) -> Result<()> {
        let leads = std::mem::take(&mut self.lead);
        // Java walks the linked list, whose traversal order is the reverse of
        // the order in which entries were prepended.
        for position in leads.iter().rev() {
            if let Some(evicted) = self.insert_tail_with_overflow(*position) {
                let next = self.wrappers[evicted].iterator().advance(target)?;
                self.wrappers[evicted].doc = next;
                self.head.add(&self.wrappers, evicted);
            }
        }
        Ok(())
    }

    /// Equivalent to the private `WANDScorer.advanceHead(int)`.
    fn advance_head(&mut self, target: i32) -> Result<Option<usize>> {
        let mut head_top = self.head.top();
        while let Some(top) = head_top {
            if self.wrappers[top].doc >= target {
                break;
            }
            match self.insert_tail_with_overflow(top) {
                Some(evicted) => {
                    let next = self.wrappers[evicted].iterator().advance(target)?;
                    self.wrappers[evicted].doc = next;
                    head_top = self.head.update_top_with(&self.wrappers, evicted);
                }
                None => {
                    self.head.pop(&self.wrappers);
                    head_top = self.head.top();
                }
            }
        }
        Ok(head_top)
    }

    /// Equivalent to the private `WANDScorer.advanceTail(DisiWrapper)`.
    fn advance_tail_entry(&mut self, disi: usize) -> Result<()> {
        let doc = self.doc;
        let next = self.wrappers[disi].iterator().advance(doc)?;
        self.wrappers[disi].doc = next;
        if next == doc {
            self.add_lead(disi)?;
        } else {
            self.head.add(&self.wrappers, disi);
        }
        Ok(())
    }

    /// Pops the entry from the tail that has the greatest score contribution,
    /// advances it to the current doc and then adds it to `lead` or `head`
    /// depending on whether it matches.
    ///
    /// Equivalent to the private `WANDScorer.advanceTail()`.
    fn advance_tail(&mut self) -> Result<()> {
        let top = self.pop_tail();
        self.advance_tail_entry(top)
    }

    /// Equivalent to the private `WANDScorer.updateMaxScores(int)`.
    fn update_max_scores(&mut self, target: i32) -> Result<()> {
        let mut new_up_to = NO_MORE_DOCS;
        // If we have entries in 'head', we treat them all as leads and take the
        // minimum of their next block boundaries as a next boundary. We don't
        // take entries in 'tail' into account on purpose: 'tail' is supposed to
        // contain the least score contributors, and taking them into account
        // might not move the boundary fast enough, so we'd waste CPU
        // re-computing the next boundary all the time. Likewise, we ignore
        // clauses whose cost is greater than the lead cost to avoid recomputing
        // per-window max scores over and over again.
        let head_entries: Vec<usize> = self.head.entries().to_vec();
        for position in &head_entries {
            let w = &self.wrappers[*position];
            if w.doc <= new_up_to && w.cost <= self.lead_cost {
                let doc = w.doc;
                new_up_to = self.wrappers[*position]
                    .scorer()
                    .advance_shallow(doc)?
                    .min(new_up_to);
            }
        }
        // Only look at the tail if none of the `head` clauses had a block we
        // could reuse and if its cost is less than or equal to the lead cost.
        if new_up_to == NO_MORE_DOCS
            && self.tail_size > 0
            && self.wrappers[self.tail[0]].cost <= self.lead_cost
        {
            let tail_top = self.tail[0];
            new_up_to = self.wrappers[tail_top].scorer().advance_shallow(target)?;
            // upTo must be on or after the least `head` doc
            if let Some(head_top) = self.head.top() {
                new_up_to = new_up_to.max(self.wrappers[head_top].doc);
            }
        }
        self.up_to = new_up_to;

        // Now update the max scores of clauses that are before upTo.
        let up_to = self.up_to;
        for position in &head_entries {
            if self.wrappers[*position].doc <= up_to {
                let max_score = self.wrappers[*position].scorer().get_max_score(new_up_to)?;
                self.wrappers[*position].scaled_max_score =
                    scale_max_score(max_score, self.scaling_factor);
            }
        }

        self.tail_max_score = 0;
        for i in 0..self.tail_size {
            let position = self.tail[i];
            self.wrappers[position].scorer().advance_shallow(target)?;
            let max_score = self.wrappers[position].scorer().get_max_score(up_to)?;
            self.wrappers[position].scaled_max_score =
                scale_max_score(max_score, self.scaling_factor);
            // the heap might need to be reordered
            self.up_heap_max_score(i);
            self.tail_max_score += self.wrappers[position].scaled_max_score;
        }

        // We need to make sure that entries in 'tail' alone cannot match a
        // competitive hit.
        while self.tail_size > 0 && self.tail_max_score >= self.min_competitive_score {
            let position = self.pop_tail();
            let next = self.wrappers[position].iterator().advance(target)?;
            self.wrappers[position].doc = next;
            self.head.add(&self.wrappers, position);
        }

        Ok(())
    }

    /// Updates `up_to` and the maximum scores of the sub-scorers so that
    /// `up_to` is greater than or equal to the next candidate after `target`.
    ///
    /// Equivalent to the private `WANDScorer.moveToNextBlock(int)`.
    fn move_to_next_block(&mut self, mut target: i32) -> Result<()> {
        debug_assert!(self.lead.is_empty());

        while self.up_to < NO_MORE_DOCS {
            if self.head.size() == 0 {
                // All clauses could fit in the tail, which means that the sum of
                // the maximum scores of sub clauses is less than the minimum
                // competitive score. Move to the next block until this condition
                // becomes false.
                target = target.max(self.up_to + 1);
                self.update_max_scores(target)?;
            } else {
                let head_top = self
                    .head
                    .top()
                    .expect("INVARIANT: the heap was just observed to be non-empty");
                if self.wrappers[head_top].doc > self.up_to {
                    // We have a next candidate but it's not in the current
                    // block. We need to move to the next block in order to not
                    // miss any potential hits between `target` and
                    // `head.top().doc`.
                    self.update_max_scores(target)?;
                    break;
                }
                break;
            }
        }

        Ok(())
    }

    /// Sets `doc` to the next potential match, and moves all clauses of `head`
    /// that are on this doc into `lead`.
    ///
    /// Equivalent to the private `WANDScorer.moveToNextCandidate()`.
    fn move_to_next_candidate(&mut self) -> Result<()> {
        // The top of `head` defines the next potential match; pop all documents
        // which are on this doc.
        let lead = self
            .head
            .pop(&self.wrappers)
            .expect("INVARIANT: the approximation only stops on a non-empty head");
        debug_assert_eq!(self.doc, self.wrappers[lead].doc);
        self.lead.clear();
        self.lead.push(lead);
        self.freq = 1;
        if self.score_mode == ScoreMode::TOP_SCORES {
            self.lead_score = f64::from(self.wrappers[lead].scorable().score()?);
        }
        while self.head.size() > 0 {
            let top = self
                .head
                .top()
                .expect("INVARIANT: the heap was just observed to be non-empty");
            if self.wrappers[top].doc != self.doc {
                break;
            }
            let popped = self
                .head
                .pop(&self.wrappers)
                .expect("INVARIANT: the heap was just observed to be non-empty");
            self.add_lead(popped)?;
        }
        Ok(())
    }

    /// Advances all entries from the tail to know about all matches on the
    /// current doc.
    ///
    /// Equivalent to the private `WANDScorer.advanceAllTail()`.
    fn advance_all_tail(&mut self) -> Result<()> {
        // We return the next doc when the sum of the scores of the potential
        // matching clauses is high enough but some of the clauses in 'tail'
        // might match as well. Since we are advancing all clauses in tail, we
        // just iterate the array without reorganizing the heap.
        for i in (0..self.tail_size).rev() {
            let position = self.tail[i];
            self.advance_tail_entry(position)?;
        }
        self.tail_size = 0;
        self.tail_max_score = 0;
        Ok(())
    }

    /// Inserts an entry in `tail` and evicts the least-costly clause if full.
    ///
    /// Equivalent to the private
    /// `WANDScorer.insertTailWithOverFlow(DisiWrapper)`.
    fn insert_tail_with_overflow(&mut self, s: usize) -> Option<usize> {
        let s_scaled = self.wrappers[s].scaled_max_score;
        if self.tail_max_score + s_scaled < self.min_competitive_score
            || self.tail_size + 1 < self.min_should_match
        {
            // we have free room for this new entry
            self.add_tail(s);
            self.tail_max_score += s_scaled;
            None
        } else if self.tail_size == 0 {
            Some(s)
        } else {
            let top = self.tail[0];
            if !greater_max_score(&self.wrappers[top], &self.wrappers[s]) {
                return Some(s);
            }
            // Swap top and s
            self.tail[0] = s;
            self.down_heap_max_score();
            self.tail_max_score =
                self.tail_max_score - self.wrappers[top].scaled_max_score + s_scaled;
            Some(top)
        }
    }

    /// Adds an entry to `tail`.
    ///
    /// Equivalent to the private `WANDScorer.addTail(DisiWrapper)`.
    fn add_tail(&mut self, s: usize) {
        self.tail[self.tail_size] = s;
        self.up_heap_max_score(self.tail_size);
        self.tail_size += 1;
    }

    /// Pops the least-costly clause from `tail`.
    ///
    /// Equivalent to the private `WANDScorer.popTail()`.
    fn pop_tail(&mut self) -> usize {
        debug_assert!(self.tail_size > 0);
        let result = self.tail[0];
        self.tail_size -= 1;
        self.tail[0] = self.tail[self.tail_size];
        self.down_heap_max_score();
        self.tail_max_score -= self.wrappers[result].scaled_max_score;
        result
    }

    /// Equivalent to the private static
    /// `WANDScorer.upHeapMaxScore(DisiWrapper[], int)`.
    fn up_heap_max_score(&mut self, mut i: usize) {
        let node = self.tail[i];
        let mut j = parent_node(i);
        while j >= 0
            && greater_max_score(&self.wrappers[node], &self.wrappers[self.tail[j as usize]])
        {
            self.tail[i] = self.tail[j as usize];
            i = j as usize;
            j = parent_node(i);
        }
        self.tail[i] = node;
    }

    /// Equivalent to the private static
    /// `WANDScorer.downHeapMaxScore(DisiWrapper[], int)`, called with
    /// `tail_size`.
    fn down_heap_max_score(&mut self) {
        let size = self.tail_size;
        if size == 0 {
            return;
        }
        let mut i = 0usize;
        let node = self.tail[0];
        let mut j = crate::search::disi_priority_queue::left_node(i);
        if j < size {
            let mut k = crate::search::disi_priority_queue::right_node(j);
            if k < size
                && greater_max_score(&self.wrappers[self.tail[k]], &self.wrappers[self.tail[j]])
            {
                j = k;
            }
            if greater_max_score(&self.wrappers[self.tail[j]], &self.wrappers[node]) {
                loop {
                    self.tail[i] = self.tail[j];
                    i = j;
                    j = crate::search::disi_priority_queue::left_node(i);
                    k = crate::search::disi_priority_queue::right_node(j);
                    if k < size
                        && greater_max_score(
                            &self.wrappers[self.tail[k]],
                            &self.wrappers[self.tail[j]],
                        )
                    {
                        j = k;
                    }
                    if !(j < size
                        && greater_max_score(&self.wrappers[self.tail[j]], &self.wrappers[node]))
                    {
                        break;
                    }
                }
                self.tail[i] = node;
            }
        }
    }
}

impl DocIdSetIterator for WANDCore {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        let target = self.doc + 1;
        self.advance(target)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        // Move 'lead' iterators back to the tail
        self.push_back_leads(target)?;

        // Make sure `head` is also on or beyond `target`
        let mut head_top = self.advance_head(target)?;

        if self.score_mode == ScoreMode::TOP_SCORES
            && head_top.map_or(true, |top| self.wrappers[top].doc > self.up_to)
        {
            // Update score bounds if necessary
            self.move_to_next_block(target)?;
            debug_assert!(self.up_to >= target);
            head_top = self.head.top();
        }

        self.doc = match head_top {
            None => NO_MORE_DOCS,
            Some(top) => self.wrappers[top].doc,
        };
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl TwoPhaseIterator for WANDCore {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self
    }

    fn matches(&mut self) -> Result<bool> {
        self.move_to_next_candidate()?;

        let mut scaled_lead_score = 0i64;
        if self.score_mode == ScoreMode::TOP_SCORES {
            scaled_lead_score = scale_max_score(
                MathUtil::sum_upper_bound(self.lead_score, FLOAT_MANTISSA_BITS) as f32,
                self.scaling_factor,
            );
        }

        while scaled_lead_score < self.min_competitive_score || self.freq < self.min_should_match {
            if scaled_lead_score + self.tail_max_score < self.min_competitive_score
                || self.freq + self.tail_size < self.min_should_match
            {
                return Ok(false);
            }
            // a match on doc is still possible, try to advance scorers from the
            // tail
            let prev_lead_len = self.lead.len();
            self.advance_tail()?;
            if self.score_mode == ScoreMode::TOP_SCORES && self.lead.len() != prev_lead_len {
                scaled_lead_score = scale_max_score(
                    MathUtil::sum_upper_bound(self.lead_score, FLOAT_MANTISSA_BITS) as f32,
                    self.scaling_factor,
                );
            }
        }

        Ok(true)
    }

    fn match_cost(&self) -> f32 {
        // maximum number of scorers that matches() might advance
        self.tail.len() as f32
    }
}

/// The WAND (Weak AND) scorer.
///
/// Equivalent to the `final class org.apache.lucene.search.WANDScorer`, which
/// implements the algorithm for dynamic pruning described in "Efficient Query
/// Evaluation using a Two-Level Retrieval Process" by Broder, Carmel,
/// Herscovici, Soffer and Zien, enhanced with the techniques described in
/// "Faster Top-k Document Retrieval Using Block-Max Indexes" by Ding and Suel.
///
/// For [`ScoreMode::TOP_SCORES`], the scorer maintains a feedback loop with the
/// collector so that it knows at any time the minimum score required for a hit
/// to be competitive. It enforces both `∑ max_score >= minCompetitiveScore` and
/// `freq >= minShouldMatch`, keeping its sub-scorers in three places: a `tail`
/// heap of clauses behind the desired doc ID ordered by cost, a `lead` list of
/// clauses positioned on the desired doc ID, and a `head` heap of clauses
/// beyond the desired doc ID ordered by doc ID.
pub struct WANDScorer {
    core: WANDCore,
}

impl std::fmt::Debug for WANDScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WANDScorer")
            .field("min_should_match", &self.core.min_should_match)
            .field("doc", &self.core.doc)
            .finish_non_exhaustive()
    }
}

impl WANDScorer {
    /// Builds a WAND scorer over the given clauses.
    ///
    /// Equivalent to `WANDScorer(Collection<Scorer>, int, ScoreMode, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `min_should_match` is not
    /// strictly less than the number of clauses, and propagates any I/O error
    /// raised while priming the score bounds.
    pub fn new(
        mut scorers: Vec<Box<dyn Scorer>>,
        min_should_match: usize,
        score_mode: ScoreMode,
        lead_cost: i64,
    ) -> Result<Self> {
        if min_should_match >= scorers.len() {
            return Err(LuceneError::IllegalArgument(
                "minShouldMatch should be < the number of scorers".to_string(),
            ));
        }

        let num_scorers = scorers.len();

        let scaling_factor = if score_mode == ScoreMode::TOP_SCORES {
            // To avoid accuracy issues with floating-point numbers, this scorer
            // operates on scaled longs. We want to retain as many significant
            // bits as possible, but not too many, otherwise operations on longs
            // would be more precise than the equivalent operations on their
            // unscaled counterparts and we might skip too many hits. So we
            // compute the maximum possible score produced by this scorer, which
            // is the sum of the maximum scores of each clause, and compute a
            // scaling factor that would preserve 24 bits of accuracy.
            let mut max_score_sum_double = 0.0f64;
            for scorer in scorers.iter_mut() {
                scorer.advance_shallow(0)?;
                let max_score = scorer.get_max_score(NO_MORE_DOCS)?;
                max_score_sum_double += f64::from(max_score);
            }
            let max_score_sum =
                MathUtil::sum_upper_bound(max_score_sum_double, num_scorers as i32) as f32;
            scaling_factor(max_score_sum)?
        } else {
            0
        };

        let mut wrappers = Vec::with_capacity(num_scorers);
        for scorer in scorers {
            // Ideally we would pass true when scoreMode == TOP_SCORES and false
            // otherwise, but this would break the optimization as there could
            // then be 3 different impls of DocIdSetIterator. So we pass true to
            // favor disjunctions sorted by descending score as opposed to
            // non-scoring disjunctions whose minShouldMatch is greater than 1.
            wrappers.push(DisiWrapper::new(scorer, true));
        }

        let cost = ScorerUtil::cost_with_min_should_match(
            wrappers.iter().map(|w| w.cost),
            num_scorers,
            min_should_match,
        )?;

        let mut core = WANDCore {
            wrappers,
            scaling_factor,
            min_competitive_score: 0,
            lead: Vec::with_capacity(num_scorers),
            doc: -1,
            lead_score: 0.0,
            head: DisiPriorityQueue::of_max_size(num_scorers),
            // there can be at most num_scorers - 1 scorers beyond the current
            // position
            tail: vec![0; num_scorers],
            tail_max_score: 0,
            tail_size: 0,
            cost,
            // will be computed on the first call to next_doc/advance
            up_to: -1,
            min_should_match,
            freq: 0,
            score_mode,
            lead_cost,
        };

        for position in 0..num_scorers {
            core.add_unpositioned_lead(position);
        }

        Ok(Self { core })
    }

    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if self.core.matches()? {
                return Ok(doc);
            }
            doc = self.core.next_doc()?;
        }
    }
}

impl DocIdSetIterator for WANDScorer {
    fn doc_id(&self) -> i32 {
        self.core.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.core.next_doc()?;
        self.confirm(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.core.advance(target)?;
        self.confirm(doc)
    }

    fn cost(&self) -> i64 {
        self.core.cost
    }
}

impl Scorable for WANDScorer {
    fn score(&mut self) -> Result<f32> {
        // we need to know about all matches
        self.core.advance_all_tail()?;

        let mut lead_score = self.core.lead_score;
        if self.core.score_mode != ScoreMode::TOP_SCORES {
            // With TOP_SCORES, the score was already computed on the fly.
            let leads: Vec<usize> = self.core.lead.iter().rev().copied().collect();
            for position in leads {
                lead_score += f64::from(self.core.wrappers[position].scorable().score()?);
            }
        }
        Ok(lead_score as f32)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        // Let this disjunction know about the new min score so that it can skip
        // over clauses that produce low scores.
        debug_assert!(
            self.core.score_mode == ScoreMode::TOP_SCORES,
            "minCompetitiveScore can only be set for ScoreMode.TOP_SCORES"
        );
        debug_assert!(min_score >= 0.0);
        let scaled_min_score = scale_min_score(min_score, self.core.scaling_factor);
        debug_assert!(scaled_min_score >= self.core.min_competitive_score);
        self.core.min_competitive_score = scaled_min_score;
        Ok(())
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        self.core.advance_all_tail()?;
        let mut rank = vec![usize::MAX; self.core.wrappers.len()];
        for (position_in_list, position) in self.core.lead.iter().rev().enumerate() {
            rank[*position] = position_in_list;
        }
        let mut selected: Vec<(usize, &mut DisiWrapper)> = self
            .core
            .wrappers
            .iter_mut()
            .enumerate()
            .filter(|(position, _)| rank[*position] != usize::MAX)
            .collect();
        selected.sort_by_key(|(position, _)| rank[*position]);
        Ok(selected
            .into_iter()
            .map(|(_, wrapper)| ChildScorable::new(wrapper.scorable(), "SHOULD"))
            .collect())
    }
}

impl Scorer for WANDScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.core.doc
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(&mut self.core)
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        // Propagate to improve score bounds
        for position in 0..self.core.wrappers.len() {
            let scorer = self.core.wrappers[position].scorer();
            if Scorer::doc_id(scorer) < target {
                scorer.advance_shallow(target)?;
            }
        }
        if target <= self.core.up_to {
            return Ok(self.core.up_to);
        }
        // TODO(lucene): implement
        Ok(NO_MORE_DOCS)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let num_scorers = self.core.wrappers.len();
        let mut max_score_sum = 0.0f64;
        for position in 0..num_scorers {
            let scorer = self.core.wrappers[position].scorer();
            if Scorer::doc_id(scorer) <= up_to {
                max_score_sum += f64::from(scorer.get_max_score(up_to)?);
            }
        }
        Ok(MathUtil::sum_upper_bound(max_score_sum, num_scorers as i32) as f32)
    }
}
