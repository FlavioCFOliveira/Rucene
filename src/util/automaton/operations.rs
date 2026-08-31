//! Port of `org.apache.lucene.util.automaton.Operations`.

use std::collections::HashMap;
use std::collections::VecDeque;

use crate::error::{LuceneError, Result};
use crate::util::{BytesRef, IntsRef, IntsRefBuilder, UnicodeUtil};

use super::automata::Automata;
use super::automaton::{Automaton, AutomatonBuilder, Transition, TransitionAccessor};
use super::automaton::{MAX_CODE_POINT, MIN_CODE_POINT};
use super::int_set::{FrozenIntSet, StateSet};
use super::state_pair::StatePair;
use super::too_complex_to_determinize_exception::TooComplexToDeterminizeException;

/// Result of an operation that may exceed the determinize work limit.
pub type DeterminizeResult<T> = std::result::Result<T, TooComplexToDeterminizeException>;

/// Default maximum effort that [`Operations::determinize`] should spend before
/// giving up and returning [`TooComplexToDeterminizeException`].
pub const DEFAULT_DETERMINIZE_WORK_LIMIT: i32 = 10000;

// -----------------------------------------------------------------------------
// Point transition bookkeeping, shared with NFARunAutomaton
// -----------------------------------------------------------------------------

/// Simple custom list of transitions, storing `dest`, `min`, `max` triples.
///
/// Equivalent to `Operations.TransitionList`.
#[derive(Clone, Debug, Default)]
pub(crate) struct TransitionList {
    /// `dest`, `min`, `max` for each transition.
    pub(crate) transitions: Vec<i32>,
    pub(crate) next: usize,
}

impl TransitionList {
    fn add(&mut self, t: &Transition) {
        if self.transitions.len() < self.next + 3 {
            self.transitions.resize(self.next + 3, 0);
        }
        self.transitions[self.next] = t.dest;
        self.transitions[self.next + 1] = t.min;
        self.transitions[self.next + 2] = t.max;
        self.next += 3;
    }
}

/// Holds all transitions that start on this int point, or end at this point minus
/// one.
///
/// Equivalent to `Operations.PointTransitions`.
#[derive(Clone, Debug, Default)]
pub(crate) struct PointTransitions {
    pub(crate) point: i32,
    pub(crate) ends: TransitionList,
    pub(crate) starts: TransitionList,
}

impl PointTransitions {
    fn reset(&mut self, point: i32) {
        self.point = point;
        self.ends.next = 0;
        self.starts.next = 0;
    }
}

/// Set of [`PointTransitions`], keyed by point.
///
/// Equivalent to `Operations.PointTransitionSet`.
#[derive(Clone, Debug, Default)]
pub(crate) struct PointTransitionSet {
    pub(crate) count: usize,
    pub(crate) points: Vec<PointTransitions>,
    map: HashMap<i32, usize>,
    use_hash: bool,
}

impl PointTransitionSet {
    const HASHMAP_CUTOVER: usize = 30;

    fn next(&mut self, point: i32) -> usize {
        // 1st time we are seeing this point
        if self.count == self.points.len() {
            self.points.push(PointTransitions::default());
        }
        self.points[self.count].reset(point);
        self.count += 1;
        self.count - 1
    }

    fn find(&mut self, point: i32) -> usize {
        if self.use_hash {
            if let Some(&idx) = self.map.get(&point) {
                return idx;
            }
            let idx = self.next(point);
            self.map.insert(point, idx);
            idx
        } else {
            for i in 0..self.count {
                if self.points[i].point == point {
                    return i;
                }
            }

            let p = self.next(point);
            if self.count == Self::HASHMAP_CUTOVER {
                // switch to a hash map on the fly
                debug_assert!(self.map.is_empty());
                for i in 0..self.count {
                    self.map.insert(self.points[i].point, i);
                }
                self.use_hash = true;
            }
            p
        }
    }

    pub(crate) fn reset(&mut self) {
        if self.use_hash {
            self.map.clear();
            self.use_hash = false;
        }
        self.count = 0;
    }

    pub(crate) fn sort(&mut self) {
        // Tim sort performs well on already sorted arrays; the points are unique so
        // an unstable sort produces the same permutation.
        if self.count > 1 {
            self.points[0..self.count].sort_by_key(|p| p.point);
        }
    }

    pub(crate) fn add(&mut self, t: &Transition) {
        let i = self.find(t.min);
        self.points[i].starts.add(t);
        let i = self.find(1 + t.max);
        self.points[i].ends.add(t);
    }
}

// -----------------------------------------------------------------------------
// Operations
// -----------------------------------------------------------------------------

/// Automata operations.
///
/// Equivalent to `org.apache.lucene.util.automaton.Operations`.
pub struct Operations;

impl Operations {
    /// Returns an automaton that accepts the concatenation of the languages of the
    /// given automata.
    ///
    /// Complexity: linear in total number of states.
    pub fn concatenate(list: &[Automaton]) -> Automaton {
        let mut result = Automaton::new();

        // First pass: create all states
        for a in list {
            if a.get_num_states() == 0 {
                // concatenation with empty is empty
                return Automata::make_empty();
            }
            let num_states = a.get_num_states();
            for _ in 0..num_states {
                result.create_state();
            }
        }

        // Second pass: add transitions, carefully linking accept states of A to the
        // init state of the next A:
        let mut state_offset = 0i32;
        let mut t = Transition::new();
        for i in 0..list.len() {
            let a = &list[i];
            let num_states = a.get_num_states();

            let next_a = if i == list.len() - 1 {
                None
            } else {
                Some(&list[i + 1])
            };

            for s in 0..num_states {
                let mut num_transitions = a.init_transition(s, &mut t);
                for _ in 0..num_transitions {
                    a.get_next_transition(&mut t);
                    result.add_transition_range(
                        state_offset + s,
                        state_offset + t.dest,
                        t.min,
                        t.max,
                    );
                }

                if a.is_accept(s) {
                    let mut follow_a = next_a;
                    let mut follow_offset = state_offset;
                    let mut upto = i + 1;
                    loop {
                        match follow_a {
                            Some(f) => {
                                // Adds a "virtual" epsilon transition:
                                num_transitions = f.init_transition(0, &mut t);
                                for _ in 0..num_transitions {
                                    f.get_next_transition(&mut t);
                                    result.add_transition_range(
                                        state_offset + s,
                                        follow_offset + num_states + t.dest,
                                        t.min,
                                        t.max,
                                    );
                                }
                                if f.is_accept(0) {
                                    // Keep chaining if followA accepts the empty string
                                    follow_offset += f.get_num_states();
                                    follow_a = if upto == list.len() - 1 {
                                        None
                                    } else {
                                        Some(&list[upto + 1])
                                    };
                                    upto += 1;
                                } else {
                                    break;
                                }
                            }
                            None => {
                                result.set_accept(state_offset + s, true);
                                break;
                            }
                        }
                    }
                }
            }

            state_offset += num_states;
        }

        if result.get_num_states() == 0 {
            result.create_state();
        }

        result.finish_state();
        Self::remove_dead_states(&result)
    }

    /// Returns an automaton that accepts the union of the empty string and the
    /// language of the given automaton. This may create a dead state.
    ///
    /// Complexity: linear in number of states.
    pub fn optional(a: &Automaton) -> Automaton {
        if a.is_accept(0) {
            // If the initial state is accepted, then the empty string is already
            // accepted.
            return a.clone();
        }

        let mut has_transitions_to_initial_state = false;
        let mut t = Transition::new();
        'outer: for state in 0..a.get_num_states() {
            let count = a.init_transition(state, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                if t.dest == 0 {
                    has_transitions_to_initial_state = true;
                    break 'outer;
                }
            }
        }

        if !has_transitions_to_initial_state {
            // If the automaton has no transition to the initial state, we can simply
            // mark the initial state as accepted.
            let mut result = Automaton::new();
            result.copy(a);
            if result.get_num_states() == 0 {
                result.create_state();
            }
            result.set_accept(0, true);
            return result;
        }

        let mut result = Automaton::new();
        result.create_state();
        result.set_accept(0, true);
        if a.get_num_states() > 0 {
            result.copy(a);
            result.add_epsilon(0, 1);
        }
        result.finish_state();
        result
    }

    /// Returns an automaton that accepts the Kleene star (zero or more concatenated
    /// repetitions) of the language of the given automaton.
    ///
    /// Never modifies the input automaton language. Complexity: linear in number of
    /// states.
    pub fn repeat(a: &Automaton) -> Automaton {
        if a.get_num_states() == 0 {
            // Repeating the empty automata will still only accept the empty automata.
            return a.clone();
        }

        let accept_cardinality = a.get_accept_states().iter().filter(|b| **b).count();
        if a.is_accept(0) && accept_cardinality == 1 {
            // If state 0 is the only accept state, then this automaton already
            // repeats itself.
            return a.clone();
        }

        let mut builder = AutomatonBuilder::new();
        // Create the initial state, which is accepted
        builder.create_state();
        builder.set_accept(0, true);
        let mut t = Transition::new();

        let mut state_map = vec![0i32; a.get_num_states() as usize];
        for state in 0..a.get_num_states() {
            if !a.is_accept(state) {
                state_map[state as usize] = builder.create_state();
            } else if a.get_num_transitions(state) == 0 {
                // Accept states that have no transitions get merged into state 0.
                state_map[state as usize] = 0;
            } else {
                let new_state = builder.create_state();
                state_map[state as usize] = new_state;
                builder.set_accept(new_state, true);
            }
        }

        // Now copy the automaton while renumbering states.
        for state in 0..a.get_num_states() {
            let src = state_map[state as usize];
            let count = a.init_transition(state, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                let dest = state_map[t.dest as usize];
                builder.add_transition_range(src, dest, t.min, t.max);
            }
        }

        // Now copy transitions of the initial state to our new initial state.
        let count = a.init_transition(0, &mut t);
        for _ in 0..count {
            a.get_next_transition(&mut t);
            builder.add_transition_range(0, state_map[t.dest as usize], t.min, t.max);
        }

        // Now copy transitions of the initial state to final states to make the
        // automaton repeat itself.
        for s in 0..a.get_num_states() {
            if !a.is_accept(s) {
                continue;
            }
            if state_map[s as usize] != 0 {
                let count = a.init_transition(0, &mut t);
                for _ in 0..count {
                    a.get_next_transition(&mut t);
                    builder.add_transition_range(
                        state_map[s as usize],
                        state_map[t.dest as usize],
                        t.min,
                        t.max,
                    );
                }
            }
        }

        Self::remove_dead_states(&builder.finish())
    }

    /// Returns an automaton that accepts `count` or more concatenated repetitions of
    /// the language of the given automaton.
    ///
    /// Complexity: linear in number of states and in `count`.
    pub fn repeat_min(a: &Automaton, count: i32) -> Automaton {
        if count == 0 {
            return Self::repeat(a);
        }
        let mut list: Vec<Automaton> = Vec::new();
        for _ in 0..count {
            list.push(a.clone());
        }
        list.push(Self::repeat(a));
        Self::concatenate(&list)
    }

    /// Returns an automaton that accepts between `min` and `max` (including both)
    /// concatenated repetitions of the language of the given automaton.
    ///
    /// Complexity: linear in number of states and in `min` and `max`.
    pub fn repeat_min_max(a: &Automaton, min: i32, max: i32) -> Automaton {
        if min > max {
            return Automata::make_empty();
        }

        let b = if min == 0 {
            Automata::make_empty_string()
        } else if min == 1 {
            let mut b = Automaton::new();
            b.copy(a);
            b
        } else {
            let list: Vec<Automaton> = (0..min).map(|_| a.clone()).collect();
            Self::concatenate(&list)
        };

        let mut prev_accept_states = Self::to_set(&b, 0);
        let mut builder = AutomatonBuilder::new();
        builder.copy(&b);
        for _ in min..max {
            let num_states = builder.get_num_states();
            builder.copy(a);
            for s in &prev_accept_states {
                builder.add_epsilon(*s, num_states);
            }
            prev_accept_states = Self::to_set(a, num_states);
        }

        Self::remove_dead_states(&builder.finish())
    }

    fn to_set(a: &Automaton, offset: i32) -> Vec<i32> {
        let mut result = Vec::new();
        for (s, accept) in a.get_accept_states().iter().enumerate() {
            if *accept {
                result.push(offset + s as i32);
            }
        }
        result
    }

    /// Returns a (deterministic) automaton that accepts the complement of the
    /// language of the given automaton.
    ///
    /// Complexity: linear in number of states if already deterministic and
    /// exponential otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`TooComplexToDeterminizeException`] if determinizing requires more
    /// than `determinize_work_limit` effort.
    pub fn complement(a: &Automaton, determinize_work_limit: i32) -> DeterminizeResult<Automaton> {
        let mut a = Self::totalize(&Self::determinize(a, determinize_work_limit)?);
        let num_states = a.get_num_states();
        for p in 0..num_states {
            let accept = a.is_accept(p);
            a.set_accept(p, !accept);
        }
        Ok(Self::remove_dead_states(&a))
    }

    /// Returns a (deterministic) automaton that accepts the intersection of the
    /// language of `a1` and the complement of the language of `a2`.
    ///
    /// Complexity: quadratic in number of states if `a2` is already deterministic
    /// and exponential in the number of `a2`'s states otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`TooComplexToDeterminizeException`] if determinizing requires more
    /// than `determinize_work_limit` effort.
    pub fn minus(
        a1: &Automaton,
        a2: &Automaton,
        determinize_work_limit: i32,
    ) -> DeterminizeResult<Automaton> {
        if Self::is_empty(a1) || std::ptr::eq(a1, a2) {
            return Ok(Automata::make_empty());
        }
        if Self::is_empty(a2) {
            return Ok(a1.clone());
        }
        Ok(Self::intersection(
            a1,
            &Self::complement(a2, determinize_work_limit)?,
        ))
    }

    /// Returns an automaton that accepts the intersection of the languages of the
    /// given automata. Never modifies the input automata languages.
    ///
    /// Complexity: quadratic in number of states.
    pub fn intersection(a1: &Automaton, a2: &Automaton) -> Automaton {
        if std::ptr::eq(a1, a2) {
            return a1.clone();
        }
        if a1.get_num_states() == 0 {
            return a1.clone();
        }
        if a2.get_num_states() == 0 {
            return a2.clone();
        }
        let transitions1 = a1.get_sorted_transitions();
        let transitions2 = a2.get_sorted_transitions();
        let mut c = Automaton::new();
        c.create_state();
        let mut worklist: VecDeque<StatePair> = VecDeque::new();
        let mut newstates: HashMap<StatePair, StatePair> = HashMap::new();
        let p = StatePair::with_state(0, 0, 0);
        worklist.push_back(p);
        newstates.insert(p, p);
        while let Some(p) = worklist.pop_front() {
            c.set_accept(p.s, a1.is_accept(p.s1) && a2.is_accept(p.s2));
            let t1 = &transitions1[p.s1 as usize];
            let t2 = &transitions2[p.s2 as usize];
            let mut b2 = 0usize;
            for t1n in t1.iter() {
                while b2 < t2.len() && t2[b2].max < t1n.min {
                    b2 += 1;
                }
                let mut n2 = b2;
                while n2 < t2.len() && t1n.max >= t2[n2].min {
                    if t2[n2].max >= t1n.min {
                        let q = StatePair::new(t1n.dest, t2[n2].dest);
                        let existing = newstates.get(&q).copied();
                        let r = match existing {
                            Some(r) => r,
                            None => {
                                let mut q = q;
                                q.s = c.create_state();
                                worklist.push_back(q);
                                newstates.insert(q, q);
                                q
                            }
                        };
                        let min = if t1n.min > t2[n2].min {
                            t1n.min
                        } else {
                            t2[n2].min
                        };
                        let max = if t1n.max < t2[n2].max {
                            t1n.max
                        } else {
                            t2[n2].max
                        };
                        c.add_transition_range(p.s, r.s, min, max);
                    }
                    n2 += 1;
                }
            }
        }
        c.finish_state();

        Self::remove_dead_states(&c)
    }

    /// Returns true if this automaton has any states that cannot be reached from the
    /// initial state or cannot reach an accept state.
    ///
    /// Cost is `O(numTransitions + numStates)`.
    pub fn has_dead_states(a: &Automaton) -> bool {
        let live_states = Self::get_live_states(a);
        let num_live = live_states.iter().filter(|b| **b).count();
        let num_states = a.get_num_states() as usize;
        debug_assert!(num_live <= num_states);
        num_live < num_states
    }

    /// Returns true if there are dead states reachable from an initial state.
    pub fn has_dead_states_from_initial(a: &Automaton) -> bool {
        let mut reachable_from_initial = Self::get_live_states_from_initial(a);
        let reachable_from_accept = Self::get_live_states_to_accept(a);
        for (i, v) in reachable_from_initial.iter_mut().enumerate() {
            *v = *v && !reachable_from_accept[i];
        }
        reachable_from_initial.iter().any(|b| *b)
    }

    /// Returns true if there are dead states that reach an accept state.
    pub fn has_dead_states_to_accept(a: &Automaton) -> bool {
        let reachable_from_initial = Self::get_live_states_from_initial(a);
        let mut reachable_from_accept = Self::get_live_states_to_accept(a);
        for (i, v) in reachable_from_accept.iter_mut().enumerate() {
            *v = *v && !reachable_from_initial[i];
        }
        reachable_from_accept.iter().any(|b| *b)
    }

    /// Returns an automaton that accepts the union of the languages of the given
    /// automata.
    ///
    /// Complexity: linear in number of states.
    pub fn union(list: &[Automaton]) -> Automaton {
        let mut result = Automaton::new();

        // Create initial state:
        result.create_state();

        // Copy over all automata
        for a in list {
            result.copy(a);
        }

        // Add epsilon transition from new initial state
        let mut state_offset = 1i32;
        for a in list {
            if a.get_num_states() == 0 {
                continue;
            }
            result.add_epsilon(0, state_offset);
            state_offset += a.get_num_states();
        }

        result.finish_state();

        Self::merge_accept_states_with_no_transition(&Self::remove_dead_states(&result))
    }

    /// Determinizes the given automaton.
    ///
    /// Worst case complexity: exponential in number of states.
    ///
    /// # Errors
    ///
    /// Returns [`TooComplexToDeterminizeException`] if determinizing requires more
    /// than `work_limit` effort. Higher numbers allow this operation to consume more
    /// memory and CPU but allow more complex automata; use
    /// [`DEFAULT_DETERMINIZE_WORK_LIMIT`] as a decent default if you do not otherwise
    /// know what to specify.
    pub fn determinize(a: &Automaton, work_limit: i32) -> DeterminizeResult<Automaton> {
        if a.is_deterministic() {
            // Already determinized
            return Ok(a.clone());
        }
        if a.get_num_states() <= 1 {
            // Already determinized
            return Ok(a.clone());
        }

        // subset construction
        let mut b = AutomatonBuilder::new();

        // Same initial values and state will always have the same hashCode
        let initialset = FrozenIntSet::singleton(0, 0);

        // Create state 0:
        b.create_state();

        let mut worklist: VecDeque<FrozenIntSet> = VecDeque::new();
        let mut newstate: HashMap<FrozenIntSet, i32> = HashMap::new();

        worklist.push_back(initialset.clone());

        b.set_accept(0, a.is_accept(0));
        newstate.insert(initialset, 0);

        // like Set<Integer,PointTransitions>
        let mut points = PointTransitionSet::default();

        // like HashMap<Integer,Integer>, maps state to its count
        let mut states_set = StateSet::new(5);

        let mut t = Transition::new();

        let mut effort_spent: i64 = 0;

        // LUCENE-9981: approximate conversion from what used to be a limit on number
        // of states, to maximum "effort":
        let effort_limit = i64::from(work_limit) * 10;

        while let Some(s) = worklist.pop_front() {
            // LUCENE-9981: we more carefully aggregate the net work this automaton is
            // costing us, instead of (overly simplistically) counting number of
            // determinized states:
            effort_spent += s.values.len() as i64;
            if effort_spent >= effort_limit {
                return Err(TooComplexToDeterminizeException::from_automaton(
                    a.clone(),
                    work_limit,
                ));
            }

            // Collate all outgoing transitions by min/1+max:
            for i in 0..s.values.len() {
                let s0 = s.values[i];
                let num_transitions = a.get_num_transitions(s0);
                a.init_transition(s0, &mut t);
                for _ in 0..num_transitions {
                    a.get_next_transition(&mut t);
                    points.add(&t);
                }
            }

            if points.count == 0 {
                // No outgoing transitions -- skip it
                continue;
            }

            points.sort();

            let mut last_point = -1i32;
            let mut acc_count = 0i32;

            let r = s.state;

            for i in 0..points.count {
                let point = points.points[i].point;

                if states_set.size() > 0 {
                    debug_assert!(last_point != -1);

                    let values = states_set.get_array().to_vec();
                    let hash = states_set.long_hash_code();
                    let probe = FrozenIntSet::new(values, hash, -1);
                    let existing = newstate.get(&probe).copied();
                    let q = match existing {
                        Some(q) => q,
                        None => {
                            let q = b.create_state();
                            let p = states_set.freeze(q);
                            worklist.push_back(p.clone());
                            b.set_accept(q, acc_count > 0);
                            newstate.insert(p, q);
                            q
                        }
                    };

                    b.add_transition_range(r, q, last_point, point - 1);
                }

                // process transitions that end on this point
                // (closes an overlapping interval)
                let limit = points.points[i].ends.next;
                let mut j = 0usize;
                while j < limit {
                    let dest = points.points[i].ends.transitions[j];
                    states_set.decr(dest);
                    acc_count -= i32::from(a.is_accept(dest));
                    j += 3;
                }
                points.points[i].ends.next = 0;

                // process transitions that start on this point
                // (opens a new interval)
                let limit = points.points[i].starts.next;
                let mut j = 0usize;
                while j < limit {
                    let dest = points.points[i].starts.transitions[j];
                    states_set.incr(dest);
                    acc_count += i32::from(a.is_accept(dest));
                    j += 3;
                }
                last_point = point;
                points.points[i].starts.next = 0;
            }
            points.reset();
            debug_assert!(states_set.size() == 0, "size={}", states_set.size());
        }

        let result = b.finish();
        debug_assert!(result.is_deterministic());
        Ok(result)
    }

    /// Returns true if the given automaton accepts no strings.
    pub fn is_empty(a: &Automaton) -> bool {
        if a.get_num_states() == 0 {
            // Common case: no states
            return true;
        }
        if !a.is_accept(0) && a.get_num_transitions(0) == 0 {
            // Common case: just one initial state
            return true;
        }
        if a.is_accept(0) {
            // Apparently common case: it accepts the damned empty string
            return false;
        }

        let mut work_list: VecDeque<i32> = VecDeque::new();
        let mut seen = vec![false; a.get_num_states() as usize];
        work_list.push_back(0);
        seen[0] = true;

        let mut t = Transition::new();
        while let Some(state) = work_list.pop_front() {
            if a.is_accept(state) {
                return false;
            }
            let count = a.init_transition(state, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                if !seen[t.dest as usize] {
                    work_list.push_back(t.dest);
                    seen[t.dest as usize] = true;
                }
            }
        }

        true
    }

    /// Returns true if the given automaton accepts all strings.
    ///
    /// The automaton must be deterministic, or this method may return false.
    /// Complexity: linear in number of states and transitions.
    pub fn is_total(a: &Automaton) -> bool {
        Self::is_total_range(a, MIN_CODE_POINT, MAX_CODE_POINT)
    }

    /// Returns true if the given automaton accepts all strings for the specified
    /// min/max range of the alphabet.
    ///
    /// The automaton must be deterministic, or this method may return false.
    /// Complexity: linear in number of states and transitions.
    pub fn is_total_range(a: &Automaton, min_alphabet: i32, max_alphabet: i32) -> bool {
        let states = Self::get_live_states(a);
        let mut spare = Transition::new();
        let mut seen_states = 0usize;
        for state in 0..a.get_num_states() {
            if !states[state as usize] {
                continue;
            }
            // all reachable states must be accept states
            if !a.is_accept(state) {
                return false;
            }
            // all reachable states must contain transitions covering
            // minAlphabet-maxAlphabet
            let mut previous_label = min_alphabet - 1;
            for transition in 0..a.get_num_transitions(state) {
                a.get_transition(state, transition, &mut spare);
                // no gaps are allowed
                if spare.min > previous_label + 1 {
                    return false;
                }
                previous_label = spare.max;
            }
            if previous_label < max_alphabet {
                return false;
            }
            seen_states += 1;
        }
        // we've checked all the states, automaton is either total or empty
        seen_states > 0
    }

    /// Returns true if the given string is accepted by the automaton. The input must
    /// be deterministic.
    ///
    /// Complexity: linear in the length of the string. For full performance, use
    /// [`RunAutomaton`](super::run_automaton::RunAutomaton).
    pub fn run(a: &Automaton, s: &str) -> bool {
        debug_assert!(a.is_deterministic());
        let mut state = 0i32;
        for cp in s.chars() {
            let next_state = a.step(state, cp as i32);
            if next_state == -1 {
                return false;
            }
            state = next_state;
        }
        a.is_accept(state)
    }

    /// Returns true if the given string (expressed as Unicode codepoints) is accepted
    /// by the automaton. The input must be deterministic.
    ///
    /// Complexity: linear in the length of the string.
    pub fn run_ints(a: &Automaton, s: &IntsRef) -> bool {
        debug_assert!(a.is_deterministic());
        let mut state = 0i32;
        for i in 0..s.length {
            let next_state = a.step(state, s.ints[s.offset + i]);
            if next_state == -1 {
                return false;
            }
            state = next_state;
        }
        a.is_accept(state)
    }

    /// Returns the set of live states: a state is "live" if an accept state is
    /// reachable from it and if it is reachable from the initial state.
    fn get_live_states(a: &Automaton) -> Vec<bool> {
        let mut live = Self::get_live_states_from_initial(a);
        let to_accept = Self::get_live_states_to_accept(a);
        for (i, v) in live.iter_mut().enumerate() {
            *v = *v && to_accept[i];
        }
        live
    }

    /// Returns a bit set marking states reachable from the initial state.
    fn get_live_states_from_initial(a: &Automaton) -> Vec<bool> {
        let num_states = a.get_num_states() as usize;
        let mut live = vec![false; num_states];
        if num_states == 0 {
            return live;
        }
        let mut work_list: VecDeque<i32> = VecDeque::new();
        live[0] = true;
        work_list.push_back(0);

        let mut t = Transition::new();
        while let Some(s) = work_list.pop_front() {
            let count = a.init_transition(s, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                if !live[t.dest as usize] {
                    live[t.dest as usize] = true;
                    work_list.push_back(t.dest);
                }
            }
        }

        live
    }

    /// Returns a bit set marking states that can reach an accept state.
    fn get_live_states_to_accept(a: &Automaton) -> Vec<bool> {
        let mut builder = AutomatonBuilder::new();

        // NOTE: not quite the same thing as what reverse() does:
        let mut t = Transition::new();
        let num_states = a.get_num_states();
        for _ in 0..num_states {
            builder.create_state();
        }
        for s in 0..num_states {
            let count = a.init_transition(s, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                builder.add_transition_range(t.dest, s, t.min, t.max);
            }
        }
        let a2 = builder.finish();

        let mut work_list: VecDeque<i32> = VecDeque::new();
        let mut live = vec![false; num_states as usize];
        for (s, accept) in a.get_accept_states().iter().enumerate() {
            if *accept {
                live[s] = true;
                work_list.push_back(s as i32);
            }
        }

        while let Some(s) = work_list.pop_front() {
            let count = a2.init_transition(s, &mut t);
            for _ in 0..count {
                a2.get_next_transition(&mut t);
                if !live[t.dest as usize] {
                    live[t.dest as usize] = true;
                    work_list.push_back(t.dest);
                }
            }
        }

        live
    }

    /// Removes transitions to dead states.
    ///
    /// A state is "dead" if it is not reachable from the initial state or if no
    /// accept state is reachable from it.
    pub fn remove_dead_states(a: &Automaton) -> Automaton {
        let num_states = a.get_num_states();
        let live_set = Self::get_live_states(a);
        if live_set.iter().filter(|b| **b).count() == num_states as usize {
            return a.clone();
        }

        let mut map = vec![0i32; num_states as usize];

        let mut result = Automaton::new();
        for i in 0..num_states {
            if live_set[i as usize] {
                map[i as usize] = result.create_state();
                result.set_accept(map[i as usize], a.is_accept(i));
            }
        }

        let mut t = Transition::new();

        for i in 0..num_states {
            if live_set[i as usize] {
                let num_transitions = a.init_transition(i, &mut t);
                // filter out transitions to dead states:
                for _ in 0..num_transitions {
                    a.get_next_transition(&mut t);
                    if live_set[t.dest as usize] {
                        result.add_transition_range(
                            map[i as usize],
                            map[t.dest as usize],
                            t.min,
                            t.max,
                        );
                    }
                }
            }
        }

        result.finish_state();
        debug_assert!(!Self::has_dead_states(&result));
        result
    }

    /// Merges all accept states that do not have outgoing transitions into a single
    /// shared state.
    ///
    /// This is a subset of minimization that is much cheaper. It is useful because
    /// operations like concatenation need to connect accept states of an automaton
    /// with the start state of the next one, so having fewer accept states makes the
    /// produced automata simpler.
    pub(crate) fn merge_accept_states_with_no_transition(a: &Automaton) -> Automaton {
        let num_states = a.get_num_states();

        let mut accept_states_with_no_transition: Vec<i32> = Vec::new();

        let accept_states = a.get_accept_states();
        for i in 0..num_states {
            if accept_states[i as usize] && a.get_num_transitions(i) == 0 {
                accept_states_with_no_transition.push(i);
            }
        }

        if accept_states_with_no_transition.len() <= 1 {
            // No states to merge
            return a.clone();
        }

        // Now copy states, preserving accept states.
        let mut result = Automaton::new();
        for s in 0..num_states {
            let remapped_s = Self::remap(s, &accept_states_with_no_transition);
            while result.get_num_states() <= remapped_s {
                result.create_state();
            }
            if accept_states[s as usize] {
                result.set_accept(remapped_s, true);
            }
        }

        // Now copy transitions, making sure to remap states.
        let mut t = Transition::new();
        for s in 0..num_states {
            let remapped_source = Self::remap(s, &accept_states_with_no_transition);
            let num_transitions = a.init_transition(s, &mut t);
            for _ in 0..num_transitions {
                a.get_next_transition(&mut t);
                let remapped_dest = Self::remap(t.dest, &accept_states_with_no_transition);
                result.add_transition_range(remapped_source, remapped_dest, t.min, t.max);
            }
        }

        result.finish_state();
        result
    }

    fn remap(s: i32, combined_states: &[i32]) -> i32 {
        match combined_states.binary_search(&s) {
            Ok(_) => {
                // This state is part of the states that get combined, remap to the
                // first one.
                combined_states[0]
            }
            Err(idx) => {
                if idx <= 1 {
                    // There is either no combined state before the current state, or
                    // only the first one, which we're preserving: no renumbering
                    // needed.
                    s
                } else {
                    // Subtract the number of states that get combined into the first
                    // combined state.
                    s - (idx as i32 - 1)
                }
            }
        }
    }

    /// Returns the longest sequence of code points that is a prefix of all accepted
    /// strings and visits each state at most once.
    ///
    /// The automaton must not have dead states.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the automaton has dead states
    /// reachable from the initial state.
    fn get_common_prefix_code_points(a: &Automaton) -> Result<Vec<i32>> {
        if Self::has_dead_states_from_initial(a) {
            return Err(LuceneError::IllegalArgument(
                "input automaton has dead states".to_string(),
            ));
        }
        let mut builder: Vec<i32> = Vec::new();
        if Self::is_empty(a) {
            return Ok(builder);
        }
        let num_states = a.get_num_states() as usize;
        let mut scratch = Transition::new();
        // NOTE: Lucene also keeps a `visited` bit set here, but never reads it; it is
        // omitted rather than reproduced as dead state.
        let mut current = vec![false; num_states];
        let mut next = vec![false; num_states];
        current[0] = true; // start with initial state
        'algorithm: loop {
            let mut label = -1i32;
            // do a pass, stepping all current paths forward once
            for (state, in_current) in current.iter().enumerate() {
                if !*in_current {
                    continue;
                }
                // if it is an accept state, we are done
                if a.is_accept(state as i32) {
                    break 'algorithm;
                }
                for transition in 0..a.get_num_transitions(state as i32) {
                    a.get_transition(state as i32, transition, &mut scratch);
                    if label == -1 {
                        label = scratch.min;
                    }
                    // either a range of labels, or a label that doesn't match all the
                    // other paths this round
                    if scratch.min != scratch.max || scratch.min != label {
                        break 'algorithm;
                    }
                    // mark target state for next iteration
                    next[scratch.dest as usize] = true;
                }
            }

            debug_assert!(
                label != -1,
                "we should not get here since we checked no dead-end states up front!?"
            );

            // add the label to the prefix
            builder.push(label);
            // swap "current" with "next", clear "next"
            std::mem::swap(&mut current, &mut next);
            next.iter_mut().for_each(|v| *v = false);
        }
        Ok(builder)
    }

    /// Returns the longest string that is a prefix of all accepted strings and visits
    /// each state at most once.
    ///
    /// The automaton must not have dead states. If this automaton has already been
    /// converted to UTF-8 (e.g. using [`UTF32ToUTF8`](super::utf32_to_utf8::UTF32ToUTF8))
    /// then you should use [`Operations::get_common_prefix_bytes_ref`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the automaton has dead states
    /// reachable from the initial state, or if the prefix is not a valid sequence of
    /// Unicode scalar values.
    pub fn get_common_prefix(a: &Automaton) -> Result<String> {
        let code_points = Self::get_common_prefix_code_points(a)?;
        UnicodeUtil::new_string(&code_points, 0, code_points.len())
    }

    /// Returns the longest [`BytesRef`] that is a prefix of all accepted strings and
    /// visits each state at most once.
    ///
    /// The returned prefix can be empty and might possibly include a UTF-8 fragment
    /// of a full Unicode character.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the automaton has dead states
    /// reachable from the initial state, or [`LuceneError::IllegalState`] if the
    /// automaton is not binary.
    pub fn get_common_prefix_bytes_ref(a: &Automaton) -> Result<BytesRef> {
        let prefix = Self::get_common_prefix_code_points(a)?;
        let mut bytes = Vec::with_capacity(prefix.len());
        for ch in prefix {
            if ch > 255 {
                return Err(LuceneError::IllegalState(
                    "automaton is not binary".to_string(),
                ));
            }
            bytes.push(ch as u8);
        }

        Ok(BytesRef::new(bytes))
    }

    /// If this automaton accepts a single input, returns it; else returns `None`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the automaton is not
    /// deterministic.
    pub fn get_singleton(a: &Automaton) -> Result<Option<IntsRef>> {
        if !a.is_deterministic() {
            return Err(LuceneError::IllegalArgument(
                "input automaton must be deterministic".to_string(),
            ));
        }
        let mut builder = IntsRefBuilder::new();
        let mut visited = vec![false; a.get_num_states() as usize];
        let mut s = 0i32;
        let mut t = Transition::new();
        loop {
            visited[s as usize] = true;
            if !a.is_accept(s) {
                if a.get_num_transitions(s) == 1 {
                    a.get_transition(s, 0, &mut t);
                    if t.min == t.max && !visited[t.dest as usize] {
                        builder.append(t.min);
                        s = t.dest;
                        continue;
                    }
                }
            } else if a.get_num_transitions(s) == 0 {
                return Ok(Some(builder.get()));
            }

            // Automaton accepts more than one string:
            return Ok(None);
        }
    }

    /// Returns the longest [`BytesRef`] that is a suffix of all accepted strings.
    ///
    /// Worst case complexity: quadratic with number of states plus transitions. The
    /// returned suffix can be empty.
    ///
    /// # Errors
    ///
    /// Returns an error if the reversed automaton has dead states reachable from the
    /// initial state, or if it is not binary.
    pub fn get_common_suffix_bytes_ref(a: &Automaton) -> Result<BytesRef> {
        // reverse the language of the automaton, then reverse its common prefix.
        let r = Self::reverse(a);
        let mut prefix = Self::get_common_prefix_bytes_ref(&r)?;
        Self::reverse_bytes(&mut prefix);
        Ok(prefix)
    }

    fn reverse_bytes(reference: &mut BytesRef) {
        if reference.length <= 1 {
            return;
        }
        let num = reference.length >> 1;
        for i in reference.offset..(reference.offset + num) {
            let j = reference.offset * 2 + reference.length - i - 1;
            reference.bytes.swap(i, j);
        }
    }

    /// Returns an automaton accepting the reverse language.
    pub fn reverse(a: &Automaton) -> Automaton {
        if Self::is_empty(a) {
            return Automaton::new();
        }

        let num_states = a.get_num_states();

        // Build a new automaton with all edges reversed
        let mut builder = AutomatonBuilder::new();

        // Initial node; we'll add epsilon transitions in the end:
        builder.create_state();

        for _ in 0..num_states {
            builder.create_state();
        }

        // Old initial state becomes new accept state:
        builder.set_accept(1, true);

        let mut t = Transition::new();
        for s in 0..num_states {
            let num_transitions = a.get_num_transitions(s);
            a.init_transition(s, &mut t);
            for _ in 0..num_transitions {
                a.get_next_transition(&mut t);
                builder.add_transition_range(t.dest + 1, s + 1, t.min, t.max);
            }
        }

        let mut result = builder.finish();

        for (s, accept) in a.get_accept_states().iter().enumerate() {
            if *accept {
                result.add_epsilon(0, s as i32 + 1);
            }
        }

        result.finish_state();

        Self::remove_dead_states(&result)
    }

    /// Returns a new automaton accepting the same language with added transitions to
    /// a dead state so that from every state and every label there is a transition.
    pub(crate) fn totalize(a: &Automaton) -> Automaton {
        let mut result = Automaton::new();
        let num_states = a.get_num_states();
        for i in 0..num_states {
            result.create_state();
            result.set_accept(i, a.is_accept(i));
        }

        let dead_state = result.create_state();
        result.add_transition_range(dead_state, dead_state, MIN_CODE_POINT, MAX_CODE_POINT);

        let mut t = Transition::new();
        for i in 0..num_states {
            let mut maxi = MIN_CODE_POINT;
            let count = a.init_transition(i, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                result.add_transition_range(i, t.dest, t.min, t.max);
                if t.min > maxi {
                    result.add_transition_range(i, dead_state, maxi, t.min - 1);
                }
                if t.max + 1 > maxi {
                    maxi = t.max + 1;
                }
            }

            if maxi <= MAX_CODE_POINT {
                result.add_transition_range(i, dead_state, maxi, MAX_CODE_POINT);
            }
        }

        result.finish_state();
        result
    }

    /// Returns the topological sort of all states reachable from the initial state.
    ///
    /// This method assumes that the automaton does not contain cycles. The CPU cost
    /// is `O(numTransitions)`, and the implementation is non-recursive, so it will
    /// not exhaust the stack for automata matching long strings. If there are dead
    /// states in the automaton, they are removed from the returned list.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the input automaton has a cycle.
    pub fn topo_sort_states(a: &Automaton) -> Result<Vec<i32>> {
        if a.get_num_states() == 0 {
            return Ok(Vec::new());
        }
        let num_states = a.get_num_states();
        let mut states = vec![0i32; num_states as usize];
        let upto = Self::topo_sort_states_inner(a, &mut states)?;

        if upto < states.len() {
            // There were dead states
            states.truncate(upto);
        }

        // Reverse the order:
        states.reverse();

        Ok(states)
    }

    /// Performs a topological sort on the states of the given automaton, returning
    /// the number of states in the final sorted list.
    fn topo_sort_states_inner(a: &Automaton, states: &mut [i32]) -> Result<usize> {
        let mut on_stack = vec![false; a.get_num_states() as usize];
        let mut visited = vec![false; a.get_num_states() as usize];
        let mut stack: Vec<i32> = Vec::new();
        stack.push(0); // Assuming that the initial state is 0.
        let mut upto = 0usize;
        let mut t = Transition::new();

        while let Some(&state) = stack.last() {
            // Just peek, don't remove the state yet
            let count = a.init_transition(state, &mut t);
            let mut pushed = false;
            for _ in 0..count {
                a.get_next_transition(&mut t);
                if !visited[t.dest as usize] {
                    visited[t.dest as usize] = true;
                    stack.push(t.dest); // Push the next unvisited state onto the stack
                    on_stack[state as usize] = true;
                    pushed = true;
                    break; // Exit the loop, we'll continue from here next iteration
                } else if on_stack[t.dest as usize] {
                    // If the state is on the current recursion stack, we have
                    // detected a cycle
                    return Err(LuceneError::IllegalArgument(
                        "Input automaton has a cycle.".to_string(),
                    ));
                }
            }

            // If we haven't pushed any new state onto the stack, we're done with it
            if !pushed {
                on_stack[state as usize] = false;
                stack.pop();
                states[upto] = state;
                upto += 1;
            }
        }
        Ok(upto)
    }
}
