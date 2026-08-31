//! Building the Levenshtein automata of a fuzzy query, ported from
//! `org.apache.lucene.search.FuzzyAutomatonBuilder`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::util::automaton::{CompiledAutomaton, LevenshteinAutomata, MAXIMUM_SUPPORTED_DISTANCE};
use crate::util::UnicodeUtil;

/// Builds a set of [`CompiledAutomaton`] for fuzzy matching on a given term,
/// with a specified maximum edit distance, fixed prefix and whether or not to
/// allow transpositions.
///
/// Equivalent to the package-private
/// `org.apache.lucene.search.FuzzyAutomatonBuilder`; it is public here because
/// Rust has no package visibility.
pub struct FuzzyAutomatonBuilder {
    term: String,
    max_edits: i32,
    lev_builder: LevenshteinAutomata,
    prefix: String,
    term_length: i32,
}

impl std::fmt::Debug for FuzzyAutomatonBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FuzzyAutomatonBuilder")
            .field("term", &self.term)
            .field("max_edits", &self.max_edits)
            .field("prefix", &self.prefix)
            .field("term_length", &self.term_length)
            .finish_non_exhaustive()
    }
}

impl FuzzyAutomatonBuilder {
    /// Prepares the Levenshtein builder for `term`.
    ///
    /// Equivalent to
    /// `FuzzyAutomatonBuilder(String, int, int, boolean)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `max_edits` is outside
    /// `0..=`[`MAXIMUM_SUPPORTED_DISTANCE`] or `prefix_length` is negative,
    /// which are the two `IllegalArgumentException`s Java throws, and
    /// propagates any error the Levenshtein builder raises.
    pub fn new(
        term: &str,
        max_edits: i32,
        prefix_length: i32,
        transpositions: bool,
    ) -> Result<Self> {
        if !(0..=MAXIMUM_SUPPORTED_DISTANCE).contains(&max_edits) {
            return Err(LuceneError::IllegalArgument(format!(
                "max edits must be 0..{MAXIMUM_SUPPORTED_DISTANCE}, inclusive; got: {max_edits}"
            )));
        }
        if prefix_length < 0 {
            return Err(LuceneError::IllegalArgument(
                "prefixLength cannot be less than 0".to_string(),
            ));
        }
        let code_points: Vec<i32> = term.chars().map(|c| c as i32).collect();
        let term_length = code_points.len() as i32;
        let prefix_length = (prefix_length as usize).min(code_points.len());
        let suffix = code_points[prefix_length..].to_vec();
        // `Character.MAX_CODE_POINT`.
        let lev_builder = LevenshteinAutomata::new(suffix, 0x10FFFF, transpositions)?;
        let prefix = UnicodeUtil::new_string(&code_points, 0, prefix_length)?;
        Ok(Self {
            term: term.to_string(),
            max_edits,
            lev_builder,
            prefix,
            term_length,
        })
    }

    /// Builds one compiled automaton per edit distance in `0..=max_edits`.
    ///
    /// Equivalent to `FuzzyAutomatonBuilder.buildAutomatonSet()`.
    ///
    /// # Errors
    ///
    /// Returns the "term too complex" error Java reports as a
    /// `FuzzyTermsEnum.FuzzyTermsException`.
    pub fn build_automaton_set(&self) -> Result<Vec<CompiledAutomaton>> {
        let mut compiled = Vec::with_capacity(self.max_edits as usize + 1);
        for i in 0..=self.max_edits {
            compiled.push(self.compile(i)?);
        }
        Ok(compiled)
    }

    /// Builds the compiled automaton for the maximum edit distance.
    ///
    /// Equivalent to `FuzzyAutomatonBuilder.buildMaxEditAutomaton()`.
    ///
    /// # Errors
    ///
    /// Returns the "term too complex" error Java reports as a
    /// `FuzzyTermsEnum.FuzzyTermsException`.
    pub fn build_max_edit_automaton(&self) -> Result<CompiledAutomaton> {
        self.compile(self.max_edits)
    }

    fn compile(&self, n: i32) -> Result<CompiledAutomaton> {
        let automaton = self
            .lev_builder
            .to_automaton_with_prefix(n, &self.prefix)
            .map_err(|e| self.too_complex(&e))?
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "max edits must be 0..{MAXIMUM_SUPPORTED_DISTANCE}, inclusive; got: {n}"
                ))
            })?;
        CompiledAutomaton::new(automaton, true, false, false).map_err(|e| self.too_complex(&e))
    }

    /// Equivalent to wrapping a `TooComplexToDeterminizeException` in a
    /// `FuzzyTermsEnum.FuzzyTermsException`, which reports that there was an
    /// issue creating a fuzzy query for the term. It typically occurs with
    /// terms longer than 220 UTF-8 characters, but is also possible with
    /// shorter terms consisting of UTF-32 code points.
    fn too_complex(&self, cause: &LuceneError) -> LuceneError {
        LuceneError::ResourceLimit(format!("Term too complex: {} ({cause})", self.term))
    }

    /// Returns the number of code points of the term.
    ///
    /// Equivalent to `FuzzyAutomatonBuilder.getTermLength()`.
    pub fn get_term_length(&self) -> i32 {
        self.term_length
    }
}
