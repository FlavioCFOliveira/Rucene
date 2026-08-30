//! Bucket-table disjunction scoring, ported from
//! `org.apache.lucene.search.BooleanScorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::index::DocAndFloatFeatureBuffer;
use crate::search::bit_set_doc_id_stream::BitSetDocIdStream;
use crate::search::bit_set_util;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::index_priority_queue::{IndexOrder, IndexPriorityQueue};
use crate::search::scorable::SimpleScorable;
use crate::search::scorer::Scorer;
use crate::search::scorer_util::ScorerUtil;
use crate::util::{Bits, FixedBitSet};

/// The base-two logarithm of the window size.
///
/// Equivalent to `BooleanScorer.SHIFT`.
const SHIFT: i32 = 12;
/// The number of documents scored per batch.
///
/// Equivalent to `BooleanScorer.SIZE`.
const SIZE: i32 = 1 << SHIFT;
/// The mask that turns a doc ID into its position inside a window.
///
/// Equivalent to `BooleanScorer.MASK`.
const MASK: i32 = SIZE - 1;

/// One entry of the bucket table: the running score and match count of a
/// document inside the current window.
///
/// Equivalent to the static `BooleanScorer.Bucket`.
#[derive(Debug, Default, Clone, Copy)]
struct Bucket {
    score: f64,
    freq: i32,
}

/// Orders clauses by the document they are positioned on.
///
/// Equivalent to `BooleanScorer.HeadPriorityQueue`.
struct ByDoc;

impl IndexOrder<DisiWrapper> for ByDoc {
    fn less_than(a: &DisiWrapper, b: &DisiWrapper) -> bool {
        a.doc < b.doc
    }
}

/// Orders clauses by cost.
///
/// Equivalent to `BooleanScorer.TailPriorityQueue`, whose `get(int)` is
/// [`IndexPriorityQueue::get`].
struct ByCost;

impl IndexOrder<DisiWrapper> for ByCost {
    fn less_than(a: &DisiWrapper, b: &DisiWrapper) -> bool {
        a.cost < b.cost
    }
}

/// A [`BulkScorer`] used for pure disjunctions, and for disjunctions that have
/// low values of minimum-should-match and dense clauses.
///
/// Equivalent to the `final org.apache.lucene.search.BooleanScorer`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and
/// [`BooleanScorerSupplier`](crate::search::BooleanScorerSupplier), which
/// builds it, lives in a sibling module. It scores documents in batches of
/// 4,096.
///
/// **Divergence from Lucene 10.5.0.** Java's two priority queues hold
/// references to the clauses whose `doc` field the scorer mutates while they sit
/// in the heap. Rust forbids that aliasing, so the clauses live in an array
/// owned by this scorer and the heaps hold *positions* into it — see
/// [`IndexPriorityQueue`], whose heap layout is Lucene's, so the order in which
/// equal clauses come out is unchanged.
pub struct BooleanScorer {
    /// One bucket per doc ID in the window, present when scores are needed or
    /// when frequencies have to be counted.
    buckets: Option<Vec<Bucket>>,
    matching: FixedBitSet,
    wrappers: Vec<DisiWrapper>,
    leads: Vec<usize>,
    head: IndexPriorityQueue<DisiWrapper, ByDoc>,
    tail: IndexPriorityQueue<DisiWrapper, ByCost>,
    score: SimpleScorable,
    min_should_match: usize,
    cost: i64,
    needs_scores: bool,
    doc_and_score_buffer: DocAndFloatFeatureBuffer,
}

impl std::fmt::Debug for BooleanScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BooleanScorer")
            .field("clauses", &self.wrappers.len())
            .field("min_should_match", &self.min_should_match)
            .field("needs_scores", &self.needs_scores)
            .field("cost", &self.cost)
            .finish()
    }
}

/// Message used where a heap is known to be non-empty at that point of the
/// algorithm; Java would raise a `NullPointerException` instead.
const NON_EMPTY_INVARIANT: &str =
    "INVARIANT: the head queue holds at least one clause at this point";

impl BooleanScorer {
    /// Builds a bucket-table disjunction scorer.
    ///
    /// Equivalent to
    /// `BooleanScorer(Collection<Scorer>, int, boolean)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `min_should_match` is
    /// outside `1..=scorers.len()` or when fewer than two scorers are supplied,
    /// which are the two `IllegalArgumentException`s Java throws, and
    /// propagates the priority-queue construction error of
    /// [`ScorerUtil::cost_with_min_should_match`].
    pub fn new(
        scorers: Vec<Box<dyn Scorer>>,
        min_should_match: usize,
        needs_scores: bool,
    ) -> Result<Self> {
        if min_should_match < 1 || min_should_match > scorers.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "minShouldMatch should be within 1..num_scorers. Got {min_should_match}"
            )));
        }
        if scorers.len() <= 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "This scorer can only be used with two scorers or more, got {}",
                scorers.len()
            )));
        }
        let buckets = if needs_scores || min_should_match > 1 {
            Some(vec![Bucket::default(); SIZE as usize])
        } else {
            None
        };
        let num_scorers = scorers.len();
        let leads = vec![0usize; num_scorers];
        let mut head: IndexPriorityQueue<DisiWrapper, ByDoc> =
            IndexPriorityQueue::new(num_scorers - min_should_match + 1);
        let mut tail: IndexPriorityQueue<DisiWrapper, ByCost> =
            IndexPriorityQueue::new(min_should_match - 1);

        let mut wrappers: Vec<DisiWrapper> = Vec::with_capacity(num_scorers);
        let mut costs: Vec<i64> = Vec::with_capacity(num_scorers);
        for scorer in scorers {
            let wrapper = DisiWrapper::new(scorer, false);
            costs.push(wrapper.cost);
            wrappers.push(wrapper);
            let position = wrappers.len() - 1;
            if let Some(evicted) = tail.insert_with_overflow(&wrappers, position) {
                head.add(&wrappers, evicted);
            }
        }
        let cost = ScorerUtil::cost_with_min_should_match(
            costs.iter().copied(),
            num_scorers,
            min_should_match,
        )?;

        Ok(Self {
            buckets,
            matching: FixedBitSet::new(SIZE as usize),
            wrappers,
            leads,
            head,
            tail,
            score: SimpleScorable::new(),
            min_should_match,
            cost,
            needs_scores,
            doc_and_score_buffer: DocAndFloatFeatureBuffer::new(),
        })
    }

    /// Equivalent to the private
    /// `BooleanScorer.scoreWindowIntoBitSetAndReplay`.
    fn score_window_into_bit_set_and_replay(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        base: i32,
        min: i32,
        max: i32,
        num_scorers: usize,
    ) -> CollectionResult<()> {
        let has_buckets = self.buckets.is_some();
        for i in 0..num_scorers {
            let w = self.leads[i];
            debug_assert!(self.wrappers[w].doc < max);

            if self.wrappers[w].doc < min {
                self.wrappers[w].iterator().advance(min)?;
            }
            if !has_buckets {
                // This doesn't apply live docs, so we'll need to apply them
                // later.
                let Self {
                    wrappers, matching, ..
                } = self;
                wrappers[w].iterator().into_bit_set(max, matching, base)?;
            } else if self.needs_scores {
                loop {
                    {
                        let Self {
                            wrappers,
                            doc_and_score_buffer,
                            ..
                        } = self;
                        wrappers[w].scorer().next_docs_and_scores(
                            max,
                            accept_docs,
                            doc_and_score_buffer,
                        )?;
                    }
                    if self.doc_and_score_buffer.size == 0 {
                        break;
                    }
                    for index in 0..self.doc_and_score_buffer.size {
                        let doc = self.doc_and_score_buffer.docs[index];
                        let score = self.doc_and_score_buffer.features[index];
                        let d = (doc & MASK) as usize;
                        self.matching.set(d);
                        let bucket =
                            &mut self.buckets.as_mut().expect(
                                "INVARIANT: the bucket table was just observed to be present",
                            )[d];
                        bucket.freq += 1;
                        bucket.score += f64::from(score);
                    }
                }
            } else {
                // Scores are not needed but we need to keep track of freqs to
                // know which hits match.
                debug_assert!(self.min_should_match > 1);
                let mut doc = self.wrappers[w].iterator().doc_id();
                while doc < max {
                    if accept_docs.map_or(true, |bits| bits.get(doc as usize)) {
                        let d = (doc & MASK) as usize;
                        self.matching.set(d);
                        let bucket =
                            &mut self.buckets.as_mut().expect(
                                "INVARIANT: the bucket table was just observed to be present",
                            )[d];
                        bucket.freq += 1;
                    }
                    doc = self.wrappers[w].iterator().next_doc()?;
                }
            }

            self.wrappers[w].doc = self.wrappers[w].iterator().doc_id();
        }

        if !has_buckets {
            if let Some(accept_docs) = accept_docs {
                // In this case, live docs have not been applied yet.
                bit_set_util::apply_mask(accept_docs, &mut self.matching, base);
            }
            let Self {
                matching, score, ..
            } = self;
            let mut stream = BitSetDocIdStream::new(matching, base);
            collector.collect_stream(&mut stream, score)?;
        } else {
            let Self {
                matching,
                buckets,
                score,
                min_should_match,
                ..
            } = self;
            let buckets = buckets
                .as_mut()
                .expect("INVARIANT: the bucket table was just observed to be present");
            let min_should_match = *min_should_match as i32;
            let bit_array = matching.get_bits();
            for (idx, word) in bit_array.iter().enumerate() {
                let mut bits = *word;
                while bits != 0 {
                    let ntz = bits.trailing_zeros() as usize;
                    let index_in_window = (idx << 6) | ntz;
                    let bucket = &mut buckets[index_in_window];
                    if bucket.freq >= min_should_match {
                        score.set_score(bucket.score as f32);
                        collector.collect(base | index_in_window as i32, score)?;
                    }
                    bucket.freq = 0;
                    bucket.score = 0.0;
                    bits ^= 1u64 << ntz;
                }
            }
        }

        self.matching.clear_all();
        Ok(())
    }

    /// Equivalent to the private `BooleanScorer.advance(int)`.
    fn advance_to(&mut self, min: i32) -> Result<usize> {
        debug_assert_eq!(self.tail.size(), self.min_should_match - 1);
        let mut head_top = self.head.top().expect(NON_EMPTY_INVARIANT);
        let mut tail_top = self.tail.top();
        while self.wrappers[head_top].doc < min {
            let swap = match tail_top {
                None => false,
                Some(tail_position) => {
                    self.wrappers[head_top].cost > self.wrappers[tail_position].cost
                }
            };
            if !swap {
                let doc = self.wrappers[head_top].iterator().advance(min)?;
                self.wrappers[head_top].doc = doc;
                head_top = self
                    .head
                    .update_top(&self.wrappers)
                    .expect(NON_EMPTY_INVARIANT);
            } else {
                // swap the top of head and tail
                let previous_head_top = head_top;
                let tail_position = tail_top.expect("INVARIANT: swap implies a tail top");
                let doc = self.wrappers[tail_position].iterator().advance(min)?;
                self.wrappers[tail_position].doc = doc;
                head_top = self
                    .head
                    .update_top_with(&self.wrappers, tail_position)
                    .expect(NON_EMPTY_INVARIANT);
                tail_top = self.tail.update_top_with(&self.wrappers, previous_head_top);
            }
        }
        Ok(head_top)
    }

    /// Equivalent to the private `BooleanScorer.scoreWindowMultipleScorers`.
    fn score_window_multiple_scorers(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        window_base: i32,
        window_min: i32,
        window_max: i32,
        mut max_freq: usize,
    ) -> CollectionResult<()> {
        while max_freq < self.min_should_match
            && max_freq + self.tail.size() >= self.min_should_match
        {
            // a match is still possible
            let candidate = self
                .tail
                .pop(&self.wrappers)
                .expect("INVARIANT: the loop condition implies a non-empty tail");
            if self.wrappers[candidate].doc < window_min {
                let doc = self.wrappers[candidate].iterator().advance(window_min)?;
                self.wrappers[candidate].doc = doc;
            }
            if self.wrappers[candidate].doc < window_max {
                self.leads[max_freq] = candidate;
                max_freq += 1;
            } else {
                self.head.add(&self.wrappers, candidate);
            }
        }

        if max_freq >= self.min_should_match {
            // There might be matches in other scorers from the tail too
            for i in 0..self.tail.size() {
                self.leads[max_freq] = self.tail.get(i);
                max_freq += 1;
            }
            self.tail.clear();

            self.score_window_into_bit_set_and_replay(
                collector,
                accept_docs,
                window_base,
                window_min,
                window_max,
                max_freq,
            )?;
        }

        // Push back scorers into head and tail
        for i in 0..max_freq {
            let lead = self.leads[i];
            if let Some(evicted) = self.head.insert_with_overflow(&self.wrappers, lead) {
                self.tail.add(&self.wrappers, evicted);
            }
        }
        Ok(())
    }

    /// Equivalent to the private `BooleanScorer.scoreWindowSingleScorer`.
    fn score_window_single_scorer(
        &mut self,
        w: usize,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        window_min: i32,
        window_max: i32,
        max: i32,
    ) -> CollectionResult<()> {
        debug_assert_eq!(self.tail.size(), 0);
        let head_top = self.head.top().expect(NON_EMPTY_INVARIANT);
        let next_window_base = self.wrappers[head_top].doc & !MASK;
        let end = window_max.max(max.min(next_window_base));

        let mut doc = self.wrappers[w].doc;
        if doc < window_min {
            doc = self.wrappers[w].iterator().advance(window_min)?;
        }
        collector.set_scorer(self.wrappers[w].scorable())?;
        while doc < end {
            if accept_docs.map_or(true, |bits| bits.get(doc as usize)) {
                collector.collect(doc, self.wrappers[w].scorable())?;
            }
            doc = self.wrappers[w].iterator().next_doc()?;
        }
        self.wrappers[w].doc = doc;

        // reset the scorer that should be used for the general case
        collector.set_scorer(&mut self.score)?;
        Ok(())
    }

    /// Equivalent to the private `BooleanScorer.scoreWindow`.
    fn score_window(
        &mut self,
        top: usize,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<usize> {
        // find the window that the next match belongs to
        let window_base = self.wrappers[top].doc & !MASK;
        let window_min = min.max(window_base);
        let window_max = max.min(window_base.wrapping_add(SIZE));

        // Fill 'leads' with all scorers from 'head' that are in the right
        // window
        self.leads[0] = self.head.pop(&self.wrappers).expect(NON_EMPTY_INVARIANT);
        let mut max_freq = 1;
        while self.head.size() > 0
            && self.wrappers[self.head.top().expect(NON_EMPTY_INVARIANT)].doc < window_max
        {
            self.leads[max_freq] = self.head.pop(&self.wrappers).expect(NON_EMPTY_INVARIANT);
            max_freq += 1;
        }

        if self.min_should_match == 1 && max_freq == 1 {
            // special case: only one scorer can match in the current window, we
            // can collect directly
            let bulk_scorer = self.leads[0];
            self.score_window_single_scorer(
                bulk_scorer,
                collector,
                accept_docs,
                window_min,
                window_max,
                max,
            )?;
            Ok(self.head.add(&self.wrappers, bulk_scorer))
        } else {
            // general case, collect through a bit set first and then replay
            self.score_window_multiple_scorers(
                collector,
                accept_docs,
                window_base,
                window_min,
                window_max,
                max_freq,
            )?;
            Ok(self.head.top().expect(NON_EMPTY_INVARIANT))
        }
    }
}

impl BulkScorer for BooleanScorer {
    fn cost(&self) -> i64 {
        self.cost
    }

    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        collector.set_scorer(&mut self.score)?;

        let mut top = self.advance_to(min)?;
        while self.wrappers[top].doc < max {
            top = self.score_window(top, collector, accept_docs, min, max)?;
        }

        Ok(self.wrappers[top].doc)
    }
}
