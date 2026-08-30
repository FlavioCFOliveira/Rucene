//! Port of `org.apache.lucene.util.automaton.TooComplexToDeterminizeException`.

use crate::error::LuceneError;

use super::automaton::Automaton;
use super::reg_exp::RegExp;

/// This exception is thrown when determinizing an automaton would require too much
/// work.
///
/// Equivalent to `org.apache.lucene.util.automaton.TooComplexToDeterminizeException`.
///
/// # Divergence from Lucene 10.5.0
///
/// Lucene models this as an unchecked `RuntimeException`. Rucene has no exceptions,
/// so this is a plain error type that converts into [`LuceneError::ResourceLimit`]
/// (the crate's "a resource limit was violated" variant) so it can flow through
/// `Result` chains with `?`.
#[derive(Clone, Debug)]
pub struct TooComplexToDeterminizeException {
    message: String,
    automaton: Option<Box<Automaton>>,
    reg_exp: Option<Box<RegExp>>,
    determinize_work_limit: i32,
}

impl TooComplexToDeterminizeException {
    /// Use this constructor when the automaton failed to determinize.
    pub fn from_automaton(automaton: Automaton, determinize_work_limit: i32) -> Self {
        let message = format!(
            "Determinizing automaton with {} states and {} transitions would require more than {} effort.",
            automaton.get_num_states(),
            automaton.get_num_transitions_total(),
            determinize_work_limit
        );
        Self {
            message,
            automaton: Some(Box::new(automaton)),
            reg_exp: None,
            determinize_work_limit,
        }
    }

    /// Use this constructor when the [`RegExp`] failed to convert to an automaton.
    pub fn from_reg_exp(reg_exp: &RegExp, cause: &TooComplexToDeterminizeException) -> Self {
        let message = format!(
            "Determinizing {} would require more than {} effort.",
            reg_exp.get_original_string().unwrap_or(""),
            cause.determinize_work_limit
        );
        Self {
            message,
            automaton: cause.automaton.clone(),
            reg_exp: Some(Box::new(reg_exp.clone())),
            determinize_work_limit: cause.determinize_work_limit,
        }
    }

    /// Returns the automaton that caused this exception, if any.
    pub fn get_automaton(&self) -> Option<&Automaton> {
        self.automaton.as_deref()
    }

    /// Returns the [`RegExp`] that caused this exception, if any.
    pub fn get_reg_exp(&self) -> Option<&RegExp> {
        self.reg_exp.as_deref()
    }

    /// Returns the maximum allowed determinize effort.
    pub fn get_determinize_work_limit(&self) -> i32 {
        self.determinize_work_limit
    }
}

impl std::fmt::Display for TooComplexToDeterminizeException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TooComplexToDeterminizeException {}

impl From<TooComplexToDeterminizeException> for LuceneError {
    fn from(value: TooComplexToDeterminizeException) -> Self {
        LuceneError::ResourceLimit(value.message)
    }
}
