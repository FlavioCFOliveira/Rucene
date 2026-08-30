//! Port of `org.apache.lucene.util.automaton.CompiledAutomaton`.

use crate::codecs::postings::{empty_terms_enum, SingleTermsEnum, Terms, TermsEnum};
use crate::error::Result;
use crate::util::{BytesRef, BytesRefBuilder, IntsRef, StringHelper, UnicodeUtil};

use super::automata::Automata;
use super::automaton::{Automaton, Transition, TransitionAccessor};
use super::byte_run_automaton::ByteRunAutomaton;
use super::nfa_run_automaton::NFARunAutomaton;
use super::operations::Operations;
use super::run_automaton::ByteRunnable;
use super::utf32_to_utf8::UTF32ToUTF8;

/// Automata are compiled into different internal forms for the most efficient
/// execution depending upon the language they accept.
///
/// Equivalent to `CompiledAutomaton.AUTOMATON_TYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomatonType {
    /// Automaton that accepts no strings.
    None,
    /// Automaton that accepts all possible strings.
    All,
    /// Automaton that accepts only a single fixed string.
    Single,
    /// Catch-all for any other automata.
    Normal,
}

/// Immutable class holding compiled details for a given [`Automaton`].
///
/// Equivalent to `org.apache.lucene.util.automaton.CompiledAutomaton`. The automaton
/// can be either deterministic or non-deterministic. A deterministic automaton must
/// not have dead states but is not necessarily minimal, and is executed using
/// [`ByteRunAutomaton`]; a non-deterministic one is executed using
/// [`NFARunAutomaton`].
#[derive(Clone, Debug)]
pub struct CompiledAutomaton {
    /// If `simplify` was true this is the "simplified" type; else this is
    /// [`AutomatonType::Normal`].
    pub automaton_type: AutomatonType,

    /// For [`AutomatonType::Single`] this is the singleton term.
    pub term: Option<BytesRef>,

    /// Matcher for quickly determining if a byte slice is accepted; only valid for
    /// [`AutomatonType::Normal`].
    pub run_automaton: Option<ByteRunAutomaton>,

    /// Transitions indexed by state number for traversal. The state numbering is
    /// consistent with [`CompiledAutomaton::run_automaton`]. Only valid for
    /// [`AutomatonType::Normal`].
    pub automaton: Option<Automaton>,

    /// Matcher run directly on an NFA; it determinizes the state on demand and
    /// caches it. Note that this field and [`CompiledAutomaton::run_automaton`] are
    /// never both non-`None`.
    pub nfa_run_automaton: Option<NFARunAutomaton>,

    /// Shared common suffix accepted by the automaton. Only valid for
    /// [`AutomatonType::Normal`], and only when the automaton accepts an infinite
    /// language. This is `None` if the common suffix has length 0.
    pub common_suffix_ref: Option<BytesRef>,

    /// Indicates if the automaton accepts a finite set of strings. Only valid for
    /// [`AutomatonType::Normal`].
    pub finite: bool,

    /// Which state, if any, accepts all suffixes, else `-1`.
    pub sink_state: i32,
}

impl CompiledAutomaton {
    /// Creates this, passing `simplify = true` so that we try to simplify the
    /// automaton.
    ///
    /// # Errors
    ///
    /// Returns an error if the automaton could not be compiled.
    pub fn from_automaton(automaton: Automaton) -> Result<Self> {
        Self::new(automaton, false, true, false)
    }

    /// Returns the sink state, if present, else `-1`.
    fn find_sink_state(automaton: &Automaton) -> i32 {
        let num_states = automaton.get_num_states();
        let mut t = Transition::new();
        let mut found_state = -1i32;
        for s in 0..num_states {
            if automaton.is_accept(s) {
                let count = automaton.init_transition(s, &mut t);
                let mut is_sink_state = false;
                for _ in 0..count {
                    automaton.get_next_transition(&mut t);
                    if t.dest == s && t.min == 0 && t.max == 0xff {
                        is_sink_state = true;
                        break;
                    }
                }
                if is_sink_state {
                    found_state = s;
                    break;
                }
            }
        }

        found_state
    }

    /// Compiles an automaton.
    ///
    /// If `simplify` is true, possibly expensive operations are run to determine
    /// whether the automaton is one of the cases in [`AutomatonType`]. Set `finite`
    /// to true if the automaton is finite, otherwise set it to false if it is
    /// infinite or you do not know. Set `is_binary` when the caller already built a
    /// byte-based automaton.
    ///
    /// # Errors
    ///
    /// Returns an error if determinizing the byte automaton exceeds the work limit,
    /// or if the singleton term cannot be decoded.
    pub fn new(
        mut automaton: Automaton,
        finite: bool,
        simplify: bool,
        is_binary: bool,
    ) -> Result<Self> {
        if automaton.get_num_states() == 0 {
            automaton = Automaton::new();
            automaton.create_state();
        }

        // simplify requires a DFA
        if simplify && automaton.is_deterministic() {
            // Test whether the automaton is a "simple" form and if so, don't create a
            // runAutomaton. Note that on a large automaton these tests could be
            // costly:

            if Operations::is_empty(&automaton) {
                // matches nothing
                return Ok(Self {
                    automaton_type: AutomatonType::None,
                    term: None,
                    common_suffix_ref: None,
                    run_automaton: None,
                    automaton: None,
                    finite: true,
                    sink_state: -1,
                    nfa_run_automaton: None,
                });
            }

            // NOTE: only approximate, because the automaton may not be minimal:
            let is_total = if is_binary {
                Operations::is_total_range(&automaton, 0, 0xff)
            } else {
                Operations::is_total(&automaton)
            };

            if is_total {
                // matches all possible strings
                return Ok(Self {
                    automaton_type: AutomatonType::All,
                    term: None,
                    common_suffix_ref: None,
                    run_automaton: None,
                    automaton: None,
                    finite: false,
                    sink_state: -1,
                    nfa_run_automaton: None,
                });
            }

            let singleton = Operations::get_singleton(&automaton)?;

            if let Some(singleton) = singleton {
                // matches a fixed string
                let term = if is_binary {
                    StringHelper::ints_ref_to_bytes_ref(&singleton)?
                } else {
                    Self::ints_ref_to_utf8_bytes_ref(&singleton)?
                };
                return Ok(Self {
                    automaton_type: AutomatonType::Single,
                    term: Some(term),
                    common_suffix_ref: None,
                    run_automaton: None,
                    automaton: None,
                    finite: true,
                    sink_state: -1,
                    nfa_run_automaton: None,
                });
            }
        }

        let mut binary = if is_binary {
            // Caller already built the binary automaton themselves, e.g. PrefixQuery
            // does this since it can be provided with a binary (not necessarily
            // UTF-8!) term:
            automaton.clone()
        } else {
            // Incoming automaton is unicode, and we must convert to UTF-8 to match
            // what's in the index:
            UTF32ToUTF8::new().convert(&automaton)
        };

        // compute a common suffix for infinite DFAs, this is an optimization for
        // "leading wildcard" so don't burn cycles on it if the DFA is finite, or
        // largeish
        let common_suffix_ref = if finite
            || automaton.get_num_states() + automaton.get_num_transitions_total() > 1000
        {
            None
        } else {
            let suffix = Operations::get_common_suffix_bytes_ref(&binary)?;
            if suffix.length == 0 {
                None
            } else {
                Some(suffix)
            }
        };

        if !automaton.is_deterministic() && !binary.is_deterministic() {
            Ok(Self {
                automaton_type: AutomatonType::Normal,
                term: None,
                common_suffix_ref,
                run_automaton: None,
                automaton: None,
                finite,
                sink_state: -1,
                nfa_run_automaton: Some(NFARunAutomaton::new(binary, 0xff)),
            })
        } else {
            // We already had a DFA (or errored out); according to Mike, UTF32toUTF8
            // won't "blow up".
            binary = Operations::determinize(&binary, i32::MAX)?;
            let run_automaton = ByteRunAutomaton::new(binary, true)?;

            let automaton = run_automaton.automaton().clone();

            // TODO (Lucene): this is a bit fragile because if the automaton is not
            // minimized there could be more than one sink state, but auto-prefix will
            // fail to run for those.
            let sink_state = Self::find_sink_state(&automaton);

            Ok(Self {
                automaton_type: AutomatonType::Normal,
                term: None,
                common_suffix_ref,
                run_automaton: Some(run_automaton),
                automaton: Some(automaton),
                finite,
                sink_state,
                nfa_run_automaton: None,
            })
        }
    }

    fn ints_ref_to_utf8_bytes_ref(ints: &IntsRef) -> Result<BytesRef> {
        let s = UnicodeUtil::new_string(&ints.ints, ints.offset, ints.length)?;
        Ok(BytesRef::new(s.into_bytes()))
    }

    fn add_tail(
        &self,
        mut state: i32,
        term: &mut BytesRefBuilder,
        mut idx: usize,
        lead_label: i32,
    ) -> BytesRef {
        let automaton = self
            .automaton
            .as_ref()
            .expect("INVARIANT: floor is only valid for AutomatonType::Normal");
        let run_automaton = self
            .run_automaton
            .as_ref()
            .expect("INVARIANT: floor is only valid for AutomatonType::Normal");
        let mut transition = Transition::new();

        // Find the biggest transition that's < label
        let mut max_index = -1i32;
        let num_transitions = automaton.init_transition(state, &mut transition);
        for i in 0..num_transitions {
            automaton.get_next_transition(&mut transition);
            if transition.min < lead_label {
                max_index = i;
            } else {
                // Transitions are always sorted
                break;
            }
        }

        debug_assert!(max_index != -1);
        automaton.get_transition(state, max_index, &mut transition);

        // Append floorLabel
        let floor_label = if transition.max > lead_label - 1 {
            lead_label - 1
        } else {
            transition.max
        };
        term.grow(1 + idx);
        term.set_byte_at(idx, floor_label as u8);

        state = transition.dest;
        idx += 1;

        // Push down to the last accept state
        loop {
            let num_transitions = automaton.get_num_transitions(state);
            if num_transitions == 0 {
                debug_assert!(run_automaton.is_accept(state));
                term.set_length(idx);
                return term.get();
            }
            // We are pushing "top" -- so get the last label of the last transition:
            automaton.get_transition(state, num_transitions - 1, &mut transition);
            term.grow(1 + idx);
            term.set_byte_at(idx, transition.max as u8);
            state = transition.dest;
            idx += 1;
        }
    }

    /// Returns a [`TermsEnum`] intersecting the provided [`Terms`] with the terms
    /// accepted by this automaton.
    ///
    /// # Errors
    ///
    /// Returns an error if the terms enum could not be created.
    pub fn get_terms_enum(&self, terms: &dyn Terms) -> Result<Box<dyn TermsEnum>> {
        match self.automaton_type {
            AutomatonType::None => Ok(empty_terms_enum()),
            AutomatonType::All => terms.iterator(),
            AutomatonType::Single => {
                let term = self
                    .term
                    .as_ref()
                    .expect("INVARIANT: SINGLE always carries a term")
                    .clone();
                Ok(Box::new(SingleTermsEnum::new(terms.iterator()?, term)))
            }
            AutomatonType::Normal => terms.intersect(self, None),
        }
    }

    /// Finds the largest term accepted by this automaton that is less than or equal
    /// to the provided input term.
    ///
    /// The result is placed in `output`; it is fine for `output` and `input` to hold
    /// the same bytes. Returns `None` if there is no floor term, i.e. the provided
    /// input term is before the first term accepted by this automaton.
    pub fn floor(&self, input: &BytesRef, output: &mut BytesRefBuilder) -> Option<BytesRef> {
        let automaton = self.automaton.as_ref()?;
        let run_automaton = self.run_automaton.as_ref()?;
        let mut transition = Transition::new();

        let mut state = 0i32;

        // Special case the empty string:
        if input.length == 0 {
            return if run_automaton.is_accept(state) {
                output.clear();
                Some(output.get())
            } else {
                None
            };
        }

        let mut stack: Vec<i32> = Vec::new();

        let mut idx = 0usize;
        loop {
            let mut label = i32::from(input.bytes[input.offset + idx]);
            let mut next_state = run_automaton.step(state, label);

            if idx == input.length - 1 {
                if next_state != -1 && run_automaton.is_accept(next_state) {
                    // Input string is accepted
                    output.grow(1 + idx);
                    output.set_byte_at(idx, label as u8);
                    output.set_length(input.length);
                    return Some(output.get());
                } else {
                    next_state = -1;
                }
            }

            if next_state == -1 {
                // Pop back to a state that has a transition <= our label:
                loop {
                    let num_transitions = automaton.get_num_transitions(state);
                    if num_transitions == 0 {
                        debug_assert!(run_automaton.is_accept(state));
                        output.set_length(idx);
                        return Some(output.get());
                    }
                    automaton.get_transition(state, 0, &mut transition);

                    if label - 1 < transition.min {
                        if run_automaton.is_accept(state) {
                            output.set_length(idx);
                            return Some(output.get());
                        }
                        // pop
                        match stack.pop() {
                            None => return None,
                            Some(popped) => {
                                state = popped;
                                idx -= 1;
                                label = i32::from(input.bytes[input.offset + idx]);
                            }
                        }
                    } else {
                        break;
                    }
                }

                return Some(self.add_tail(state, output, idx, label));
            } else {
                output.grow(1 + idx);
                output.set_byte_at(idx, label as u8);
                stack.push(state);
                state = next_state;
                idx += 1;
            }
        }
    }

    /// Gets a [`ByteRunnable`] instance; it will be different depending on whether an
    /// NFA or a DFA was passed in, and it is not guaranteed to be non-`None`.
    pub fn get_byte_runnable(&self) -> Option<&dyn ByteRunnable> {
        // they can be both None but not both Some
        debug_assert!(self.nfa_run_automaton.is_none() || self.run_automaton.is_none());
        match &self.nfa_run_automaton {
            None => self.run_automaton.as_ref().map(|r| r as &dyn ByteRunnable),
            Some(nfa) => Some(nfa as &dyn ByteRunnable),
        }
    }

    /// Gets a [`TransitionAccessor`] instance; it will be different depending on
    /// whether an NFA or a DFA was passed in, and it is not guaranteed to be
    /// non-`None`.
    pub fn get_transition_accessor(&self) -> Option<&dyn TransitionAccessor> {
        // they can be both None but not both Some
        debug_assert!(self.nfa_run_automaton.is_none() || self.automaton.is_none());
        match &self.nfa_run_automaton {
            None => self
                .automaton
                .as_ref()
                .map(|a| a as &dyn TransitionAccessor),
            Some(nfa) => Some(nfa as &dyn TransitionAccessor),
        }
    }

    /// Returns a [`ByteRunAutomaton`] describing how this automaton matches terms,
    /// for query introspection.
    ///
    /// Equivalent to the `AUTOMATON_TYPE.ALL` branch of `CompiledAutomaton.visit`.
    ///
    /// # Errors
    ///
    /// Returns an error if the "any string" automaton cannot be compiled.
    pub fn any_string_run_automaton() -> Result<ByteRunAutomaton> {
        ByteRunAutomaton::from_utf32(Automata::make_any_string())
    }
}

impl PartialEq for CompiledAutomaton {
    fn eq(&self, other: &Self) -> bool {
        if self.automaton_type != other.automaton_type {
            return false;
        }
        match self.automaton_type {
            AutomatonType::Single => self.term == other.term,
            AutomatonType::Normal => {
                self.run_automaton == other.run_automaton
                    && self.nfa_run_automaton == other.nfa_run_automaton
            }
            _ => true,
        }
    }
}

impl Eq for CompiledAutomaton {}
