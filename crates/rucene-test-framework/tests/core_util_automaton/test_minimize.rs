//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestMinimize.java`:
//! builds some randomish NFA/DFA and minimizes them.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{Operations, RegExp};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, det, random, test};
use crate::minimization_operations::MinimizationOperations;

/// `TestMinimize`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testBasic", test_basic),
        test("testAgainstBrzozowski", test_against_brzozowski),
        test("testMinimizeHuge", test_minimize_huge).annotate(Nightly),
    ];
    class("org.apache.lucene.util.automaton.TestMinimize", methods)
}

/// `testBasic()`: the minimal and non-minimal are compared to ensure they
/// are the same.
fn test_basic() -> Result<()> {
    let num = LuceneTestCase::at_least(200);
    for _ in 0..num {
        let a = AutomatonTestUtil::random_automaton(&*random());
        let la = det(Operations::determinize(
            &Operations::remove_dead_states(&a),
            i32::MAX,
        ));
        let lb = MinimizationOperations::minimize(&a, i32::MAX);
        Assert::assert_true(AutomatonTestUtil::same_language(&la, &lb));
    }
    Ok(())
}

/// `testAgainstBrzozowski()`: compare minimized against minimized with a
/// slower, simple impl. We check not only that they are the same, but that
/// #states/#transitions are the same.
fn test_against_brzozowski() -> Result<()> {
    let num = LuceneTestCase::at_least(200);
    for _ in 0..num {
        let a = AutomatonTestUtil::random_automaton(&*random());
        let a = AutomatonTestUtil::minimize_simple(&a);
        let b = MinimizationOperations::minimize(&a, i32::MAX);
        Assert::assert_true(AutomatonTestUtil::same_language(&a, &b));
        assert_equals(&a.get_num_states(), &b.get_num_states());
        let num_states = a.get_num_states();

        let mut sum1 = 0;
        for s in 0..num_states {
            sum1 += a.get_num_transitions(s);
        }
        let mut sum2 = 0;
        for s in 0..num_states {
            sum2 += b.get_num_transitions(s);
        }

        assert_equals(&sum1, &sum2);
    }
    Ok(())
}

/// `testMinimizeHuge()` (`@Nightly`): n^2 space usage in Hopcroft
/// minimization?
fn test_minimize_huge() -> Result<()> {
    let a = RegExp::with_flags("+-*(A|.....|BC)*]", RegExp::NONE)?.to_automaton()?;
    let b = MinimizationOperations::minimize(&a, 1000000);
    Assert::assert_true(b.is_deterministic());
    Ok(())
}
