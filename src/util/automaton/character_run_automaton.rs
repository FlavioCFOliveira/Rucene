//! Port of `org.apache.lucene.util.automaton.CharacterRunAutomaton`.

use crate::error::Result;

use super::automaton::{Automaton, MAX_CODE_POINT};
use super::run_automaton::RunAutomaton;

/// Automaton representation for matching Unicode code points.
///
/// Equivalent to `org.apache.lucene.util.automaton.CharacterRunAutomaton`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterRunAutomaton {
    inner: RunAutomaton,
}

impl CharacterRunAutomaton {
    /// Constructs from a DFA.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// if the automaton is not deterministic.
    pub fn new(a: Automaton) -> Result<Self> {
        Ok(Self {
            inner: RunAutomaton::new(a, MAX_CODE_POINT + 1)?,
        })
    }

    /// Returns true if the given string is accepted by this automaton.
    pub fn run(&self, s: &str) -> bool {
        let mut p = 0i32;
        for cp in s.chars() {
            p = self.inner.step(p, cp as i32);
            if p == -1 {
                return false;
            }
        }
        self.inner.is_accept(p)
    }

    /// Returns true if the given code point slice is accepted by this automaton.
    ///
    /// This is the counterpart of Lucene's `run(char[], int, int)`; Rust `char`
    /// values are already whole code points, so no surrogate pairing is needed.
    pub fn run_chars(&self, s: &[char], offset: usize, length: usize) -> bool {
        let mut p = 0i32;
        for &c in &s[offset..offset + length] {
            p = self.inner.step(p, c as i32);
            if p == -1 {
                return false;
            }
        }
        self.inner.is_accept(p)
    }

    /// Returns a reference to the underlying run table.
    pub fn run_automaton(&self) -> &RunAutomaton {
        &self.inner
    }

    /// Returns the source automaton used to build this table.
    pub fn automaton(&self) -> &Automaton {
        self.inner.automaton()
    }

    /// Returns the number of states in the automaton.
    pub fn size(&self) -> i32 {
        self.inner.size()
    }

    /// Returns the acceptance status for the given state.
    pub fn is_accept(&self, state: i32) -> bool {
        self.inner.is_accept(state)
    }

    /// Returns the state obtained by reading the given code point from the given
    /// state.
    pub fn step(&self, state: i32, c: i32) -> i32 {
        self.inner.step(state, c)
    }

    /// Returns the array of codepoint class interval start points.
    pub fn get_char_intervals(&self) -> &[i32] {
        self.inner.get_char_intervals()
    }
}
