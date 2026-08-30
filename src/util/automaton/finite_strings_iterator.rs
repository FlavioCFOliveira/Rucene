//! Port of `org.apache.lucene.util.automaton.FiniteStringsIterator`.

use crate::error::{LuceneError, Result};
use crate::util::IntsRef;
use crate::util::IntsRefBuilder;

use super::automaton::{Automaton, Transition, TransitionAccessor};

/// Nodes for the path stack.
///
/// Equivalent to the private `FiniteStringsIterator.PathNode`.
#[derive(Clone, Debug, Default)]
struct PathNode {
    /// Which state the path node ends on, whose transitions we are enumerating.
    state: i32,
    /// Which state the current transition leads to.
    to: i32,
    /// Which transition we are on.
    transition: i32,
    /// Which label we are on, in the min-max range of the current transition.
    label: i32,
    t: Transition,
}

impl PathNode {
    fn reset_state(&mut self, a: &Automaton, state: i32) {
        debug_assert!(a.get_num_transitions(state) != 0);
        self.state = state;
        self.transition = 0;
        a.get_transition(state, 0, &mut self.t);
        self.label = self.t.min;
        self.to = self.t.dest;
    }

    /// Returns the next label of the current transition, or advances to the next
    /// transition and returns its first label if the current one is exhausted.
    ///
    /// Returns `-1` if there are no more transitions.
    fn next_label(&mut self, a: &Automaton) -> i32 {
        if self.label > self.t.max {
            // We've exhausted the current transition's labels; move to the next
            // transition:
            self.transition += 1;
            if self.transition >= a.get_num_transitions(self.state) {
                // We're done iterating transitions leaving this state
                self.label = -1;
                return -1;
            }
            a.get_transition(self.state, self.transition, &mut self.t);
            self.label = self.t.min;
            self.to = self.t.dest;
        }
        let label = self.label;
        self.label += 1;
        label
    }
}

/// Iterates all accepted strings.
///
/// Equivalent to `org.apache.lucene.util.automaton.FiniteStringsIterator`.
///
/// If the [`Automaton`] has cycles then this iterator may return an error, but this
/// is not guaranteed. Be aware that the iteration order is implementation dependent
/// and may change across releases. If the automaton is not determinized then it is
/// possible this iterator will return duplicates.
pub struct FiniteStringsIterator<'a> {
    /// Automaton to create the finite strings from.
    a: &'a Automaton,
    /// The state where each path should stop, or `-1` if only accepted states should
    /// be final.
    end_state: i32,
    /// Tracks which states are in the current path, for cycle detection.
    path_states: Vec<bool>,
    /// Builder for the current finite string.
    string: IntsRefBuilder,
    /// Stack to hold our current state in the recursion/iteration.
    nodes: Vec<PathNode>,
    /// Emit the empty string?
    emit_empty_string: bool,
}

impl<'a> FiniteStringsIterator<'a> {
    /// Creates an iterator over all the finite strings of `a`.
    pub fn new(a: &'a Automaton) -> Self {
        Self::with_bounds(a, 0, -1)
    }

    /// Creates an iterator over all the finite strings of `a`, starting each path at
    /// `start_state` and stopping it at `end_state` (or `-1` if only accepted states
    /// should be final).
    pub fn with_bounds(a: &'a Automaton, start_state: i32, end_state: i32) -> Self {
        let mut nodes = vec![PathNode::default(); 16];
        let mut string = IntsRefBuilder::new();
        let path_states = vec![false; a.get_num_states() as usize];
        string.set_length(0);
        let emit_empty_string = a.is_accept(0);

        let mut this = Self {
            a,
            end_state,
            path_states,
            string,
            nodes: Vec::new(),
            emit_empty_string,
        };

        // Start iteration with node startState.
        if a.get_num_states() > start_state && a.get_num_transitions(start_state) > 0 {
            this.path_states[start_state as usize] = true;
            nodes[0].reset_state(a, start_state);
            this.string.append(start_state);
        }
        this.nodes = nodes;
        this
    }

    /// Generates the next finite string.
    ///
    /// The returned value is only valid until the next call of this method. Returns
    /// `None` when no more finite strings are available.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if a cycle is detected.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<IntsRef>> {
        // Special case the empty string, as usual:
        if self.emit_empty_string {
            self.emit_empty_string = false;
            return Ok(Some(IntsRef::new(Vec::new())));
        }

        let mut depth = self.string.length();
        while depth > 0 {
            // Get next label leaving the current node:
            let label = self.nodes[depth - 1].next_label(self.a);
            if label != -1 {
                self.string.set_int_at(depth - 1, label);

                let to = self.nodes[depth - 1].to;
                if self.a.get_num_transitions(to) != 0 && to != self.end_state {
                    // Now recurse: the destination of this transition has outgoing
                    // transitions:
                    if self.path_states[to as usize] {
                        return Err(LuceneError::IllegalArgument(
                            "automaton has cycles".to_string(),
                        ));
                    }
                    self.path_states[to as usize] = true;

                    // Push node onto stack:
                    self.grow_stack(depth);
                    self.nodes[depth].reset_state(self.a, to);
                    depth += 1;
                    self.string.set_length(depth);
                    self.string.grow(depth);
                } else if self.end_state == to || self.a.is_accept(to) {
                    // This transition leads to an accept state, so we save the
                    // current string:
                    return Ok(Some(self.string.get()));
                }
            } else {
                // No more transitions leaving this state, pop/return back to the
                // previous state:
                let state = self.nodes[depth - 1].state;
                debug_assert!(self.path_states[state as usize]);
                self.path_states[state as usize] = false;
                depth -= 1;
                self.string.set_length(depth);

                if self.a.is_accept(state) {
                    // This transition leads to an accept state, so we save the
                    // current string:
                    return Ok(Some(self.string.get()));
                }
            }
        }

        // Finished iteration.
        Ok(None)
    }

    /// Grows the path stack, if required.
    fn grow_stack(&mut self, depth: usize) {
        if self.nodes.len() == depth {
            self.nodes
                .resize(depth + 1 + (depth >> 3), PathNode::default());
        }
    }
}
