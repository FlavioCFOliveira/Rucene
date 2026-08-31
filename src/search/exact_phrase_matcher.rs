//! Exact phrase matching, ported from
//! `org.apache.lucene.search.ExactPhraseMatcher`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::{FreqAndNormBuffer, Impacts, ImpactsSource};
use crate::search::conjunction_utils::ConjunctionUtils;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::impacts_disi::ImpactsDISI;
use crate::search::max_score_cache::MaxScoreCache;
use crate::search::phrase_matcher::{
    IteratorWithImpacts, PhraseImpactsDISI, PhraseMatcher, SharedPostings,
};
use crate::search::phrase_query::PostingsAndFreq;
use crate::search::score_mode::ScoreMode;
use crate::search::sim_scorer_source::{SharedSimScorer, SharedSimScorerRef};

/// One phrase term's postings and its state within the current document.
///
/// Equivalent to the private
/// `ExactPhraseMatcher.PostingsAndPosition`.
#[derive(Debug)]
struct PostingsAndPosition {
    postings: SharedPostings,
    offset: i32,
    freq: i32,
    up_to: i32,
    pos: i32,
}

/// Expert: finds exact phrases.
///
/// Equivalent to the `final org.apache.lucene.search.ExactPhraseMatcher`.
pub struct ExactPhraseMatcher {
    postings: Vec<PostingsAndPosition>,
    impacts_approximation: PhraseImpactsDISI,
    /// Whether [`approximation`](PhraseMatcher::approximation) is the
    /// impacts-aware iterator, which is what Java's `approximation` field holds
    /// under [`ScoreMode::TOP_SCORES`].
    use_impacts: bool,
    freqs_loaded: bool,
    match_cost: f32,
}

impl std::fmt::Debug for ExactPhraseMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExactPhraseMatcher")
            .field("terms", &self.postings.len())
            .field("match_cost", &self.match_cost)
            .finish_non_exhaustive()
    }
}

impl ExactPhraseMatcher {
    /// Expert: creates an exact phrase matcher.
    ///
    /// Equivalent to
    /// `ExactPhraseMatcher(PhraseQuery.PostingsAndFreq[], ScoreMode, SimScorer, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when fewer than two terms are supplied, which is what
    /// `ConjunctionUtils.intersectIterators` rejects.
    pub fn new(
        postings: Vec<PostingsAndFreq>,
        score_mode: ScoreMode,
        scorer: SharedSimScorer,
        match_cost: f32,
    ) -> Result<Self> {
        let iterators: Vec<Box<dyn DocIdSetIterator>> = postings
            .iter()
            .map(|p| Box::new(p.postings().clone()) as Box<dyn DocIdSetIterator>)
            .collect();
        let approximation = ConjunctionUtils::intersect_iterators(iterators)?;
        let impacts_source: Box<dyn ImpactsSource> = Box::new(merge_impacts(
            postings.iter().map(|p| p.postings().clone()).collect(),
        ));

        let impacts_approximation = ImpactsDISI::new(
            IteratorWithImpacts::from_scorer_iterator(approximation, impacts_source),
            MaxScoreCache::new(Box::new(SharedSimScorerRef::new(scorer))),
        );

        // TODO(lucene): only do this when this is the top-level scoring clause
        // (`ScorerSupplier#setTopLevelScoringClause`) to save the overhead of
        // wrapping with `ImpactsDISI` when it would not help. This port keeps
        // Lucene's current, unconditional behaviour.
        let use_impacts = score_mode == ScoreMode::TOP_SCORES;

        let postings_and_positions = postings
            .into_iter()
            .map(|posting| PostingsAndPosition {
                offset: posting.position(),
                postings: posting.into_postings(),
                freq: 0,
                up_to: 0,
                pos: -1,
            })
            .collect();

        Ok(Self {
            postings: postings_and_positions,
            impacts_approximation,
            use_impacts,
            freqs_loaded: false,
            match_cost,
        })
    }

    /// Advances the given position enum to the first position on or after
    /// `target`, returning `false` if the enum was exhausted before reaching it.
    ///
    /// Equivalent to the private static
    /// `ExactPhraseMatcher.advancePosition(PostingsAndPosition, int)`.
    fn advance_position(posting: &mut PostingsAndPosition, target: i32) -> Result<bool> {
        while posting.pos < target {
            if posting.up_to == posting.freq {
                return Ok(false);
            }
            posting.pos = posting.postings.next_position()?;
            posting.up_to += 1;
        }
        Ok(true)
    }
}

impl PhraseMatcher for ExactPhraseMatcher {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.use_impacts {
            &mut self.impacts_approximation
        } else {
            self.impacts_approximation.inner()
        }
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        if self.use_impacts {
            &self.impacts_approximation
        } else {
            self.impacts_approximation.inner_ref()
        }
    }

    fn max_freq(&mut self) -> Result<f32> {
        // Load freqs eagerly so that `max_freq` can be called before
        // `reset_positions` in TOP_SCORES mode. `PhraseScorer` uses this to
        // short-circuit non-competitive documents before paying the cost of
        // `reset_positions` and `next_match`.
        let mut min_freq = self.postings[0].postings.freq()?;
        self.postings[0].freq = min_freq;
        for i in 1..self.postings.len() {
            let f = self.postings[i].postings.freq()?;
            self.postings[i].freq = f;
            min_freq = min_freq.min(f);
        }
        self.freqs_loaded = true;
        Ok(min_freq as f32)
    }

    fn reset_positions(&mut self) -> Result<()> {
        if self.freqs_loaded {
            // Freqs were already loaded by `max_freq`; only reset the position
            // state.
            self.freqs_loaded = false;
            for posting in &mut self.postings {
                posting.pos = -1;
                posting.up_to = 0;
            }
        } else {
            // Freqs not yet loaded; the original single-loop path.
            for posting in &mut self.postings {
                posting.freq = posting.postings.freq()?;
                posting.pos = -1;
                posting.up_to = 0;
            }
        }
        Ok(())
    }

    fn next_match(&mut self) -> Result<bool> {
        {
            let lead = &mut self.postings[0];
            if lead.up_to < lead.freq {
                lead.pos = lead.postings.next_position()?;
                lead.up_to += 1;
            } else {
                return Ok(false);
            }
        }
        'advance_head: loop {
            let phrase_pos = self.postings[0].pos - self.postings[0].offset;
            for j in 1..self.postings.len() {
                let expected_pos = phrase_pos + self.postings[j].offset;

                // Advance up to the same position as the lead.
                if !Self::advance_position(&mut self.postings[j], expected_pos)? {
                    break 'advance_head;
                }

                if self.postings[j].pos != expected_pos {
                    // We advanced too far.
                    let target =
                        self.postings[j].pos - self.postings[j].offset + self.postings[0].offset;
                    if Self::advance_position(&mut self.postings[0], target)? {
                        continue 'advance_head;
                    } else {
                        break 'advance_head;
                    }
                }
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn sloppy_weight(&self) -> f32 {
        1.0
    }

    fn start_position(&self) -> i32 {
        self.postings[0].pos
    }

    fn end_position(&self) -> i32 {
        self.postings[self.postings.len() - 1].pos
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(self.postings[0].postings.start_offset())
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(self.postings[self.postings.len() - 1].postings.end_offset())
    }

    fn get_match_cost(&self) -> f32 {
        self.match_cost
    }

    fn set_min_competitive_score(&mut self, min_score: f32) {
        self.impacts_approximation
            .set_min_competitive_score(min_score);
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        let (source, cache) = self.impacts_approximation.split_mut();
        cache.advance_shallow(source, target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let (source, cache) = self.impacts_approximation.split_mut();
        cache.get_max_score(source, up_to)
    }
}

/// Merges the impacts of the several terms of an exact phrase.
///
/// Equivalent to the package-private static
/// `ExactPhraseMatcher.mergeImpacts(ImpactsEnum[])`.
pub fn merge_impacts(impacts_enums: Vec<SharedPostings>) -> MergedImpactsSource {
    // Iteration of block boundaries uses the impacts enum with the lower cost;
    // this is consistent with `BlockMaxConjunctionScorer`.
    let mut lead_index = 0usize;
    let mut lead_set = false;
    for (i, e) in impacts_enums.iter().enumerate() {
        if !lead_set || e.cost() < impacts_enums[lead_index].cost() {
            lead_index = i;
            lead_set = true;
        }
    }
    MergedImpactsSource {
        impacts_enums,
        lead_index,
    }
}

/// The [`ImpactsSource`] [`merge_impacts`] builds.
///
/// Equivalent to the anonymous `ImpactsSource` of
/// `ExactPhraseMatcher.mergeImpacts`.
pub struct MergedImpactsSource {
    impacts_enums: Vec<SharedPostings>,
    lead_index: usize,
}

impl std::fmt::Debug for MergedImpactsSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergedImpactsSource")
            .field("terms", &self.impacts_enums.len())
            .field("lead_index", &self.lead_index)
            .finish()
    }
}

impl ImpactsSource for MergedImpactsSource {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        for impacts_enum in &mut self.impacts_enums {
            impacts_enum.advance_shallow(target)?;
        }
        Ok(())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        let mut impacts = Vec::with_capacity(self.impacts_enums.len());
        for impacts_enum in &mut self.impacts_enums {
            impacts.push(impacts_enum.get_impacts()?);
        }
        Ok(Box::new(MergedImpacts {
            impacts,
            lead_index: self.lead_index,
        }))
    }
}

/// The merged view of the impacts of every phrase term.
///
/// Equivalent to the anonymous `Impacts` of
/// `ExactPhraseMatcher.mergeImpacts`.
struct MergedImpacts {
    impacts: Vec<Box<dyn Impacts>>,
    lead_index: usize,
}

/// One term's impact list, walked in parallel with the others.
///
/// Equivalent to the static nested `SubIterator` of
/// `ExactPhraseMatcher.mergeImpacts`.
struct SubIterator {
    buffer: FreqAndNormBuffer,
    index: usize,
    freq: i32,
    norm: i64,
    exhausted: bool,
}

impl SubIterator {
    fn new(buffer: FreqAndNormBuffer) -> Self {
        let mut it = Self {
            buffer,
            index: 0,
            freq: 0,
            norm: 0,
            exhausted: false,
        };
        it.next();
        it
    }

    fn next(&mut self) {
        if self.index >= self.buffer.size {
            self.exhausted = true;
        } else {
            self.freq = self.buffer.freqs[self.index];
            self.norm = self.buffer.norms[self.index];
            self.index += 1;
        }
    }
}

/// A binary heap of [`SubIterator`]s ordered by frequency.
///
/// Equivalent to the anonymous `PriorityQueue<SubIterator>` of
/// `ExactPhraseMatcher.mergeImpacts`.
///
/// **Divergence from Lucene 10.5.0.** This is a local heap rather than
/// [`crate::util::PriorityQueue`], because the algorithm mutates the top
/// element in place — `top.next()` followed by `pq.updateTop()` — which the
/// shared queue cannot express: its `top()` hands out a shared reference.
/// `up_heap` and `down_heap` reproduce `org.apache.lucene.util.PriorityQueue`
/// exactly.
struct SubIteratorQueue {
    heap: Vec<Option<SubIterator>>,
    size: usize,
}

impl SubIteratorQueue {
    fn new(max_size: usize) -> Self {
        let heap_size = if max_size == 0 { 2 } else { max_size + 1 };
        Self {
            heap: (0..heap_size).map(|_| None).collect(),
            size: 0,
        }
    }

    fn less_than(a: &SubIterator, b: &SubIterator) -> bool {
        a.freq < b.freq
    }

    fn add(&mut self, element: SubIterator) {
        let index = self.size + 1;
        self.heap[index] = Some(element);
        self.size = index;
        self.up_heap(index);
    }

    fn top(&self) -> &SubIterator {
        self.heap[1]
            .as_ref()
            .expect("INVARIANT: the queue is not empty while merging impacts")
    }

    fn top_mut(&mut self) -> &mut SubIterator {
        self.heap[1]
            .as_mut()
            .expect("INVARIANT: the queue is not empty while merging impacts")
    }

    fn iter(&self) -> impl Iterator<Item = &SubIterator> {
        self.heap[1..=self.size].iter().filter_map(|s| s.as_ref())
    }

    fn update_top(&mut self) {
        self.down_heap(1);
    }

    fn up_heap(&mut self, orig_pos: usize) {
        let mut i = orig_pos;
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: up_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i >> 1;
        while j > 0 && Self::less_than(&node, self.heap[j].as_ref().expect(occupied)) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j >>= 1;
        }
        self.heap[i] = Some(node);
    }

    fn down_heap(&mut self, mut i: usize) {
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: down_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size
            && Self::less_than(
                self.heap[k].as_ref().expect(occupied),
                self.heap[j].as_ref().expect(occupied),
            )
        {
            j = k;
        }
        while j <= self.size && Self::less_than(self.heap[j].as_ref().expect(occupied), &node) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size
                && Self::less_than(
                    self.heap[k].as_ref().expect(occupied),
                    self.heap[j].as_ref().expect(occupied),
                )
            {
                j = k;
            }
        }
        self.heap[i] = Some(node);
    }
}

impl MergedImpacts {
    /// Returns the minimum level whose impacts are valid up to `doc_id_up_to`,
    /// or `-1` if there is no such level.
    ///
    /// Equivalent to the private `getLevel(Impacts, int)`.
    fn get_level(impacts: &dyn Impacts, doc_id_up_to: i32) -> i32 {
        for level in 0..impacts.num_levels() {
            if impacts.doc_id_up_to(level) >= doc_id_up_to {
                return level;
            }
        }
        -1
    }
}

impl Impacts for MergedImpacts {
    fn num_levels(&self) -> i32 {
        // Delegate to the lead.
        self.impacts[self.lead_index].num_levels()
    }

    fn doc_id_up_to(&self, level: i32) -> i32 {
        // Delegate to the lead.
        self.impacts[self.lead_index].doc_id_up_to(level)
    }

    fn get_impacts(&self, level: i32) -> FreqAndNormBuffer {
        let doc_id_up_to = self.doc_id_up_to(level);
        let mut merged_impacts = FreqAndNormBuffer::new();
        merged_impacts.grow_no_copy(1);

        let mut has_impacts = false;
        let mut only_impact_list: Option<FreqAndNormBuffer> = None;
        let mut sub_iterators: Vec<SubIterator> = Vec::with_capacity(self.impacts.len());
        for impacts in &self.impacts {
            let impacts_level = Self::get_level(&**impacts, doc_id_up_to);
            if impacts_level == -1 {
                // This instance does not have useful impacts; ignoring it is
                // safe.
                continue;
            }

            let impact_list = impacts.get_impacts(impacts_level);
            if impact_list.size > 0 && impact_list.freqs[0] == i32::MAX && impact_list.norms[0] == 1
            {
                // Dummy impacts, ignore them too.
                continue;
            }

            if !has_impacts {
                has_impacts = true;
                only_impact_list = Some(impact_list.clone());
            } else {
                // There are multiple impacts.
                only_impact_list = None;
            }
            sub_iterators.push(SubIterator::new(impact_list));
        }

        if !has_impacts {
            merged_impacts.freqs[0] = i32::MAX;
            merged_impacts.norms[0] = 1;
            merged_impacts.size = 1;
            return merged_impacts;
        } else if let Some(only_impact_list) = only_impact_list {
            return only_impact_list;
        }

        // Idea: merge impacts by freq. The tricky thing is that freq values not
        // in the impacts must be considered too. For instance if the list of
        // impacts is `[{freq=2,norm=10}, {freq=4,norm=12}]`, there might well be
        // a document with a freq of 2 and a length of 11, which was just not
        // added to the list of impacts because `{freq=2,norm=10}` is more
        // competitive. The impacts are walked in parallel through a priority
        // queue ordered by freq: at any time, the competitive impact consists of
        // the lowest freq among all entries of the queue — the top — and the
        // highest norm, tracked separately.
        let mut pq = SubIteratorQueue::new(sub_iterators.len());
        for sub_iterator in sub_iterators {
            pq.add(sub_iterator);
        }
        merged_impacts.size = 0;
        let mut current_freq = pq.top().freq;
        let mut current_norm = 0i64;
        for it in pq.iter() {
            if (it.norm as u64) > (current_norm as u64) {
                current_norm = it.norm;
            }
        }

        'outer: loop {
            if merged_impacts.size > 0
                && merged_impacts.norms[merged_impacts.size - 1] == current_norm
            {
                merged_impacts.freqs[merged_impacts.size - 1] = current_freq;
            } else {
                merged_impacts.add(current_freq, current_norm);
            }

            loop {
                {
                    let top = pq.top_mut();
                    top.next();
                    if top.exhausted {
                        // At least one clause does not have any more documents
                        // below the current norm, so further clauses can safely
                        // be ignored: the only reason they have more impacts is
                        // that they cover more documents we are not interested
                        // in.
                        break 'outer;
                    }
                    if (top.norm as u64) > (current_norm as u64) {
                        current_norm = top.norm;
                    }
                }
                pq.update_top();
                if pq.top().freq != current_freq {
                    break;
                }
            }

            current_freq = pq.top().freq;
        }

        merged_impacts
    }
}
