//! Port of `org.apache.lucene.util.automaton.AutomatonProvider`.

use crate::error::Result;

use super::automaton::Automaton;

/// Automaton provider for `RegExp.to_automaton_with_provider`.
///
/// Equivalent to `org.apache.lucene.util.automaton.AutomatonProvider`.
pub trait AutomatonProvider {
    /// Returns the automaton of the given name.
    ///
    /// # Errors
    ///
    /// Returns an error if the automaton could not be loaded.
    fn get_automaton(&self, name: &str) -> Result<Automaton>;
}
