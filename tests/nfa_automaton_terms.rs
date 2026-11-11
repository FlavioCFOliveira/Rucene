//! An automaton compiled as an NFA runs through `Terms::intersect` and
//! `AutomatonTermsEnum`, as Lucene 10.5.0 runs it (rmp task #298).
//!
//! `CompiledAutomaton` keeps a non-deterministic automaton as an
//! `NFARunAutomaton`, which Java's `FieldReader.intersect`
//! (`FieldReader.java:176-177`) and `AutomatonTermsEnum`
//! (`AutomatonTermsEnum.java:85-86`) read through `getByteRunnable()` and
//! `getTransitionAccessor()`. Every expectation is what Lucene 10.5.0 printed
//! for the same index and automata (oracle
//! `scratchpad/oracle/passI/nfa/NfaOracle.java`, output `out.txt`): the 1364
//! terms over `abcd` of one to five letters, each in its own document of a
//! `StringField` `f`, one segment.

use std::sync::Arc;

use rucene::document::{Document, Store, StringField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{AutomatonTermsEnum, Terms, TermsEnum};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::automaton::{Automaton, ByteRunnable, CompiledAutomaton};
use rucene::util::BytesRef;

/// Every term over `abcd` of one to five letters, by length then in order.
fn terms() -> Vec<String> {
    let mut out = Vec::new();
    let mut level = vec![String::new()];
    for _ in 0..5 {
        let next: Vec<String> = level
            .iter()
            .flat_map(|prefix| "abcd".chars().map(move |c| format!("{prefix}{c}")))
            .collect();
        out.extend(next.iter().cloned());
        level = next;
    }
    out
}

/// `(a|b)*abb`: state 0 loops on `a` and `b` and also steps to 1 on `a`.
fn nfa_suffix() -> Automaton {
    let mut a = Automaton::new();
    let (s0, s1, s2, s3) = (
        a.create_state(),
        a.create_state(),
        a.create_state(),
        a.create_state(),
    );
    a.add_transition_range(s0, s0, i32::from(b'a'), i32::from(b'b'));
    a.add_transition(s0, s1, i32::from(b'a'));
    a.finish_state();
    a.add_transition(s1, s2, i32::from(b'b'));
    a.finish_state();
    a.add_transition(s2, s3, i32::from(b'b'));
    a.finish_state();
    a.set_accept(s3, true);
    a.finish_state();
    a
}

/// `ab|ab(c|d)`: state 0 steps to 1 and to 2 on `a`.
fn nfa_finite() -> Automaton {
    let mut a = Automaton::new();
    let states: Vec<i32> = (0..6).map(|_| a.create_state()).collect();
    a.add_transition(states[0], states[1], i32::from(b'a'));
    a.add_transition(states[0], states[2], i32::from(b'a'));
    a.finish_state();
    a.add_transition(states[1], states[3], i32::from(b'b'));
    a.finish_state();
    a.add_transition(states[2], states[4], i32::from(b'b'));
    a.finish_state();
    a.set_accept(states[3], true);
    a.finish_state();
    a.add_transition_range(states[4], states[5], i32::from(b'c'), i32::from(b'd'));
    a.finish_state();
    a.set_accept(states[5], true);
    a.finish_state();
    a
}

fn drain(mut terms_enum: Box<dyn TermsEnum>) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(term) = terms_enum.next().unwrap() {
        out.push(String::from_utf8(term.slice().to_vec()).unwrap());
    }
    out
}

/// The terms of field `f`, with the reader they are read from: the reader
/// stays open while the terms are used, as a Java caller holds it (closing
/// it closes the files the terms read).
fn field_terms() -> (Arc<dyn directory_reader::DirectoryReader>, Box<dyn Terms>) {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(NoMergePolicy::instance()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for term in terms() {
        let mut document = Document::new();
        document.add(Box::new(StringField::new("f", term, Store::NO).unwrap()));
        writer.add_document(&document).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    let reader = directory_reader::open(dir).unwrap();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    assert_eq!(leaves.len(), 1);
    let terms = leaves[0].leaf_reader().terms("f").unwrap().unwrap();
    assert_eq!(terms.size().unwrap(), 1364, "Java: size=1364");
    (reader, terms)
}

/// Java: `nfa1 … byteRunnable=NFARunAutomaton commonSuffix=abb`, then the
/// terms of `intersect` from each start term and of `AutomatonTermsEnum`.
#[test]
fn an_nfa_with_a_loop_returns_javas_terms() {
    let automaton = nfa_suffix();
    assert!(!automaton.is_deterministic());
    let compiled = CompiledAutomaton::new(automaton, false, false, false).unwrap();
    assert!(compiled.nfa_run_automaton.is_some() && compiled.run_automaton.is_none());
    assert_eq!(
        compiled
            .common_suffix_ref
            .as_ref()
            .map(|s| s.slice().to_vec()),
        Some(b"abb".to_vec())
    );
    let (_reader, terms) = field_terms();
    let all = ["aaabb", "aabb", "ababb", "abb", "baabb", "babb", "bbabb"];
    for (start, expected) in [
        (None, &all[..]),
        (Some("ab"), &all[2..]),
        (Some("bab"), &all[5..]),
        (Some("aab"), &all[1..]),
        (Some("abca"), &all[4..]),
    ] {
        let start_term = start.map(|s| BytesRef::new(s.as_bytes().to_vec()));
        assert_eq!(
            drain(terms.intersect(&compiled, start_term.as_ref()).unwrap()),
            expected,
            "intersect from {start:?}"
        );
    }
    assert_eq!(
        drain(AutomatonTermsEnum::new(terms.iterator().unwrap(), compiled).unwrap()),
        all
    );
}

/// Java: `nfa2 … commonSuffix=null`. The start terms `bab` and `aab` are left
/// out: Java's `IntersectTermsEnum.seekToStartTerm` steps the
/// `NFARunAutomaton` from its dead state and throws
/// `ArrayIndexOutOfBoundsException: Index -1 out of bounds for length 10`.
#[test]
fn a_finite_nfa_returns_javas_terms() {
    let automaton = nfa_finite();
    assert!(!automaton.is_deterministic());
    let compiled = CompiledAutomaton::new(automaton, true, false, false).unwrap();
    assert!(compiled.nfa_run_automaton.is_some() && compiled.common_suffix_ref.is_none());
    let (_reader, terms) = field_terms();
    for (start, expected) in [
        (None, &["ab", "abc", "abd"][..]),
        (Some("ab"), &["abc", "abd"][..]),
        (Some("abca"), &["abd"][..]),
    ] {
        let start_term = start.map(|s| BytesRef::new(s.as_bytes().to_vec()));
        assert_eq!(
            drain(terms.intersect(&compiled, start_term.as_ref()).unwrap()),
            expected,
            "intersect from {start:?}"
        );
    }
    assert_eq!(
        drain(AutomatonTermsEnum::new(terms.iterator().unwrap(), compiled).unwrap()),
        ["ab", "abc", "abd"]
    );
}

/// Java: `nfa2 intersect start=bab -> THROW
/// java.lang.ArrayIndexOutOfBoundsException: Index -1 out of bounds for length
/// 10`, and likewise from `aab`: `IntersectTermsEnum.seekToStartTerm` reaches a
/// block whose prefix walks the `NFARunAutomaton` into its dead state, and
/// `getState` steps from it (`IntersectTermsEnum.java:229-238`; with
/// assertions on, its `assert state != -1` fails first). The port fails at
/// the same point: its `debug_assert!` there, or the index of the dead state.
#[test]
fn a_start_term_past_the_dead_state_fails_as_java_does() {
    let compiled = CompiledAutomaton::new(nfa_finite(), true, false, false).unwrap();
    let (_reader, terms) = field_terms();
    for start in ["bab", "aab"] {
        let start_term = BytesRef::new(start.as_bytes().to_vec());
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            terms.intersect(&compiled, Some(&start_term)).map(drain)
        }));
        assert!(
            outcome.is_err(),
            "intersect from {start:?} returned {outcome:?}"
        );
    }
}

/// Java's enums share the query's one `NFARunAutomaton`
/// (`CompiledAutomaton.java:88,257,486-492`): the states one enum determinizes
/// are numbered in the query's automaton, where the next enum finds them.
#[test]
fn enums_of_one_query_share_its_nfa_cache() {
    let compiled = CompiledAutomaton::new(nfa_suffix(), false, false, false).unwrap();
    let nfa = Arc::clone(compiled.nfa_run_automaton.as_ref().unwrap());
    assert!(Arc::ptr_eq(
        compiled.clone().nfa_run_automaton.as_ref().unwrap(),
        &nfa
    ));
    let initial = nfa.size();
    let (_reader, terms) = field_terms();
    let first = drain(terms.intersect(&compiled, None).unwrap());
    let after_intersect = nfa.size();
    assert!(
        after_intersect > initial,
        "the enum determinized states in the query's automaton ({initial} -> {after_intersect})"
    );
    let second =
        drain(AutomatonTermsEnum::new(terms.iterator().unwrap(), compiled.clone()).unwrap());
    assert_eq!(first, second);
    assert!(nfa.size() >= after_intersect);
}
