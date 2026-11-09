//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestStringsToAutomaton.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene::util::automaton::{
    Automata, Automaton, ByteRunnable, CompiledAutomaton, FiniteStringsIterator, Operations,
    StringsToAutomaton, DEFAULT_DETERMINIZE_WORK_LIMIT, MAX_STRING_UNION_TERM_LENGTH,
};
use rucene::util::fst::Util;
use rucene::util::{BytesRef, BytesRefBuilder};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, JavaHashSet};
use crate::minimization_operations::MinimizationOperations;

/// `TestStringsToAutomaton`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testBasic", test_basic),
        test("testBasicBinary", test_basic_binary),
        test("testRandomMinimized", test_random_minimized),
        test("testRandomUnicodeOnly", test_random_unicode_only),
        test("testRandomBinary", test_random_binary),
        test("testLargeTerms", test_large_terms),
    ];
    class(
        "org.apache.lucene.util.automaton.TestStringsToAutomaton",
        methods,
    )
}

fn test_basic() -> Result<()> {
    let mut terms = basic_terms();
    terms.sort();

    let a = build(&terms, false)?;
    check_automaton(&terms, &a, false)?;
    check_minimized(&a);
    Ok(())
}

fn test_basic_binary() -> Result<()> {
    let mut terms = basic_terms();
    terms.sort();

    let a = build(&terms, true)?;
    check_automaton(&terms, &a, true)?;
    check_minimized(&a);
    Ok(())
}

fn test_random_minimized() -> Result<()> {
    let iters = if RandomizedTest::is_nightly() { 20 } else { 5 };
    for _ in 0..iters {
        let build_binary = random().next_boolean();
        let size = random().next_int_range(2, 50);
        let mut terms: JavaHashSet<BytesRef> = JavaHashSet::new();
        let mut automaton_list = Vec::with_capacity(size as usize);
        for _ in 0..size {
            if build_binary {
                let t = TestUtil::random_binary_term_with(&*random(), 8);
                terms.add(t.clone());
                automaton_list.push(Automata::make_binary(&t));
            } else {
                let s = TestUtil::random_realistic_unicode_string_max(&*random(), 8);
                terms.add(LuceneTestCase::new_bytes_ref_str(&s));
                automaton_list.push(Automata::make_string(&s));
            }
        }
        let mut sorted_terms = terms.to_vec();
        sorted_terms.sort();

        let expected = MinimizationOperations::minimize(
            &Operations::union(&automaton_list),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        );
        let actual = build(&sorted_terms, build_binary)?;
        assert_same_automaton(&expected, &actual);
    }
    Ok(())
}

fn test_random_unicode_only() -> Result<()> {
    test_random(false)
}

fn test_random_binary() -> Result<()> {
    test_random(true)
}

fn test_large_terms() -> Result<()> {
    let b10k = vec![b'a'; 10_000];
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        build(&[BytesRef::new(b10k.clone())], false)
    });
    Assert::assert_true(e.message().unwrap_or("").starts_with(&format!(
        "This builder doesn't allow terms that are larger than {MAX_STRING_UNION_TERM_LENGTH} UTF-8 bytes"
    )));

    let b1k = b10k[0..1000].to_vec();
    build(&[BytesRef::new(b1k)], false)?; // no exception
    Ok(())
}

/// `testRandom(allowBinary)`.
fn test_random(allow_binary: bool) -> Result<()> {
    let iters = if RandomizedTest::is_nightly() { 50 } else { 10 };
    for _ in 0..iters {
        let size = random().next_int_range(500, 2_000);
        let mut terms: JavaHashSet<BytesRef> = JavaHashSet::with_capacity(size as usize);
        for _ in 0..size {
            if allow_binary && random().next_int_bound(10) < 2 {
                // Sometimes random bytes term that isn't necessarily valid unicode
                terms.add(LuceneTestCase::new_bytes_ref_from(
                    &TestUtil::random_binary_term(&*random()),
                ));
            } else {
                terms.add(LuceneTestCase::new_bytes_ref_str(
                    &TestUtil::random_realistic_unicode_string(&*random()),
                ));
            }
        }

        let mut sorted = terms.to_vec();
        sorted.sort();
        let a = build(&sorted, allow_binary)?;
        check_automaton(&sorted, &a, allow_binary)?;
    }
    Ok(())
}

/// `checkAutomaton(expected, a, isBinary)`.
fn check_automaton(expected: &[BytesRef], a: &Automaton, is_binary: bool) -> Result<()> {
    let c = CompiledAutomaton::new(a.clone(), true, false, is_binary)?;
    let run_automaton = c
        .run_automaton
        .as_ref()
        .expect("a NORMAL automaton has a run automaton");

    // Make sure every expected term is accepted
    for t in expected {
        let readable = if is_binary {
            t.to_string()
        } else {
            t.utf8_to_string()?
        };
        Assert::assert_true_msg(
            &format!("{readable} should be found but wasn't"),
            run_automaton.run_range(&t.bytes, t.offset, t.length),
        );
    }

    // Make sure every term produced by the automaton is expected
    let mut scratch = BytesRefBuilder::new();
    let automaton = c.automaton.as_ref().expect("a NORMAL automaton is kept");
    let mut it = FiniteStringsIterator::new(automaton);
    while let Some(r) = it.next()? {
        let t = Util::to_bytes_ref(&r, &mut scratch)?;
        Assert::assert_true(expected.contains(&t));
    }
    Ok(())
}

/// `checkMinimized(a)`.
fn check_minimized(a: &Automaton) {
    let minimized = MinimizationOperations::minimize(a, DEFAULT_DETERMINIZE_WORK_LIMIT);
    assert_same_automaton(&minimized, a);
}

/// `assertSameAutomaton(a, b)`.
fn assert_same_automaton(a: &Automaton, b: &Automaton) {
    assert_equals(&a.get_num_states(), &b.get_num_states());
    assert_equals(
        &a.get_num_transitions_total(),
        &b.get_num_transitions_total(),
    );
    Assert::assert_true(AutomatonTestUtil::same_language(a, b));
}

/// `basicTerms()`.
fn basic_terms() -> Vec<BytesRef> {
    vec![
        LuceneTestCase::new_bytes_ref_str("dog"),
        LuceneTestCase::new_bytes_ref_str("day"),
        LuceneTestCase::new_bytes_ref_str("dad"),
        LuceneTestCase::new_bytes_ref_str("cats"),
        LuceneTestCase::new_bytes_ref_str("cat"),
    ]
}

/// `build(terms, asBinary)`: through the `Collection` overload or the
/// `BytesRefIterator` one (`TermIterator`), at random. Both overloads of
/// `StringsToAutomaton.build` are one Rust function over an iterator.
fn build(terms: &[BytesRef], as_binary: bool) -> Result<Automaton> {
    if random().next_boolean() {
        StringsToAutomaton::build(terms.to_vec(), as_binary)
    } else {
        StringsToAutomaton::build(TermIterator::new(terms), as_binary)
    }
}

/// `TermIterator implements BytesRefIterator`.
struct TermIterator<'a> {
    it: std::slice::Iter<'a, BytesRef>,
}

impl<'a> TermIterator<'a> {
    fn new(terms: &'a [BytesRef]) -> Self {
        TermIterator { it: terms.iter() }
    }
}

impl Iterator for TermIterator<'_> {
    type Item = BytesRef;

    /// `next()`: `null` when exhausted.
    fn next(&mut self) -> Option<BytesRef> {
        self.it.next().cloned()
    }
}
