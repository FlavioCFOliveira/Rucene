//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestCompiledAutomaton.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::util::automaton::{
    Automata, Automaton, AutomatonType, CompiledAutomaton, Operations, RegExp,
    DEFAULT_DETERMINIZE_WORK_LIMIT, MAX_CODE_POINT,
};
use rucene::util::{Accountable, BytesRef, BytesRefBuilder};
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::ram_usage_tester::{ModelBuilder, RamUsageTester};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, assert_not_null, assert_null, class,
    det, random, test, unchecked, verbose, JavaHashSet,
};

/// `TestCompiledAutomaton`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testRandom", test_random),
        test("testBasic", test_basic),
        test("testBinaryAll", test_binary_all),
        test("testUnicodeAll", test_unicode_all),
        test("testBinarySingleton", test_binary_singleton),
        test("testUnicodeSingleton", test_unicode_singleton),
        test("testRamBytesUsed", test_ram_bytes_used),
    ];
    class(
        "org.apache.lucene.util.automaton.TestCompiledAutomaton",
        methods,
    )
}

/// `build(determinizeWorkLimit, strings...)`.
fn build(_determinize_work_limit: i32, strings: &[String]) -> Result<CompiledAutomaton> {
    let mut terms: Vec<BytesRef> = Vec::new();
    for s in strings {
        terms.push(BytesRef::from_char_sequence(s));
    }
    terms.sort();
    let a = Automata::make_string_union(terms)?;
    CompiledAutomaton::new(a, true, false, false)
}

/// `testFloor(c, input, expected)`.
fn test_floor(c: &CompiledAutomaton, input: &str, expected: Option<&str>) {
    let b = BytesRef::from_char_sequence(input);
    let result = c.floor(&b, &mut BytesRefBuilder::new());
    match expected {
        None => assert_null(&result),
        Some(expected) => {
            assert_not_null(&result);
            let result = result.expect("asserted not null");
            assert_equals_msg(
                Some(&format!(
                    "actual={} vs expected={expected} (input={input})",
                    unchecked(result.utf8_to_string())
                )),
                &result,
                &BytesRef::from_char_sequence(expected),
            );
        }
    }
}

/// `testTerms(determinizeWorkLimit, terms)`.
fn test_terms(determinize_work_limit: i32, terms: &[String]) -> Result<()> {
    let c = build(determinize_work_limit, terms)?;
    let mut term_bytes: Vec<BytesRef> = terms
        .iter()
        .map(|t| BytesRef::from_char_sequence(t))
        .collect();
    term_bytes.sort();

    if verbose() {
        println!("\nTEST: terms in unicode order");
        for t in &term_bytes {
            println!("  {}", t.utf8_to_string()?);
        }
        // System.out.println(c.utf8.toDot());
    }

    for _ in 0..100 * *RANDOM_MULTIPLIER {
        let s = if random().next_int_bound(10) == 1 {
            terms[random().next_int_bound(terms.len() as i32) as usize].clone()
        } else {
            random_string()
        };
        if verbose() {
            println!("\nTEST: floor({s})");
        }
        let expected = match term_bytes.binary_search(&BytesRef::from_char_sequence(&s)) {
            Ok(_) => Some(s.clone()),
            Err(loc) => {
                // term doesn't exist
                if loc == 0 {
                    None
                } else {
                    Some(term_bytes[loc - 1].utf8_to_string()?)
                }
            }
        };
        if verbose() {
            println!("  expected={}", expected.as_deref().unwrap_or("null"));
        }
        test_floor(&c, &s, expected.as_deref());
    }
    Ok(())
}

fn test_random() -> Result<()> {
    let num_terms = LuceneTestCase::at_least(400);
    let mut terms: JavaHashSet<String> = JavaHashSet::new();
    while terms.size() != num_terms as usize {
        terms.add(random_string());
    }
    test_terms(num_terms * 100, &terms.to_vec())
}

/// `randomString()`.
fn random_string() -> String {
    // return _TestUtil.randomSimpleString(random);
    TestUtil::random_realistic_unicode_string(&*random())
}

fn test_basic() -> Result<()> {
    let c = build(
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        &["fob".to_string(), "foo".to_string(), "goo".to_string()],
    )?;
    test_floor(&c, "goo", Some("goo"));
    test_floor(&c, "ga", Some("foo"));
    test_floor(&c, "g", Some("foo"));
    test_floor(&c, "foc", Some("fob"));
    test_floor(&c, "foz", Some("foo"));
    test_floor(&c, "f", None);
    test_floor(&c, "", None);
    test_floor(&c, "aa", None);
    test_floor(&c, "zzz", Some("goo"));
    Ok(())
}

/// LUCENE-6367.
fn test_binary_all() -> Result<()> {
    let mut a = Automaton::new();
    let state = a.create_state();
    a.set_accept(state, true);
    a.add_transition_range(state, state, 0, 0xff);
    a.finish_state();

    let ca = CompiledAutomaton::new(a, false, true, true)?;
    assert_equals(&AutomatonType::All, &ca.automaton_type);
    Ok(())
}

/// LUCENE-6367.
fn test_unicode_all() -> Result<()> {
    let mut a = Automaton::new();
    let state = a.create_state();
    a.set_accept(state, true);
    a.add_transition_range(state, state, 0, MAX_CODE_POINT);
    a.finish_state();

    let ca = CompiledAutomaton::new(a, false, true, false)?;
    assert_equals(&AutomatonType::All, &ca.automaton_type);
    Ok(())
}

/// LUCENE-6367.
fn test_binary_singleton() -> Result<()> {
    // This is just ascii so we can pretend it's binary:
    let a = Automata::make_string("foobar");
    let ca = CompiledAutomaton::new(a, true, true, true)?;
    assert_equals(&AutomatonType::Single, &ca.automaton_type);
    Ok(())
}

/// LUCENE-6367.
fn test_unicode_singleton() -> Result<()> {
    let a = Automata::make_string(&TestUtil::random_realistic_unicode_string(&*random()));
    let ca = CompiledAutomaton::new(a, true, true, false)?;
    assert_equals(&AutomatonType::Single, &ca.automaton_type);
    Ok(())
}

fn test_ram_bytes_used() -> Result<()> {
    let a = det(Operations::determinize(
        &RegExp::new(".*foo.*bar.*baz.*")?.to_automaton()?,
        i32::MAX,
    ));
    let ca = CompiledAutomaton::new(a, false, true, false)?;
    assert_equals(&AutomatonType::Normal, &ca.automaton_type);
    assert_not_null(&ca.run_automaton);
    let reported = ca.ram_bytes_used();
    let actual = RamUsageTester::ram_used(&*ModelBuilder::new().compiled_automaton(&ca));
    assert_equals_delta(actual as f64, reported as f64, actual as f64 * 0.10);
    Ok(())
}
