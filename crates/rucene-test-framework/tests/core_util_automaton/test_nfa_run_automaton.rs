//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestNFARunAutomaton.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::random::ints_sized_range_dyn;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::automaton_query::AutomatonQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::util::automaton::{
    Automata, Automaton, NFARunAutomaton, Operations, RegExp, Transition, TransitionAccessor,
    DEFAULT_DETERMINIZE_WORK_LIMIT, MAX_CODE_POINT,
};
use rucene::util::{Accountable, BytesRef, IntsRef};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::automaton::automaton_test_util::{
    AutomatonTestUtil, RandomAcceptedStrings,
};
use rucene_test_framework::util::ram_usage_tester::{ModelBuilder, RamUsageTester};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    arrays_to_string, assert_equals, assert_equals_delta, assert_equals_msg, assert_not_null,
    class, det, random, test, try_catch, JavaHashSet,
};

/// `TestNFARunAutomaton`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testRamUsageEstimation", test_ram_usage_estimation),
        test("testWithRandomRegex", test_with_random_regex),
        test("testRandomAccessTransition", test_random_access_transition),
        test("testRandomAutomatonQuery", test_random_automaton_query),
    ];
    class(
        "org.apache.lucene.util.automaton.TestNFARunAutomaton",
        methods,
    )
}

const FIELD: &str = "field";

fn test_ram_usage_estimation() -> Result<()> {
    let reg_exp = RegExp::with_flags(&AutomatonTestUtil::random_regexp(&*random()), RegExp::NONE)?;
    let nfa = reg_exp.to_automaton()?;
    let run_automaton = NFARunAutomaton::from_automaton(nfa);
    let estimation = run_automaton.ram_bytes_used();
    let actual = RamUsageTester::ram_used(&*ModelBuilder::new().nfa_run_automaton(&run_automaton));
    assert_equals_delta(actual as f64, estimation as f64, actual as f64 * 0.3);
    Ok(())
}

fn test_with_random_regex() -> Result<()> {
    let mut i = 0;
    while i < 100 {
        let reg_exp =
            RegExp::with_flags(&AutomatonTestUtil::random_regexp(&*random()), RegExp::NONE)?;
        let nfa = reg_exp.to_automaton()?;
        if nfa.is_deterministic() {
            // i--; continue;
            continue;
        }
        let dfa = det(Operations::determinize(
            &nfa,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        let candidate = NFARunAutomaton::from_automaton(nfa);
        let random_string_gen = match try_catch("java.lang.IllegalArgumentException", || {
            RandomAcceptedStrings::new(&dfa)
        }) {
            Ok(g) => g,
            // sometimes the automaton accept nothing and throw this exception
            Err(_) => continue,
        };

        for _round in 0..20 {
            // test order of accepted strings and random (likely rejected) strings alternatively to make
            // sure caching system works correctly
            if random().next_boolean() {
                test_accepted_string(&reg_exp, &random_string_gen, &candidate, 10);
                test_random_string(&reg_exp, &dfa, &candidate, 10);
            } else {
                test_random_string(&reg_exp, &dfa, &candidate, 10);
                test_accepted_string(&reg_exp, &random_string_gen, &candidate, 10);
            }
        }
        i += 1;
    }
    Ok(())
}

fn test_random_access_transition() -> Result<()> {
    let mut nfa = RegExp::with_flags(&AutomatonTestUtil::random_regexp(&*random()), RegExp::NONE)?
        .to_automaton()?;
    while nfa.is_deterministic() {
        nfa = RegExp::with_flags(&AutomatonTestUtil::random_regexp(&*random()), RegExp::NONE)?
            .to_automaton()?;
    }
    let run_automaton1 = NFARunAutomaton::from_automaton(nfa.clone());
    let run_automaton2 = NFARunAutomaton::from_automaton(nfa);
    assert_random_access_transition(&run_automaton1, &run_automaton2, 0, &mut HashSet::new());
    Ok(())
}

/// `assertRandomAccessTransition(automaton1, automaton2, state, visited)`.
fn assert_random_access_transition(
    automaton1: &NFARunAutomaton,
    automaton2: &NFARunAutomaton,
    state: i32,
    visited: &mut HashSet<i32>,
) {
    if visited.contains(&state) {
        return;
    }
    visited.insert(state);

    let mut t1 = Transition::new();
    let mut t2 = Transition::new();
    automaton1.init_transition(state, &mut t1);
    if random().next_boolean() {
        // init is not really necessary for t2
        automaton2.init_transition(state, &mut t2);
    }
    let num_states = automaton2.get_num_transitions(state);
    for i in 0..num_states {
        automaton1.get_next_transition(&mut t1);
        automaton2.get_transition(state, i, &mut t2);
        assert_equals(&t1.to_string(), &t2.to_string());
        assert_random_access_transition(automaton1, automaton2, t1.dest, visited);
    }
}

fn test_random_automaton_query() -> Result<()> {
    let doc_num = 50;
    let automaton_num = 50;
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    let mut vocab: JavaHashSet<String> = JavaHashSet::new();
    let mut per_doc_vocab: JavaHashSet<String> = JavaHashSet::new();
    for _ in 0..doc_num {
        per_doc_vocab.clear();
        let term_num = random().next_int_bound(20) + 30;
        while per_doc_vocab.size() < term_num as usize {
            let mut random_string;
            loop {
                random_string = TestUtil::random_unicode_string(&*random());
                if !random_string.is_empty() {
                    break;
                }
            }
            per_doc_vocab.add(random_string.clone());
            vocab.add(random_string);
        }
        let mut document = Document::new();
        let text = per_doc_vocab
            .iter()
            .fold(String::new(), |s1, s2| format!("{s1} {s2}"));
        document.add(Box::new(LuceneTestCase::new_text_field(
            FIELD,
            &text,
            Store::NO,
        )));
        writer.add_document(&document)?;
    }
    writer.commit()?;
    let reader = directory_reader::open(directory.as_directory())?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    let mut foreign_vocab: JavaHashSet<String> = JavaHashSet::new();
    while foreign_vocab.size() < vocab.size() {
        let mut random_string;
        loop {
            random_string = TestUtil::random_unicode_string(&*random());
            if !random_string.is_empty() {
                break;
            }
        }
        foreign_vocab.add(random_string);
    }

    let vocab_list = vocab.to_vec();
    let foreign_vocab_list = foreign_vocab.to_vec();

    let mut per_query_vocab: JavaHashSet<String> = JavaHashSet::new();

    let mut i = 0;
    while i < automaton_num {
        per_query_vocab.clear();
        let term_num = random().next_int_bound(40) + 30;
        while per_query_vocab.size() < term_num as usize {
            if random().next_boolean() {
                per_query_vocab.add(
                    vocab_list[random().next_int_bound(vocab_list.len() as i32) as usize].clone(),
                );
            } else {
                per_query_vocab.add(
                    foreign_vocab_list
                        [random().next_int_bound(foreign_vocab_list.len() as i32) as usize]
                        .clone(),
                );
            }
        }
        let mut a: Option<Automaton> = None;
        for term in per_query_vocab.iter() {
            a = Some(match a {
                None => Automata::make_string(term),
                Some(a) => Operations::union(&[a, Automata::make_string(term)]),
            });
        }
        let a = a.expect("at least 30 terms");
        if a.is_deterministic() {
            // i--; continue;
            continue;
        }
        let dfa_query: Arc<dyn Query> = Arc::new(AutomatonQuery::new(
            Term::new(FIELD, BytesRef::default()),
            det(Operations::determinize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT)),
        )?);
        let nfa_query = Arc::new(AutomatonQuery::new(
            Term::new(FIELD, BytesRef::default()),
            a,
        )?);
        assert_not_null(&nfa_query.get_compiled().nfa_run_automaton);
        assert_equals(
            &searcher.count(dfa_query)?,
            &searcher.count(nfa_query as Arc<dyn Query>)?,
        );
        i += 1;
    }
    reader.close()?;
    writer.close()?;
    directory.close()?;
    Ok(())
}

/// `testAcceptedString(regExp, randomStringGen, candidate, repeat)`.
fn test_accepted_string(
    reg_exp: &RegExp,
    random_string_gen: &RandomAcceptedStrings<'_>,
    candidate: &NFARunAutomaton,
    repeat: i32,
) {
    for _ in 0..repeat {
        let accepted_string = random_string_gen.get_random_accepted_string(&*random());
        Assert::assert_true_msg(
            &format!(
                "regExp: {reg_exp} testString: {}",
                arrays_to_string(&accepted_string)
            ),
            candidate.run(&accepted_string),
        );
    }
}

/// `testRandomString(regExp, dfa, candidate, repeat)`.
fn test_random_string(reg_exp: &RegExp, dfa: &Automaton, candidate: &NFARunAutomaton, repeat: i32) {
    for _ in 0..repeat {
        let size = random().next_int_bound(50);
        let r = random();
        let random_string: Vec<i32> =
            ints_sized_range_dyn(&*r, size as i64, 0, MAX_CODE_POINT).collect();
        assert_equals_msg(
            Some(&format!(
                "regExp: {reg_exp} testString: {}",
                arrays_to_string(&random_string)
            )),
            &Operations::run_ints(
                dfa,
                &IntsRef::with_offset_length(random_string.clone(), 0, random_string.len()),
            ),
            &candidate.run(&random_string),
        );
    }
}
