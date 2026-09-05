//! Port of `org.apache.lucene.util.automaton.Automaton`,
//! `org.apache.lucene.util.automaton.Transition` and
//! `org.apache.lucene.util.automaton.TransitionAccessor`.

use std::collections::HashSet;

use crate::util::{Accountable, RamUsageEstimator};

/// Minimum Unicode code point, equivalent to `Character.MIN_CODE_POINT`.
pub const MIN_CODE_POINT: i32 = 0;

/// Maximum Unicode code point, equivalent to `Character.MAX_CODE_POINT`.
pub const MAX_CODE_POINT: i32 = 0x0010_ffff;

// -----------------------------------------------------------------------------
// Transition
// -----------------------------------------------------------------------------

/// Holds one transition from an [`Automaton`]; this is typically used temporarily
/// when iterating through transitions by invoking [`TransitionAccessor::init_transition`]
/// and [`TransitionAccessor::get_next_transition`].
///
/// Equivalent to `org.apache.lucene.util.automaton.Transition`.
#[derive(Clone, Debug)]
pub struct Transition {
    /// Source state.
    pub source: i32,
    /// Destination state.
    pub dest: i32,
    /// Minimum accepted label (inclusive).
    pub min: i32,
    /// Maximum accepted label (inclusive).
    pub max: i32,
    /// Remembers where we are in the iteration; init to `-1` to provoke exception
    /// if `next` is called without first `init_transition`.
    pub(crate) transition_upto: i32,
}

impl Transition {
    /// Static estimate of the bytes one transition costs.
    ///
    /// Equivalent to `org.apache.lucene.util.automaton.Transition#BYTES_USED`,
    /// which Java computes as
    /// `RamUsageEstimator.shallowSizeOfInstance(Transition.class)`: an object
    /// header plus the five `int` fields `source`, `dest`, `min`, `max` and
    /// `transitionUpto`, rounded up to the object alignment. Measured against
    /// `lucene-core-10.5.0.jar` on a JVM with compressed references, which is
    /// the object model [`RamUsageEstimator`] encodes, `BYTES_USED` is 32.
    pub const BYTES_USED: i64 =
        RamUsageEstimator::align_object_size(RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + 5 * 4);

    /// Sole constructor, matching `new Transition()`.
    pub fn new() -> Self {
        Self {
            source: 0,
            dest: 0,
            min: 0,
            max: 0,
            // Init to -1 to provoke a panic if `get_next_transition` is called
            // without first calling `init_transition`, as Lucene does.
            transition_upto: -1,
        }
    }
}

impl Default for Transition {
    fn default() -> Self {
        Self::new()
    }
}

impl Accountable for Transition {
    /// Returns [`Transition::BYTES_USED`].
    ///
    /// Equivalent to `Transition.ramBytesUsed()`, which answers the static
    /// estimate because a transition holds nothing but primitives.
    fn ram_bytes_used(&self) -> i64 {
        Self::BYTES_USED
    }
}

impl std::fmt::Display for Transition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} --> {} {}-{}",
            self.source, self.dest, self.min, self.max
        )
    }
}

// -----------------------------------------------------------------------------
// TransitionAccessor
// -----------------------------------------------------------------------------

/// Access to transitions of an [`Automaton`].
///
/// Equivalent to `org.apache.lucene.util.automaton.TransitionAccessor`.
pub trait TransitionAccessor {
    /// Initialize the provided [`Transition`] to iterate through all transitions
    /// leaving the specified state.
    ///
    /// Returns the number of transitions leaving this state.
    fn init_transition(&self, state: i32, transition: &mut Transition) -> i32;

    /// Iterate to the next transition after the provided one.
    fn get_next_transition(&self, transition: &mut Transition);

    /// How many transitions this state has.
    fn get_num_transitions(&self, state: i32) -> i32;

    /// Fill the provided [`Transition`] with the index'th transition leaving the
    /// specified state.
    fn get_transition(&self, state: i32, index: i32, transition: &mut Transition);
}

// -----------------------------------------------------------------------------
// Automaton
// -----------------------------------------------------------------------------

/// Represents an automaton and all its states and transitions.
///
/// Equivalent to `org.apache.lucene.util.automaton.Automaton`.
///
/// States are integers and must be created using [`Automaton::create_state`]. Mark
/// a state as an accept state using [`Automaton::set_accept`]. Add transitions
/// using [`Automaton::add_transition`]. Each state must have all of its transitions
/// added at once; if this is too restrictive then use [`AutomatonBuilder`] instead.
/// State 0 is always the initial state. Once a state is finished, either because
/// you started adding transitions to another state or because you called
/// [`Automaton::finish_state`], its transitions are sorted (first by min, then max,
/// then dest) and reduced (transitions with adjacent labels going to the same dest
/// are combined).
///
/// # Divergence from Lucene 10.5.0
///
/// Lucene stores the accept states in a `java.util.BitSet` and exposes it through
/// `getAcceptStates()`. This port stores them in a growable `Vec<bool>` and exposes
/// it through [`Automaton::get_accept_states`]. The information carried is the same;
/// only the container type differs, because `java.util.BitSet` grows on demand while
/// the crate's `FixedBitSet` (the faithful counterpart of Lucene's own `FixedBitSet`)
/// does not.
#[derive(Clone, Debug)]
pub struct Automaton {
    /// Where we next write to `states`; this increments by 1 for each added state
    /// (Lucene increments by 2 because it packs two ints per state; the state count
    /// is the observable value in both).
    next_state: i32,
    /// Current state we are adding transitions to; the caller must add all
    /// transitions for this state before moving onto another state.
    cur_state: i32,
    /// Index in `transitions` where this state's leaving transitions are stored, or
    /// `-1` if this state has not added any transitions yet, followed by the number
    /// of transitions.
    states: Vec<i32>,
    /// True if this state is an accept state.
    is_accept: Vec<bool>,
    /// Holds `dest`, `min`, `max` for each transition.
    transitions: Vec<i32>,
    /// True if no state has two transitions leaving with the same label.
    deterministic: bool,
}

impl Automaton {
    /// Sole constructor; creates an automaton with no states.
    pub fn new() -> Self {
        Self::with_capacity(2, 2)
    }

    /// Creates an automaton with enough space for the given number of states and
    /// transitions.
    pub fn with_capacity(num_states: usize, num_transitions: usize) -> Self {
        Self {
            next_state: 0,
            cur_state: -1,
            states: Vec::with_capacity(num_states * 2),
            is_accept: Vec::with_capacity(num_states),
            transitions: Vec::with_capacity(num_transitions * 3),
            deterministic: true,
        }
    }

    /// Create a new state.
    pub fn create_state(&mut self) -> i32 {
        self.grow_states();
        let state = self.next_state;
        self.states.push(-1);
        self.states.push(0);
        self.is_accept.push(false);
        self.next_state += 1;
        state
    }

    /// Set or clear this state as an accept state.
    ///
    /// # Panics
    ///
    /// Panics if `state` is out of bounds, matching `Objects.checkIndex`.
    pub fn set_accept(&mut self, state: i32, accept: bool) {
        assert!(
            state >= 0 && state < self.get_num_states(),
            "Index {} out of bounds for length {}",
            state,
            self.get_num_states()
        );
        self.is_accept[state as usize] = accept;
    }

    /// Sugar to get all transitions for all states.
    ///
    /// This is object-heavy; it is better to iterate state by state instead.
    pub fn get_sorted_transitions(&self) -> Vec<Vec<Transition>> {
        let num_states = self.get_num_states();
        let mut transitions = Vec::with_capacity(num_states as usize);
        for s in 0..num_states {
            let num_transitions = self.get_num_transitions(s);
            let mut state_transitions = Vec::with_capacity(num_transitions as usize);
            for t in 0..num_transitions {
                let mut transition = Transition::new();
                TransitionAccessor::get_transition(self, s, t, &mut transition);
                state_transitions.push(transition);
            }
            transitions.push(state_transitions);
        }
        transitions
    }

    /// Returns the accept states: if the entry is `true` then that state is an
    /// accept state.
    ///
    /// Expert: use [`Automaton::is_accept`] instead, unless you really need to scan
    /// all states. Equivalent to `getAcceptStates()`.
    pub fn get_accept_states(&self) -> &[bool] {
        &self.is_accept
    }

    /// Returns true if this state is an accept state.
    pub fn is_accept(&self, state: i32) -> bool {
        if state < 0 {
            return false;
        }
        self.is_accept.get(state as usize).copied().unwrap_or(false)
    }

    /// Add a new transition with `min == max == label`.
    pub fn add_transition(&mut self, source: i32, dest: i32, label: i32) {
        self.add_transition_range(source, dest, label, label);
    }

    /// Returns whether the transition `t` is about to read comes after the one
    /// before it, in the order transitions are stored in.
    ///
    /// Equivalent to the private `Automaton.transitionSorted(Transition)`,
    /// which Java only evaluates inside an `assert`; `debug_assert!` is the
    /// same arrangement here.
    fn transition_sorted(&self, t: &Transition) -> bool {
        let upto = t.transition_upto;
        if upto == self.states[2 * t.source as usize] {
            // The transition is not initialised yet — this is the first one —
            // so there is nothing to check.
            return true;
        }

        let upto = upto as usize;
        let next_dest = self.transitions[upto];
        let next_min = self.transitions[upto + 1];
        let next_max = self.transitions[upto + 2];
        if next_min > t.min {
            return true;
        } else if next_min < t.min {
            return false;
        }

        // Min is equal; now test max.
        if next_max > t.max {
            return true;
        } else if next_max < t.max {
            return false;
        }

        // Max is also equal; now test dest.
        if next_dest > t.dest {
            return true;
        } else if next_dest < t.dest {
            return false;
        }

        // Fully equal transitions should never be seen here.
        false
    }

    /// Add a new transition with the specified source, dest, min, max.
    ///
    /// # Panics
    ///
    /// Panics if `source` or `dest` is out of bounds, or if all the transitions of
    /// `source` had already been added (Lucene throws `IllegalStateException`).
    pub fn add_transition_range(&mut self, source: i32, dest: i32, min: i32, max: i32) {
        let bounds = self.get_num_states();
        assert!(
            source >= 0 && source < bounds,
            "Index {} out of bounds for length {}",
            source,
            bounds
        );
        assert!(
            dest >= 0 && dest < bounds,
            "Index {} out of bounds for length {}",
            dest,
            bounds
        );

        self.grow_transitions();
        if self.cur_state != source {
            if self.cur_state != -1 {
                self.finish_current_state();
            }

            // Move to next source:
            self.cur_state = source;
            assert!(
                self.states[2 * source as usize] == -1,
                "from state ({}) already had transitions added",
                source
            );
            debug_assert!(self.states[2 * source as usize + 1] == 0);
            self.states[2 * source as usize] = self.transitions.len() as i32;
        }

        self.transitions.push(dest);
        self.transitions.push(min);
        self.transitions.push(max);

        // Increment transition count for this state
        self.states[2 * self.cur_state as usize + 1] += 1;
    }

    /// Add a (virtual) epsilon transition between source and dest.
    ///
    /// Dest state must already have all transitions added because this method
    /// simply copies those same transitions over to source.
    pub fn add_epsilon(&mut self, source: i32, dest: i32) {
        let mut t = Transition::new();
        let count = self.init_transition(dest, &mut t);
        for _ in 0..count {
            self.get_next_transition(&mut t);
            let (d, min, max) = (t.dest, t.min, t.max);
            self.add_transition_range(source, d, min, max);
        }
        if self.is_accept(dest) {
            self.set_accept(source, true);
        }
    }

    /// Copies over all states and transitions from `other`.
    ///
    /// The state numbers are sequentially assigned (appended).
    pub fn copy(&mut self, other: &Automaton) {
        // Bulk copy and then fixup the state pointers:
        let state_offset = self.get_num_states();
        let transition_offset = self.transitions.len() as i32;
        self.states.extend_from_slice(&other.states);
        for i in (0..other.states.len()).step_by(2) {
            let idx = 2 * state_offset as usize + i;
            if self.states[idx] != -1 {
                self.states[idx] += transition_offset;
            }
        }
        self.next_state += other.next_state;
        self.is_accept.extend_from_slice(&other.is_accept);

        // Bulk copy and then fixup dest for each transition:
        let start = self.transitions.len();
        self.transitions.extend_from_slice(&other.transitions);
        for i in (0..other.transitions.len()).step_by(3) {
            self.transitions[start + i] += state_offset;
        }

        if !other.deterministic {
            self.deterministic = false;
        }
    }

    /// Freezes the last state, sorting and reducing the transitions.
    fn finish_current_state(&mut self) {
        let cur_state = self.cur_state as usize;
        let num_transitions = self.states[2 * cur_state + 1] as usize;
        debug_assert!(num_transitions > 0);

        let offset = self.states[2 * cur_state] as usize;

        // Sort transitions by dest, ascending, then min label ascending, then max
        // label ascending. Lucene uses a stable InPlaceMergeSorter, but the ordering
        // below is total on the triple, so an unstable sort yields the same result.
        {
            let block = &mut self.transitions[offset..offset + 3 * num_transitions];
            let mut triples: Vec<[i32; 3]> =
                block.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            triples.sort_unstable_by(|a, b| {
                a[0].cmp(&b[0])
                    .then_with(|| a[1].cmp(&b[1]))
                    .then_with(|| a[2].cmp(&b[2]))
            });
            for (i, t) in triples.iter().enumerate() {
                block[3 * i] = t[0];
                block[3 * i + 1] = t[1];
                block[3 * i + 2] = t[2];
            }
        }

        // Reduce any "adjacent" transitions:
        let mut upto = 0usize;
        let mut min = -1i32;
        let mut max = -1i32;
        let mut dest = -1i32;

        for i in 0..num_transitions {
            let t_dest = self.transitions[offset + 3 * i];
            let t_min = self.transitions[offset + 3 * i + 1];
            let t_max = self.transitions[offset + 3 * i + 2];

            if dest == t_dest {
                if t_min <= max + 1 {
                    if t_max > max {
                        max = t_max;
                    }
                } else {
                    if dest != -1 {
                        self.transitions[offset + 3 * upto] = dest;
                        self.transitions[offset + 3 * upto + 1] = min;
                        self.transitions[offset + 3 * upto + 2] = max;
                        upto += 1;
                    }
                    min = t_min;
                    max = t_max;
                }
            } else {
                if dest != -1 {
                    self.transitions[offset + 3 * upto] = dest;
                    self.transitions[offset + 3 * upto + 1] = min;
                    self.transitions[offset + 3 * upto + 2] = max;
                    upto += 1;
                }
                dest = t_dest;
                min = t_min;
                max = t_max;
            }
        }

        if dest != -1 {
            // Last transition
            self.transitions[offset + 3 * upto] = dest;
            self.transitions[offset + 3 * upto + 1] = min;
            self.transitions[offset + 3 * upto + 2] = max;
            upto += 1;
        }

        // The transitions of the current state always occupy the tail of the array,
        // so dropping the reduced-away entries is a truncation.
        self.transitions.truncate(offset + 3 * upto);
        self.states[2 * cur_state + 1] = upto as i32;

        // Sort transitions by min/max/dest:
        {
            let block = &mut self.transitions[offset..offset + 3 * upto];
            let mut triples: Vec<[i32; 3]> =
                block.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            triples.sort_unstable_by(|a, b| {
                a[1].cmp(&b[1])
                    .then_with(|| a[2].cmp(&b[2]))
                    .then_with(|| a[0].cmp(&b[0]))
            });
            for (i, t) in triples.iter().enumerate() {
                block[3 * i] = t[0];
                block[3 * i + 1] = t[1];
                block[3 * i + 2] = t[2];
            }
        }

        if self.deterministic && upto > 1 {
            let mut last_max = self.transitions[offset + 2];
            for i in 1..upto {
                let min = self.transitions[offset + 3 * i + 1];
                if min <= last_max {
                    self.deterministic = false;
                    break;
                }
                last_max = self.transitions[offset + 3 * i + 2];
            }
        }
    }

    /// Returns true if this automaton is deterministic (for every state there is
    /// only one transition for each label).
    pub fn is_deterministic(&self) -> bool {
        self.deterministic
    }

    /// Finishes the current state; call this once you are done adding transitions
    /// for a state.
    ///
    /// This is automatically called if you start adding transitions to a new source
    /// state, but for the last state you add you need to call this method yourself.
    pub fn finish_state(&mut self) {
        if self.cur_state != -1 {
            self.finish_current_state();
            self.cur_state = -1;
        }
    }

    /// Finishes all open state construction.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Lucene's `Automaton` has no `finish()`; this is a pre-existing Rucene alias
    /// for [`Automaton::finish_state`], kept because `analysis::automaton_conversion`
    /// calls it.
    pub fn finish(&mut self) {
        self.finish_state();
    }

    /// How many states this automaton has.
    pub fn get_num_states(&self) -> i32 {
        self.next_state
    }

    /// How many transitions this automaton has.
    pub fn get_num_transitions_total(&self) -> i32 {
        (self.transitions.len() / 3) as i32
    }

    /// Ensures the `states` array has room for one more state.
    ///
    /// Lucene keeps `states` as a plain `int[]` whose length is the capacity and
    /// `nextState` as the write pointer, so it grows the array whenever
    /// `nextState + 2 > states.length`. This port keeps `states` as a `Vec<i32>`
    /// whose length is exactly the used prefix, so the same guarantee is
    /// obtained by reserving room for the two slots the next state occupies.
    ///
    /// Equivalent to the private `org.apache.lucene.util.automaton.Automaton#growStates`.
    fn grow_states(&mut self) {
        self.states.reserve(2);
    }

    /// Ensures the `transitions` array has room for one more transition.
    ///
    /// The counterpart of [`Automaton::grow_states`] for the three slots
    /// (`dest`, `min`, `max`) that each transition occupies.
    ///
    /// Equivalent to the private `org.apache.lucene.util.automaton.Automaton#growTransitions`.
    fn grow_transitions(&mut self) {
        self.transitions.reserve(3);
    }

    /// How many transitions this state has.
    pub fn get_num_transitions(&self, state: i32) -> i32 {
        debug_assert!(state >= 0);
        debug_assert!(state < self.get_num_states());
        let count = self.states[2 * state as usize + 1];
        if count == -1 {
            0
        } else {
            count
        }
    }

    /// Returns the dot (graphviz) representation of this automaton.
    ///
    /// This is extremely useful for visualizing the automaton.
    pub fn to_dot(&self) -> String {
        let mut b = String::new();
        b.push_str("digraph Automaton {\n");
        b.push_str("  rankdir = LR\n");
        b.push_str("  node [width=0.2, height=0.2, fontsize=8]\n");
        let num_states = self.get_num_states();
        if num_states > 0 {
            b.push_str("  initial [shape=plaintext,label=\"\"]\n");
            b.push_str("  initial -> 0\n");
        }

        let mut t = Transition::new();

        for state in 0..num_states {
            b.push_str("  ");
            b.push_str(&state.to_string());
            if self.is_accept(state) {
                b.push_str(&format!(" [shape=doublecircle,label=\"{}\"]\n", state));
            } else {
                b.push_str(&format!(" [shape=circle,label=\"{}\"]\n", state));
            }
            let num_transitions = self.init_transition(state, &mut t);
            for _ in 0..num_transitions {
                self.get_next_transition(&mut t);
                debug_assert!(t.max >= t.min);
                b.push_str("  ");
                b.push_str(&state.to_string());
                b.push_str(" -> ");
                b.push_str(&t.dest.to_string());
                b.push_str(" [label=\"");
                append_char_string(t.min, &mut b);
                if t.max != t.min {
                    b.push('-');
                    append_char_string(t.max, &mut b);
                }
                b.push_str("\"]\n");
            }
        }
        b.push('}');
        b
    }

    /// Returns the sorted array of all interval start points.
    pub fn get_start_points(&self) -> Vec<i32> {
        let mut pointset: HashSet<i32> = HashSet::new();
        pointset.insert(MIN_CODE_POINT);
        for s in 0..self.get_num_states() {
            let mut trans = self.states[2 * s as usize];
            if trans == -1 {
                continue;
            }
            let limit = trans + 3 * self.states[2 * s as usize + 1];
            while trans < limit {
                let min = self.transitions[trans as usize + 1];
                let max = self.transitions[trans as usize + 2];
                pointset.insert(min);
                if max < MAX_CODE_POINT {
                    pointset.insert(max + 1);
                }
                trans += 3;
            }
        }
        let mut points: Vec<i32> = pointset.into_iter().collect();
        points.sort_unstable();
        points
    }

    /// Performs lookup in transitions, assuming determinism.
    ///
    /// Returns the destination state, or `-1` if there is no matching outgoing
    /// transition.
    pub fn step(&self, state: i32, label: i32) -> i32 {
        self.next_inner(state, 0, label, None)
    }

    /// Looks for the next transition that matches the provided label, assuming
    /// determinism.
    ///
    /// This method is similar to [`Automaton::step`] but is used more efficiently
    /// when iterating over multiple transitions from the same source state. It keeps
    /// the latest reached transition index in the transition so the next call can
    /// continue from there instead of restarting from the first transition.
    ///
    /// Returns the destination state, or `-1` if there is no matching outgoing
    /// transition.
    pub fn next(&self, transition: &mut Transition, label: i32) -> i32 {
        let source = transition.source;
        let from = transition.transition_upto;
        self.next_inner(source, from, label, Some(transition))
    }

    /// Looks for the next transition that matches the provided label, assuming
    /// determinism.
    fn next_inner(
        &self,
        state: i32,
        from_transition_index: i32,
        label: i32,
        transition: Option<&mut Transition>,
    ) -> i32 {
        debug_assert!(state >= 0);
        debug_assert!(label >= 0);
        let state_index = 2 * state as usize;
        if state_index + 1 >= self.states.len() {
            // Lucene's `states` is a pre-allocated `int[]` that is always at least
            // two entries long, so a probe of state 0 on an automaton with no states
            // reads zeros and reports "no transitions". `RunAutomaton` relies on
            // that: it sizes itself `max(1, numStates)` and probes state 0 even for
            // an empty automaton. A `Vec` has no such tail, so the same answer is
            // produced explicitly.
            if let Some(t) = transition {
                t.dest = -1;
                t.transition_upto = from_transition_index.max(0);
            }
            return -1;
        }
        let first_transition_index = self.states[state_index];
        let num_transitions = self.states[state_index + 1];

        // Since transitions are sorted, binary search the transition for which label
        // is within [minLabel, maxLabel].
        let mut low = from_transition_index.max(0);
        let mut high = num_transitions - 1;
        while low <= high {
            let mid = (low + high) >> 1;
            let transition_index = (first_transition_index + 3 * mid) as usize;
            let min_label = self.transitions[transition_index + 1];
            if min_label > label {
                high = mid - 1;
            } else {
                let max_label = self.transitions[transition_index + 2];
                if max_label < label {
                    low = mid + 1;
                } else {
                    let dest_state = self.transitions[transition_index];
                    if let Some(t) = transition {
                        t.dest = dest_state;
                        t.min = min_label;
                        t.max = max_label;
                        t.transition_upto = mid;
                    }
                    return dest_state;
                }
            }
        }
        let dest_state = -1;
        if let Some(t) = transition {
            t.dest = dest_state;
            t.transition_upto = low;
        }
        dest_state
    }
}

impl Accountable for Automaton {
    /// Returns an estimate of the heap this automaton occupies.
    ///
    /// Equivalent to `org.apache.lucene.util.automaton.Automaton#ramBytesUsed`
    /// (`Automaton.java:935-946`), term for term:
    ///
    /// ```text
    /// NUM_BYTES_OBJECT_HEADER          // the Automaton itself
    /// + sizeOf(states)
    /// + sizeOf(transitions)
    /// + NUM_BYTES_OBJECT_HEADER        // the isAccept BitSet
    /// + isAccept.size() / 8            // its backing words
    /// + NUM_BYTES_OBJECT_REF           // BitSet.words
    /// + 2 * NUM_BYTES_OBJECT_REF       // states, transitions
    /// + 3 * Integer.BYTES              // nextState, curState, and BitSet.wordsInUse
    /// + 1                              // deterministic
    /// ```
    ///
    /// Java carries the accept states in a `java.util.BitSet` and charges
    /// `isAccept.size() / 8`, where `BitSet.size()` is the *allocated* word
    /// count times 64. This port stores them in a `Vec<bool>`, so the term is
    /// the size a `java.util.BitSet` constructed for the same number of states
    /// would report: one 64-bit word per 64 states, and never fewer than one
    /// word, which is what `new BitSet(numStates)` allocates.
    ///
    /// Java's own comment on this method records that the `BitSet` term is not
    /// fully accurate; the value is an estimate on both sides.
    fn ram_bytes_used(&self) -> i64 {
        // TODO: BitSet RAM usage (isAccept.size()/8) isn't fully accurate...
        let is_accept_words = self.is_accept.len().div_ceil(64).max(1) as i64;
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + RamUsageEstimator::size_of_int(&self.states)
            + RamUsageEstimator::size_of_int(&self.transitions)
            + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + is_accept_words * 8
            + RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + 2 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + 3 * 4
            + 1
    }
}

impl Default for Automaton {
    fn default() -> Self {
        Self::new()
    }
}

impl TransitionAccessor for Automaton {
    fn init_transition(&self, state: i32, t: &mut Transition) -> i32 {
        debug_assert!(
            state < self.next_state,
            "state={} numStates={}",
            state,
            self.next_state
        );
        t.source = state;
        t.transition_upto = self.states[2 * state as usize];
        self.get_num_transitions(state)
    }

    fn get_next_transition(&self, t: &mut Transition) {
        // Make sure there is still a transition left.
        debug_assert!(
            (t.transition_upto + 3 - self.states[2 * t.source as usize])
                <= 3 * self.states[2 * t.source as usize + 1]
        );
        // Make sure transitions are in fact sorted.
        debug_assert!(self.transition_sorted(t));

        let upto = t.transition_upto as usize;
        t.dest = self.transitions[upto];
        t.min = self.transitions[upto + 1];
        t.max = self.transitions[upto + 2];
        t.transition_upto += 3;
    }

    fn get_num_transitions(&self, state: i32) -> i32 {
        Automaton::get_num_transitions(self, state)
    }

    fn get_transition(&self, state: i32, index: i32, t: &mut Transition) {
        let mut i = (self.states[2 * state as usize] + 3 * index) as usize;
        t.source = state;
        t.dest = self.transitions[i];
        i += 1;
        t.min = self.transitions[i];
        i += 1;
        t.max = self.transitions[i];
    }
}

/// Appends `c` to `b`, escaping it the way Lucene's `Automaton.appendCharString`
/// does when rendering an automaton to dot.
pub(crate) fn append_char_string(c: i32, b: &mut String) {
    if (0x21..=0x7e).contains(&c) && c != ('\\' as i32) && c != ('"' as i32) {
        if let Some(ch) = char::from_u32(c as u32) {
            b.push(ch);
            return;
        }
    }
    b.push_str("\\\\U");
    let s = format!("{:x}", c);
    let pad = match c {
        c if c < 0x10 => 7,
        c if c < 0x100 => 6,
        c if c < 0x1000 => 5,
        c if c < 0x10000 => 4,
        c if c < 0x100000 => 3,
        c if c < 0x1000000 => 2,
        c if c < 0x10000000 => 1,
        _ => 0,
    };
    for _ in 0..pad {
        b.push('0');
    }
    b.push_str(&s);
}

// -----------------------------------------------------------------------------
// Automaton.Builder
// -----------------------------------------------------------------------------

/// Records new states and transitions and then [`AutomatonBuilder::finish`] creates
/// the [`Automaton`].
///
/// Equivalent to `org.apache.lucene.util.automaton.Automaton.Builder`. Use this when
/// you cannot create the [`Automaton`] directly because it is too restrictive to have
/// to add all transitions leaving each state at once.
#[derive(Clone, Debug)]
pub struct AutomatonBuilder {
    next_state: i32,
    is_accept: Vec<bool>,
    /// Holds `source`, `dest`, `min`, `max` for each transition.
    transitions: Vec<i32>,
}

impl AutomatonBuilder {
    /// Default constructor, pre-allocating for 16 states and transitions.
    pub fn new() -> Self {
        Self::with_capacity(16, 16)
    }

    /// Creates a builder with enough space for the given number of states and
    /// transitions.
    pub fn with_capacity(num_states: usize, num_transitions: usize) -> Self {
        Self {
            next_state: 0,
            is_accept: Vec::with_capacity(num_states),
            transitions: Vec::with_capacity(num_transitions * 4),
        }
    }

    /// Add a new transition with `min == max == label`.
    pub fn add_transition(&mut self, source: i32, dest: i32, label: i32) {
        self.add_transition_range(source, dest, label, label);
    }

    /// Add a new transition with the specified source, dest, min, max.
    pub fn add_transition_range(&mut self, source: i32, dest: i32, min: i32, max: i32) {
        self.transitions.push(source);
        self.transitions.push(dest);
        self.transitions.push(min);
        self.transitions.push(max);
    }

    /// Add a (virtual) epsilon transition between source and dest.
    ///
    /// Dest state must already have all transitions added because this method simply
    /// copies those same transitions over to source.
    pub fn add_epsilon(&mut self, source: i32, dest: i32) {
        let mut upto = 0usize;
        while upto < self.transitions.len() {
            if self.transitions[upto] == dest {
                let (d, min, max) = (
                    self.transitions[upto + 1],
                    self.transitions[upto + 2],
                    self.transitions[upto + 3],
                );
                self.add_transition_range(source, d, min, max);
            }
            upto += 4;
        }
        if self.is_accept(dest) {
            self.set_accept(source, true);
        }
    }

    /// Compiles all added states and transitions into a new [`Automaton`].
    pub fn finish(&mut self) -> Automaton {
        // Create automaton with the correct size.
        let num_states = self.next_state;
        let num_transitions = self.transitions.len() / 4;
        let mut a = Automaton::with_capacity(num_states as usize, num_transitions);

        // Create all states.
        for state in 0..num_states {
            a.create_state();
            a.set_accept(state, self.is_accept(state));
        }

        // Sort transitions by source, then min label ascending, then max label
        // ascending, then dest ascending.
        let mut quads: Vec<[i32; 4]> = self
            .transitions
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        quads.sort_unstable_by(|a, b| {
            a[0].cmp(&b[0])
                .then_with(|| a[2].cmp(&b[2]))
                .then_with(|| a[3].cmp(&b[3]))
                .then_with(|| a[1].cmp(&b[1]))
        });
        for q in &quads {
            a.add_transition_range(q[0], q[1], q[2], q[3]);
        }

        a.finish_state();

        a
    }

    /// Create a new state.
    pub fn create_state(&mut self) -> i32 {
        let state = self.next_state;
        self.is_accept.push(false);
        self.next_state += 1;
        state
    }

    /// Set or clear this state as an accept state.
    ///
    /// # Panics
    ///
    /// Panics if `state` is out of bounds, matching `Objects.checkIndex`.
    pub fn set_accept(&mut self, state: i32, accept: bool) {
        assert!(
            state >= 0 && state < self.get_num_states(),
            "Index {} out of bounds for length {}",
            state,
            self.get_num_states()
        );
        self.is_accept[state as usize] = accept;
    }

    /// Returns true if this state is an accept state.
    pub fn is_accept(&self, state: i32) -> bool {
        if state < 0 {
            return false;
        }
        self.is_accept.get(state as usize).copied().unwrap_or(false)
    }

    /// How many states this automaton has.
    pub fn get_num_states(&self) -> i32 {
        self.next_state
    }

    /// Copies over all states and transitions from `other`.
    pub fn copy(&mut self, other: &Automaton) {
        let offset = self.get_num_states();
        let other_num_states = other.get_num_states();

        // Copy all states
        self.copy_states(other);

        // Copy all transitions
        let mut t = Transition::new();
        for s in 0..other_num_states {
            let count = other.init_transition(s, &mut t);
            for _ in 0..count {
                other.get_next_transition(&mut t);
                self.add_transition_range(offset + s, offset + t.dest, t.min, t.max);
            }
        }
    }

    /// Copies over all states from `other`.
    pub fn copy_states(&mut self, other: &Automaton) {
        let other_num_states = other.get_num_states();
        for s in 0..other_num_states {
            let new_state = self.create_state();
            self.set_accept(new_state, other.is_accept(s));
        }
    }
}

impl Default for AutomatonBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod ram_bytes_used_tests {
    use super::*;
    use crate::util::automaton::byte_run_automaton::ByteRunAutomaton;
    use crate::util::automaton::compiled_automaton::CompiledAutomaton;
    use crate::util::automaton::nfa_run_automaton::NFARunAutomaton;

    /// The two-state automaton `a-z` that the Java oracle below was run on.
    ///
    /// Reproduced from:
    /// ```text
    /// Automaton.Builder b = new Automaton.Builder();
    /// int s0 = b.createState(); int s1 = b.createState();
    /// b.setAccept(s1, true);
    /// b.addTransition(s0, s1, 'a', 'z');
    /// Automaton au = b.finish();
    /// ```
    fn a_to_z() -> Automaton {
        let mut b = AutomatonBuilder::new();
        let s0 = b.create_state();
        let s1 = b.create_state();
        b.set_accept(s1, true);
        b.add_transition_range(s0, s1, 'a' as i32, 'z' as i32);
        b.finish()
    }

    #[test]
    fn transition_bytes_used_is_the_java_shallow_size() {
        // `RamUsageEstimator.shallowSizeOfInstance(Transition.class)` measured
        // against lucene-core-10.5.0.jar on a JVM with compressed references.
        assert_eq!(Transition::BYTES_USED, 32);
        assert_eq!(Transition::new().ram_bytes_used(), 32);
    }

    #[test]
    fn automaton_ram_bytes_used_matches_lucene() {
        let a = a_to_z();
        // Java's terms, written out with its own constants rather than with the
        // expression the implementation uses. For this automaton the real
        // `lucene-core-10.5.0.jar` reports `states.length == 4`,
        // `transitions.length == 3` and `isAccept.size() == 64`.
        let expected: i64 = 12                       // NUM_BYTES_OBJECT_HEADER (Automaton)
            + (16 + 4 * 4)                           // sizeOf(int[4] states)
            + ((16 + 3 * 4) + 4)                     // sizeOf(int[3] transitions), aligned to 32
            + 12                                     // NUM_BYTES_OBJECT_HEADER (isAccept BitSet)
            + 64 / 8                                 // isAccept.size() / 8
            + 4                                      // NUM_BYTES_OBJECT_REF (BitSet.words)
            + 2 * 4                                  // 2 * NUM_BYTES_OBJECT_REF
            + 3 * 4                                  // 3 * Integer.BYTES
            + 1; // deterministic
        assert_eq!(expected, 121, "the transcription of Java's terms");
        // The value the real lucene-core-10.5.0.jar reports for this automaton.
        assert_eq!(a.ram_bytes_used(), 121);
    }

    #[test]
    fn run_automaton_ram_bytes_used_matches_lucene() {
        let a = a_to_z();
        let bra = ByteRunAutomaton::new(a, false).expect("byte run automaton");
        // Java's terms for `new ByteRunAutomaton(au)`, each measured against
        // lucene-core-10.5.0.jar: BASE_RAM_BYTES 40, accept 48 (a
        // `FixedBitSet` of 2 bits), the inner automaton 121, `classmap` an
        // int[256] costing 1040, `points` an int[3] costing 32, and
        // `transitions` an int[6] costing 40.
        let expected: i64 = 40 + 48 + 121 + 1040 + 32 + 40;
        assert_eq!(expected, 1321, "the transcription of Java's terms");
        assert_eq!(bra.ram_bytes_used(), 1321);
    }

    #[test]
    fn compiled_automaton_ram_bytes_used_matches_lucene() {
        let a = a_to_z();
        let ca = CompiledAutomaton::from_automaton(a).expect("compiled automaton");
        // Java's terms: BASE_RAM_BYTES 48, a null `commonSuffixRef` and `term`
        // and `nfaRunAutomaton` costing nothing, the `ByteRunAutomaton` costing
        // 1321, and the scratch `Transition` costing 32.
        let expected: i64 = 48 + 0 + 1321 + 0 + 0 + 32;
        assert_eq!(expected, 1401, "the transcription of Java's terms");
        assert_eq!(ca.ram_bytes_used(), 1401);
    }

    #[test]
    fn nfa_run_automaton_ram_bytes_used_charges_every_java_term() {
        let a = a_to_z();
        let nfa = NFARunAutomaton::new(a.clone(), MAX_CODE_POINT + 1);
        let total = nfa.ram_bytes_used();

        // The fixed part is Java's `shallowSizeOfInstance(NFARunAutomaton.class)`,
        // measured against lucene-core-10.5.0.jar as 48 bytes.
        let base: i64 = 48;
        // The automaton term is the same value `Automaton.ramBytesUsed()`
        // answers, which the test above pins to Lucene's own 121.
        let automaton_term = a.ram_bytes_used();
        assert_eq!(automaton_term, 121);

        // Every term is non-negative and the total is strictly above the fixed
        // part plus the automaton: an empty registry and an empty `dStates`
        // array still cost their headers.
        assert!(
            total > base + automaton_term,
            "total={total} must exceed base+automaton={}",
            base + automaton_term
        );

        // Determinizing a state must charge for it: the total can only grow.
        use crate::util::automaton::run_automaton::ByteRunnable;
        let before = nfa.ram_bytes_used();
        let s = nfa.step(0, 'a' as i32);
        assert!(s >= 0, "'a' must be accepted from the initial state");
        assert!(
            nfa.ram_bytes_used() >= before,
            "determinizing a state must not lower the estimate"
        );
    }
}
