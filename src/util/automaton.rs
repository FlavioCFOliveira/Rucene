//! Minimal automaton machinery used by the blocktree term dictionary.
//!
//! This module provides byte-level deterministic finite automata and the
//! compiled forms needed to intersect a term dictionary with an automaton,
//! mirroring `org.apache.lucene.util.automaton`.
//!
//! Full regular-expression compilation and general NFA determinization are
//! intentionally out of scope; only the API surface required by
//! `Lucene103BlockTreeTermsReader::intersect(CompiledAutomaton)` is provided.

#![deny(unsafe_code)]

use std::collections::{HashSet, VecDeque};

use crate::codecs::postings::{Terms, TermsEnum};
use crate::error::{LuceneError, Result};
use crate::util::{BytesRef, FixedBitSet, IntsRef, StringHelper};

/// Minimum Unicode code point handled by the general automaton alphabet.
const MIN_CODE_POINT: i32 = 0;
/// Maximum Unicode code point handled by the general automaton alphabet.
const MAX_CODE_POINT: i32 = 0x0010_ffff;

// -----------------------------------------------------------------------------
// Transition
// -----------------------------------------------------------------------------

/// Mutable holder for a single transition while iterating an automaton.
///
/// Equivalent to `org.apache.lucene.util.automaton.Transition`.
#[derive(Clone, Default, Debug)]
pub struct Transition {
    /// Source state.
    pub source: i32,
    /// Destination state.
    pub dest: i32,
    /// Minimum accepted label (inclusive).
    pub min: i32,
    /// Maximum accepted label (inclusive).
    pub max: i32,
    /// Internal cursor used by [`TransitionAccessor`] iteration.
    pub(crate) transition_upto: i32,
}

// -----------------------------------------------------------------------------
// TransitionAccessor
// -----------------------------------------------------------------------------

/// Access to the transitions leaving a single state.
///
/// Equivalent to `org.apache.lucene.util.automaton.TransitionAccessor`.
pub trait TransitionAccessor {
    /// Initialize `transition` to iterate through the transitions leaving
    /// `state`.
    ///
    /// Returns the number of transitions leaving `state`.
    fn init_transition(&self, state: i32, transition: &mut Transition) -> i32;

    /// Advance `transition` to the next transition after the current one.
    fn get_next_transition(&self, transition: &mut Transition);

    /// Returns the number of transitions leaving `state`.
    fn get_num_transitions(&self, state: i32) -> i32;

    /// Fill `transition` with the `index`th transition leaving `state`.
    fn get_transition(&self, state: i32, index: i32, transition: &mut Transition);
}

// -----------------------------------------------------------------------------
// Automaton
// -----------------------------------------------------------------------------

/// Mutable finite automaton with integer states.
///
/// Equivalent to `org.apache.lucene.util.automaton.Automaton`.
/// State 0 is always the initial state.  All transitions for a source state
/// must be added at once; finishing a state sorts and reduces its
/// transitions.
#[derive(Clone, Debug)]
pub struct Automaton {
    next_state: i32,
    cur_state: i32,
    /// Packed `(offset, count)` pairs, one pair per state.  `offset == -1`
    /// means the state has no outgoing transitions.
    states: Vec<i32>,
    /// Packed `(dest, min, max)` triples for every transition.
    transitions: Vec<i32>,
    accept: Vec<bool>,
    deterministic: bool,
}

impl Automaton {
    /// Creates an empty automaton.
    pub fn new() -> Self {
        Self {
            next_state: 0,
            cur_state: -1,
            states: Vec::new(),
            transitions: Vec::new(),
            accept: Vec::new(),
            deterministic: true,
        }
    }

    /// Creates a new state and returns its identifier.
    pub fn create_state(&mut self) -> i32 {
        let state = self.next_state;
        self.states.extend_from_slice(&[-1, 0]);
        self.accept.push(false);
        self.next_state += 1;
        state
    }

    /// Marks `state` as an accept state (or clears the mark).
    pub fn set_accept(&mut self, state: i32, accept: bool) {
        assert!(
            (state as usize) < self.accept.len(),
            "state {} out of bounds",
            state
        );
        self.accept[state as usize] = accept;
    }

    /// Returns true if `state` is an accept state.
    pub fn is_accept(&self, state: i32) -> bool {
        self.accept.get(state as usize).copied().unwrap_or(false)
    }

    /// Returns the number of states in this automaton.
    pub fn get_num_states(&self) -> i32 {
        self.next_state
    }

    /// Returns the number of transitions leaving `state`.
    pub fn get_num_transitions(&self, state: i32) -> i32 {
        let idx = 2 * (state as usize) + 1;
        self.states.get(idx).copied().unwrap_or(0)
    }

    /// Adds a transition from `source` to `dest` on a single label.
    pub fn add_transition(&mut self, source: i32, dest: i32, label: i32) {
        self.add_transition_range(source, dest, label, label);
    }

    /// Adds a transition from `source` to `dest` on all labels in `[min, max]`.
    pub fn add_transition_range(&mut self, source: i32, dest: i32, min: i32, max: i32) {
        assert!(
            (source as usize) < self.next_state as usize,
            "source state {} out of bounds",
            source
        );
        assert!(
            (dest as usize) < self.next_state as usize,
            "dest state {} out of bounds",
            dest
        );

        if self.cur_state != source {
            if self.cur_state != -1 {
                self.finish_current_state();
            }
            self.cur_state = source;
            let idx = 2 * (source as usize);
            assert!(
                self.states[idx] == -1,
                "state {} already had transitions added",
                source
            );
            self.states[idx] = self.transitions.len() as i32;
        }

        self.transitions.push(dest);
        self.transitions.push(min);
        self.transitions.push(max);

        let count_idx = 2 * (source as usize) + 1;
        self.states[count_idx] += 1;
    }

    /// Finishes the current state, sorting and reducing its transitions.
    fn finish_current_state(&mut self) {
        let state = self.cur_state;
        let idx = 2 * (state as usize);
        let offset = self.states[idx] as usize;
        let end = self.transitions.len();
        let count = (end - offset) / 3;

        if count == 0 {
            self.cur_state = -1;
            return;
        }

        let mut tr: Vec<(i32, i32, i32)> = (0..count)
            .map(|i| {
                (
                    self.transitions[offset + 3 * i],
                    self.transitions[offset + 3 * i + 1],
                    self.transitions[offset + 3 * i + 2],
                )
            })
            .collect();

        // Sort by (dest, min, max) so adjacent transitions to the same dest
        // can be merged.
        tr.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });

        // Reduce adjacent/overlapping transitions with the same destination.
        let mut reduced: Vec<(i32, i32, i32)> = Vec::with_capacity(count);
        for (dest, min, max) in tr {
            if let Some(last) = reduced.last_mut() {
                if last.0 == dest && min <= last.2 + 1 {
                    if max > last.2 {
                        last.2 = max;
                    }
                    continue;
                }
            }
            reduced.push((dest, min, max));
        }

        // Sort by (min, max, dest) to enable deterministic overlap checks
        // and fast binary search.
        reduced.sort_unstable_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.0.cmp(&b.0))
        });

        // Detect non-deterministic overlap.
        if reduced.len() > 1 && self.deterministic {
            let mut last_max = reduced[0].2;
            for r in reduced.iter().skip(1) {
                if r.1 <= last_max {
                    self.deterministic = false;
                    break;
                }
                last_max = r.2;
            }
        }

        let new_count = reduced.len();
        let removed = count - new_count;
        let old_end = self.transitions.len() as i32;

        // Replace the old transition block with the reduced block.
        self.transitions.splice(
            offset..end,
            reduced.into_iter().flat_map(|(d, m, x)| [d, m, x]),
        );

        // Update offsets for any states whose transition blocks came after ours
        // in the packed transitions array.
        for s in 0..self.next_state {
            if s == state {
                continue;
            }
            let off_idx = 2 * (s as usize);
            let off = self.states[off_idx];
            if off != -1 && off >= old_end {
                self.states[off_idx] -= 3 * removed as i32;
            }
        }

        self.states[idx + 1] = new_count as i32;
        self.cur_state = -1;
    }

    /// Finishes the current state if one is open.
    pub fn finish_state(&mut self) {
        if self.cur_state != -1 {
            self.finish_current_state();
        }
    }

    /// Finishes all open state construction.
    pub fn finish(&mut self) {
        self.finish_state();
    }

    /// Returns true if the automaton is deterministic.
    pub fn is_deterministic(&self) -> bool {
        self.deterministic
    }

    fn transition_offset(&self, state: i32) -> usize {
        self.states[2 * (state as usize)] as usize
    }
}

impl Default for Automaton {
    fn default() -> Self {
        Self::new()
    }
}

impl TransitionAccessor for Automaton {
    fn init_transition(&self, state: i32, transition: &mut Transition) -> i32 {
        assert!(
            state < self.next_state,
            "state {} out of bounds (num_states={})",
            state,
            self.next_state
        );
        let count = self.get_num_transitions(state);
        let offset = self.states[2 * (state as usize)];
        assert!(
            offset >= 0 || count == 0,
            "state {} has count={} but invalid offset={}",
            state,
            count,
            offset
        );
        transition.source = state;
        transition.transition_upto = offset;
        count
    }

    fn get_next_transition(&self, transition: &mut Transition) {
        let upto = transition.transition_upto as usize;
        transition.dest = self.transitions[upto];
        transition.min = self.transitions[upto + 1];
        transition.max = self.transitions[upto + 2];
        transition.transition_upto += 3;
    }

    fn get_num_transitions(&self, state: i32) -> i32 {
        Automaton::get_num_transitions(self, state)
    }

    fn get_transition(&self, state: i32, index: i32, transition: &mut Transition) {
        let offset = self.transition_offset(state) + 3 * (index as usize);
        transition.source = state;
        transition.dest = self.transitions[offset];
        transition.min = self.transitions[offset + 1];
        transition.max = self.transitions[offset + 2];
    }
}

impl Automaton {
    /// Returns the destination state reached from `state` on `label`, or `-1`
    /// if no transition matches.
    ///
    /// Requires a deterministic automaton with sorted, non-overlapping
    /// transitions.
    pub fn step(&self, state: i32, label: i32) -> i32 {
        let num = self.get_num_transitions(state);
        if num == 0 {
            return -1;
        }
        let offset = self.transition_offset(state);
        let mut low = 0i32;
        let mut high = num - 1;
        while low <= high {
            let mid = ((low + high) >> 1) as usize;
            let min_label = self.transitions[offset + 3 * mid + 1];
            if min_label > label {
                high = mid as i32 - 1;
            } else {
                let max_label = self.transitions[offset + 3 * mid + 2];
                if max_label < label {
                    low = mid as i32 + 1;
                } else {
                    return self.transitions[offset + 3 * mid];
                }
            }
        }
        -1
    }

    /// Returns all interval start points used by this automaton's transitions.
    pub fn get_start_points(&self) -> Vec<i32> {
        let mut points: HashSet<i32> = HashSet::new();
        points.insert(MIN_CODE_POINT);
        for s in 0..self.next_state {
            let offset = self.transition_offset(s);
            let count = self.get_num_transitions(s) as usize;
            for i in 0..count {
                let min = self.transitions[offset + 3 * i + 1];
                let max = self.transitions[offset + 3 * i + 2];
                points.insert(min);
                if max < MAX_CODE_POINT {
                    points.insert(max + 1);
                }
            }
        }
        let mut sorted: Vec<i32> = points.into_iter().collect();
        sorted.sort_unstable();
        sorted
    }

    /// Returns a copy of the transitions leaving each state.
    pub fn get_sorted_transitions(&self) -> Vec<Vec<Transition>> {
        let num_states = self.get_num_states();
        let mut result = Vec::with_capacity(num_states as usize);
        for s in 0..num_states {
            let count = self.get_num_transitions(s);
            let mut state_transitions = Vec::with_capacity(count as usize);
            let mut t = Transition::default();
            for i in 0..count {
                self.get_transition(s, i, &mut t);
                state_transitions.push(t.clone());
            }
            result.push(state_transitions);
        }
        result
    }
}

// -----------------------------------------------------------------------------
// ByteRunnable
// -----------------------------------------------------------------------------

/// Automaton that can be run against a byte slice.
///
/// Equivalent to `org.apache.lucene.util.automaton.ByteRunnable`.
pub trait ByteRunnable {
    /// Returns the state reached from `state` on byte `c`, or `-1` if none.
    fn step(&self, state: i32, c: i32) -> i32;

    /// Returns true if `state` is an accept state.
    fn is_accept(&self, state: i32) -> bool;

    /// Returns the number of states.
    fn size(&self) -> i32;

    /// Returns true if `bytes` is accepted by this automaton.
    fn run(&self, bytes: &[u8]) -> bool {
        let mut state = 0i32;
        for &b in bytes {
            state = self.step(state, b as i32);
            if state == -1 {
                return false;
            }
        }
        self.is_accept(state)
    }
}

// -----------------------------------------------------------------------------
// RunAutomaton
// -----------------------------------------------------------------------------

/// Deterministic automaton compiled into a dense transition table.
///
/// Equivalent to `org.apache.lucene.util.automaton.RunAutomaton`.
#[derive(Clone, Debug)]
pub struct RunAutomaton {
    /// Source automaton (kept for inspection / recompilation).
    automaton: Automaton,
    alphabet_size: i32,
    size: i32,
    accept: FixedBitSet,
    points: Vec<i32>,
    classmap: Vec<i32>,
    transitions: Vec<i32>,
}

impl RunAutomaton {
    /// Compiles `automaton` into a runnable table over `alphabet_size` symbols.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `automaton` is not
    /// deterministic.
    pub fn new(automaton: Automaton, alphabet_size: i32) -> Result<Self> {
        if !automaton.is_deterministic() {
            return Err(LuceneError::IllegalArgument(
                "automaton must be deterministic".to_string(),
            ));
        }

        let points = automaton.get_start_points();
        let num_states = automaton.get_num_states();
        let size = num_states.max(1);
        let mut accept = FixedBitSet::new(size as usize);
        for s in 0..num_states {
            if automaton.is_accept(s) {
                accept.set(s as usize);
            }
        }

        let mut transitions = vec![-1i32; (size as usize) * points.len()];
        for s in 0..size {
            for (c, &point) in points.iter().enumerate() {
                let dest = automaton.step(s, point);
                transitions[(s as usize) * points.len() + c] = dest;
            }
        }

        let classmap_len = (256_i32).min(alphabet_size) as usize;
        let mut classmap = vec![0i32; classmap_len];
        let mut i = 0usize;
        for (j, cm) in classmap.iter_mut().enumerate() {
            while i + 1 < points.len() && j == points[i + 1] as usize {
                i += 1;
            }
            *cm = i as i32;
        }

        Ok(Self {
            automaton,
            alphabet_size,
            size,
            accept,
            points,
            classmap,
            transitions,
        })
    }

    /// Returns the number of states.
    pub fn size(&self) -> i32 {
        self.size
    }

    /// Returns true if `state` is an accept state.
    pub fn is_accept(&self, state: i32) -> bool {
        self.accept.get(state as usize)
    }

    /// Returns the state reached from `state` on symbol `c`.
    pub fn step(&self, state: i32, c: i32) -> i32 {
        assert!(c < self.alphabet_size, "symbol {} out of alphabet", c);
        let class = if (c as usize) < self.classmap.len() {
            self.classmap[c as usize]
        } else {
            self.char_class(c)
        };
        self.transitions[(state as usize) * self.points.len() + class as usize]
    }

    fn char_class(&self, c: i32) -> i32 {
        let mut a = 0usize;
        let mut b = self.points.len();
        while b - a > 1 {
            let d = (a + b) >> 1;
            if self.points[d] > c {
                b = d;
            } else if self.points[d] < c {
                a = d;
            } else {
                return d as i32;
            }
        }
        a as i32
    }

    /// Returns the class interval start points used by this table.
    pub fn get_char_intervals(&self) -> &[i32] {
        &self.points
    }

    /// Returns the source automaton used to build this table.
    pub fn automaton(&self) -> &Automaton {
        &self.automaton
    }
}

impl ByteRunnable for RunAutomaton {
    fn step(&self, state: i32, c: i32) -> i32 {
        self.step(state, c)
    }

    fn is_accept(&self, state: i32) -> bool {
        self.is_accept(state)
    }

    fn size(&self) -> i32 {
        self.size
    }
}

// -----------------------------------------------------------------------------
// ByteRunAutomaton
// -----------------------------------------------------------------------------

/// Byte-level runnable automaton operating on the 256-symbol alphabet.
///
/// Equivalent to `org.apache.lucene.util.automaton.ByteRunAutomaton`.
#[derive(Clone, Debug)]
pub struct ByteRunAutomaton {
    inner: RunAutomaton,
}

impl ByteRunAutomaton {
    /// Compiles `automaton` for matching byte slices.
    ///
    /// When `is_binary` is false the automaton is assumed to be a Unicode
    /// codepoint DFA; this minimal port only supports codepoints that fit in a
    /// single byte.  Full UTF-32 to UTF-8 expansion is out of scope here.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` for a non-deterministic input, or
    /// `LuceneError::UnsupportedOperation` if a non-binary automaton contains
    /// labels outside the byte range.
    pub fn new(automaton: Automaton, is_binary: bool) -> Result<Self> {
        let binary = if is_binary {
            automaton
        } else {
            Self::convert(automaton)?
        };
        Ok(Self {
            inner: RunAutomaton::new(binary, 256)?,
        })
    }

    fn convert(automaton: Automaton) -> Result<Automaton> {
        if !automaton.is_deterministic() {
            return Err(LuceneError::IllegalArgument(
                "automaton must be deterministic".to_string(),
            ));
        }
        let mut t = Transition::default();
        for s in 0..automaton.get_num_states() {
            let count = automaton.init_transition(s, &mut t);
            for _ in 0..count {
                automaton.get_next_transition(&mut t);
                if t.min < 0 || t.max > 255 {
                    return Err(LuceneError::UnsupportedOperation(
                        "UTF-32 to UTF-8 conversion is not implemented in this minimal port"
                            .to_string(),
                    ));
                }
            }
        }
        Ok(automaton)
    }

    /// Returns a reference to the underlying run table.
    pub fn run_automaton(&self) -> &RunAutomaton {
        &self.inner
    }

    /// Returns the source automaton used to build this table.
    pub fn automaton(&self) -> &Automaton {
        self.inner.automaton()
    }
}

impl ByteRunnable for ByteRunAutomaton {
    fn step(&self, state: i32, c: i32) -> i32 {
        self.inner.step(state, c)
    }

    fn is_accept(&self, state: i32) -> bool {
        self.inner.is_accept(state)
    }

    fn size(&self) -> i32 {
        self.inner.size()
    }
}

// -----------------------------------------------------------------------------
// CompiledAutomaton
// -----------------------------------------------------------------------------

/// Classifies the compiled form of an automaton.
///
/// Equivalent to `CompiledAutomaton.AUTOMATON_TYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomatonType {
    /// Accepts no strings.
    None,
    /// Accepts all strings.
    All,
    /// Accepts exactly one fixed string.
    Single,
    /// Any other automaton.
    Normal,
}

/// Immutable compiled details for an automaton.
///
/// Equivalent to `org.apache.lucene.util.automaton.CompiledAutomaton`.
#[derive(Clone, Debug)]
pub struct CompiledAutomaton {
    /// Class of this automaton.
    pub automaton_type: AutomatonType,
    /// Singleton term; only set for [`AutomatonType::Single`].
    pub term: Option<BytesRef>,
    /// Byte-level matcher; only set for [`AutomatonType::Normal`].
    pub run_automaton: Option<ByteRunAutomaton>,
    /// Transition accessor consistent with `run_automaton`; only set for
    /// [`AutomatonType::Normal`].
    pub automaton: Option<Automaton>,
    /// Common suffix of all accepted strings, if known.
    pub common_suffix_ref: Option<BytesRef>,
    /// True if the language is finite.
    pub finite: bool,
    /// Sink state accepting all suffixes, or `-1` if none.
    pub sink_state: i32,
}

impl CompiledAutomaton {
    /// Compiles an automaton with the requested simplification options.
    ///
    /// `finite` should be `true` when the language is known to be finite.
    /// When `simplify` is true the automaton is classified as
    /// `NONE`/`ALL`/`SINGLE` when possible.  `is_binary` indicates that the
    /// input is already byte-based.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` or `LuceneError::UnsupportedOperation`
    /// if the automaton cannot be compiled in this minimal port.
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

        if simplify && automaton.is_deterministic() {
            if operations::is_empty(&automaton) {
                return Ok(Self {
                    automaton_type: AutomatonType::None,
                    term: None,
                    run_automaton: None,
                    automaton: None,
                    common_suffix_ref: None,
                    finite: true,
                    sink_state: -1,
                });
            }

            let total = if is_binary {
                operations::is_total_range(&automaton, 0, 255)
            } else {
                operations::is_total(&automaton)
            };
            if total {
                return Ok(Self {
                    automaton_type: AutomatonType::All,
                    term: None,
                    run_automaton: None,
                    automaton: None,
                    common_suffix_ref: None,
                    finite: false,
                    sink_state: -1,
                });
            }

            if let Some(singleton) = operations::get_singleton(&automaton) {
                let term = if is_binary {
                    StringHelper::ints_ref_to_bytes_ref(&singleton).unwrap_or_default()
                } else {
                    Self::ints_ref_to_utf8_bytes_ref(&singleton)
                };
                return Ok(Self {
                    automaton_type: AutomatonType::Single,
                    term: Some(term),
                    run_automaton: None,
                    automaton: None,
                    common_suffix_ref: None,
                    finite: true,
                    sink_state: -1,
                });
            }
        }

        let binary = if is_binary {
            automaton
        } else {
            return Err(LuceneError::UnsupportedOperation(
                "non-binary UTF-32 to UTF-8 CompiledAutomaton is not implemented in this minimal port"
                    .to_string(),
            ));
        };

        let total_transitions: i32 = (0..binary.get_num_states())
            .map(|s| binary.get_num_transitions(s))
            .sum();
        let common_suffix_ref = if finite || (binary.get_num_states() + total_transitions) > 1000 {
            None
        } else {
            operations::get_common_suffix_bytes_ref(&binary)
        };

        let run_automaton = ByteRunAutomaton::new(binary, true)?;
        let sink_state = Self::find_sink_state(run_automaton.automaton());
        let automaton = run_automaton.automaton().clone();

        Ok(Self {
            automaton_type: AutomatonType::Normal,
            term: None,
            run_automaton: Some(run_automaton),
            automaton: Some(automaton),
            common_suffix_ref,
            finite,
            sink_state,
        })
    }

    fn ints_ref_to_utf8_bytes_ref(ints: &IntsRef) -> BytesRef {
        let slice = ints.slice();
        let mut bytes = Vec::with_capacity(slice.len() * 4);
        for &cp in slice {
            let c = cp as u32;
            if c < 0x80 {
                bytes.push(c as u8);
            } else if c < 0x800 {
                bytes.push((0xC0 | (c >> 6)) as u8);
                bytes.push((0x80 | (c & 0x3F)) as u8);
            } else if c < 0x10000 {
                bytes.push((0xE0 | (c >> 12)) as u8);
                bytes.push((0x80 | ((c >> 6) & 0x3F)) as u8);
                bytes.push((0x80 | (c & 0x3F)) as u8);
            } else {
                bytes.push((0xF0 | (c >> 18)) as u8);
                bytes.push((0x80 | ((c >> 12) & 0x3F)) as u8);
                bytes.push((0x80 | ((c >> 6) & 0x3F)) as u8);
                bytes.push((0x80 | (c & 0x3F)) as u8);
            }
        }
        BytesRef::new(bytes)
    }

    fn find_sink_state(automaton: &Automaton) -> i32 {
        let mut t = Transition::default();
        for s in 0..automaton.get_num_states() {
            if automaton.is_accept(s) {
                let count = automaton.init_transition(s, &mut t);
                for i in 0..count {
                    automaton.get_transition(s, i, &mut t);
                    if t.dest == s && t.min == 0 && t.max == 255 {
                        return s;
                    }
                }
            }
        }
        -1
    }

    /// Returns a `TermsEnum` intersecting `terms` with this automaton.
    ///
    /// Equivalent to `CompiledAutomaton.getTermsEnum`.
    pub fn get_terms_enum(&self, terms: &dyn Terms) -> Result<Box<dyn TermsEnum>> {
        use crate::codecs::postings::{empty_terms_enum, SingleTermsEnum};
        match self.automaton_type {
            AutomatonType::None => Ok(empty_terms_enum()),
            AutomatonType::All => terms.iterator(),
            AutomatonType::Single => {
                let term = self
                    .term
                    .as_ref()
                    .expect("SINGLE CompiledAutomaton must have a term")
                    .clone();
                Ok(Box::new(SingleTermsEnum::new(terms.iterator()?, term)))
            }
            AutomatonType::Normal => terms.intersect(self, None),
        }
    }
}

// -----------------------------------------------------------------------------
// Automata builder helpers
// -----------------------------------------------------------------------------

/// Helpers that build common small automata.
///
/// Equivalent to `org.apache.lucene.util.automaton.Automata`.
pub mod automata {
    use super::*;

    /// Returns an automaton that accepts no strings.
    pub fn make_empty() -> Automaton {
        let mut a = Automaton::new();
        a.finish_state();
        a
    }

    /// Returns an automaton that accepts all strings.
    pub fn make_any_string() -> Automaton {
        let mut a = Automaton::new();
        let s = a.create_state();
        a.set_accept(s, true);
        a.add_transition_range(s, s, MIN_CODE_POINT, MAX_CODE_POINT);
        a.finish_state();
        a
    }

    /// Returns an automaton that accepts all binary terms.
    fn make_any_binary() -> Automaton {
        let mut a = Automaton::new();
        let s = a.create_state();
        a.set_accept(s, true);
        a.add_transition_range(s, s, 0, 255);
        a.finish_state();
        a
    }

    /// Returns an automaton that accepts all non-empty binary terms.
    fn make_non_empty_binary() -> Automaton {
        let mut a = Automaton::new();
        let s1 = a.create_state();
        let s2 = a.create_state();
        a.set_accept(s2, true);
        a.add_transition_range(s1, s2, 0, 255);
        a.add_transition_range(s2, s2, 0, 255);
        a.finish_state();
        a
    }

    /// Returns an automaton that accepts a single binary term.
    fn make_binary(term: &BytesRef) -> Automaton {
        let mut a = Automaton::new();
        let mut last = a.create_state();
        for i in 0..term.length {
            let s = a.create_state();
            let label = term.bytes[term.offset + i] as i32;
            a.add_transition(last, s, label);
            last = s;
        }
        a.set_accept(last, true);
        a.finish_state();
        a
    }

    /// Returns an automaton that accepts the single byte string `bytes`.
    pub fn make_string(bytes: &[u8]) -> Automaton {
        let mut a = Automaton::new();
        let mut last = a.create_state();
        for &b in bytes {
            let s = a.create_state();
            a.add_transition(last, s, b as i32);
            last = s;
        }
        a.set_accept(last, true);
        a.finish_state();
        a
    }

    /// Returns true if every byte after `prefix_len` in `max` is zero.
    fn suffix_is_zeros(max: &BytesRef, prefix_len: usize) -> bool {
        if max.length <= prefix_len {
            return true;
        }
        max.bytes[max.offset + prefix_len..max.offset + max.length]
            .iter()
            .all(|&b| b == 0)
    }

    /// Returns an automaton accepting binary terms inside `[min, max]`.
    ///
    /// `None` for `min`/`max` denotes an open-ended bound; the corresponding
    /// `*_inclusive` flag must be `true` for open ends.
    pub fn make_binary_interval(
        min: Option<&BytesRef>,
        max: Option<&BytesRef>,
        min_inclusive: bool,
        max_inclusive: bool,
    ) -> Result<Automaton> {
        if min.is_none() && !min_inclusive {
            return Err(LuceneError::IllegalArgument(
                "min_inclusive must be true when min is None".to_string(),
            ));
        }
        if max.is_none() && !max_inclusive {
            return Err(LuceneError::IllegalArgument(
                "max_inclusive must be true when max is None".to_string(),
            ));
        }

        let min_ref = min.unwrap_or_else(|| {
            static EMPTY: std::sync::OnceLock<BytesRef> = std::sync::OnceLock::new();
            EMPTY.get_or_init(BytesRef::default)
        });
        let min_inclusive = min_inclusive || min.is_none();

        let cmp = max
            .map(|max_ref| min_ref.cmp(max_ref))
            .unwrap_or(std::cmp::Ordering::Less);

        if let Some(max_ref) = max {
            match cmp {
                std::cmp::Ordering::Equal => {
                    return if !min_inclusive || !max_inclusive {
                        Ok(make_empty())
                    } else {
                        Ok(make_binary(min_ref))
                    };
                }
                std::cmp::Ordering::Greater => return Ok(make_empty()),
                std::cmp::Ordering::Less => {}
            }

            if max_ref.length > min_ref.length
                && StringHelper::starts_with(max_ref, min_ref)
                && suffix_is_zeros(max_ref, min_ref.length)
            {
                let mut max_length = max_ref.length;
                if !max_inclusive {
                    max_length -= 1;
                }
                if max_length == min_ref.length {
                    return if min_inclusive {
                        Ok(make_binary(min_ref))
                    } else {
                        Ok(make_empty())
                    };
                }

                let mut a = Automaton::new();
                let mut last = a.create_state();
                for i in 0..min_ref.length {
                    let s = a.create_state();
                    let label = min_ref.bytes[min_ref.offset + i] as i32;
                    a.add_transition(last, s, label);
                    last = s;
                }
                if min_inclusive {
                    a.set_accept(last, true);
                }
                for _ in min_ref.length..max_length {
                    let s = a.create_state();
                    a.add_transition(last, s, 0);
                    a.set_accept(s, true);
                    last = s;
                }
                a.finish_state();
                return Ok(a);
            }
        } else if min_ref.length == 0 {
            return if min_inclusive {
                Ok(make_any_binary())
            } else {
                Ok(make_non_empty_binary())
            };
        }

        // General case.
        let mut a = Automaton::new();
        let start_state = a.create_state();
        let sink_state = a.create_state();
        a.set_accept(sink_state, true);
        a.add_transition_range(sink_state, sink_state, 0, 255);

        let mut equal_prefix = true;
        let mut last_state = start_state;
        let mut first_max_state = -1i32;
        let mut shared_prefix_length = 0usize;

        for i in 0..min_ref.length {
            let min_label = min_ref.bytes[min_ref.offset + i] as i32;
            let max_label = max.and_then(|max_ref| {
                if equal_prefix && i < max_ref.length {
                    Some(max_ref.bytes[max_ref.offset + i] as i32)
                } else {
                    None
                }
            });

            let next_state =
                if min_inclusive && i == min_ref.length - 1 && max_label != Some(min_label) {
                    sink_state
                } else {
                    a.create_state()
                };

            if equal_prefix {
                if let Some(ml) = max_label {
                    if min_label == ml {
                        a.add_transition(last_state, next_state, min_label);
                    } else {
                        assert!(ml > min_label);
                        a.add_transition(last_state, next_state, min_label);
                        if ml > min_label + 1 {
                            a.add_transition_range(last_state, sink_state, min_label + 1, ml - 1);
                        }
                        if max_inclusive || i < max.unwrap().length - 1 {
                            first_max_state = a.create_state();
                            if i < max.unwrap().length - 1 {
                                a.set_accept(first_max_state, true);
                            }
                            a.add_transition(last_state, first_max_state, ml);
                        }
                        equal_prefix = false;
                        shared_prefix_length = i;
                    }
                } else {
                    equal_prefix = false;
                    shared_prefix_length = 0;
                    a.add_transition_range(last_state, sink_state, min_label + 1, 255);
                    a.add_transition(last_state, next_state, min_label);
                }
            } else {
                a.add_transition(last_state, next_state, min_label);
                if min_label < 255 {
                    a.add_transition_range(last_state, sink_state, min_label + 1, 255);
                }
            }
            last_state = next_state;
        }

        if !equal_prefix && last_state != sink_state && last_state != start_state {
            a.add_transition_range(last_state, sink_state, 0, 255);
        }
        if min_inclusive {
            a.set_accept(last_state, true);
        }

        if let Some(max_ref) = max {
            if first_max_state == -1 {
                shared_prefix_length = min_ref.length;
            } else {
                last_state = first_max_state;
                shared_prefix_length += 1;
            }
            for i in shared_prefix_length..max_ref.length {
                let max_label = max_ref.bytes[max_ref.offset + i] as i32;
                if max_label > 0 {
                    a.add_transition_range(last_state, sink_state, 0, max_label - 1);
                }
                if max_inclusive || i < max_ref.length - 1 {
                    let next_state = a.create_state();
                    if i < max_ref.length - 1 {
                        a.set_accept(next_state, true);
                    }
                    a.add_transition(last_state, next_state, max_label);
                    last_state = next_state;
                }
            }
            if max_inclusive {
                a.set_accept(last_state, true);
            }
        }

        a.finish_state();
        Ok(a)
    }
}

// -----------------------------------------------------------------------------
// Operations
// -----------------------------------------------------------------------------

/// Minimal automaton operations needed by [`CompiledAutomaton`].
///
/// Equivalent to a subset of `org.apache.lucene.util.automaton.Operations`.
pub mod operations {
    use super::*;

    /// Returns true if the automaton accepts no strings.
    pub fn is_empty(a: &Automaton) -> bool {
        if a.get_num_states() == 0 {
            return true;
        }
        if !a.is_accept(0) && a.get_num_transitions(0) == 0 {
            return true;
        }
        if a.is_accept(0) {
            return false;
        }

        let mut seen = vec![false; a.get_num_states() as usize];
        let mut queue = VecDeque::new();
        seen[0] = true;
        queue.push_back(0i32);
        let mut t = Transition::default();
        while let Some(state) = queue.pop_front() {
            if a.is_accept(state) {
                return false;
            }
            let count = a.init_transition(state, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                if !seen[t.dest as usize] {
                    seen[t.dest as usize] = true;
                    queue.push_back(t.dest);
                }
            }
        }
        true
    }

    /// Returns true if the automaton accepts all strings.
    pub fn is_total(a: &Automaton) -> bool {
        is_total_range(a, MIN_CODE_POINT, MAX_CODE_POINT)
    }

    /// Returns true if the automaton accepts all strings over `[min_alphabet,
    /// max_alphabet]`.
    pub fn is_total_range(a: &Automaton, min_alphabet: i32, max_alphabet: i32) -> bool {
        let live = get_live_states(a);
        let mut t = Transition::default();
        let mut seen_states = 0usize;
        for state in 0..a.get_num_states() {
            if !live.get(state as usize) {
                continue;
            }
            if !a.is_accept(state) {
                return false;
            }
            let mut previous_label = min_alphabet - 1;
            let count = a.get_num_transitions(state);
            for i in 0..count {
                a.get_transition(state, i, &mut t);
                if t.min > previous_label + 1 {
                    return false;
                }
                previous_label = t.max;
            }
            if previous_label < max_alphabet {
                return false;
            }
            seen_states += 1;
        }
        seen_states > 0
    }

    /// If `a` accepts a single string, returns it; otherwise `None`.
    pub fn get_singleton(a: &Automaton) -> Option<IntsRef> {
        let mut builder: Vec<i32> = Vec::new();
        let mut visited = vec![false; a.get_num_states() as usize];
        let mut s = 0i32;
        let mut t = Transition::default();
        loop {
            visited[s as usize] = true;
            if !a.is_accept(s) {
                if a.get_num_transitions(s) == 1 {
                    a.get_transition(s, 0, &mut t);
                    if t.min == t.max && !visited[t.dest as usize] {
                        builder.push(t.min);
                        s = t.dest;
                        continue;
                    }
                }
            } else if a.get_num_transitions(s) == 0 {
                return Some(IntsRef::new(builder));
            }
            return None;
        }
    }

    /// Returns the longest byte suffix shared by all accepted strings, if any.
    pub fn get_common_suffix_bytes_ref(a: &Automaton) -> Option<BytesRef> {
        if is_empty(a) {
            return None;
        }

        let live = get_live_states(a);
        let reverse = reverse_adjacency(a);

        let mut current: Vec<i32> = (0..a.get_num_states())
            .filter(|s| a.is_accept(*s) && live.get(*s as usize))
            .collect();
        let mut suffix: Vec<u8> = Vec::new();

        loop {
            if current.contains(&0) {
                break;
            }

            let mut label: Option<u8> = None;
            let mut next_set: Vec<i32> = Vec::new();
            let mut possible = true;

            for &state in current.iter() {
                let incoming = reverse.get(state as usize)?;
                let mut state_labels: Vec<u8> = Vec::new();
                for (src, min, max) in incoming {
                    if !live.get(*src as usize) {
                        continue;
                    }
                    for l in *min..=*max {
                        if (0..=255).contains(&l) {
                            state_labels.push(l as u8);
                        }
                    }
                }
                state_labels.sort_unstable();
                state_labels.dedup();

                if state_labels.is_empty() {
                    possible = false;
                    break;
                }

                if let Some(l) = label {
                    if state_labels.len() != 1 || state_labels[0] != l {
                        possible = false;
                        break;
                    }
                } else {
                    if state_labels.len() != 1 {
                        possible = false;
                        break;
                    }
                    label = Some(state_labels[0]);
                }
            }

            if !possible {
                break;
            }

            let l = label.unwrap() as i32;
            for &state in current.iter() {
                let incoming = reverse.get(state as usize).unwrap();
                for (src, min, max) in incoming {
                    if !live.get(*src as usize) {
                        continue;
                    }
                    if l >= *min && l <= *max && !next_set.contains(src) {
                        next_set.push(*src);
                    }
                }
            }

            suffix.push(l as u8);
            current = next_set;
        }

        if suffix.is_empty() {
            return None;
        }
        suffix.reverse();
        Some(BytesRef::new(suffix))
    }

    /// Determinize an automaton.
    ///
    /// This minimal implementation returns a clone of the input when it is
    /// already deterministic and has no overlapping transitions.  Full subset
    /// construction is out of scope.
    pub fn determinize(a: &Automaton, _max_determinized_states: i64) -> Result<Automaton> {
        if a.is_deterministic() {
            Ok(a.clone())
        } else {
            Err(LuceneError::UnsupportedOperation(
                "general NFA determinization is not implemented in this minimal port".to_string(),
            ))
        }
    }

    fn get_live_states(a: &Automaton) -> FixedBitSet {
        let num_states = a.get_num_states() as usize;
        let mut from_initial = FixedBitSet::new(num_states);
        let mut queue = VecDeque::new();
        from_initial.set(0);
        queue.push_back(0i32);
        let mut t = Transition::default();
        while let Some(s) = queue.pop_front() {
            let count = a.init_transition(s, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                if !from_initial.get(t.dest as usize) {
                    from_initial.set(t.dest as usize);
                    queue.push_back(t.dest);
                }
            }
        }

        let to_accept = reverse_reachable(a);

        let mut live = FixedBitSet::new(num_states);
        for s in 0..num_states {
            if from_initial.get(s) && to_accept.get(s) {
                live.set(s);
            }
        }
        live
    }

    fn reverse_reachable(a: &Automaton) -> FixedBitSet {
        let num_states = a.get_num_states() as usize;
        let reverse = reverse_adjacency(a);
        let mut seen = FixedBitSet::new(num_states);
        let mut queue = VecDeque::new();
        for s in 0..a.get_num_states() {
            if a.is_accept(s) {
                seen.set(s as usize);
                queue.push_back(s);
            }
        }
        while let Some(state) = queue.pop_front() {
            let Some(incoming) = reverse.get(state as usize) else {
                continue;
            };
            for (src, _min, _max) in incoming {
                if !seen.get(*src as usize) {
                    seen.set(*src as usize);
                    queue.push_back(*src);
                }
            }
        }
        let mut result = FixedBitSet::new(num_states);
        for s in 0..num_states {
            if seen.get(s) {
                result.set(s);
            }
        }
        result
    }

    fn reverse_adjacency(a: &Automaton) -> Vec<Vec<(i32, i32, i32)>> {
        let num_states = a.get_num_states() as usize;
        let mut reverse: Vec<Vec<(i32, i32, i32)>> = vec![Vec::new(); num_states];
        let mut t = Transition::default();
        for s in 0..a.get_num_states() {
            let count = a.init_transition(s, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                reverse[t.dest as usize].push((s, t.min, t.max));
            }
        }
        reverse
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::postings::{EmptyPostingsEnum, PostingsEnum, Terms, TermsEnum};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn empty_automaton_accepts_nothing() {
        let a = automata::make_empty();
        assert!(operations::is_empty(&a));
        assert_eq!(a.get_num_states(), 0);
    }

    #[test]
    fn any_string_binary_matches_any_bytes() {
        // Build a byte-based "any string" automaton manually so it can be run.
        let mut a = Automaton::new();
        let s = a.create_state();
        a.set_accept(s, true);
        a.add_transition_range(s, s, 0, 255);
        a.finish_state();

        let run = ByteRunAutomaton::new(a, true).unwrap();
        assert!(run.run(b""));
        assert!(run.run(b"hello"));
        assert!(run.run(b"\x00\xff"));
    }

    #[test]
    fn fixed_string_automaton_matches_exactly() {
        let a = automata::make_string(b"lucene");
        let run = ByteRunAutomaton::new(a, true).unwrap();
        assert!(run.run(b"lucene"));
        assert!(!run.run(b"lucen"));
        assert!(!run.run(b"lucene!"));
        assert!(!run.run(b"nucene"));
    }

    #[test]
    fn binary_interval_accepts_in_range() {
        let min = BytesRef::new(b"aa".to_vec());
        let max = BytesRef::new(b"zz".to_vec());
        let a = automata::make_binary_interval(Some(&min), Some(&max), true, true).unwrap();
        let run = ByteRunAutomaton::new(a, true).unwrap();
        assert!(run.run(b"aa"));
        assert!(run.run(b"mn"));
        assert!(run.run(b"zz"));
        assert!(!run.run(b"a"));
        assert!(!run.run(b"zzz"));
    }

    #[test]
    fn compiled_automaton_classifies_simple_types() {
        let empty = automata::make_empty();
        let compiled = CompiledAutomaton::new(empty, true, true, true).unwrap();
        assert_eq!(compiled.automaton_type, AutomatonType::None);

        let any = automata::make_any_string();
        let compiled = CompiledAutomaton::new(any, false, true, false).unwrap();
        assert_eq!(compiled.automaton_type, AutomatonType::All);

        let single = automata::make_string(b"cat");
        let compiled = CompiledAutomaton::new(single, true, true, true).unwrap();
        assert_eq!(compiled.automaton_type, AutomatonType::Single);
        assert_eq!(compiled.term, Some(BytesRef::new(b"cat".to_vec())));

        // A non-trivial range compiles to NORMAL.
        let min = BytesRef::new(b"a".to_vec());
        let max = BytesRef::new(b"z".to_vec());
        let range = automata::make_binary_interval(Some(&min), Some(&max), true, true).unwrap();
        let compiled = CompiledAutomaton::new(range, true, true, true).unwrap();
        assert_eq!(compiled.automaton_type, AutomatonType::Normal);
        assert!(compiled.run_automaton.is_some());
        assert!(compiled.automaton.is_some());
    }

    #[test]
    fn compiled_automaton_run_matches_binary_interval() {
        let min = BytesRef::new(b"a".to_vec());
        let max = BytesRef::new(b"z".to_vec());
        let range = automata::make_binary_interval(Some(&min), Some(&max), true, true).unwrap();
        let compiled = CompiledAutomaton::new(range, true, true, true).unwrap();
        let run = compiled.run_automaton.as_ref().unwrap();
        assert!(run.run(b"a"));
        assert!(run.run(b"m"));
        assert!(run.run(b"z"));
        assert!(!run.run(b""));
        assert!(run.run(b"aa"));
        assert!(run.run(b"az"));
    }

    // Stub Terms / TermsEnum --------------------------------------------------

    #[derive(Debug, Clone)]
    struct ListTermsEnum {
        terms: Vec<BytesRef>,
        pos: usize,
    }

    impl TermsEnum for ListTermsEnum {
        fn term(&self) -> &BytesRef {
            &self.terms[self.pos]
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> crate::error::Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(EmptyPostingsEnum))
        }
    }

    #[derive(Debug)]
    struct IntersectTermsEnum;

    impl TermsEnum for IntersectTermsEnum {
        fn term(&self) -> &BytesRef {
            static MARKER: std::sync::OnceLock<BytesRef> = std::sync::OnceLock::new();
            MARKER.get_or_init(|| BytesRef::new(b"intersect".to_vec()))
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> crate::error::Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(EmptyPostingsEnum))
        }
    }

    struct StubTerms {
        terms: Vec<BytesRef>,
        intersect_called: AtomicBool,
    }

    impl Terms for StubTerms {
        fn iterator(&self) -> crate::error::Result<Box<dyn TermsEnum>> {
            Ok(Box::new(ListTermsEnum {
                terms: self.terms.clone(),
                pos: 0,
            }))
        }

        fn size(&self) -> i64 {
            self.terms.len() as i64
        }

        fn doc_count(&self) -> i32 {
            -1
        }

        fn sum_total_term_freq(&self) -> i64 {
            -1
        }

        fn sum_doc_freq(&self) -> i64 {
            -1
        }

        fn has_freqs(&self) -> bool {
            false
        }

        fn has_positions(&self) -> bool {
            false
        }

        fn has_payloads(&self) -> bool {
            false
        }

        fn has_offsets(&self) -> bool {
            false
        }

        fn min(&self) -> crate::error::Result<Option<&BytesRef>> {
            Ok(None)
        }

        fn max(&self) -> crate::error::Result<Option<&BytesRef>> {
            Ok(None)
        }

        fn intersect(
            &self,
            _automaton: &CompiledAutomaton,
            _skip_ahead: Option<&BytesRef>,
        ) -> crate::error::Result<Box<dyn TermsEnum>> {
            self.intersect_called.store(true, Ordering::SeqCst);
            Ok(Box::new(IntersectTermsEnum))
        }
    }

    #[test]
    fn get_terms_enum_dispatches_by_type() {
        let terms = StubTerms {
            terms: vec![
                BytesRef::new(b"alpha".to_vec()),
                BytesRef::new(b"beta".to_vec()),
            ],
            intersect_called: AtomicBool::new(false),
        };

        // NONE returns an empty iterator.
        let none = CompiledAutomaton::new(automata::make_empty(), true, true, true).unwrap();
        let it = none.get_terms_enum(&terms).unwrap();
        assert_eq!(it.term(), &BytesRef::default());

        // ALL returns the full iterator positioned on the first term.
        let all = CompiledAutomaton::new(automata::make_any_string(), false, true, false).unwrap();
        let it = all.get_terms_enum(&terms).unwrap();
        assert_eq!(it.term(), &BytesRef::new(b"alpha".to_vec()));

        // SINGLE returns a SingleTermsEnum reporting the fixed term.
        let single =
            CompiledAutomaton::new(automata::make_string(b"beta"), true, true, true).unwrap();
        let it = single.get_terms_enum(&terms).unwrap();
        assert_eq!(it.term(), &BytesRef::new(b"beta".to_vec()));

        // NORMAL delegates to Terms::intersect.
        let min = BytesRef::new(b"a".to_vec());
        let max = BytesRef::new(b"z".to_vec());
        let range = automata::make_binary_interval(Some(&min), Some(&max), true, true).unwrap();
        let normal = CompiledAutomaton::new(range, true, true, true).unwrap();
        assert!(!terms.intersect_called.load(Ordering::SeqCst));
        let it = normal.get_terms_enum(&terms).unwrap();
        assert!(terms.intersect_called.load(Ordering::SeqCst));
        assert_eq!(it.term(), &BytesRef::new(b"intersect".to_vec()));
    }

    #[test]
    fn transition_accessor_iterates_transitions() {
        let a = automata::make_string(b"ab");
        let mut t = Transition::default();
        let count = a.init_transition(0, &mut t);
        assert_eq!(count, 1);
        a.get_next_transition(&mut t);
        assert_eq!(t.source, 0);
        assert_eq!(t.dest, 1);
        assert_eq!(t.min, b'a' as i32);
        assert_eq!(t.max, b'a' as i32);
    }

    #[test]
    fn run_automaton_reports_size_and_accept() {
        let a = automata::make_string(b"x");
        let run = RunAutomaton::new(a, 256).unwrap();
        assert_eq!(run.size(), 2);
        assert!(run.is_accept(1));
        assert!(!run.is_accept(0));
        assert_eq!(run.step(0, b'x' as i32), 1);
        assert_eq!(run.step(0, b'y' as i32), -1);
    }
}
