//! Finite-state automata, ported from `org.apache.lucene.util.automaton`.
//!
//! The package builds, combines, determinizes and runs finite automata over Unicode
//! code points or over UTF-8 bytes. It is what backs `RegexpQuery`, `WildcardQuery`,
//! `FuzzyQuery` and `PrefixQuery`: the byte ranges produced here decide which terms
//! of the on-disk terms dictionary a query matches, so the construction is
//! reproduced from Apache Lucene Core 10.5.0 without simplification.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`Automata`] | `Automata` |
//! | [`Automaton`] | `Automaton` |
//! | [`AutomatonBuilder`] | `Automaton.Builder` |
//! | [`AutomatonProvider`] | `AutomatonProvider` |
//! | [`ByteRunAutomaton`] | `ByteRunAutomaton` |
//! | [`ByteRunnable`] | `ByteRunnable` |
//! | [`CaseFolding`] | `CaseFolding` |
//! | [`CharacterRunAutomaton`] | `CharacterRunAutomaton` |
//! | [`CompiledAutomaton`] | `CompiledAutomaton` |
//! | [`AutomatonType`] | `CompiledAutomaton.AUTOMATON_TYPE` |
//! | [`FiniteStringsIterator`] | `FiniteStringsIterator` |
//! | [`FrozenIntSet`] | `FrozenIntSet` |
//! | [`IntSet`] | `IntSet` |
//! | [`Lev1ParametricDescription`] | `Lev1ParametricDescription` |
//! | [`Lev1TParametricDescription`] | `Lev1TParametricDescription` |
//! | [`Lev2ParametricDescription`] | `Lev2ParametricDescription` |
//! | [`Lev2TParametricDescription`] | `Lev2TParametricDescription` |
//! | [`LevenshteinAutomata`] | `LevenshteinAutomata` |
//! | [`ParametricDescription`] | `LevenshteinAutomata.ParametricDescription` |
//! | [`LimitedFiniteStringsIterator`] | `LimitedFiniteStringsIterator` |
//! | [`NFARunAutomaton`] | `NFARunAutomaton` |
//! | [`Operations`] | `Operations` |
//! | [`RegExp`] | `RegExp` |
//! | [`Kind`] | `RegExp.Kind` |
//! | [`RunAutomaton`] | `RunAutomaton` |
//! | [`StatePair`] | `StatePair` |
//! | [`StateSet`] | `StateSet` |
//! | [`StringsToAutomaton`] | `StringsToAutomaton` |
//! | [`TooComplexToDeterminizeException`] | `TooComplexToDeterminizeException` |
//! | [`Transition`] | `Transition` |
//! | [`TransitionAccessor`] | `TransitionAccessor` |
//! | [`UTF32ToUTF8`] | `UTF32ToUTF8` |
//!
//! # Building and running an automaton
//!
//! [`Automaton`] is built state by state; all transitions leaving a state must be
//! added before moving on to the next one, or [`AutomatonBuilder`] can be used when
//! that is too restrictive. [`Operations`] combines automata (union, intersection,
//! concatenation, repetition, complement) and determinizes them under an explicit
//! work limit. Running an automaton over bytes goes through [`ByteRunAutomaton`], and
//! over code points through [`CharacterRunAutomaton`]; [`CompiledAutomaton`] wraps
//! either together with the simplifications the terms dictionary relies on.

#![deny(unsafe_code)]

pub mod automata;
// `util::automaton::automaton` mirrors Lucene's `automaton/Automaton.java`, the way
// `util::fst::fst` already mirrors `fst/FST.java`.
#[allow(clippy::module_inception)]
pub mod automaton;
pub mod automaton_provider;
pub mod byte_run_automaton;
pub mod case_folding;
pub mod character_run_automaton;
pub mod compiled_automaton;
pub mod finite_strings_iterator;
pub mod int_set;
pub mod lev1_parametric_description;
pub mod lev1t_parametric_description;
pub mod lev2_parametric_description;
pub mod lev2t_parametric_description;
pub mod levenshtein_automata;
pub mod limited_finite_strings_iterator;
pub mod nfa_run_automaton;
pub mod operations;
pub mod reg_exp;
pub mod run_automaton;
pub mod state_pair;
pub mod strings_to_automaton;
pub mod too_complex_to_determinize_exception;
pub mod utf32_to_utf8;

pub use automata::{Automata, MAX_STRING_UNION_TERM_LENGTH};
pub use automaton::{
    Automaton, AutomatonBuilder, Transition, TransitionAccessor, MAX_CODE_POINT, MIN_CODE_POINT,
};
pub use automaton_provider::AutomatonProvider;
pub use byte_run_automaton::ByteRunAutomaton;
pub use case_folding::CaseFolding;
pub use character_run_automaton::CharacterRunAutomaton;
pub use compiled_automaton::{AutomatonType, CompiledAutomaton};
pub use finite_strings_iterator::FiniteStringsIterator;
pub use int_set::{FrozenIntSet, IntSet, StateSet};
pub use lev1_parametric_description::Lev1ParametricDescription;
pub use lev1t_parametric_description::Lev1TParametricDescription;
pub use lev2_parametric_description::Lev2ParametricDescription;
pub use lev2t_parametric_description::Lev2TParametricDescription;
pub use levenshtein_automata::{
    unpack, LevenshteinAutomata, ParametricDescription, MAXIMUM_SUPPORTED_DISTANCE,
};
pub use limited_finite_strings_iterator::LimitedFiniteStringsIterator;
pub use nfa_run_automaton::NFARunAutomaton;
pub use operations::{DeterminizeResult, Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
pub use reg_exp::{Kind, Kind as RegExpKind, RegExp};
pub use run_automaton::{ByteRunnable, RunAutomaton};
pub use state_pair::StatePair;
pub use strings_to_automaton::StringsToAutomaton;
pub use too_complex_to_determinize_exception::TooComplexToDeterminizeException;
pub use utf32_to_utf8::UTF32ToUTF8;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::postings::{EmptyPostingsEnum, PostingsEnum, Terms, TermsEnum};
    use crate::util::BytesRef;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn empty_automaton_accepts_nothing() {
        let a = Automata::make_empty();
        assert!(Operations::is_empty(&a));
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
        let a = Automata::make_binary(&BytesRef::new(b"lucene".to_vec()));
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
        let a = Automata::make_binary_interval(Some(&min), true, Some(&max), true).unwrap();
        let run = ByteRunAutomaton::new(a, true).unwrap();
        assert!(run.run(b"aa"));
        assert!(run.run(b"mn"));
        assert!(run.run(b"zz"));
        assert!(!run.run(b"a"));
        assert!(!run.run(b"zzz"));
    }

    #[test]
    fn compiled_automaton_classifies_simple_types() {
        let empty = Automata::make_empty();
        let compiled = CompiledAutomaton::new(empty, true, true, true).unwrap();
        assert_eq!(compiled.automaton_type, AutomatonType::None);

        let any = Automata::make_any_string();
        let compiled = CompiledAutomaton::new(any, false, true, false).unwrap();
        assert_eq!(compiled.automaton_type, AutomatonType::All);

        let single = Automata::make_binary(&BytesRef::new(b"cat".to_vec()));
        let compiled = CompiledAutomaton::new(single, true, true, true).unwrap();
        assert_eq!(compiled.automaton_type, AutomatonType::Single);
        assert_eq!(compiled.term, Some(BytesRef::new(b"cat".to_vec())));

        // A non-trivial range compiles to NORMAL.
        let min = BytesRef::new(b"a".to_vec());
        let max = BytesRef::new(b"z".to_vec());
        let range = Automata::make_binary_interval(Some(&min), true, Some(&max), true).unwrap();
        let compiled = CompiledAutomaton::new(range, true, true, true).unwrap();
        assert_eq!(compiled.automaton_type, AutomatonType::Normal);
        assert!(compiled.run_automaton.is_some());
        assert!(compiled.automaton.is_some());
    }

    #[test]
    fn compiled_automaton_run_matches_binary_interval() {
        let min = BytesRef::new(b"a".to_vec());
        let max = BytesRef::new(b"z".to_vec());
        let range = Automata::make_binary_interval(Some(&min), true, Some(&max), true).unwrap();
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
        atts: crate::util::attribute::AttributeSource,
    }

    impl ListTermsEnum {
        fn new(terms: Vec<BytesRef>) -> Self {
            Self {
                terms,
                pos: 0,
                atts: crate::util::attribute::AttributeSource::new(),
            }
        }
    }

    impl TermsEnum for ListTermsEnum {
        fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
            &mut self.atts
        }

        fn term(&self) -> crate::error::Result<BytesRef> {
            Ok(self.terms[self.pos].clone())
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> crate::error::Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(EmptyPostingsEnum::new()))
        }

        fn seek_exact(&mut self, _text: &BytesRef) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn seek_ceil(
            &mut self,
            _text: &BytesRef,
        ) -> crate::error::Result<crate::index::SeekStatus> {
            Ok(crate::index::SeekStatus::END)
        }

        fn seek_ord(&mut self, ord: i64) -> crate::error::Result<()> {
            self.pos = ord as usize;
            Ok(())
        }

        fn seek_term_state(
            &mut self,
            _text: &BytesRef,
            _state: &dyn crate::index::TermState,
        ) -> crate::error::Result<()> {
            Ok(())
        }

        fn ord(&self) -> crate::error::Result<i64> {
            Ok(self.pos as i64)
        }

        fn doc_freq(&self) -> crate::error::Result<i32> {
            Ok(0)
        }

        fn total_term_freq(&self) -> crate::error::Result<i64> {
            Ok(0)
        }

        fn impacts(
            &mut self,
            _flags: i32,
        ) -> crate::error::Result<Box<dyn crate::index::ImpactsEnum>> {
            Err(crate::error::LuceneError::UnsupportedOperation(
                "impacts not supported".to_string(),
            ))
        }

        fn term_state(&mut self) -> crate::error::Result<Box<dyn crate::index::TermState>> {
            Ok(Box::new(
                crate::codecs::term_state::BlockTermState::default(),
            ))
        }

        fn next(&mut self) -> crate::error::Result<Option<BytesRef>> {
            if self.pos >= self.terms.len() {
                Ok(None)
            } else {
                let term = self.terms[self.pos].clone();
                self.pos += 1;
                Ok(Some(term))
            }
        }
    }

    #[derive(Debug)]
    struct IntersectTermsEnum {
        atts: crate::util::attribute::AttributeSource,
    }

    impl IntersectTermsEnum {
        fn new() -> Self {
            Self {
                atts: crate::util::attribute::AttributeSource::new(),
            }
        }
    }

    impl TermsEnum for IntersectTermsEnum {
        fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
            &mut self.atts
        }

        fn term(&self) -> crate::error::Result<BytesRef> {
            Ok(BytesRef::new(b"intersect".to_vec()))
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> crate::error::Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(EmptyPostingsEnum::new()))
        }

        fn seek_exact(&mut self, _text: &BytesRef) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn seek_ceil(
            &mut self,
            _text: &BytesRef,
        ) -> crate::error::Result<crate::index::SeekStatus> {
            Ok(crate::index::SeekStatus::END)
        }

        fn seek_ord(&mut self, _ord: i64) -> crate::error::Result<()> {
            Ok(())
        }

        fn seek_term_state(
            &mut self,
            _text: &BytesRef,
            _state: &dyn crate::index::TermState,
        ) -> crate::error::Result<()> {
            Ok(())
        }

        fn ord(&self) -> crate::error::Result<i64> {
            Ok(-1)
        }

        fn doc_freq(&self) -> crate::error::Result<i32> {
            Ok(0)
        }

        fn total_term_freq(&self) -> crate::error::Result<i64> {
            Ok(0)
        }

        fn impacts(
            &mut self,
            _flags: i32,
        ) -> crate::error::Result<Box<dyn crate::index::ImpactsEnum>> {
            Err(crate::error::LuceneError::UnsupportedOperation(
                "impacts not supported".to_string(),
            ))
        }

        fn term_state(&mut self) -> crate::error::Result<Box<dyn crate::index::TermState>> {
            Ok(Box::new(
                crate::codecs::term_state::BlockTermState::default(),
            ))
        }

        fn next(&mut self) -> crate::error::Result<Option<BytesRef>> {
            Ok(None)
        }
    }

    struct StubTerms {
        terms: Vec<BytesRef>,
        intersect_called: AtomicBool,
    }

    impl Terms for StubTerms {
        fn iterator(&self) -> crate::error::Result<Box<dyn TermsEnum>> {
            Ok(Box::new(ListTermsEnum::new(self.terms.clone())))
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

        fn min(&self) -> crate::error::Result<Option<BytesRef>> {
            Ok(None)
        }

        fn max(&self) -> crate::error::Result<Option<BytesRef>> {
            Ok(None)
        }

        fn intersect(
            &self,
            _automaton: &CompiledAutomaton,
            _skip_ahead: Option<&BytesRef>,
        ) -> crate::error::Result<Box<dyn TermsEnum>> {
            self.intersect_called.store(true, Ordering::SeqCst);
            Ok(Box::new(IntersectTermsEnum::new()))
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
        let none = CompiledAutomaton::new(Automata::make_empty(), true, true, true).unwrap();
        let mut it = none.get_terms_enum(&terms).unwrap();
        assert_eq!(it.next().unwrap(), None);

        // ALL returns the full iterator positioned on the first term.
        let all = CompiledAutomaton::new(Automata::make_any_string(), false, true, false).unwrap();
        let it = all.get_terms_enum(&terms).unwrap();
        assert_eq!(it.term().unwrap(), BytesRef::new(b"alpha".to_vec()));

        // SINGLE returns a SingleTermsEnum reporting the fixed term.
        let single = CompiledAutomaton::new(
            Automata::make_binary(&BytesRef::new(b"beta".to_vec())),
            true,
            true,
            true,
        )
        .unwrap();
        let it = single.get_terms_enum(&terms).unwrap();
        assert_eq!(it.term().unwrap(), BytesRef::new(b"beta".to_vec()));

        // NORMAL delegates to Terms::intersect.
        let min = BytesRef::new(b"a".to_vec());
        let max = BytesRef::new(b"z".to_vec());
        let range = Automata::make_binary_interval(Some(&min), true, Some(&max), true).unwrap();
        let normal = CompiledAutomaton::new(range, true, true, true).unwrap();
        assert!(!terms.intersect_called.load(Ordering::SeqCst));
        let it = normal.get_terms_enum(&terms).unwrap();
        assert!(terms.intersect_called.load(Ordering::SeqCst));
        assert_eq!(it.term().unwrap(), BytesRef::new(b"intersect".to_vec()));
    }

    #[test]
    fn transition_accessor_iterates_transitions() {
        let a = Automata::make_binary(&BytesRef::new(b"ab".to_vec()));
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
        let a = Automata::make_binary(&BytesRef::new(b"x".to_vec()));
        let run = RunAutomaton::new(a, 256).unwrap();
        assert_eq!(run.size(), 2);
        assert!(run.is_accept(1));
        assert!(!run.is_accept(0));
        assert_eq!(run.step(0, b'x' as i32), 1);
        assert_eq!(run.step(0, b'y' as i32), -1);
    }
}
