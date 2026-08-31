//! Port of `org.apache.lucene.util.automaton.NFARunAutomaton`.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::internal::hppc::BitMixer;

use super::automaton::{Automaton, Transition, TransitionAccessor, MAX_CODE_POINT};
use super::int_set::StateSet;
use super::operations::PointTransitionSet;
use super::run_automaton::ByteRunnable;

/// State ordinal of "no such state".
const MISSING: i32 = -1;

/// Marker for a transition that has not been determinized yet.
const NOT_COMPUTED: i32 = -2;

/// One lazily determinized DFA state, standing for a set of NFA states.
///
/// Equivalent to the private `NFARunAutomaton.DState`.
#[derive(Clone, Debug)]
struct DState {
    nfa_states: Vec<i32>,
    /// Lazily initialized the first time a caller wants to add a new transition.
    transitions: Option<Vec<i32>>,
    hash_code: i32,
    is_accept: bool,
    minimal_transition: Option<Transition>,
    computed_transitions: usize,
    outgoing_transitions: i32,
}

impl DState {
    fn new(nfa_states: Vec<i32>, automaton: &Automaton) -> Self {
        debug_assert!(!nfa_states.is_empty());
        let mut hash_code = nfa_states.len() as i32;
        let mut is_accept = false;
        for &s in &nfa_states {
            hash_code = hash_code.wrapping_add(BitMixer::mix_i32(s));
            if automaton.is_accept(s) {
                is_accept = true;
            }
        }
        Self {
            nfa_states,
            transitions: None,
            hash_code,
            is_accept,
            minimal_transition: None,
            computed_transitions: 0,
            outgoing_transitions: 0,
        }
    }

    /// The hash code Lucene computes for this state; kept for parity even though the
    /// registry below is keyed by the (equivalent) NFA state array.
    fn hash_code(&self) -> i32 {
        self.hash_code
    }
}

/// Mutable determinization state, kept behind a [`RefCell`] so the read-only
/// [`ByteRunnable`] and [`TransitionAccessor`] contracts can be honoured.
#[derive(Debug, Default)]
struct Inner {
    d_state_to_ord: HashMap<(i32, Vec<i32>), i32>,
    d_states: Vec<DState>,
    /// Reusable, forked from `Operations.determinize`.
    transition_set: PointTransitionSet,
    /// Reusable.
    states_set: StateSet,
}

/// A [`RunAutomaton`](super::run_automaton::RunAutomaton) that does not require a
/// DFA.
///
/// Equivalent to `org.apache.lucene.util.automaton.NFARunAutomaton`. It lazily
/// determinizes on demand, memoizing the generated DFA states that have been
/// explored. Implemented based on <https://swtch.com/~rsc/regexp/regexp1.html>.
///
/// # Divergences from Lucene 10.5.0
///
/// * Lucene states that this class is **not** thread-safe and mutates its fields
///   from methods that are logically read-only. Rucene keeps the same single-threaded
///   contract but holds the mutable determinization state in a [`RefCell`], because
///   `ByteRunnable::step` and `TransitionAccessor` take `&self`.
/// * `getSize()` returns the *capacity* of Lucene's backing `DState[]` array; this
///   port returns the number of DFA states discovered so far, which is the value that
///   array length stands in for.
#[derive(Debug)]
pub struct NFARunAutomaton {
    automaton: Automaton,
    points: Vec<i32>,
    alphabet_size: i32,
    /// Map from char number to class.
    classmap: Vec<i32>,
    inner: RefCell<Inner>,
}

impl Clone for NFARunAutomaton {
    fn clone(&self) -> Self {
        Self {
            automaton: self.automaton.clone(),
            points: self.points.clone(),
            alphabet_size: self.alphabet_size,
            classmap: self.classmap.clone(),
            inner: RefCell::new(Inner {
                d_state_to_ord: self.inner.borrow().d_state_to_ord.clone(),
                d_states: self.inner.borrow().d_states.clone(),
                transition_set: PointTransitionSet::default(),
                states_set: StateSet::new(5),
            }),
        }
    }
}

impl PartialEq for NFARunAutomaton {
    fn eq(&self, other: &Self) -> bool {
        self.alphabet_size == other.alphabet_size && self.points == other.points
    }
}

impl Eq for NFARunAutomaton {}

impl NFARunAutomaton {
    /// Constructor, assuming the alphabet size is the whole Unicode code point space.
    ///
    /// The incoming automaton should be an NFA; for a DFA please use
    /// [`RunAutomaton`](super::run_automaton::RunAutomaton) for better efficiency.
    pub fn from_automaton(automaton: Automaton) -> Self {
        Self::new(automaton, MAX_CODE_POINT + 1)
    }

    /// Constructor with an explicit alphabet size.
    pub fn new(automaton: Automaton, alphabet_size: i32) -> Self {
        let points = automaton.get_start_points();

        // Set alphabet table for optimal run performance.
        let mut classmap = vec![0i32; 256.min(alphabet_size).max(0) as usize];
        let mut i = 0usize;
        for (j, slot) in classmap.iter_mut().enumerate() {
            if i + 1 < points.len() && j as i32 == points[i + 1] {
                i += 1;
            }
            *slot = i as i32;
        }

        let this = Self {
            automaton,
            points,
            alphabet_size,
            classmap,
            inner: RefCell::new(Inner {
                d_state_to_ord: HashMap::new(),
                d_states: Vec::new(),
                transition_set: PointTransitionSet::default(),
                states_set: StateSet::new(5),
            }),
        };
        {
            let mut inner = this.inner.borrow_mut();
            inner.find_d_state(Some(vec![0]), &this.automaton);
        }
        this
    }

    /// Returns the source automaton this NFA runs.
    pub fn automaton(&self) -> &Automaton {
        &self.automaton
    }

    /// Gets the character class of the given codepoint.
    pub fn get_char_class(&self, c: i32) -> i32 {
        debug_assert!(c < self.alphabet_size);

        if (c as usize) < self.classmap.len() {
            return self.classmap[c as usize];
        }

        // binary search
        let mut a = 0usize;
        let mut b = self.points.len();
        while b - a > 1 {
            let d = (a + b) >> 1;
            match self.points[d].cmp(&c) {
                std::cmp::Ordering::Greater => b = d,
                std::cmp::Ordering::Less => a = d,
                std::cmp::Ordering::Equal => return d as i32,
            }
        }
        a as i32
    }

    /// Runs through a given codepoint array and returns whether it is accepted.
    pub fn run(&self, s: &[i32]) -> bool {
        let mut p = 0i32;
        for &c in s {
            p = ByteRunnable::step(self, p, c);
            if p == MISSING {
                return false;
            }
        }
        self.inner.borrow().d_states[p as usize].is_accept
    }

    fn set_transition_accordingly(&self, t: &mut Transition, inner: &Inner) {
        t.dest = inner.d_states[t.source as usize]
            .transitions
            .as_ref()
            .expect("INVARIANT: transitions were initialized by determinize")
            [t.transition_upto as usize];
        t.min = self.points[t.transition_upto as usize];
        if t.transition_upto == self.points.len() as i32 - 1 {
            t.max = self.alphabet_size - 1;
        } else {
            t.max = self.points[t.transition_upto as usize + 1] - 1;
        }
    }
}

impl Inner {
    /// Returns the ordinal of the given DFA state, generating a new ordinal if the
    /// given DFA state is a new one. `None` maps to [`MISSING`].
    fn find_d_state(&mut self, nfa_states: Option<Vec<i32>>, automaton: &Automaton) -> i32 {
        let Some(nfa_states) = nfa_states else {
            return MISSING;
        };
        let d_state = DState::new(nfa_states, automaton);
        let key = (d_state.hash_code(), d_state.nfa_states.clone());
        if let Some(&ord) = self.d_state_to_ord.get(&key) {
            return ord;
        }
        let ord = self.d_state_to_ord.len() as i32;
        self.d_state_to_ord.insert(key, ord);
        debug_assert!(ord as usize >= self.d_states.len());
        self.d_states.push(d_state);
        ord
    }

    fn init_transitions(&mut self, ord: i32, num_points: usize) {
        let d = &mut self.d_states[ord as usize];
        if d.transitions.is_none() {
            d.transitions = Some(vec![NOT_COMPUTED; num_points]);
        }
    }

    fn assign_transition(&mut self, ord: i32, char_class: i32, dest: i32) {
        let d = &mut self.d_states[ord as usize];
        let transitions = d
            .transitions
            .as_mut()
            .expect("INVARIANT: init_transitions runs before assign_transition");
        if transitions[char_class as usize] == NOT_COMPUTED {
            d.computed_transitions += 1;
            transitions[char_class as usize] = dest;
            if dest != MISSING {
                d.outgoing_transitions += 1;
            }
        }
    }

    fn transition_at(&self, ord: i32, char_class: i32) -> i32 {
        self.d_states[ord as usize]
            .transitions
            .as_ref()
            .expect("INVARIANT: init_transitions runs first")[char_class as usize]
    }

    /// Given the list of NFA states of `ord` and a character `c`, computes the output
    /// list of NFA states, wrapped as a DFA state.
    ///
    /// Also records the minimal transition interval covering `c` on `ord`.
    fn step_d_state(
        &mut self,
        ord: i32,
        c: i32,
        automaton: &Automaton,
        alphabet_size: i32,
    ) -> Option<Vec<i32>> {
        self.states_set.reset();
        let mut left = -1i32;
        let mut right = alphabet_size;
        let mut step_transition = Transition::new();
        let nfa_states = self.d_states[ord as usize].nfa_states.clone();
        for nfa_state in nfa_states {
            let num_transitions = automaton.init_transition(nfa_state, &mut step_transition);
            for _ in 0..num_transitions {
                automaton.get_next_transition(&mut step_transition);
                if step_transition.min <= c && step_transition.max >= c {
                    self.states_set.incr(step_transition.dest);
                    left = step_transition.min.max(left);
                    right = step_transition.max.min(right);
                }
                if step_transition.max < c {
                    left = (step_transition.max + 1).max(left);
                }
                if step_transition.min > c {
                    right = (step_transition.min - 1).min(right);
                    // transitions in the automaton are sorted
                    break;
                }
            }
        }
        if self.states_set.size() == 0 {
            return None;
        }
        let mut minimal_transition = Transition::new();
        minimal_transition.min = left;
        minimal_transition.max = right;
        self.d_states[ord as usize].minimal_transition = Some(minimal_transition);
        Some(self.states_set.get_array().to_vec())
    }

    fn next_state(
        &mut self,
        ord: i32,
        char_class: i32,
        automaton: &Automaton,
        points: &[i32],
        alphabet_size: i32,
    ) -> i32 {
        self.init_transitions(ord, points.len());
        debug_assert!((char_class as usize) < points.len());
        if self.transition_at(ord, char_class) == NOT_COMPUTED {
            let stepped =
                self.step_d_state(ord, points[char_class as usize], automaton, alphabet_size);
            let dest = self.find_d_state(stepped, automaton);
            self.assign_transition(ord, char_class, dest);
            // we could potentially update more than one char class
            if let Some(minimal_transition) = self.d_states[ord as usize].minimal_transition.clone()
            {
                let value = self.transition_at(ord, char_class);
                // to the left
                let mut cls = char_class;
                while cls > 0 {
                    cls -= 1;
                    if points[cls as usize] < minimal_transition.min {
                        break;
                    }
                    debug_assert!(
                        self.transition_at(ord, cls) == NOT_COMPUTED
                            || self.transition_at(ord, cls) == value
                    );
                    self.assign_transition(ord, cls, value);
                }
                // to the right
                let mut cls = char_class;
                while (cls as usize) < points.len() - 1 {
                    cls += 1;
                    if points[cls as usize] > minimal_transition.max {
                        break;
                    }
                    debug_assert!(
                        self.transition_at(ord, cls) == NOT_COMPUTED
                            || self.transition_at(ord, cls) == value
                    );
                    self.assign_transition(ord, cls, value);
                }
                self.d_states[ord as usize].minimal_transition = None;
            }
        }
        self.transition_at(ord, char_class)
    }

    /// Determinizes this state only.
    fn determinize_state(
        &mut self,
        ord: i32,
        automaton: &Automaton,
        points: &[i32],
        alphabet_size: i32,
    ) {
        {
            let d = &self.d_states[ord as usize];
            if let Some(transitions) = &d.transitions {
                if d.computed_transitions == transitions.len() {
                    // already determinized
                    return;
                }
            }
        }
        self.init_transitions(ord, points.len());
        // Mostly forked from Operations.determinize
        self.transition_set.reset();
        let mut step_transition = Transition::new();
        let nfa_states = self.d_states[ord as usize].nfa_states.clone();
        for nfa_state in &nfa_states {
            let num_transitions = automaton.init_transition(*nfa_state, &mut step_transition);
            for _ in 0..num_transitions {
                automaton.get_next_transition(&mut step_transition);
                self.transition_set.add(&step_transition);
            }
        }
        if self.transition_set.count == 0 {
            // no outgoing transitions
            let len = points.len();
            for cls in 0..len {
                self.force_transition(ord, cls as i32, MISSING);
            }
            self.d_states[ord as usize].computed_transitions = len;
            return;
        }

        // could use a PQ (heap) instead, since transitions for each state are sorted
        self.transition_set.sort();
        self.states_set.reset();
        let mut last_point = -1i32;
        let mut char_class = 0i32;
        for i in 0..self.transition_set.count {
            let point = self.transition_set.points[i].point;
            if self.states_set.size() > 0 {
                debug_assert!(last_point != -1);
                let values = self.states_set.get_array().to_vec();
                let dest_ord = self.find_d_state(Some(values), automaton);
                while points[char_class as usize] < last_point {
                    self.assign_transition(ord, char_class, MISSING);
                    char_class += 1;
                }
                debug_assert!(points[char_class as usize] == last_point);
                while (char_class as usize) < points.len() && points[char_class as usize] < point {
                    self.assign_transition(ord, char_class, dest_ord);
                    char_class += 1;
                }
                debug_assert!(
                    (char_class as usize == points.len() && point == alphabet_size)
                        || points[char_class as usize] == point
                );
            }

            // process transitions that end on this point
            // (closes an overlapping interval)
            let limit = self.transition_set.points[i].ends.next;
            let mut j = 0usize;
            while j < limit {
                let dest = self.transition_set.points[i].ends.transitions[j];
                self.states_set.decr(dest);
                j += 3;
            }
            self.transition_set.points[i].ends.next = 0;

            // process transitions that start on this point
            // (opens a new interval)
            let limit = self.transition_set.points[i].starts.next;
            let mut j = 0usize;
            while j < limit {
                let dest = self.transition_set.points[i].starts.transitions[j];
                self.states_set.incr(dest);
                j += 3;
            }

            last_point = point;
            self.transition_set.points[i].starts.next = 0;
        }
        debug_assert!(self.states_set.size() == 0);
        // no more outgoing transitions, set the rest of the transitions to MISSING
        for cls in char_class as usize..points.len() {
            self.force_transition(ord, cls as i32, MISSING);
        }
        self.d_states[ord as usize].computed_transitions = points.len();
    }

    /// Writes a transition slot unconditionally, mirroring Lucene's
    /// `Arrays.fill(transitions, from, to, MISSING)`, which bypasses
    /// `assignTransition` and therefore leaves the counters untouched.
    fn force_transition(&mut self, ord: i32, char_class: i32, dest: i32) {
        let d = &mut self.d_states[ord as usize];
        let transitions = d
            .transitions
            .as_mut()
            .expect("INVARIANT: init_transitions runs first");
        transitions[char_class as usize] = dest;
    }
}

impl ByteRunnable for NFARunAutomaton {
    /// For a given state and an incoming codepoint, returns the next state, or
    /// `-1` if the transition does not exist.
    fn step(&self, state: i32, c: i32) -> i32 {
        let char_class = self.get_char_class(c);
        let mut inner = self.inner.borrow_mut();
        inner.next_state(
            state,
            char_class,
            &self.automaton,
            &self.points,
            self.alphabet_size,
        )
    }

    fn is_accept(&self, state: i32) -> bool {
        self.inner.borrow().d_states[state as usize].is_accept
    }

    fn size(&self) -> i32 {
        self.inner.borrow().d_states.len() as i32
    }
}

impl TransitionAccessor for NFARunAutomaton {
    fn init_transition(&self, state: i32, t: &mut Transition) -> i32 {
        t.source = state;
        t.transition_upto = -1;
        self.get_num_transitions(state)
    }

    fn get_next_transition(&self, t: &mut Transition) {
        debug_assert!(t.transition_upto < self.points.len() as i32 - 1 && t.transition_upto >= -1);
        let inner = self.inner.borrow();
        loop {
            t.transition_upto += 1;
            let value = inner.transition_at(t.source, t.transition_upto);
            if value != MISSING {
                debug_assert!(value != NOT_COMPUTED);
                break;
            }
        }

        self.set_transition_accordingly(t, &inner);
    }

    fn get_num_transitions(&self, state: i32) -> i32 {
        let mut inner = self.inner.borrow_mut();
        inner.determinize_state(state, &self.automaton, &self.points, self.alphabet_size);
        inner.d_states[state as usize].outgoing_transitions
    }

    fn get_transition(&self, state: i32, index: i32, t: &mut Transition) {
        {
            let mut inner = self.inner.borrow_mut();
            inner.determinize_state(state, &self.automaton, &self.points, self.alphabet_size);
        }
        let inner = self.inner.borrow();
        let mut outgoing_transitions = -1i32;
        t.transition_upto = -1;
        t.source = state;
        while outgoing_transitions < index && t.transition_upto < self.points.len() as i32 - 1 {
            t.transition_upto += 1;
            if inner.transition_at(t.source, t.transition_upto) != MISSING {
                outgoing_transitions += 1;
            }
        }
        debug_assert!(outgoing_transitions == index);

        self.set_transition_accordingly(t, &inner);
    }
}
