//! Terms iterator intersected with a compiled automaton.
//!
//! Equivalent to `org.apache.lucene.index.AutomatonTermsEnum`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::index::terms::{AcceptStatus, FilteredTermsEnum, FilteredTermsEnumFilter, TermsEnum};
use crate::util::automaton::{
    Automaton, AutomatonType, ByteRunAutomaton, ByteRunnable, CompiledAutomaton, Transition,
    TransitionAccessor,
};
use crate::util::{ArrayUtil, BytesRef, BytesRefBuilder, StringHelper};

/// Maximum unsigned byte value.
const MAX_BYTE: i32 = 0xff;

/// A [`FilteredTermsEnum`] that enumerates the terms accepted by a DFA.
///
/// Equivalent to `org.apache.lucene.index.AutomatonTermsEnum`.
pub struct AutomatonTermsEnum {
    /// Byte-level matcher for the compiled automaton.
    byte_runnable: ByteRunAutomaton,
    /// Common suffix shared by all accepted strings, if known.
    common_suffix_ref: Option<BytesRef>,
    /// Whether the accepted language is finite.
    finite: bool,
    /// Sorted transitions for each state.
    transition_accessor: Automaton,
    /// Visited-state timestamps for infinite automata loop detection.
    visited: Vec<i16>,
    /// Generation counter for visited-state pruning.
    cur_gen: i16,
    /// Working seek term, mutated during `next_string`.
    seek_bytes_ref: BytesRefBuilder,
    /// True when the enum should scan linearly between `seek_bytes_ref` and
    /// `linear_upper_bound`.
    linear: bool,
    /// Upper bound of the linear scan region.
    linear_upper_bound: BytesRef,
    /// Reusable transition holder.
    transition: Transition,
    /// Saved automaton states while walking the current term.
    saved_states: Vec<i32>,
}

impl AutomatonTermsEnum {
    /// Creates an enumerator over `tenum` restricted to the language of
    /// `compiled`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the compiled automaton is not a
    /// normal automaton (e.g. `NONE`, `ALL`, or `SINGLE`).
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        tenum: Box<dyn TermsEnum>,
        compiled: CompiledAutomaton,
    ) -> Result<Box<dyn TermsEnum>> {
        if compiled.automaton_type != AutomatonType::Normal {
            return Err(LuceneError::IllegalArgument(
                "AutomatonTermsEnum only supports CompiledAutomaton::Normal; use CompiledAutomaton::get_terms_enum for special cases".to_string(),
            ));
        }
        let byte_runnable = compiled
            .run_automaton
            .ok_or_else(|| {
                LuceneError::IllegalState("compiled automaton has no byte runnable".to_string())
            })?
            .clone();
        let transition_accessor = compiled
            .automaton
            .ok_or_else(|| {
                LuceneError::IllegalState(
                    "compiled automaton has no transition accessor".to_string(),
                )
            })?
            .clone();
        let visited = if compiled.finite {
            Vec::new()
        } else {
            vec![-1i16; byte_runnable.size() as usize]
        };
        let filter = Box::new(Self {
            byte_runnable,
            common_suffix_ref: compiled.common_suffix_ref,
            finite: compiled.finite,
            transition_accessor,
            visited,
            cur_gen: 0,
            seek_bytes_ref: BytesRefBuilder::new(),
            linear: false,
            linear_upper_bound: BytesRef::default(),
            transition: Transition::default(),
            saved_states: Vec::new(),
        });
        Ok(Box::new(FilteredTermsEnum::new_with_seek(
            tenum,
            filter,
            BytesRef::new(Vec::new()),
        )))
    }

    fn set_visited(&mut self, state: i32) {
        if !self.finite {
            let idx = state as usize;
            if idx >= self.visited.len() {
                let new_len = ArrayUtil::oversize(idx + 1, std::mem::size_of::<i16>());
                self.visited.resize(new_len, -1);
            }
            self.visited[idx] = self.cur_gen;
        }
    }

    fn is_visited(&self, state: i32) -> bool {
        !self.finite
            && (state as usize) < self.visited.len()
            && self.visited[state as usize] == self.cur_gen
    }

    fn set_linear(&mut self, position: usize) {
        assert!(!self.linear);
        let mut state = 0i32;
        let mut max_interval = MAX_BYTE;
        for i in 0..position {
            state = self
                .byte_runnable
                .step(state, self.seek_bytes_ref.byte_at(i) as i32);
            assert!(state >= 0);
        }
        let num_transitions = self.transition_accessor.get_num_transitions(state);
        self.transition_accessor
            .init_transition(state, &mut self.transition);
        for _ in 0..num_transitions {
            self.transition_accessor
                .get_next_transition(&mut self.transition);
            let current = self.seek_bytes_ref.byte_at(position) as i32;
            if self.transition.min <= current && current <= self.transition.max {
                max_interval = self.transition.max;
                break;
            }
        }
        if max_interval != MAX_BYTE {
            max_interval += 1;
        }
        let length = position + 1;
        if self.linear_upper_bound.bytes.len() < length {
            let oversize = ArrayUtil::oversize(length, std::mem::size_of::<u8>());
            self.linear_upper_bound.bytes.resize(oversize, 0);
        }
        self.linear_upper_bound.bytes[..position]
            .copy_from_slice(&self.seek_bytes_ref.bytes()[..position]);
        self.linear_upper_bound.bytes[position] = max_interval as u8;
        self.linear_upper_bound.length = length;
        self.linear = true;
    }

    fn next_string(&mut self) -> bool {
        let mut pos = 0usize;
        self.saved_states
            .resize(self.seek_bytes_ref.length() + 1, 0);
        self.saved_states[0] = 0;

        loop {
            if !self.finite {
                self.cur_gen = self.cur_gen.wrapping_add(1);
                if self.cur_gen == 0 {
                    // Generation wrapped: clear visited state.
                    for v in &mut self.visited {
                        *v = -1;
                    }
                }
            }
            self.linear = false;

            // Walk the automaton until a byte is rejected.
            let mut state = self.saved_states[pos];
            while pos < self.seek_bytes_ref.length() {
                self.set_visited(state);
                let next_state = self
                    .byte_runnable
                    .step(state, self.seek_bytes_ref.byte_at(pos) as i32);
                if next_state == -1 {
                    break;
                }
                self.saved_states[pos + 1] = next_state;
                if !self.linear && self.is_visited(next_state) {
                    self.set_linear(pos);
                }
                state = next_state;
                pos += 1;
            }

            if self.next_string_from_state(state, pos) {
                return true;
            }
            pos = match self.backtrack(pos) {
                Some(p) => p,
                None => return false,
            };
            let new_state = self.byte_runnable.step(
                self.saved_states[pos],
                self.seek_bytes_ref.byte_at(pos) as i32,
            );
            if new_state >= 0 && self.byte_runnable.is_accept(new_state) {
                return true;
            }
            if !self.finite {
                pos = 0;
            }
        }
    }

    fn next_string_from_state(&mut self, state: i32, position: usize) -> bool {
        let mut c = 0i32;
        if position < self.seek_bytes_ref.length() {
            c = self.seek_bytes_ref.byte_at(position) as i32;
            if c == MAX_BYTE {
                return false;
            }
            c += 1;
        }

        self.seek_bytes_ref.set_length(position);
        self.set_visited(state);

        let num_transitions = self.transition_accessor.get_num_transitions(state);
        self.transition_accessor
            .init_transition(state, &mut self.transition);
        for _ in 0..num_transitions {
            self.transition_accessor
                .get_next_transition(&mut self.transition);
            if self.transition.max >= c {
                let next_char = std::cmp::max(c, self.transition.min);
                self.seek_bytes_ref.append(next_char as u8);
                let mut state = self.transition.dest;
                while !self.is_visited(state) && !self.byte_runnable.is_accept(state) {
                    self.set_visited(state);
                    self.transition_accessor
                        .init_transition(state, &mut self.transition);
                    self.transition_accessor
                        .get_next_transition(&mut self.transition);
                    state = self.transition.dest;
                    self.seek_bytes_ref.append(self.transition.min as u8);
                    if !self.linear && self.is_visited(state) {
                        self.set_linear(self.seek_bytes_ref.length() - 1);
                    }
                }
                return true;
            }
        }
        false
    }

    fn backtrack(&mut self, position: usize) -> Option<usize> {
        let mut position = position;
        while position > 0 {
            position -= 1;
            let mut next_char = self.seek_bytes_ref.byte_at(position) as i32;
            if next_char != MAX_BYTE {
                next_char += 1;
                self.seek_bytes_ref.set_byte_at(position, next_char as u8);
                self.seek_bytes_ref.set_length(position + 1);
                return Some(position);
            }
        }
        None
    }
}

impl FilteredTermsEnumFilter for AutomatonTermsEnum {
    fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus> {
        let suffix_ok = self
            .common_suffix_ref
            .as_ref()
            .map_or(true, |suffix| StringHelper::ends_with(term, suffix));
        if suffix_ok {
            if self.byte_runnable.run(term.slice()) {
                if self.linear {
                    Ok(AcceptStatus::Yes)
                } else {
                    Ok(AcceptStatus::YesAndSeek)
                }
            } else if self.linear && term.slice() < self.linear_upper_bound.slice() {
                Ok(AcceptStatus::No)
            } else {
                Ok(AcceptStatus::NoAndSeek)
            }
        } else if self.linear && term.slice() < self.linear_upper_bound.slice() {
            Ok(AcceptStatus::No)
        } else {
            Ok(AcceptStatus::NoAndSeek)
        }
    }

    fn next_seek_term(&mut self, current_term: Option<&BytesRef>) -> Result<Option<BytesRef>> {
        if let Some(term) = current_term {
            self.seek_bytes_ref.copy_ref(term);
        } else {
            assert_eq!(self.seek_bytes_ref.length(), 0);
            if self.byte_runnable.is_accept(0) {
                return Ok(Some(self.seek_bytes_ref.get()));
            }
        }

        if self.next_string() {
            Ok(Some(self.seek_bytes_ref.get()))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::postings_enum::{EmptyPostingsEnum, ImpactsEnum, PostingsEnum};
    use crate::index::terms::{SeekStatus, TermsEnum};
    use crate::util::automaton::{Automaton, CompiledAutomaton};
    use crate::util::BytesRef;

    /// List-backed [`TermsEnum`] used by the automaton tests.
    struct ListTermsEnum {
        terms: Vec<BytesRef>,
        pos: usize,
    }

    impl ListTermsEnum {
        fn new(terms: Vec<BytesRef>) -> Self {
            Self { terms, pos: 0 }
        }
    }

    impl TermsEnum for ListTermsEnum {
        fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
            panic!("not needed")
        }

        fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
            match self.terms.binary_search(text) {
                Ok(i) => {
                    self.pos = i;
                    Ok(true)
                }
                Err(_) => Ok(false),
            }
        }

        fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
            match self.terms.binary_search(text) {
                Ok(i) => {
                    self.pos = i;
                    Ok(SeekStatus::FOUND)
                }
                Err(i) => {
                    self.pos = i;
                    if i >= self.terms.len() {
                        Ok(SeekStatus::END)
                    } else {
                        Ok(SeekStatus::NOT_FOUND)
                    }
                }
            }
        }

        fn seek_ord(&mut self, ord: i64) -> Result<()> {
            self.pos = ord as usize;
            Ok(())
        }

        fn term(&self) -> Result<BytesRef> {
            Ok(self.terms[self.pos].clone())
        }

        fn ord(&self) -> Result<i64> {
            Ok(self.pos as i64)
        }

        fn doc_freq(&self) -> Result<i32> {
            Ok(1)
        }

        fn total_term_freq(&self) -> Result<i64> {
            Ok(1)
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(EmptyPostingsEnum::new()))
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
            Err(LuceneError::IllegalState(
                "impacts not supported".to_string(),
            ))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            if self.pos + 1 >= self.terms.len() {
                self.pos = self.terms.len();
                return Ok(None);
            }
            self.pos += 1;
            Ok(Some(self.terms[self.pos].clone()))
        }
    }

    fn automaton_for_prefix(prefix: &[u8]) -> CompiledAutomaton {
        let mut automaton = Automaton::new();
        let initial = automaton.create_state();
        let mut last = initial;
        for &b in prefix {
            let state = automaton.create_state();
            automaton.add_transition_range(last, state, b as i32, b as i32);
            automaton.finish_state();
            last = state;
        }
        automaton.set_accept(last, true);
        // Self-loop on the final state so the language is a true prefix (not a
        // singleton).  This keeps CompiledAutomaton from simplifying to Single.
        automaton.add_transition_range(last, last, 0, 255);
        automaton.finish_state();
        automaton.finish();
        CompiledAutomaton::new(automaton, false, true, true).unwrap()
    }

    #[test]
    fn automaton_terms_enum_filters_by_prefix() {
        let inner = Box::new(ListTermsEnum::new(vec![
            BytesRef::new(b"a".to_vec()),
            BytesRef::new(b"ab".to_vec()),
            BytesRef::new(b"abc".to_vec()),
            BytesRef::new(b"b".to_vec()),
        ]));
        let compiled = automaton_for_prefix(b"ab");
        let mut it = AutomatonTermsEnum::new(inner, compiled).unwrap();
        assert_eq!(it.next().unwrap().unwrap().slice(), b"ab");
        assert_eq!(it.next().unwrap().unwrap().slice(), b"abc");
        assert!(it.next().unwrap().is_none());
    }
}
