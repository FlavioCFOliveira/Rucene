//! Sloppy phrase matching, ported from
//! `org.apache.lucene.search.SloppyPhraseMatcher`.

#![deny(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::error::Result;
use crate::index::{ImpactsSource, Term};
use crate::search::conjunction_utils::ConjunctionUtils;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::impacts_disi::ImpactsDISI;
use crate::search::max_score_cache::MaxScoreCache;
use crate::search::phrase_matcher::{
    DummyImpactsSource, IteratorWithImpacts, PhraseImpactsDISI, PhraseMatcher,
};
use crate::search::phrase_positions::PhrasePositions;
use crate::search::phrase_query::PostingsAndFreq;
use crate::search::phrase_queue::PhraseQueue;
use crate::search::score_mode::ScoreMode;
use crate::search::sim_scorer_source::{SharedSimScorer, SharedSimScorerRef};

/// A small growable bit set over the repeating terms of a phrase.
///
/// **Divergence from Lucene 10.5.0.** Java uses
/// `org.apache.lucene.util.FixedBitSet` here, for its `intersects`, `or`,
/// `nextSetBit`, `cardinality` and `ensureCapacity` operations;
/// [`crate::util::FixedBitSet`] does not expose the first four, and that module
/// is not part of this batch. This local set implements exactly those
/// operations over one boolean per bit. The sets are as large as the number of
/// repeating terms of a phrase — a handful — so the representation costs
/// nothing observable, and every operation has the same meaning.
#[derive(Debug, Clone, Default)]
struct TermBits {
    bits: Vec<bool>,
}

impl TermBits {
    fn new(length: usize) -> Self {
        Self {
            bits: vec![false; length],
        }
    }

    fn set(&mut self, index: usize) {
        if index >= self.bits.len() {
            self.bits.resize(index + 1, false);
        }
        self.bits[index] = true;
    }

    fn get(&self, index: usize) -> bool {
        self.bits.get(index).copied().unwrap_or(false)
    }

    fn clear(&mut self, index: usize) {
        if index < self.bits.len() {
            self.bits[index] = false;
        }
    }

    fn cardinality(&self) -> usize {
        self.bits.iter().filter(|b| **b).count()
    }

    fn length(&self) -> usize {
        self.bits.len()
    }

    fn ensure_capacity(&mut self, index: usize) {
        if index >= self.bits.len() {
            self.bits.resize(index + 1, false);
        }
    }

    fn intersects(&self, other: &TermBits) -> bool {
        self.bits.iter().zip(&other.bits).any(|(a, b)| *a && *b)
    }

    fn or(&mut self, other: &TermBits) {
        if other.bits.len() > self.bits.len() {
            self.bits.resize(other.bits.len(), false);
        }
        for (i, b) in other.bits.iter().enumerate() {
            if *b {
                self.bits[i] = true;
            }
        }
    }

    fn set_bits(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits
            .iter()
            .enumerate()
            .filter_map(|(i, b)| if *b { Some(i) } else { None })
    }
}

/// Finds all slop-valid position combinations encountered while traversing the
/// [`PhrasePositions`].
///
/// Equivalent to the `final org.apache.lucene.search.SloppyPhraseMatcher`. The
/// sloppy frequency contribution of a match depends on the distance: it is
/// highest for distance `0`, an exact match, and gets lower as the distance
/// grows. For the query `"a b"~2`, a document `x a b a y` can be matched twice:
/// once for `a b`, at distance `0`, and once for `b a`, at distance `2`.
///
/// Possibly not all valid combinations are encountered, because for efficiency
/// the least phrase position is always propagated, which allows basing the
/// search on a priority queue and moving forward faster. As a result, the
/// document `a b c b a` scores differently for the queries `"a b c"~4` and
/// `"c b a"~4`, although they really are equivalent; similarly, for the
/// document `a b c b a f g`, the query `"c b"~2` gets the same score as
/// `"g f"~2`, although `"c b"~2` could be matched twice.
pub struct SloppyPhraseMatcher {
    phrase_positions: Vec<PhrasePositions>,
    slop: i32,
    num_postings: usize,
    /// For advancing the minimum position.
    pq: PhraseQueue,
    capture_lead_match: bool,
    impacts_approximation: PhraseImpactsDISI,
    /// Whether the approximation is the impacts-aware iterator. Java's
    /// `approximation` field is always the plain conjunction here, because
    /// sloppy phrases use dummy impacts; the flag keeps the shape of
    /// [`ExactPhraseMatcher`](crate::search::ExactPhraseMatcher) so that the
    /// two read alike.
    use_impacts: bool,
    /// The current largest phrase position.
    end: i32,
    lead_position: i32,
    lead_offset: i32,
    lead_end_offset: i32,
    lead_ord: i32,
    /// Indicates that there are repetitions, as checked in the first candidate
    /// document.
    has_rpts: bool,
    /// Only check for repetitions in the first candidate document.
    checked_rpts: bool,
    has_multi_term_rpts: bool,
    /// Each group holds the phrase positions that repeat each other — that is,
    /// carry the same term — sorted by their query offset.
    rpt_groups: Vec<Vec<usize>>,
    /// A temporary stack for switching colliding repeating positions.
    rpt_stack: Vec<usize>,
    positioned: bool,
    match_length: i32,
    freqs_loaded: bool,
    match_cost: f32,
}

impl std::fmt::Debug for SloppyPhraseMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SloppyPhraseMatcher")
            .field("slop", &self.slop)
            .field("terms", &self.num_postings)
            .field("match_cost", &self.match_cost)
            .finish_non_exhaustive()
    }
}

impl SloppyPhraseMatcher {
    /// Creates a sloppy phrase matcher.
    ///
    /// Equivalent to
    /// `SloppyPhraseMatcher(PhraseQuery.PostingsAndFreq[], int, ScoreMode, SimScorer, float, boolean)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when fewer than two terms are supplied, which is what
    /// `ConjunctionUtils.intersectIterators` rejects.
    pub fn new(
        postings: Vec<PostingsAndFreq>,
        slop: i32,
        score_mode: ScoreMode,
        scorer: SharedSimScorer,
        match_cost: f32,
        capture_lead_match: bool,
    ) -> Result<Self> {
        let num_postings = postings.len();
        let pq = PhraseQueue::new(num_postings);

        let iterators: Vec<Box<dyn DocIdSetIterator>> = postings
            .iter()
            .map(|p| Box::new(p.postings().clone()) as Box<dyn DocIdSetIterator>)
            .collect();
        let approximation = ConjunctionUtils::intersect_iterators(iterators)?;

        let mut phrase_positions = Vec::with_capacity(num_postings);
        for (i, posting) in postings.into_iter().enumerate() {
            let position = posting.position();
            let terms = posting.terms().to_vec();
            phrase_positions.push(PhrasePositions::new(
                posting.into_postings(),
                position,
                i as i32,
                terms,
            ));
        }

        // What would be a good upper bound of the sloppy frequency? A sum of
        // the sub frequencies would be correct, but it is usually so much
        // higher than the actual sloppy frequency that it does not help skip
        // irrelevant documents. As a consequence, for now, sloppy phrase
        // queries use dummy impacts.
        let impacts_source: Box<dyn ImpactsSource> = Box::new(DummyImpactsSource);
        let impacts_approximation = ImpactsDISI::new(
            IteratorWithImpacts::from_scorer_iterator(approximation, impacts_source),
            MaxScoreCache::new(Box::new(SharedSimScorerRef::new(scorer))),
        );

        // Java's `approximation` field is the plain conjunction, whatever the
        // score mode; the impacts-aware wrapper is only used for the score
        // cache and the minimum competitive score.
        let use_impacts = false;
        let _ = score_mode;

        Ok(Self {
            phrase_positions,
            slop,
            num_postings,
            pq,
            capture_lead_match,
            impacts_approximation,
            use_impacts,
            end: i32::MIN,
            lead_position: i32::MAX,
            lead_offset: 0,
            lead_end_offset: 0,
            lead_ord: 0,
            has_rpts: false,
            checked_rpts: false,
            has_multi_term_rpts: false,
            rpt_groups: Vec::new(),
            rpt_stack: Vec::new(),
            positioned: false,
            match_length: i32::MAX,
            freqs_loaded: false,
            match_cost,
        })
    }

    /// Advances a phrase position and updates `end`, returning `false` when it
    /// is exhausted.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.advancePP(PhrasePositions)`.
    fn advance_pp(&mut self, pp: usize) -> Result<bool> {
        if !self.phrase_positions[pp].next_position()? {
            return Ok(false);
        }
        if self.phrase_positions[pp].position > self.end {
            self.end = self.phrase_positions[pp].position;
        }
        Ok(true)
    }

    /// Captures the lead match, when offsets are exposed.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.captureLead(PhrasePositions)`.
    fn capture_lead(&mut self, pp: usize) -> Result<()> {
        if !self.capture_lead_match {
            return Ok(());
        }
        let position = self.phrase_positions[pp].position + self.phrase_positions[pp].offset;
        self.lead_ord = self.phrase_positions[pp].ord;
        self.lead_position = position;
        self.lead_offset = self.phrase_positions[pp].postings.start_offset();
        self.lead_end_offset = self.phrase_positions[pp].postings.end_offset();
        Ok(())
    }

    /// Resolves a repeater collision by advancing the lesser of the two
    /// colliding positions.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.advanceRpts(PhrasePositions)`. There can only be
    /// one collision, since by the initialisation there were none before `pp`
    /// was advanced.
    fn advance_rpts(&mut self, mut pp: usize) -> Result<bool> {
        if self.phrase_positions[pp].rpt_group < 0 {
            // Not a repeater.
            return Ok(true);
        }
        let group = self.phrase_positions[pp].rpt_group as usize;
        let rg_len = self.rpt_groups[group].len();
        // For re-queuing after the collisions are resolved.
        let mut bits = TermBits::new(rg_len);
        let k0 = self.phrase_positions[pp].rpt_ind;
        loop {
            let k = self.collide(pp);
            if k < 0 {
                break;
            }
            let other = self.rpt_groups[group][k as usize];
            // Always advance the lesser of the (only) two colliding positions.
            pp = self.lesser(pp, other);
            if !self.advance_pp(pp)? {
                // Exhausted.
                return Ok(false);
            }
            if k != k0 {
                // Careful: mark only those currently in the queue.
                bits.ensure_capacity(k as usize);
                // Mark that the other position needs to be re-queued.
                bits.set(k as usize);
            }
        }
        // Collisions resolved, now re-queue: empty (partially) the queue until
        // every position advanced for resolving collisions has been seen.
        let mut n = 0usize;
        let num_bits = bits.length();
        while bits.cardinality() > 0 {
            let Some(pp2) = self.pq.pop(&self.phrase_positions) else {
                break;
            };
            if self.rpt_stack.len() <= n {
                self.rpt_stack.resize(n + 1, 0);
            }
            self.rpt_stack[n] = pp2;
            n += 1;
            let rpt_group = self.phrase_positions[pp2].rpt_group;
            let rpt_ind = self.phrase_positions[pp2].rpt_ind;
            if rpt_group >= 0
                // This bit may not have been set.
                && (rpt_ind as usize) < num_bits
                && bits.get(rpt_ind as usize)
            {
                bits.clear(rpt_ind as usize);
            }
        }
        // Add back to the queue.
        for i in (0..n).rev() {
            self.pq.add(&self.phrase_positions, self.rpt_stack[i]);
        }
        Ok(true)
    }

    /// Compares two phrase positions, but only by position and offset.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.lesser(PhrasePositions, PhrasePositions)`.
    fn lesser(&self, pp: usize, pp2: usize) -> usize {
        let a = &self.phrase_positions[pp];
        let b = &self.phrase_positions[pp2];
        if a.position < b.position || (a.position == b.position && a.offset < b.offset) {
            pp
        } else {
            pp2
        }
    }

    /// Returns the index within the repeat group of a position colliding with
    /// `pp`, or `-1` if there is none.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.collide(PhrasePositions)`.
    fn collide(&self, pp: usize) -> i32 {
        let tp_pos = self.tp_pos(pp);
        let group = self.phrase_positions[pp].rpt_group as usize;
        for &pp2 in &self.rpt_groups[group] {
            if pp2 != pp && self.tp_pos(pp2) == tp_pos {
                return self.phrase_positions[pp2].rpt_ind;
            }
        }
        -1
    }

    /// The actual position in the document of a phrase position; this relies on
    /// `position = tpPos - offset`.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.tpPos(PhrasePositions)`.
    fn tp_pos(&self, pp: usize) -> i32 {
        self.phrase_positions[pp].position + self.phrase_positions[pp].offset
    }

    /// Initialises the phrase positions in place, a one-time initialisation for
    /// this matcher, on the first document matching all terms: check whether
    /// there are repetitions, and if there are, find the groups of repetitions.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.initPhrasePositions()`, which returns `false` when
    /// the positions are exhausted and the current document therefore does not
    /// match.
    fn init_phrase_positions(&mut self) -> Result<bool> {
        self.end = i32::MIN;
        if !self.checked_rpts {
            return self.init_first_time();
        }
        if !self.has_rpts {
            self.init_simple()?;
            // Positions available.
            return Ok(true);
        }
        self.init_complex()
    }

    /// No repeats: the simplest and most common case.
    ///
    /// Equivalent to the private `SloppyPhraseMatcher.initSimple()`.
    fn init_simple(&mut self) -> Result<()> {
        self.pq.clear();
        // Position the phrase positions and build the queue from the list.
        for i in 0..self.phrase_positions.len() {
            self.phrase_positions[i].first_position()?;
            if self.phrase_positions[i].position > self.end {
                self.end = self.phrase_positions[i].position;
            }
            self.pq.add(&self.phrase_positions, i);
        }
        Ok(())
    }

    /// With repeats: not so simple.
    ///
    /// Equivalent to the private `SloppyPhraseMatcher.initComplex()`.
    fn init_complex(&mut self) -> Result<bool> {
        self.place_first_positions()?;
        if !self.advance_repeat_groups()? {
            // Positions exhausted.
            return Ok(false);
        }
        self.fill_queue();
        // Positions available.
        Ok(true)
    }

    /// Moves every phrase position to its first position.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.placeFirstPositions()`.
    fn place_first_positions(&mut self) -> Result<()> {
        for pp in &mut self.phrase_positions {
            pp.first_position()?;
        }
        Ok(())
    }

    /// Fills the queue; all the phrase positions are already placed.
    ///
    /// Equivalent to the private `SloppyPhraseMatcher.fillQueue()`.
    fn fill_queue(&mut self) {
        self.pq.clear();
        for i in 0..self.phrase_positions.len() {
            if self.phrase_positions[i].position > self.end {
                self.end = self.phrase_positions[i].position;
            }
            self.pq.add(&self.phrase_positions, i);
        }
    }

    /// Advances the repeat groups at the start of a document.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.advanceRepeatGroups()`. At initialisation, each
    /// repetition group is sorted by query offset, which gives the start
    /// condition: no collisions.
    ///
    /// Case 1, no multi-term repeats: it is sufficient to advance each position
    /// in the group by one less than its group index, so the lesser one is not
    /// advanced, the second is advanced once, the third twice, and so on.
    ///
    /// Case 2, multi-term repeats: more involved, since some may not collide.
    fn advance_repeat_groups(&mut self) -> Result<bool> {
        for g in 0..self.rpt_groups.len() {
            if self.has_multi_term_rpts {
                // More involved: some may not collide.
                let mut i = 0usize;
                while i < self.rpt_groups[g].len() {
                    let mut incr = 1usize;
                    let pp = self.rpt_groups[g][i];
                    loop {
                        let k = self.collide(pp);
                        if k < 0 {
                            break;
                        }
                        let other = self.rpt_groups[g][k as usize];
                        let pp2 = self.lesser(pp, other);
                        // At initialisation, always advance the position with
                        // the higher offset.
                        if !self.advance_pp(pp2)? {
                            // Exhausted.
                            return Ok(false);
                        }
                        if (self.phrase_positions[pp2].rpt_ind as usize) < i {
                            // Should not happen?
                            incr = 0;
                            break;
                        }
                    }
                    // Java writes `for (int i = 0; i < rg.length; i += incr)`,
                    // so `incr == 0` re-runs the body with the same index.
                    i += incr;
                }
            } else {
                // Simpler: we know exactly how much to advance.
                for j in 1..self.rpt_groups[g].len() {
                    for _ in 0..j {
                        let pp = self.rpt_groups[g][j];
                        if !self.phrase_positions[pp].next_position()? {
                            // Positions exhausted.
                            return Ok(false);
                        }
                    }
                }
            }
        }
        // Positions available.
        Ok(true)
    }

    /// Initialises with a check for repeats; heavy work, but done only for the
    /// first candidate document.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.initFirstTime()`.
    ///
    /// If there are repetitions, a check is made for multi-term postings.
    /// Without them, once the positions are placed in the first candidate
    /// document, the repeats and their groups are visible. With them, a more
    /// complex check is needed, up front, as there may be hidden collisions:
    /// for example, if `P1` has `{A,B}`, `P2` has `{B,C}` and the first
    /// document is `A C B`, then at the start `P1` points at `A` and `P2` at
    /// `C`, and it would not be identified that `P1` and `P2` are repetitions
    /// of each other.
    fn init_first_time(&mut self) -> Result<bool> {
        self.checked_rpts = true;
        self.place_first_positions()?;

        let rpt_terms = self.repeating_terms();
        self.has_rpts = !rpt_terms.order.is_empty();

        if self.has_rpts {
            // Needed with repetitions.
            self.rpt_stack = vec![0; self.num_postings];
            let rgs = self.gather_rpt_groups(&rpt_terms);
            self.sort_rpt_groups(rgs);
            if !self.advance_repeat_groups()? {
                // Positions exhausted.
                return Ok(false);
            }
        }

        self.fill_queue();
        // Positions available.
        Ok(true)
    }

    /// Sorts each repetition group by query offset.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.sortRptGroups(ArrayList<ArrayList<PhrasePositions>>)`.
    /// It is done once, at the first document, and lets each later document be
    /// initialised faster.
    fn sort_rpt_groups(&mut self, rgs: Vec<Vec<usize>>) {
        self.rpt_groups = Vec::with_capacity(rgs.len());
        for mut rg in rgs {
            // Java sorts with `Comparator.comparingInt(pp -> pp.offset)` and a
            // stable merge sort; `sort_by_key` is stable too.
            rg.sort_by_key(|&pp| self.phrase_positions[pp].offset);
            for (j, &pp) in rg.iter().enumerate() {
                // This index is used for efficient re-queuing.
                self.phrase_positions[pp].rpt_ind = j as i32;
            }
            self.rpt_groups.push(rg);
        }
    }

    /// Detects the repetition groups; done once, for the first document.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.gatherRptGroups(LinkedHashMap<Term, Integer>)`.
    ///
    /// A possible solution for identifying the groups would be to create a
    /// single repetition group made of all repeating positions, but that would
    /// slow down the collision check, as every position would have to be
    /// checked. Instead, connected regions are computed on the bipartite graph
    /// of postings and terms.
    fn gather_rpt_groups(&mut self, rpt_terms: &RepeatingTerms) -> Vec<Vec<usize>> {
        let rpp = self.repeating_pps(rpt_terms);
        let mut res: Vec<Vec<usize>> = Vec::new();
        if !self.has_multi_term_rpts {
            // Simpler: no multi-terms, so we can base this on the positions in
            // the first document.
            for i in 0..rpp.len() {
                let pp = rpp[i];
                if self.phrase_positions[pp].rpt_group >= 0 {
                    // Already marked as a repetition.
                    continue;
                }
                let tp_pos = self.tp_pos(pp);
                for &pp2 in rpp.iter().skip(i + 1) {
                    if self.phrase_positions[pp2].rpt_group >= 0
                        // Not a repetition: the two positions are originally at
                        // the same offset.
                        || self.phrase_positions[pp2].offset == self.phrase_positions[pp].offset
                        // Not a repetition.
                        || self.tp_pos(pp2) != tp_pos
                    {
                        continue;
                    }
                    // A repetition.
                    let mut g = self.phrase_positions[pp].rpt_group;
                    if g < 0 {
                        g = res.len() as i32;
                        self.phrase_positions[pp].rpt_group = g;
                        res.push(vec![pp]);
                    }
                    self.phrase_positions[pp2].rpt_group = g;
                    res[g as usize].push(pp2);
                }
            }
        } else {
            // More involved: there are multi-terms.
            let mut bb = self.pp_terms_bit_sets(&rpp, rpt_terms);
            union_term_groups(&mut bb);
            let tg = term_groups(rpt_terms, &bb);
            let num_distinct_group_ids: BTreeSet<usize> = tg.values().copied().collect();
            // Java collects the members of each group in a `HashSet`, whose
            // iteration order is unspecified; a `BTreeSet` keyed by the phrase
            // position's ordinal makes the group order deterministic here.
            // `sort_rpt_groups` then sorts each group by offset, so the two
            // agree except where offsets tie, and there Java's order was
            // already arbitrary.
            let mut tmp: Vec<BTreeSet<usize>> = (0..num_distinct_group_ids.len())
                .map(|_| BTreeSet::new())
                .collect();
            for &pp in &rpp {
                let terms = self.phrase_positions[pp].terms.clone();
                for t in &terms {
                    if rpt_terms.contains(t) {
                        let g = *tg
                            .get(t)
                            .expect("INVARIANT: every repeating term has a group");
                        tmp[g].insert(pp);
                        debug_assert!(
                            self.phrase_positions[pp].rpt_group == -1
                                || self.phrase_positions[pp].rpt_group == g as i32
                        );
                        self.phrase_positions[pp].rpt_group = g as i32;
                    }
                }
            }
            for hs in tmp {
                res.push(hs.into_iter().collect());
            }
        }
        res
    }

    /// Finds the repeating terms and assigns them ordinal values.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.repeatingTerms()`, which answers a
    /// `LinkedHashMap<Term, Integer>`: the insertion order matters, because
    /// [`term_groups`] indexes the key array by the stored ordinal.
    fn repeating_terms(&self) -> RepeatingTerms {
        let mut tord = RepeatingTerms::default();
        let mut tcnt: BTreeMap<Term, i32> = BTreeMap::new();
        for pp in &self.phrase_positions {
            for t in &pp.terms {
                let cnt = tcnt.entry(t.clone()).or_insert(0);
                *cnt += 1;
                if *cnt == 2 {
                    tord.insert(t.clone());
                }
            }
        }
        tord
    }

    /// Finds the repeating phrase positions and, for each, updates
    /// `has_multi_term_rpts` when it carries several terms.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.repeatingPPs(HashMap<Term, Integer>)`.
    fn repeating_pps(&mut self, rpt_terms: &RepeatingTerms) -> Vec<usize> {
        let mut rp = Vec::new();
        for i in 0..self.phrase_positions.len() {
            let mut is_repeating = false;
            for t in &self.phrase_positions[i].terms {
                if rpt_terms.contains(t) {
                    is_repeating = true;
                    break;
                }
            }
            if is_repeating {
                rp.push(i);
                self.has_multi_term_rpts |= self.phrase_positions[i].terms.len() > 1;
            }
        }
        rp
    }

    /// For each repeating phrase position, sets the ordinal of each of its
    /// repeating terms in a bit set.
    ///
    /// Equivalent to the private
    /// `SloppyPhraseMatcher.ppTermsBitSets(PhrasePositions[], HashMap<Term, Integer>)`.
    fn pp_terms_bit_sets(&self, rpp: &[usize], tord: &RepeatingTerms) -> Vec<TermBits> {
        let mut bb = Vec::with_capacity(rpp.len());
        for &pp in rpp {
            let mut b = TermBits::new(tord.order.len());
            for t in &self.phrase_positions[pp].terms {
                if let Some(ord) = tord.get(t) {
                    b.set(ord);
                }
            }
            bb.push(b);
        }
        bb
    }
}

/// The repeating terms of a phrase, in the order in which each of them reached
/// a count of two.
///
/// Equivalent to the `LinkedHashMap<Term, Integer>` of
/// `SloppyPhraseMatcher.repeatingTerms()`. The stored ordinal is the insertion
/// index, which is what `termGroups` relies on when it indexes the key array.
#[derive(Debug, Default)]
struct RepeatingTerms {
    order: Vec<Term>,
    index: BTreeMap<Term, usize>,
}

impl RepeatingTerms {
    fn insert(&mut self, term: Term) {
        let ord = self.order.len();
        self.order.push(term.clone());
        self.index.insert(term, ord);
    }

    fn contains(&self, term: &Term) -> bool {
        self.index.contains_key(term)
    }

    fn get(&self, term: &Term) -> Option<usize> {
        self.index.get(term).copied()
    }
}

/// Unions the term-group bit sets until they are disjoint, so that each group
/// has different terms.
///
/// Equivalent to the private
/// `SloppyPhraseMatcher.unionTermGroups(ArrayList<FixedBitSet>)`, which is
/// `O(n^2)`.
fn union_term_groups(bb: &mut Vec<TermBits>) {
    let mut i = 0usize;
    while !bb.is_empty() && i + 1 < bb.len() {
        let mut incr = 1usize;
        let mut j = i + 1;
        while j < bb.len() {
            if bb[i].intersects(&bb[j]) {
                let other = bb[j].clone();
                bb[i].or(&other);
                bb.remove(j);
                incr = 0;
            } else {
                j += 1;
            }
        }
        if incr == 0 {
            continue;
        }
        i += incr;
    }
}

/// Maps each term to the single group that contains it.
///
/// Equivalent to the private
/// `SloppyPhraseMatcher.termGroups(LinkedHashMap<Term, Integer>, ArrayList<FixedBitSet>)`.
fn term_groups(tord: &RepeatingTerms, bb: &[TermBits]) -> BTreeMap<Term, usize> {
    let mut tg = BTreeMap::new();
    for (i, bits) in bb.iter().enumerate() {
        // `i` is the group number.
        for ord in bits.set_bits() {
            if let Some(term) = tord.order.get(ord) {
                tg.insert(term.clone(), i);
            }
        }
    }
    tg
}

impl PhraseMatcher for SloppyPhraseMatcher {
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
        // `reset_positions` and `init_phrase_positions`.
        let mut max_freq = 0f32;
        for pp in &mut self.phrase_positions {
            pp.freq = pp.postings.freq()?;
            max_freq += pp.freq as f32;
        }
        self.freqs_loaded = true;
        Ok(max_freq)
    }

    fn reset_positions(&mut self) -> Result<()> {
        if self.freqs_loaded {
            // Freqs already loaded by `max_freq`.
            self.freqs_loaded = false;
        } else {
            // Freqs not yet loaded; load them now.
            for pp in &mut self.phrase_positions {
                pp.freq = pp.postings.freq()?;
            }
        }
        self.positioned = self.init_phrase_positions()?;
        self.match_length = i32::MAX;
        self.lead_position = i32::MAX;
        Ok(())
    }

    fn next_match(&mut self) -> Result<bool> {
        if !self.positioned {
            return Ok(false);
        }
        // If the queue is not full, then `positioned` is false.
        let mut pp = self
            .pq
            .pop(&self.phrase_positions)
            .expect("INVARIANT: positioned implies a full queue");
        self.capture_lead(pp)?;
        self.match_length = self.end - self.phrase_positions[pp].position;
        let mut next = self.phrase_positions[self
            .pq
            .top()
            .expect("INVARIANT: positioned implies a full queue")]
        .position;
        while self.advance_pp(pp)? {
            if self.has_rpts && !self.advance_rpts(pp)? {
                // Positions exhausted.
                break;
            }
            if self.phrase_positions[pp].position > next {
                // Done minimising the current match length.
                self.pq.add(&self.phrase_positions, pp);
                if self.match_length <= self.slop {
                    return Ok(true);
                }
                pp = self
                    .pq
                    .pop(&self.phrase_positions)
                    .expect("INVARIANT: positioned implies a full queue");
                next = self.phrase_positions[self
                    .pq
                    .top()
                    .expect("INVARIANT: positioned implies a full queue")]
                .position;
                self.match_length = self.end - self.phrase_positions[pp].position;
            } else {
                let match_length2 = self.end - self.phrase_positions[pp].position;
                if match_length2 < self.match_length {
                    self.match_length = match_length2;
                }
            }
            self.capture_lead(pp)?;
        }
        self.positioned = false;
        Ok(self.match_length <= self.slop)
    }

    fn sloppy_weight(&self) -> f32 {
        1.0 / (1.0 + self.match_length as f32)
    }

    fn start_position(&self) -> i32 {
        // When a match is detected, the top postings is advanced until it has
        // moved beyond its successor, to ensure that the match is of minimal
        // width. That means the lead position has to be recorded before it is
        // advanced. However, the priority queue does not guarantee that the top
        // postings is in fact the earliest in the list, so every term has to be
        // cycled through to check; this is slow, but `Matches` is slow anyway.
        let mut lead_position = self.lead_position;
        for pp in &self.phrase_positions {
            lead_position = lead_position.min(pp.position + pp.offset);
        }
        lead_position
    }

    fn end_position(&self) -> i32 {
        let mut end_position = self.lead_position;
        for pp in &self.phrase_positions {
            if pp.ord != self.lead_ord {
                end_position = end_position.max(pp.position + pp.offset);
            }
        }
        end_position
    }

    fn start_offset(&self) -> Result<i32> {
        // See `start_position` for why every term is visited.
        let mut lead_offset = self.lead_offset;
        for pp in &self.phrase_positions {
            lead_offset = lead_offset.min(pp.postings.start_offset());
        }
        Ok(lead_offset)
    }

    fn end_offset(&self) -> Result<i32> {
        let mut end_offset = self.lead_end_offset;
        for pp in &self.phrase_positions {
            if pp.ord != self.lead_ord {
                end_offset = end_offset.max(pp.postings.end_offset());
            }
        }
        Ok(end_offset)
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
