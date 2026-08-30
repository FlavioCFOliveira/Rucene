//! Disjunctions of approximations, ported from
//! `org.apache.lucene.search.DisjunctionDISIApproximation`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::disi_priority_queue::DisiPriorityQueue;
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::FixedBitSet;

/// A [`DocIdSetIterator`] which is a disjunction of the approximations of the
/// provided iterators.
///
/// Equivalent to the `final class
/// org.apache.lucene.search.DisjunctionDISIApproximation`.
///
/// **Divergence from Lucene 10.5.0.** Java holds every clause twice: the heap
/// and the linear list hold `DisiWrapper` references, and the caller that built
/// them keeps its own. Rust forbids that, so this type owns the wrappers and
/// the heap and the linear list hold **positions** into them; see
/// [`DisiPriorityQueue`]. To keep the construction order reachable — the
/// order in which `DisjunctionSumScorer` and `DisjunctionMaxScorer` walk their
/// sub-scorers, which decides how their score upper bounds round — the
/// permutation that the cost-descending sort applies is remembered and exposed
/// through [`sub_scorer`](Self::sub_scorer).
pub struct DisjunctionDISIApproximation {
    /// The clauses, sorted by descending cost, as Java's local `wrappers` array
    /// is after `Arrays.sort`.
    wrappers: Vec<DisiWrapper>,
    /// `original_order[i]` is the position, in [`wrappers`](Self::wrappers), of
    /// the `i`-th clause as the caller supplied it.
    original_order: Vec<usize>,
    /// Heap of iterators that lead iteration.
    lead_iterators: DisiPriorityQueue,
    /// Iterators that will likely advance on every call to `next_doc` /
    /// `advance`.
    other_iterators: Vec<usize>,
    cost: i64,
    lead_top: usize,
    min_other_doc: i32,
    doc: i32,
}

impl std::fmt::Debug for DisjunctionDISIApproximation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisjunctionDISIApproximation")
            .field("clauses", &self.wrappers.len())
            .field("cost", &self.cost)
            .field("doc", &self.doc)
            .finish_non_exhaustive()
    }
}

impl DisjunctionDISIApproximation {
    /// Creates a disjunction over the given clauses.
    ///
    /// Equivalent to `DisjunctionDISIApproximation.of(Collection<? extends
    /// DisiWrapper>, long)` and to the constructor it delegates to.
    ///
    /// # Panics
    ///
    /// Panics when `sub_iterators` is empty, which Java would report as a
    /// `ArrayIndexOutOfBoundsException`; every caller in Lucene supplies at
    /// least two clauses.
    pub fn new(sub_iterators: Vec<DisiWrapper>, lead_cost: i64) -> Self {
        assert!(
            !sub_iterators.is_empty(),
            "a DisjunctionDISIApproximation needs at least one clause"
        );

        // Using a heap to store disjunctive clauses is great for exhaustive
        // evaluation, when a single clause needs to move through the heap on
        // every iteration on average. However, when intersecting with a
        // selective filter, it is possible that all clauses need advancing,
        // which makes the reordering cost scale in O(N * log(N)) per advance()
        // call when checking clauses linearly would scale in O(N).
        // To protect against this reordering overhead, we try to have 1.5
        // clauses or less that advance on every advance() call by only putting
        // clauses into the heap as long as Σ min(1, cost / leadCost) <= 1.5, or
        // Σ min(leadCost, cost) <= 1.5 * leadCost. Other clauses are checked
        // linearly.
        let mut order: Vec<usize> = (0..sub_iterators.len()).collect();
        // Sort by descending cost. Java uses a stable sort, and so does this.
        order.sort_by(|a, b| sub_iterators[*b].cost.cmp(&sub_iterators[*a].cost));

        let mut wrappers: Vec<Option<DisiWrapper>> = sub_iterators.into_iter().map(Some).collect();
        let mut sorted: Vec<DisiWrapper> = Vec::with_capacity(wrappers.len());
        // `original_order[i]` is where the i-th supplied clause landed.
        let mut original_order = vec![0usize; wrappers.len()];
        for (position, source) in order.iter().enumerate() {
            original_order[*source] = position;
            sorted.push(
                wrappers[*source]
                    .take()
                    .expect("INVARIANT: a sort permutation visits each position once"),
            );
        }
        let wrappers = sorted;

        let mut reorder_threshold = lead_cost.wrapping_add(lead_cost >> 1);
        if reorder_threshold < 0 {
            // overflow
            reorder_threshold = i64::MAX;
        }

        // track total cost
        let mut cost: i64 = 0;
        // Split `wrappers` into those that will remain out of the PQ, and those
        // that will go in (PQ entries at the end). `last_idx` is the last index
        // of the wrappers that will remain out.
        let mut reorder_cost: i64 = 0;
        let len = wrappers.len() as i64;
        let mut last_idx = len - 1;
        while last_idx >= 0 {
            let last_cost = wrappers[last_idx as usize].cost;
            let inc = last_cost.min(lead_cost);
            let next = reorder_cost.wrapping_add(inc);
            if next < 0 || next > reorder_threshold {
                break;
            }
            reorder_cost = next;
            cost = cost.wrapping_add(last_cost);
            last_idx -= 1;
        }

        // Make lead_iterators not empty. This helps save conditionals in the
        // implementation which are rarely tested.
        if last_idx == len - 1 {
            cost = cost.wrapping_add(wrappers[last_idx as usize].cost);
            last_idx -= 1;
        }

        // Build the PQ:
        debug_assert!(last_idx >= -1 && last_idx < len - 1);
        let pq_len = (len - last_idx - 1) as usize;
        let mut lead_iterators = DisiPriorityQueue::of_max_size(pq_len);
        let pq_entries: Vec<usize> = ((last_idx + 1) as usize..wrappers.len()).collect();
        lead_iterators.add_all(&wrappers, &pq_entries);

        // Build the non-PQ list:
        let other_iterators: Vec<usize> = (0..(last_idx + 1) as usize).collect();
        let mut min_other_doc = i32::MAX;
        for position in &other_iterators {
            cost = cost.wrapping_add(wrappers[*position].cost);
            min_other_doc = min_other_doc.min(wrappers[*position].doc);
        }

        let lead_top = lead_iterators
            .top()
            .expect("INVARIANT: the heap was made non-empty above");

        Self {
            wrappers,
            original_order,
            lead_iterators,
            other_iterators,
            cost,
            lead_top,
            min_other_doc,
            doc: -1,
        }
    }

    /// Returns the number of clauses.
    pub fn len(&self) -> usize {
        self.wrappers.len()
    }

    /// Returns whether there is no clause at all; there never is, because
    /// [`new`](Self::new) requires at least one.
    pub fn is_empty(&self) -> bool {
        self.wrappers.is_empty()
    }

    /// Returns the `i`-th clause **in the order the caller supplied it**.
    ///
    /// Equivalent to indexing the `subScorers` list that
    /// `DisjunctionScorer`'s subclasses keep beside the approximation.
    pub fn sub_scorer(&mut self, i: usize) -> &mut DisiWrapper {
        &mut self.wrappers[self.original_order[i]]
    }

    /// Returns the clause at `position` within this approximation's own,
    /// cost-descending order — the positions [`top_list`](Self::top_list)
    /// returns.
    pub fn wrapper(&mut self, position: usize) -> &mut DisiWrapper {
        &mut self.wrappers[position]
    }

    /// Returns every clause, in this approximation's own order.
    pub fn wrappers(&self) -> &[DisiWrapper] {
        &self.wrappers
    }

    /// Returns every clause for mutation, in this approximation's own order.
    pub fn wrappers_mut(&mut self) -> &mut [DisiWrapper] {
        &mut self.wrappers
    }

    /// Returns the positions of the clauses positioned on the current doc, in
    /// the order in which Java's linked list is traversed.
    ///
    /// Equivalent to `DisjunctionDISIApproximation.topList()`.
    pub fn top_list(&self) -> Vec<usize> {
        if self.wrappers[self.lead_top].doc < self.min_other_doc {
            self.lead_iterators.top_list(&self.wrappers)
        } else {
            self.compute_top_list()
        }
    }

    /// Equivalent to the private
    /// `DisjunctionDISIApproximation.computeTopList()`.
    fn compute_top_list(&self) -> Vec<usize> {
        debug_assert!(self.wrappers[self.lead_top].doc >= self.min_other_doc);
        let lead = if self.wrappers[self.lead_top].doc == self.min_other_doc {
            self.lead_iterators.top_list(&self.wrappers)
        } else {
            Vec::new()
        };
        // Java prepends every matching `other` onto the list, so the traversal
        // starts at the last matching one and ends with the lead list.
        let mut top_list: Vec<usize> = self
            .other_iterators
            .iter()
            .filter(|position| self.wrappers[**position].doc == self.min_other_doc)
            .copied()
            .collect();
        top_list.reverse();
        top_list.extend(lead);
        top_list
    }
}

impl DocIdSetIterator for DisjunctionDISIApproximation {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn cost(&self) -> i64 {
        self.cost
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.wrappers[self.lead_top].doc < self.min_other_doc {
            let cur_doc = self.wrappers[self.lead_top].doc;
            loop {
                let next = self.wrappers[self.lead_top].approximation().next_doc()?;
                self.wrappers[self.lead_top].doc = next;
                self.lead_top = self
                    .lead_iterators
                    .update_top(&self.wrappers)
                    .expect("INVARIANT: the lead heap is never empty");
                if self.wrappers[self.lead_top].doc != cur_doc {
                    break;
                }
            }
            self.doc = self.wrappers[self.lead_top].doc.min(self.min_other_doc);
            Ok(self.doc)
        } else {
            let target = self.min_other_doc + 1;
            self.advance(target)
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        while self.wrappers[self.lead_top].doc < target {
            let next = self.wrappers[self.lead_top]
                .approximation()
                .advance(target)?;
            self.wrappers[self.lead_top].doc = next;
            self.lead_top = self
                .lead_iterators
                .update_top(&self.wrappers)
                .expect("INVARIANT: the lead heap is never empty");
        }

        self.min_other_doc = i32::MAX;
        for k in 0..self.other_iterators.len() {
            let position = self.other_iterators[k];
            if self.wrappers[position].doc < target {
                let next = self.wrappers[position].approximation().advance(target)?;
                self.wrappers[position].doc = next;
            }
            self.min_other_doc = self.min_other_doc.min(self.wrappers[position].doc);
        }

        self.doc = self.wrappers[self.lead_top].doc.min(self.min_other_doc);
        Ok(self.doc)
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        while self.wrappers[self.lead_top].doc < up_to {
            self.wrappers[self.lead_top]
                .approximation()
                .into_bit_set(up_to, bit_set, offset)?;
            self.wrappers[self.lead_top].doc = self.wrappers[self.lead_top].approximation_doc_id();
            self.lead_top = self
                .lead_iterators
                .update_top(&self.wrappers)
                .expect("INVARIANT: the lead heap is never empty");
        }

        self.min_other_doc = i32::MAX;
        for k in 0..self.other_iterators.len() {
            let position = self.other_iterators[k];
            self.wrappers[position]
                .approximation()
                .into_bit_set(up_to, bit_set, offset)?;
            self.wrappers[position].doc = self.wrappers[position].approximation_doc_id();
            self.min_other_doc = self.min_other_doc.min(self.wrappers[position].doc);
        }

        self.doc = self.wrappers[self.lead_top].doc.min(self.min_other_doc);
        Ok(())
    }

    /// **Divergence from Lucene 10.5.0.** Java inspects the heap-led clauses
    /// that are already positioned on the current doc and takes the greatest of
    /// their `docIDRunEnd()`, which needs to advance nothing but does need a
    /// mutable handle on each sub-approximation. This trait's `doc_id_run_end`
    /// takes `&self`, so the port answers the always-sound base value, `doc +
    /// 1`. Runs are therefore reported shorter than Lucene would report them,
    /// which costs bulk-scoring throughput on dense clauses and changes no
    /// match.
    fn doc_id_run_end(&self) -> Result<i32> {
        debug_assert!(self.doc != NO_MORE_DOCS);
        Ok(self.doc + 1)
    }
}
