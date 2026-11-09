//! Port of the tests of `lucene/core/src/test/org/apache/lucene/util/automaton/`
//! (sprint 9, task #400): `TestAutomaton`, `TestCompiledAutomaton`,
//! `TestDeterminism`, `TestDeterminizeLexicon`, `TestFiniteStringsIterator`,
//! `TestIntSet`, `TestLevenshteinAutomata`,
//! `TestLimitedFiniteStringsIterator`, `TestMinimize`,
//! `TestNFARunAutomaton`, `TestOperations`, `TestRegExp`,
//! `TestRegExpParsing`, `TestStringsToAutomaton` and `TestUTF32ToUTF8`, with
//! their package's test helper `MinimizationOperations`. Each class runs as
//! a suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit `NO_OUTPUT`
// bindings line by line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

#[path = "core_util_automaton/minimization_operations.rs"]
mod minimization_operations;
#[path = "core_util_automaton/test_automaton.rs"]
mod test_automaton;
#[path = "core_util_automaton/test_compiled_automaton.rs"]
mod test_compiled_automaton;
#[path = "core_util_automaton/test_determinism.rs"]
mod test_determinism;
#[path = "core_util_automaton/test_determinize_lexicon.rs"]
mod test_determinize_lexicon;
#[path = "core_util_automaton/test_finite_strings_iterator.rs"]
mod test_finite_strings_iterator;
#[path = "core_util_automaton/test_int_set.rs"]
mod test_int_set;
#[path = "core_util_automaton/test_levenshtein_automata.rs"]
mod test_levenshtein_automata;
#[path = "core_util_automaton/test_limited_finite_strings_iterator.rs"]
mod test_limited_finite_strings_iterator;
#[path = "core_util_automaton/test_minimize.rs"]
mod test_minimize;
#[path = "core_util_automaton/test_nfa_run_automaton.rs"]
mod test_nfa_run_automaton;
#[path = "core_util_automaton/test_operations.rs"]
mod test_operations;
#[path = "core_util_automaton/test_reg_exp.rs"]
mod test_reg_exp;
#[path = "core_util_automaton/test_reg_exp_parsing.rs"]
mod test_reg_exp_parsing;
#[path = "core_util_automaton/test_strings_to_automaton.rs"]
mod test_strings_to_automaton;
#[path = "core_util_automaton/test_utf32_to_utf8.rs"]
mod test_utf32_to_utf8;

use core_support::run;

#[test]
fn test_automaton() {
    run(test_automaton::class_def);
}

#[test]
fn test_compiled_automaton() {
    run(test_compiled_automaton::class_def);
}

#[test]
fn test_determinism() {
    run(test_determinism::class_def);
}

#[test]
fn test_determinize_lexicon() {
    run(test_determinize_lexicon::class_def);
}

#[test]
fn test_finite_strings_iterator() {
    run(test_finite_strings_iterator::class_def);
}

#[test]
fn test_int_set() {
    run(test_int_set::class_def);
}

#[test]
fn test_levenshtein_automata() {
    run(test_levenshtein_automata::class_def);
}

#[test]
fn test_limited_finite_strings_iterator() {
    run(test_limited_finite_strings_iterator::class_def);
}

#[test]
fn test_minimize() {
    run(test_minimize::class_def);
}

#[test]
fn test_nfa_run_automaton() {
    run(test_nfa_run_automaton::class_def);
}

#[test]
fn test_operations() {
    run(test_operations::class_def);
}

#[test]
fn test_reg_exp() {
    run(test_reg_exp::class_def);
}

#[test]
fn test_reg_exp_parsing() {
    run(test_reg_exp_parsing::class_def);
}

#[test]
fn test_strings_to_automaton() {
    run(test_strings_to_automaton::class_def);
}

#[test]
fn test_utf32_to_utf8() {
    run(test_utf32_to_utf8::class_def);
}
