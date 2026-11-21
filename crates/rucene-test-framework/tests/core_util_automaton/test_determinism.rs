//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestDeterminism.java`:
//! not completely thorough, but tries to test determinism correctness
//! somewhat randomly.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{
    Automata, Automaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, det, random, test};

/// `TestDeterminism`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testRegexps", test_regexps),
        test("testAgainstSimple", test_against_simple),
    ];
    class("org.apache.lucene.util.automaton.TestDeterminism", methods)
}

/// `testRegexps()`: test a bunch of random regular expressions.
fn test_regexps() -> Result<()> {
    let num = LuceneTestCase::at_least(500);
    for _ in 0..num {
        assert_automaton(
            RegExp::with_flags(&AutomatonTestUtil::random_regexp(&*random()), RegExp::NONE)?
                .to_automaton()?,
        );
    }
    Ok(())
}

/// `testAgainstSimple()`: test against a simple, unoptimized det.
fn test_against_simple() -> Result<()> {
    let num = LuceneTestCase::at_least(200);
    for _ in 0..num {
        let a = AutomatonTestUtil::random_automaton(&*random());
        let a = AutomatonTestUtil::determinize_simple(&a);
        let b = det(Operations::determinize(&a, i32::MAX));
        // TODO: more verifications possible?
        Assert::assert_true(AutomatonTestUtil::same_language(&a, &b));
    }
    Ok(())
}

/// `assertAutomaton(a)`.
fn assert_automaton(a: Automaton) {
    let a = det(Operations::determinize(
        &Operations::remove_dead_states(&a),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));

    // complement(complement(a)) = a
    let mut equivalent = det(Operations::complement(
        &det(Operations::complement(&a, DEFAULT_DETERMINIZE_WORK_LIMIT)),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    Assert::assert_true(AutomatonTestUtil::same_language(&a, &equivalent));

    // a union a = a
    equivalent = det(Operations::determinize(
        &Operations::remove_dead_states(&Operations::union(&[a.clone(), a.clone()])),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    Assert::assert_true(AutomatonTestUtil::same_language(&a, &equivalent));

    // a intersect a = a
    equivalent = det(Operations::determinize(
        &Operations::remove_dead_states(&Operations::intersection(&a, &a)),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    Assert::assert_true(AutomatonTestUtil::same_language(&a, &equivalent));

    // a minus a = empty
    let empty = det(Operations::minus(&a, &a, DEFAULT_DETERMINIZE_WORK_LIMIT));
    Assert::assert_true(Operations::is_empty(&empty));

    // as long as don't accept the empty string
    // then optional(a) - empty = a
    if !Operations::run(&a, "") {
        // System.out.println("test " + a);
        let optional = Operations::optional(&a);
        // System.out.println("optional " + optional);
        equivalent = det(Operations::minus(
            &optional,
            &Automata::make_empty_string(),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        // System.out.println("equiv " + equivalent);
        Assert::assert_true(AutomatonTestUtil::same_language(&a, &equivalent));
    }
}
