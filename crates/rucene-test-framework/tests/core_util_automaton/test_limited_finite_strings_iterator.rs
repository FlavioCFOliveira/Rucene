//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestLimitedFiniteStringsIterator.java`:
//! test for `LimitedFiniteStringsIterator`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{Automata, LimitedFiniteStringsIterator, Operations};
use rucene::util::fst::Util;
use rucene::util::{IntsRef, IntsRefBuilder};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestLimitedFiniteStringsIterator`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testRandomFiniteStrings", test_random_finite_strings),
        test("testInvalidLimitNegative", test_invalid_limit_negative),
        test("testInvalidLimitNull", test_invalid_limit_null),
        test("testSingleton", test_singleton),
        test("testLimit", test_limit),
        test("testSize", test_size),
    ];
    class(
        "org.apache.lucene.util.automaton.TestLimitedFiniteStringsIterator",
        methods,
    )
}

/// `getFiniteStrings(iterator)` of `TestFiniteStringsIterator` over a
/// `LimitedFiniteStringsIterator`.
fn get_finite_strings(iterator: &mut LimitedFiniteStringsIterator<'_>) -> Result<Vec<IntsRef>> {
    let mut result = Vec::new();
    while let Some(finite_string) = iterator.next()? {
        result.push(IntsRef::deep_copy_of(&finite_string));
    }
    Ok(result)
}

fn test_random_finite_strings() -> Result<()> {
    // Just makes sure we can run on any random finite
    // automaton:
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let a = AutomatonTestUtil::random_automaton(&*random());
        // Must pass a limit because the random automaton
        // can accept MANY strings:
        let outcome =
            LimitedFiniteStringsIterator::new(&a, TestUtil::next_int(&*random(), 1, 1000))
                .and_then(|mut it| get_finite_strings(&mut it));
        match outcome {
            // NOTE: cannot do this, because the method is not
            // guaranteed to detect cycles when you have a limit
            // assertTrue(AutomatonTestUtil.isFinite(a));
            Ok(_) => {}
            Err(iae) if is_instance(&iae, "java.lang.IllegalArgumentException") => {
                Assert::assert_false(AutomatonTestUtil::is_finite(&a));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn test_invalid_limit_negative() -> Result<()> {
    let a = AutomatonTestUtil::random_automaton(&*random());
    expect_throws("java.lang.IllegalArgumentException", || {
        LimitedFiniteStringsIterator::new(&a, -7)
    });
    Ok(())
}

fn test_invalid_limit_null() -> Result<()> {
    let a = AutomatonTestUtil::random_automaton(&*random());
    expect_throws("java.lang.IllegalArgumentException", || {
        LimitedFiniteStringsIterator::new(&a, 0)
    });
    Ok(())
}

fn test_singleton() -> Result<()> {
    let a = Automata::make_string("foobar");
    let actual = get_finite_strings(&mut LimitedFiniteStringsIterator::new(&a, 1)?)?;
    assert_equals(&1, &actual.len());
    let mut scratch = IntsRefBuilder::new();
    Util::to_utf32("foobar", &mut scratch);
    Assert::assert_true(actual.contains(&scratch.get()));
    Ok(())
}

fn test_limit() -> Result<()> {
    let a = Operations::union(&[Automata::make_string("foo"), Automata::make_string("bar")]);

    // Test without limit
    let mut without_limit = LimitedFiniteStringsIterator::new(&a, -1)?;
    assert_equals(&2, &get_finite_strings(&mut without_limit)?.len());

    // Test with limit
    let mut with_limit = LimitedFiniteStringsIterator::new(&a, 1)?;
    assert_equals(&1, &get_finite_strings(&mut with_limit)?.len());
    Ok(())
}

fn test_size() -> Result<()> {
    let a = Operations::union(&[Automata::make_string("foo"), Automata::make_string("bar")]);
    let mut iterator = LimitedFiniteStringsIterator::new(&a, -1)?;
    let actual = get_finite_strings(&mut iterator)?;
    assert_equals(&2, &actual.len());
    assert_equals(&2, &iterator.size());
    Ok(())
}
