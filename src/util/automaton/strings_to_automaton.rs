//! Port of `org.apache.lucene.util.automaton.StringsToAutomaton`.

use std::collections::HashMap;

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

/// Key used to intern states in the registry: two states are equal if they have the
/// same finality, the same labels, and the same (already interned) target states.
type StateKey = (bool, Vec<i32>, Vec<usize>);

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
    state_registry: Option<HashMap<StateKey, usize>>,
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
    /// Returns [`LuceneError::IllegalArgument`] if a term is longer than
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

    fn key_of(&self, state: usize) -> StateKey {
        let s = &self.arena[state];
        (s.is_final, s.labels.clone(), s.states.clone())
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

    fn add(&mut self, current: &BytesRef, as_binary: bool) -> Result<()> {
        if current.length > MAX_STRING_UNION_TERM_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "This builder doesn't allow terms that are larger than {} UTF-8 bytes, got {}",
                MAX_STRING_UNION_TERM_LENGTH,
                current.to_hex_string()
            )));
        }
        debug_assert!(self.state_registry.is_some(), "Automaton already built.");
        if let Some(previous) = &self.previous {
            if previous.cmp(current) == std::cmp::Ordering::Greater {
                return Err(LuceneError::IllegalArgument(format!(
                    "Input must be in sorted UTF-8 order: {} >= {}",
                    previous.to_hex_string(),
                    current.to_hex_string()
                )));
            }
        }
        self.previous = Some(BytesRef::deep_copy_of(current));

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

        let key = self.key_of(child);
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
