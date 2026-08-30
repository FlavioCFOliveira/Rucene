//! Dense conjunction bulk scoring, ported from
//! `org.apache.lucene.search.DenseConjunctionBulkScorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::bit_set_doc_id_stream::BitSetDocIdStream;
use crate::search::bit_set_util;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::conjunction_disi::ConjunctionMember;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::scorable::{Scorable, SimpleScorable};
use crate::search::scorer::Scorer;
use crate::util::{Bits, FixedBitSet, MathUtil};

/// The size of the windows this scorer intersects clauses over.
///
/// Equivalent to `DenseConjunctionBulkScorer.WINDOW_SIZE`. Lucene keeps it
/// small on purpose, so that gaps in the postings of clauses that do not lead
/// iteration can still be taken advantage of.
pub const WINDOW_SIZE: i32 = 4096;

/// The inverse of the density above which clauses are intersected through bit
/// sets.
///
/// Equivalent to `DenseConjunctionBulkScorer.DENSITY_THRESHOLD_INVERSE`, which
/// is `Long.SIZE / 2`: bit sets are only used when more than one in 32
/// documents is expected to match.
pub const DENSITY_THRESHOLD_INVERSE: i32 = (i64::BITS / 2) as i32;

/// A [`BulkScorer`] implementation of
/// [`ConjunctionScorer`](crate::search::ConjunctionScorer) specialised for
/// dense clauses.
///
/// Equivalent to `org.apache.lucene.search.DenseConjunctionBulkScorer`, which
/// is package-private in Java; it is public here because Rust has no package
/// visibility and the suppliers that build it live in sibling modules. Whenever
/// sensible it intersects clauses by loading their matches into a bit set and
/// and-ing those bit sets together.
///
/// **Divergence from Lucene 10.5.0.** Java's private `DisiWrapper` record holds
/// an approximation and the optional two-phase iterator it came from — two
/// aliases of the same object. Rust cannot own an approximation apart from the
/// iterator that produces it, so the clauses are
/// [`ConjunctionMember`]s, whose accessors expose exactly the `docID`,
/// `docIDRunEnd` and `intoBitSet` that Java's record does. The lists of clauses
/// that Java rebuilds per window become lists of *positions* into the clause
/// array, for the same reason.
pub struct DenseConjunctionBulkScorer {
    max_doc: i32,
    iterators: Vec<ConjunctionMember>,
    scorable: SimpleScorable,

    window_matches: FixedBitSet,
    clause_window_matches: FixedBitSet,
    window_clauses: Vec<usize>,
    // Reused by the leap-frog path.
    window_approximations: Vec<usize>,
    window_two_phases: Vec<usize>,
}

impl std::fmt::Debug for DenseConjunctionBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DenseConjunctionBulkScorer")
            .field("max_doc", &self.max_doc)
            .field("clauses", &self.iterators.len())
            .finish_non_exhaustive()
    }
}

impl DenseConjunctionBulkScorer {
    /// Builds a dense conjunction over the given filters.
    ///
    /// Equivalent to
    /// `DenseConjunctionBulkScorer.of(List<Scorer>, int, float)`, which splits
    /// each scorer into its two-phase view when it has one and its plain
    /// iterator otherwise — the split [`ConjunctionMember::from_scorer`]
    /// performs.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn of(filters: Vec<Box<dyn Scorer>>, max_doc: i32, constant_score: f32) -> Result<Self> {
        let members = filters
            .into_iter()
            .map(ConjunctionMember::from_scorer)
            .collect();
        Self::new(members, max_doc, constant_score)
    }

    /// Builds a dense conjunction over the given clauses.
    ///
    /// Equivalent to `DenseConjunctionBulkScorer(List<DocIdSetIterator>,
    /// List<TwoPhaseIterator>, int, float)`, with the two Java lists already
    /// collected into [`ConjunctionMember`]s.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when no clause is supplied,
    /// which is the `IllegalArgumentException` Java throws.
    pub fn new(
        mut members: Vec<ConjunctionMember>,
        max_doc: i32,
        constant_score: f32,
    ) -> Result<Self> {
        if members.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "Expected one or more iterators, got 0".to_string(),
            ));
        }
        // Plain approximations before two-phase ones, so matches() only runs on
        // docs that already satisfy every approximation; within each group,
        // cheapest approximation first (lead skipping). Java's `List.sort` and
        // Rust's `sort_by` are both stable.
        members.sort_by(|a, b| {
            let a_key = u8::from(a.has_two_phase());
            let b_key = u8::from(b.has_two_phase());
            a_key.cmp(&b_key).then_with(|| a.cost().cmp(&b.cost()))
        });
        let mut scorable = SimpleScorable::new();
        scorable.set_score(constant_score);
        Ok(Self {
            max_doc,
            iterators: members,
            scorable,
            window_matches: FixedBitSet::new(WINDOW_SIZE as usize),
            clause_window_matches: FixedBitSet::new(WINDOW_SIZE as usize),
            window_clauses: Vec::new(),
            window_approximations: Vec::new(),
            window_two_phases: Vec::new(),
        })
    }

    /// Equivalent to the private static `DenseConjunctionBulkScorer.advance`.
    fn advance_in_window(set: &FixedBitSet, i: i32) -> i32 {
        if i >= WINDOW_SIZE {
            NO_MORE_DOCS
        } else {
            bit_set_util::next_set_bit(set, i as usize)
        }
    }

    /// Equivalent to the private `DenseConjunctionBulkScorer.scoreWindow`.
    fn score_window(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        // Advance all iterators to the first doc that is greater than or equal
        // to min. This is important as this is the only place where we can take
        // advantage of a large gap between consecutive matches in any clause.
        let mut min = min;
        for i in 0..self.iterators.len() {
            if self.iterators[i].doc_id() >= min {
                min = self.iterators[i].doc_id();
            } else {
                min = self.iterators[i].advance(min)?;
            }
            if min >= max {
                return Ok(min);
            }
        }

        // Partition clauses of the conjunction into:
        //  - clauses that don't fully match the first half of the window and
        //    get evaluated via #intoBitSet or leap-frog,
        //  - other clauses that are used to compute the greatest possible
        //    window size that they fully match.
        // This logic helps align scoring windows with the natural
        // #docIDRunEnd() boundaries of the data, which helps evaluate fewer
        // clauses per window - without allowing windows to become too small
        // thanks to the WINDOW_SIZE/2 threshold.
        let mut min_doc_id_run_end = max;
        let min_run_end_threshold = MathUtil::unsigned_min(min.wrapping_add(WINDOW_SIZE / 2), max);
        for i in 0..self.iterators.len() {
            let doc_id_run_end = self.iterators[i].doc_id_run_end()?;
            if self.iterators[i].doc_id() > min || doc_id_run_end < min_run_end_threshold {
                self.window_clauses.push(i);
            } else {
                min_doc_id_run_end = min_doc_id_run_end.min(doc_id_run_end);
            }
        }

        if accept_docs.is_none() && self.window_clauses.is_empty() {
            // We have a large range of doc IDs that all match.
            collector.collect_range(min, min_doc_id_run_end, &mut self.scorable)?;
            return Ok(min_doc_id_run_end);
        }

        let bitset_window_max =
            MathUtil::unsigned_min(min_doc_id_run_end, WINDOW_SIZE.wrapping_add(min));

        // The bit-set path pays a fixed per-window cost (materialize the lead,
        // applyMask, cardinality, BitSetDocIdStream); it wins when most
        // candidates must be tested anyway. But for a sparse window confirmed
        // by a two-phase clause that can be skipped, plain leap-frog -- which
        // only touches surviving docs and never materializes a bit set -- is
        // cheaper.
        //
        // A two-phase clause whose approximation matches every doc (cost >=
        // maxDoc, e.g. a skip-indexed doc-values range, whose block iterator
        // reports NO_MORE_DOCS) cannot be skipped and stays on the bit-set path
        // for its vectorized intoBitSet. A selective approximation (cost <
        // maxDoc, e.g. a phrase) can be skipped, so a sparse window is cheaper
        // via leap-frog.
        let mut skippable_two_phase = false;
        for position in &self.window_clauses {
            let member = &self.iterators[*position];
            if member.has_two_phase() && member.cost() < i64::from(self.max_doc) {
                skippable_two_phase = true;
                break;
            }
        }
        // "sparse" mirrors the bit set's own WINDOW_SIZE/4 bulk-confirm cutoff
        // (leadCost <= maxDoc/4).
        let sparse = !self.window_clauses.is_empty()
            && self.iterators[self.window_clauses[0]].cost() <= i64::from(self.max_doc) / 4;

        if skippable_two_phase && sparse {
            for position in &self.window_clauses {
                self.window_approximations.push(*position);
                if self.iterators[*position].has_two_phase() {
                    self.window_two_phases.push(*position);
                }
            }
            let iterators = &self.iterators;
            self.window_two_phases.sort_by(|a, b| {
                iterators[*a]
                    .match_cost()
                    .total_cmp(&iterators[*b].match_cost())
            });
            let Self {
                iterators,
                scorable,
                window_approximations,
                window_two_phases,
                ..
            } = self;
            score_window_using_leap_frog(
                collector,
                accept_docs,
                iterators,
                window_approximations,
                window_two_phases,
                scorable,
                min,
                bitset_window_max,
            )?;
            self.window_approximations.clear();
            self.window_two_phases.clear();
        } else {
            let Self {
                iterators,
                scorable,
                window_matches,
                clause_window_matches,
                window_clauses,
                ..
            } = self;
            score_window_using_bit_set(
                collector,
                accept_docs,
                iterators,
                window_clauses,
                scorable,
                window_matches,
                clause_window_matches,
                min,
                bitset_window_max,
            )?;
        }
        self.window_clauses.clear();

        Ok(bitset_window_max)
    }
}

/// Equivalent to the private
/// `DenseConjunctionBulkScorer.scoreWindowUsingBitSet`.
#[allow(clippy::too_many_arguments)]
fn score_window_using_bit_set(
    collector: &mut dyn LeafCollector,
    accept_docs: Option<&dyn Bits>,
    members: &mut [ConjunctionMember],
    clauses: &[usize],
    scorable: &mut SimpleScorable,
    window_matches: &mut FixedBitSet,
    clause_window_matches: &mut FixedBitSet,
    window_base: i32,
    window_max: i32,
) -> CollectionResult<()> {
    debug_assert!(window_max > window_base);
    debug_assert!(bit_set_util::scan_is_empty(window_matches));
    debug_assert!(bit_set_util::scan_is_empty(clause_window_matches));

    if clauses.is_empty() {
        // This happens if all clauses fully matched the window and there are
        // deleted docs.
        window_matches.set_range(0, (window_max - window_base) as usize);
    } else {
        let lead = clauses[0];
        if members[lead].doc_id() < window_base {
            members[lead].advance(window_base)?;
        }
        members[lead].into_bit_set(window_max, window_matches, window_base)?;
    }

    if let Some(accept_docs) = accept_docs {
        // Apply live docs.
        bit_set_util::apply_mask(accept_docs, window_matches, window_base);
    }

    let window_size = window_max - window_base;
    let threshold = window_size / DENSITY_THRESHOLD_INVERSE;
    // Above this many surviving docs, decoding a two-phase clause's whole
    // window in one shot beats confirming each survivor one at a time; below it
    // we confirm survivors only.
    let bulk_confirm_threshold = window_size / 4;
    // The leading clause at index 0 is already applied.
    let mut up_to = 1;
    let mut cardinality = window_matches.cardinality() as i32;
    while up_to < clauses.len() && cardinality >= threshold {
        let other = clauses[up_to];
        if members[other].doc_id() < window_base {
            members[other].advance(window_base)?;
        }
        if members[other].has_two_phase() && cardinality < bulk_confirm_threshold {
            // Sparse survivors + per-doc confirmation: confirm only the docs
            // that survived the cheaper clauses (the bit set gates matches()),
            // never decoding a doc another clause excluded.
            let mut window_match = bit_set_util::next_set_bit(window_matches, 0);
            while window_match != NO_MORE_DOCS {
                let doc = window_base + window_match;
                let mut other_doc = members[other].doc_id();
                if other_doc < doc {
                    other_doc = members[other].advance(doc)?;
                }
                if other_doc != doc || !members[other].matches()? {
                    window_matches.clear(window_match as usize);
                }
                window_match =
                    DenseConjunctionBulkScorer::advance_in_window(window_matches, window_match + 1);
            }
        } else {
            // Dense survivors, or a plain iterator: load this clause's matches
            // in bulk and intersect. For a two-phase clause this still confirms
            // matches() via its (possibly vectorized) intoBitSet.
            members[other].into_bit_set(window_max, clause_window_matches, window_base)?;
            bit_set_util::and_in_place(window_matches, clause_window_matches);
            clause_window_matches.clear_all();
        }
        up_to += 1;
        cardinality = window_matches.cardinality() as i32;
    }

    if up_to < clauses.len() {
        // If the leading clause is sparse on this doc ID range or if the
        // intersection became sparse after applying a few clauses, we finish
        // evaluating the intersection using the traditional leap-frog approach.
        // This proved important with a query such as "+secretary +of +state" on
        // wikibigall, where the intersection becomes sparse after intersecting
        // "secretary" and "state". As the leap-frog only visits surviving docs,
        // two-phase clauses confirm matches() here only on docs that no cheaper
        // clause already excluded.
        let mut window_match = bit_set_util::next_set_bit(window_matches, 0);
        'advance_head: while window_match != NO_MORE_DOCS {
            let doc = window_base + window_match;
            // First confirm every remaining approximation is on doc...
            for position in &clauses[up_to..] {
                let mut other_doc = members[*position].doc_id();
                if other_doc < doc {
                    other_doc = members[*position].advance(doc)?;
                }
                if doc != other_doc {
                    window_match = DenseConjunctionBulkScorer::advance_in_window(
                        window_matches,
                        other_doc - window_base,
                    );
                    continue 'advance_head;
                }
            }
            // ...then run the (more expensive) two-phase confirmations, only on
            // surviving docs.
            for position in &clauses[up_to..] {
                if members[*position].has_two_phase() && !members[*position].matches()? {
                    window_match = DenseConjunctionBulkScorer::advance_in_window(
                        window_matches,
                        window_match + 1,
                    );
                    continue 'advance_head;
                }
            }
            collector.collect(doc, scorable)?;
            window_match =
                DenseConjunctionBulkScorer::advance_in_window(window_matches, window_match + 1);
        }
    } else {
        let mut stream = BitSetDocIdStream::new(window_matches, window_base);
        collector.collect_stream(&mut stream, scorable)?;
    }

    window_matches.clear_all();
    Ok(())
}

/// Confirms two-phase `matches()` only on docs that survived every
/// approximation, without materialising a window bit set.
///
/// Equivalent to the private static
/// `DenseConjunctionBulkScorer.scoreWindowUsingLeapFrog`. Cheaper than the
/// bit-set path for sparse windows with a skippable two-phase clause.
#[allow(clippy::too_many_arguments)]
fn score_window_using_leap_frog(
    collector: &mut dyn LeafCollector,
    accept_docs: Option<&dyn Bits>,
    members: &mut [ConjunctionMember],
    approximations: &[usize],
    two_phases: &[usize],
    scorable: &mut SimpleScorable,
    min: i32,
    max: i32,
) -> CollectionResult<()> {
    debug_assert!(!two_phases.is_empty());
    debug_assert!(approximations.len() >= two_phases.len());

    if approximations.len() == 1 {
        // scoreWindowUsingLeapFrog is only used if there is at least one
        // two-phase iterator, so our single clause is a two-phase iterator.
        debug_assert_eq!(two_phases.len(), 1);
        let clause = approximations[0];
        if members[clause].doc_id() < min {
            members[clause].advance(min)?;
        }
        let mut doc = members[clause].doc_id();
        while doc < max {
            if accept_docs.map_or(true, |bits| bits.get(doc as usize))
                && members[clause].matches()?
            {
                collector.collect(doc, scorable)?;
            }
            doc = members[clause].next_doc()?;
        }
        return Ok(());
    }

    let lead1 = approximations[0];
    let lead2 = approximations[1];

    if members[lead1].doc_id() < min {
        members[lead1].advance(min)?;
    }

    let mut doc = members[lead1].doc_id();
    'advance_head: while doc < max {
        if accept_docs.is_some_and(|bits| !bits.get(doc as usize)) {
            doc = members[lead1].next_doc()?;
            continue;
        }
        let mut doc2 = members[lead2].doc_id();
        if doc2 < doc {
            doc2 = members[lead2].advance(doc)?;
        }
        if doc != doc2 {
            doc = members[lead1].advance(doc2.min(max))?;
            continue;
        }
        for position in &approximations[2..] {
            let mut doc_n = members[*position].doc_id();
            if doc_n < doc {
                doc_n = members[*position].advance(doc)?;
            }
            if doc != doc_n {
                doc = members[lead1].advance(doc_n.min(max))?;
                continue 'advance_head;
            }
        }
        for position in two_phases {
            if !members[*position].matches()? {
                doc = members[lead1].next_doc()?;
                continue 'advance_head;
            }
        }
        collector.collect(doc, scorable)?;
        doc = members[lead1].next_doc()?;
    }
    Ok(())
}

impl BulkScorer for DenseConjunctionBulkScorer {
    fn cost(&self) -> i64 {
        self.iterators[0].cost()
    }

    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        collector.set_scorer(&mut self.scorable)?;

        // Java copies the clause list and appends the collector's competitive
        // iterator to the copy; Rust appends it to the clause array and removes
        // it again, which keeps the positions of the other clauses stable.
        let competitive = collector.competitive_iterator()?;
        let appended = competitive.is_some();
        if let Some(competitive) = competitive {
            self.iterators
                .push(ConjunctionMember::from_iterator(competitive));
        }

        let result = self.score_inner(collector, accept_docs, min, max);

        if appended {
            self.iterators.pop();
        }
        result
    }
}

impl DenseConjunctionBulkScorer {
    /// The body of [`BulkScorer::score`], run with the collector's competitive
    /// iterator appended to the clauses.
    fn score_inner(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        let mut min = min;
        for member in &self.iterators {
            min = min.max(member.doc_id());
        }

        let max = max.min(self.max_doc);

        if self.iterators[0].doc_id() < min {
            min = self.iterators[0].advance(min)?;
        }

        let score = self.scorable.score()?;
        while min < max {
            if self.scorable.min_competitive_score() > score {
                return Ok(NO_MORE_DOCS);
            }
            min = self.score_window(collector, accept_docs, min, max)?;
        }

        let lead_doc = self.iterators[0].doc_id();
        if lead_doc > max {
            Ok(lead_doc)
        } else if max >= self.max_doc {
            Ok(NO_MORE_DOCS)
        } else {
            Ok(max)
        }
    }
}
