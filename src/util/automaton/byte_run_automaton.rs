//! Port of `org.apache.lucene.util.automaton.ByteRunAutomaton`.

use crate::error::{LuceneError, Result};

use super::automaton::Automaton;
use super::operations::Operations;
use super::run_automaton::{ByteRunnable, RunAutomaton};
use super::utf32_to_utf8::UTF32ToUTF8;

/// Automaton representation for matching UTF-8 bytes.
///
/// Equivalent to `org.apache.lucene.util.automaton.ByteRunAutomaton`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteRunAutomaton {
    inner: RunAutomaton,
}

impl ByteRunAutomaton {
    /// Converts the incoming automaton to byte-based (with
    /// [`UTF32ToUTF8`]) first.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the automaton is not
    /// deterministic.
    pub fn from_utf32(a: Automaton) -> Result<Self> {
        Self::new(a, false)
    }

    /// Expert: if `is_binary` is true, the input is already byte-based.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the automaton is not
    /// deterministic.
    pub fn new(a: Automaton, is_binary: bool) -> Result<Self> {
        let binary = if is_binary { a } else { Self::convert(&a)? };
        Ok(Self {
            inner: RunAutomaton::new(binary, 256)?,
        })
    }

    /// Converts a UTF-32 automaton to the equivalent byte automaton.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the automaton is not
    /// deterministic, and a resource-limit error if determinizing the byte automaton
    /// exceeds the work limit.
    pub fn convert(a: &Automaton) -> Result<Automaton> {
        if !a.is_deterministic() {
            return Err(LuceneError::IllegalArgument(
                "Automaton must be deterministic".to_string(),
            ));
        }
        // we checked the input is a DFA, according to Mike this determinization is
        // contained :)
        let utf8 = UTF32ToUTF8::new().convert(a);
        Ok(Operations::determinize(&utf8, i32::MAX)?)
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

    /// Returns the state obtained by reading the given byte from the given state.
    pub fn step(&self, state: i32, c: i32) -> i32 {
        self.inner.step(state, c)
    }

    /// Returns the array of codepoint class interval start points.
    pub fn get_char_intervals(&self) -> &[i32] {
        self.inner.get_char_intervals()
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
