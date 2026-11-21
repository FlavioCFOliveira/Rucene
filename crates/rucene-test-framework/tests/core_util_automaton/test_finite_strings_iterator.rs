//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestFiniteStringsIterator.java`:
//! test for `FiniteStringsIterator`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{
    Automata, Automaton, FiniteStringsIterator, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene::util::fst::Util;
use rucene::util::{BytesRef, IntsRef, IntsRefBuilder};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, det, random, test, verbose, JavaHashSet};
use crate::minimization_operations::MinimizationOperations;
use crate::test_operations;

/// `TestFiniteStringsIterator`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testRandomFiniteStrings1", test_random_finite_strings1),
        test("testFiniteStringsBasic", test_finite_strings_basic),
        test("testFiniteStringsEatsStack", test_finite_strings_eats_stack),
        test("testWithCycle", test_with_cycle),
        test("testSingletonNoLimit", test_singleton_no_limit),
        test("testShortAccept", test_short_accept),
        test("testSingleString", test_single_string),
    ];
    class(
        "org.apache.lucene.util.automaton.TestFiniteStringsIterator",
        methods,
    )
}

fn test_random_finite_strings1() -> Result<()> {
    let num_strings = LuceneTestCase::at_least(100);
    if verbose() {
        println!("TEST: numStrings={num_strings}");
    }

    let mut strings: JavaHashSet<IntsRef> = JavaHashSet::new();
    let mut automata = Vec::new();
    let mut scratch = IntsRefBuilder::new();
    for _ in 0..num_strings {
        let s = TestUtil::random_simple_string_between(&*random(), 1, 200);
        Util::to_utf32(&s, &mut scratch);
        if strings.add(scratch.to_ints_ref()) {
            automata.push(Automata::make_string(&s));
            if verbose() {
                println!("  add string={s}");
            }
        }
    }

    // TODO: we could sometimes use
    // DaciukMihovAutomatonBuilder here

    // TODO: what other random things can we do here...
    let mut a = Operations::union(&automata);
    if random().next_boolean() {
        a = MinimizationOperations::minimize(&a, 1000000);
        if verbose() {
            println!("TEST: a.minimize numStates={}", a.get_num_states());
        }
    } else if random().next_boolean() {
        if verbose() {
            println!("TEST: a.determinize");
        }
        a = det(Operations::determinize(&a, 1000000));
    } else if random().next_boolean() {
        if verbose() {
            println!("TEST: a.removeDeadStates");
        }
        a = Operations::remove_dead_states(&a);
    }

    let actual = get_finite_strings(FiniteStringsIterator::new(&a))?;
    assert_finite_strings_recursive(&a, &actual);

    if strings != JavaHashSet::from_collection(actual.iter().cloned()) {
        println!(
            "strings.size()={} actual.size={}",
            strings.size(),
            actual.len()
        );
        let mut x = strings.to_vec();
        x.sort();
        let mut y = actual.clone();
        y.sort();
        let end = x.len().min(y.len());
        for i in 0..end {
            println!(
                "  i={i} string={} actual={}",
                to_string(&x[i]),
                to_string(&y[i])
            );
        }
        randomizedtesting::junit::Assert::fail("wrong strings found");
    }
    Ok(())
}

/// `testFiniteStringsBasic()`: basic test for `getFiniteStrings`.
fn test_finite_strings_basic() -> Result<()> {
    let a = Operations::union(&[Automata::make_string("dog"), Automata::make_string("duck")]);
    let a = MinimizationOperations::minimize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT);
    let actual = get_finite_strings(FiniteStringsIterator::new(&a))?;
    assert_finite_strings_recursive(&a, &actual);
    assert_equals(&2, &actual.len());
    let mut dog = IntsRefBuilder::new();
    Util::to_ints_ref(&BytesRef::from_char_sequence("dog"), &mut dog);
    Assert::assert_true(actual.contains(&dog.get()));
    let mut duck = IntsRefBuilder::new();
    Util::to_ints_ref(&BytesRef::from_char_sequence("duck"), &mut duck);
    Assert::assert_true(actual.contains(&duck.get()));
    Ok(())
}

fn test_finite_strings_eats_stack() -> Result<()> {
    let mut chars = vec![0u16; 50000];
    let len = chars.len();
    TestUtil::random_fixed_length_unicode_string(&*random(), &mut chars, 0, len);
    let big_string1 = String::from_utf16(&chars).expect("valid UTF-16");
    TestUtil::random_fixed_length_unicode_string(&*random(), &mut chars, 0, len);
    let big_string2 = String::from_utf16(&chars).expect("valid UTF-16");
    let a = Operations::union(&[
        Automata::make_string(&big_string1),
        Automata::make_string(&big_string2),
    ]);
    let actual = get_finite_strings(FiniteStringsIterator::new(&a))?;
    assert_equals(&2, &actual.len());
    let mut scratch = IntsRefBuilder::new();
    Util::to_utf32(&big_string1, &mut scratch);
    Assert::assert_true(actual.contains(&scratch.get()));
    Util::to_utf32(&big_string2, &mut scratch);
    Assert::assert_true(actual.contains(&scratch.get()));
    Ok(())
}

fn test_with_cycle() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || -> Result<()> {
        let a = RegExp::with_flags("abc.*", RegExp::NONE)?.to_automaton()?;
        let iterator = FiniteStringsIterator::new(&a);
        get_finite_strings(iterator)?;
        Ok(())
    });
    Ok(())
}

fn test_singleton_no_limit() -> Result<()> {
    let a = Automata::make_string("foobar");
    let actual = get_finite_strings(FiniteStringsIterator::new(&a))?;
    assert_equals(&1, &actual.len());
    let mut scratch = IntsRefBuilder::new();
    Util::to_utf32("foobar", &mut scratch);
    Assert::assert_true(actual.contains(&scratch.get()));
    Ok(())
}

fn test_short_accept() -> Result<()> {
    let a = Operations::union(&[Automata::make_string("x"), Automata::make_string("xy")]);
    let a = MinimizationOperations::minimize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT);
    let actual = get_finite_strings(FiniteStringsIterator::new(&a))?;
    assert_equals(&2, &actual.len());
    let mut x = IntsRefBuilder::new();
    Util::to_ints_ref(&BytesRef::from_char_sequence("x"), &mut x);
    Assert::assert_true(actual.contains(&x.get()));
    let mut xy = IntsRefBuilder::new();
    Util::to_ints_ref(&BytesRef::from_char_sequence("xy"), &mut xy);
    Assert::assert_true(actual.contains(&xy.get()));
    Ok(())
}

fn test_single_string() -> Result<()> {
    let mut a = Automaton::new();
    let start = a.create_state();
    let end = a.create_state();
    a.set_accept(end, true);
    a.add_transition_range(start, end, 'a' as i32, 'a' as i32);
    a.finish_state();
    let accepted = test_operations::get_finite_strings(&a);
    assert_equals(&1, &accepted.size());
    let mut ints_ref = IntsRefBuilder::new();
    ints_ref.append('a' as i32);
    Assert::assert_true(accepted.contains(&ints_ref.to_ints_ref()));
    Ok(())
}

/// `getFiniteStrings(iterator)`: all strings generated by the iterator.
pub fn get_finite_strings(mut iterator: FiniteStringsIterator<'_>) -> Result<Vec<IntsRef>> {
    let mut result = Vec::new();
    while let Some(finite_string) = iterator.next()? {
        result.push(IntsRef::deep_copy_of(&finite_string));
    }
    Ok(result)
}

/// `assertFiniteStringsRecursive(automaton, actual)`: check that strings
/// the automaton returns are as expected.
fn assert_finite_strings_recursive(automaton: &Automaton, actual: &[IntsRef]) {
    let expected = AutomatonTestUtil::get_finite_strings_recursive(automaton, -1);
    // Check that no string is emitted twice.
    assert_equals(&expected.len(), &actual.len());
    let expected: JavaHashSet<IntsRef> = JavaHashSet::from_collection(
        expected
            .into_iter()
            .map(|v| IntsRef::with_offset_length(v.clone(), 0, v.len())),
    );
    assert_equals(
        &expected,
        &JavaHashSet::from_collection(actual.iter().cloned()),
    );
}

/// `toString(ints)` (ascii only!).
fn to_string(ints: &IntsRef) -> String {
    let bytes: Vec<u8> = ints.slice().iter().map(|&i| i as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}
