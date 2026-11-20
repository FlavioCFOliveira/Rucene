//! Port of `org.apache.lucene.util.automaton.StringsToAutomaton`.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::error::{LuceneError, Result};
use crate::util::{BytesRef, UnicodeUtil};

use super::automata::MAX_STRING_UNION_TERM_LENGTH;
use super::automaton::{Automaton, AutomatonBuilder};

/// DFSA state with `char` labels on transitions.
///
/// Equivalent to the private `StringsToAutomaton.State`. Lucene interns states by
/// object identity; this port keeps every state in an arena and interns them by
/// arena index, which is the same relation because states are interned in
/// post-order, so a child index is unique per right-language by the time its parent
/// is registered.
#[derive(Clone, Debug, Default)]
struct State {
    /// Labels of outgoing transitions, indexed identically to `states` and sorted
    /// lexicographically.
    labels: Vec<i32>,
    /// States reachable from outgoing transitions, indexed identically to `labels`.
    states: Vec<usize>,
    /// True if this state corresponds to the end of at least one input sequence.
    is_final: bool,
}

impl State {
    /// Equivalent to `State.equals(Object)`: two states are equal if they have
    /// the same finality, the same labels, and the same (already interned)
    /// target states, compared by reference
    /// (`StringsToAutomaton.java:88-94`). Reference identity is arena-index
    /// identity here (see the type's doc).
    fn equals(&self, other: &State) -> bool {
        self.is_final == other.is_final
            && self.labels == other.labels
            && Self::reference_equals(&self.states, &other.states)
    }

    /// Equivalent to `State.hashCode()`: the hash code of the *current* status
    /// of this state (`StringsToAutomaton.java:96-115`). Java mixes in
    /// `System.identityHashCode` of every target state; the target's arena
    /// index stands in for that identity hash (see the type's doc).
    fn hash_code(&self) -> i32 {
        let mut hash: i32 = i32::from(self.is_final);

        hash ^= hash.wrapping_mul(31).wrapping_add(self.labels.len() as i32);
        for &c in &self.labels {
            hash ^= hash.wrapping_mul(31).wrapping_add(c);
        }

        // Compare the right-language of this state using reference-identity of
        // outgoing states. This is possible because states are interned (stored
        // in registry) and traversed in post-order, so any outgoing transitions
        // are already interned.
        for &s in &self.states {
            hash ^= s as i32;
        }

        hash
    }

    /// Equivalent to the private static
    /// `State.referenceEquals(Object[], Object[])`: compares two lists of
    /// states for reference equality (`StringsToAutomaton.java:162-175`).
    fn reference_equals(a1: &[usize], a2: &[usize]) -> bool {
        if a1.len() != a2.len() {
            return false;
        }

        for (s1, s2) in a1.iter().zip(a2) {
            if s1 != s2 {
                return false;
            }
        }

        true
    }
}

impl PartialEq for State {
    /// [`State::equals`], so the registry interns by Java's `equals`.
    fn eq(&self, other: &Self) -> bool {
        self.equals(other)
    }
}

impl Eq for State {}

impl Hash for State {
    /// [`State::hash_code`], so the registry hashes by Java's `hashCode`.
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.hash_code());
    }
}

/// Builds a minimal, deterministic [`Automaton`] that accepts a set of strings.
///
/// Equivalent to `org.apache.lucene.util.automaton.StringsToAutomaton`. Implements
/// the algorithm described in *Incremental Construction of Minimal Acyclic
/// Finite-State Automata* by Daciuk, Mihov, Watson and Watson. This requires sorted
/// input data, but is very fast (nearly linear with the input size). It also offers
/// the ability to directly build a binary [`Automaton`] representation. Users should
/// access this functionality through [`Automata`](super::automata::Automata).
pub struct StringsToAutomaton {
    /// Arena holding every state; index 0 is the root.
    arena: Vec<State>,
    /// A "registry" for state interning.
    state_registry: Option<HashMap<State, usize>>,
    /// Used for input order checking.
    previous: Option<BytesRef>,
}

impl Default for StringsToAutomaton {
    fn default() -> Self {
        Self::new()
    }
}

impl StringsToAutomaton {
    /// Creates an empty builder whose root state has no transitions.
    fn new() -> Self {
        Self {
            arena: vec![State::default()],
            state_registry: Some(HashMap::new()),
            previous: None,
        }
    }

    /// Builds a minimal, deterministic automaton from a sorted sequence of
    /// [`BytesRef`] representing strings in UTF-8.
    ///
    /// These strings must be binary-sorted. Creates an [`Automaton`] with either
    /// UTF-8 codepoints as transition labels or binary (compiled) transition labels,
    /// based on `as_binary`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) if a term is longer than
    /// [`MAX_STRING_UNION_TERM_LENGTH`] or if the input is not sorted, and a decode
    /// error if a term is not valid UTF-8 when `as_binary` is false.
    pub fn build<I>(input: I, as_binary: bool) -> Result<Automaton>
    where
        I: IntoIterator<Item = BytesRef>,
    {
        let mut builder = Self::new();

        for b in input {
            builder.add(&b, as_binary)?;
        }

        Ok(builder.complete_and_convert())
    }

    /// Returns the target state of a transition leaving `state` labeled with
    /// `label`, or `None` if no such transition exists.
    fn get_state(&self, state: usize, label: i32) -> Option<usize> {
        let s = &self.arena[state];
        s.labels
            .binary_search(&label)
            .ok()
            .map(|index| s.states[index])
    }

    /// Returns true if this state has any children (outgoing transitions).
    fn has_children(&self, state: usize) -> bool {
        !self.arena[state].labels.is_empty()
    }

    /// Creates a new outgoing transition labeled `label` and returns the newly
    /// created target state for this transition.
    fn new_state(&mut self, state: usize, label: i32) -> usize {
        debug_assert!(
            self.arena[state].labels.binary_search(&label).is_err(),
            "State already has transition labeled: {}",
            label
        );

        let created = self.arena.len();
        self.arena.push(State::default());
        let s = &mut self.arena[state];
        s.labels.push(label);
        s.states.push(created);
        created
    }

    /// Returns the most recent transition's target state.
    fn last_child(&self, state: usize) -> usize {
        debug_assert!(self.has_children(state), "No outgoing transitions.");
        *self.arena[state]
            .states
            .last()
            .expect("INVARIANT: checked has_children above")
    }

    /// Returns the associated state if the most recent transition is labeled with
    /// `label`.
    fn last_child_labeled(&self, state: usize, label: i32) -> Option<usize> {
        let s = &self.arena[state];
        let result = match s.labels.last() {
            Some(&last) if last == label => s.states.last().copied(),
            _ => None,
        };
        debug_assert_eq!(result, self.get_state(state, label));
        result
    }

    /// Replaces the last added outgoing transition's target state with `target`.
    fn replace_last_child(&mut self, state: usize, target: usize) {
        debug_assert!(self.has_children(state), "No outgoing transitions.");
        let s = &mut self.arena[state];
        let idx = s.states.len() - 1;
        s.states[idx] = target;
    }

    /// Internal recursive traversal for conversion.
    fn convert(
        &self,
        a: &mut AutomatonBuilder,
        s: usize,
        visited: &mut HashMap<usize, i32>,
    ) -> i32 {
        if let Some(&converted) = visited.get(&s) {
            return converted;
        }

        let converted = a.create_state();
        a.set_accept(converted, self.arena[s].is_final);

        visited.insert(s, converted);
        for i in 0..self.arena[s].labels.len() {
            let target = self.arena[s].states[i];
            let dest = self.convert(a, target, visited);
            a.add_transition(converted, dest, self.arena[s].labels[i]);
        }

        converted
    }

    /// Called after adding all terms; performs final minimization and converts to a
    /// standard [`Automaton`].
    fn complete_and_convert(mut self) -> Automaton {
        // Final minimization:
        if self.has_children(0) {
            self.replace_or_register(0);
        }
        self.state_registry = None;

        // Convert:
        let mut a = AutomatonBuilder::new();
        let mut visited = HashMap::new();
        self.convert(&mut a, 0, &mut visited);
        a.finish()
    }

    /// Copies `current` into an internal buffer.
    ///
    /// Returns `true`, as Java does so the call can sit inside an assertion.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.util.automaton.StringsToAutomaton#setPrevious`.
    fn set_previous(&mut self, current: &BytesRef) -> bool {
        self.previous = Some(BytesRef::deep_copy_of(current));
        true
    }

    fn add(&mut self, current: &BytesRef, as_binary: bool) -> Result<()> {
        if current.length > MAX_STRING_UNION_TERM_LENGTH {
            return Err(LuceneError::illegal_argument(format!(
                "This builder doesn't allow terms that are larger than {} UTF-8 bytes, got {}",
                MAX_STRING_UNION_TERM_LENGTH,
                current.to_hex_string()
            )));
        }
        debug_assert!(self.state_registry.is_some(), "Automaton already built.");
        if let Some(previous) = &self.previous {
            if previous.cmp(current) == std::cmp::Ordering::Greater {
                return Err(LuceneError::illegal_argument(format!(
                    "Input must be in sorted UTF-8 order: {} >= {}",
                    previous.to_hex_string(),
                    current.to_hex_string()
                )));
            }
        }
        self.set_previous(current);

        // Descend in the automaton (find the matching prefix).
        let bytes = &current.bytes;
        let mut pos = current.offset;
        let max = current.offset + current.length;
        let mut state = 0usize;
        if as_binary {
            while pos < max {
                match self.last_child_labeled(state, i32::from(bytes[pos])) {
                    Some(next) => {
                        state = next;
                        pos += 1;
                    }
                    None => break,
                }
            }
        } else {
            while pos < max {
                let code_point = UnicodeUtil::code_point_at(bytes, pos)?;
                match self.last_child_labeled(state, code_point.code_point as i32) {
                    Some(next) => {
                        state = next;
                        pos += code_point.num_bytes;
                    }
                    None => break,
                }
            }
        }

        if self.has_children(state) {
            self.replace_or_register(state);
        }

        // Add suffix
        if as_binary {
            while pos < max {
                state = self.new_state(state, i32::from(bytes[pos]));
                pos += 1;
            }
        } else {
            while pos < max {
                let code_point = UnicodeUtil::code_point_at(&current.bytes, pos)?;
                state = self.new_state(state, code_point.code_point as i32);
                pos += code_point.num_bytes;
            }
        }
        self.arena[state].is_final = true;
        Ok(())
    }

    /// Replaces the last child of `state` with an already registered state, or
    /// registers the last child state.
    fn replace_or_register(&mut self, state: usize) {
        let child = self.last_child(state);

        if self.has_children(child) {
            self.replace_or_register(child);
        }

        let key = self.arena[child].clone();
        let registered = self
            .state_registry
            .as_ref()
            .expect("INVARIANT: the registry is only dropped once every term was added")
            .get(&key)
            .copied();
        match registered {
            Some(registered) => self.replace_last_child(state, registered),
            None => {
                self.state_registry
                    .as_mut()
                    .expect("INVARIANT: checked just above")
                    .insert(key, child);
            }
        }
    }
}

#[cfg(test)]
mod state_member_tests {
    use super::*;
    use std::hash::{BuildHasher, RandomState};

    fn state(is_final: bool, labels: &[i32], states: &[usize]) -> State {
        State {
            labels: labels.to_vec(),
            states: states.to_vec(),
            is_final,
        }
    }

    /// #594: `State.equals(Object)` compares finality, labels, and target
    /// states by reference (here, arena index); `referenceEquals` compares
    /// two target lists element by element.
    #[test]
    fn state_equals_and_reference_equals_match_java() {
        let a = state(true, &[97, 98], &[3, 5]);
        assert!(a.equals(&state(true, &[97, 98], &[3, 5])));
        assert!(!a.equals(&state(false, &[97, 98], &[3, 5])));
        assert!(!a.equals(&state(true, &[97, 99], &[3, 5])));
        assert!(!a.equals(&state(true, &[97, 98], &[3, 6])));
        assert!(State::reference_equals(&[1, 2], &[1, 2]));
        assert!(!State::reference_equals(&[1, 2], &[1]));
        assert!(!State::reference_equals(&[1, 2], &[2, 1]));
        assert!(State::reference_equals(&[], &[]));
        assert_eq!(a, state(true, &[97, 98], &[3, 5]), "PartialEq is equals");
    }

    /// #594: `State.hashCode()` (`StringsToAutomaton.java:96-115`), with the
    /// arena index standing in for `System.identityHashCode`; the expected
    /// value is Java's formula over `isFinal = true`, labels `{97, 98}` and
    /// targets 3 and 5.
    #[test]
    fn state_hash_code_matches_java_formula() {
        let a = state(true, &[97, 98], &[3, 5]);
        assert_eq!(a.hash_code(), EXPECTED_HASH);
        assert_eq!(state(false, &[], &[]).hash_code(), 0);
        let hasher = RandomState::new();
        assert_eq!(
            hasher.hash_one(&a),
            hasher.hash_one(state(true, &[97, 98], &[3, 5]))
        );
    }

    const EXPECTED_HASH: i32 = 35_910;

    /// #594: the registry interns states by `equals`/`hashCode`, so states
    /// with the same right language are shared: `{"ab", "cb"}` builds the
    /// minimal three-state automaton.
    #[test]
    fn registry_interns_states_with_the_same_right_language() {
        let automaton = StringsToAutomaton::build(
            [BytesRef::new(b"ab".to_vec()), BytesRef::new(b"cb".to_vec())],
            false,
        )
        .unwrap();
        assert_eq!(automaton.get_num_states(), 3);
    }
}
