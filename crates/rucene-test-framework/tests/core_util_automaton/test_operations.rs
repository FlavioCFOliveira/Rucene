//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestOperations.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::util::automaton::{
    Automata, Automaton, AutomatonBuilder, FiniteStringsIterator, LimitedFiniteStringsIterator,
    Operations, RegExp, Transition, TransitionAccessor, DEFAULT_DETERMINIZE_WORK_LIMIT,
    MAX_CODE_POINT, MIN_CODE_POINT,
};
use rucene::util::{BytesRef, IntsRef, UnicodeUtil};
use rucene_test_framework::util::automaton::automaton_test_util::{
    AutomatonTestUtil, RandomAcceptedStrings,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, class, det, expect_throws_panic, random, test, try_catch, unchecked, JavaHashSet,
};
use randomizedtesting::junit::Assert;

/// `TestOperations`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testStringUnion", test_string_union),
        test(
            "testEmptyLanguageConcatenate",
            test_empty_language_concatenate,
        ),
        test("testCycledAutomaton", test_cycled_automaton),
        test("testTopoSortStates", test_topo_sort_states),
        test(
            "testEmptySingletonNFAConcatenate",
            test_empty_singleton_nfa_concatenate,
        ),
        test(
            "testGetRandomAcceptedString",
            test_get_random_accepted_string,
        ),
        test("testIsFiniteEatsStack", test_is_finite_eats_stack),
        test("testIsTotal", test_is_total),
        test("testRepeatEmptyLanguage", test_repeat_empty_language),
        test("testRepeatEmptyString", test_repeat_empty_string),
        test("testRepeatChar", test_repeat_char),
        test("testRepeatOptionalChar", test_repeat_optional_char),
        test("testRepeatTwoChar", test_repeat_two_char),
        test("testRepeatOptionalTwoChar", test_repeat_optional_two_char),
        test("testRepeatConcatenation", test_repeat_concatenation),
        test(
            "testRepeatOptionalConcatenation",
            test_repeat_optional_concatenation,
        ),
        test(
            "testRepeatConcatenateOptional",
            test_repeat_concatenate_optional,
        ),
        test(
            "testMergeAcceptStatesWithNoTransition",
            test_merge_accept_states_with_no_transition,
        ),
        test("testDuelRepeat", test_duel_repeat),
        test("testOptional", test_optional),
        test("testOptionalOptional", test_optional_optional),
        test("testOptionalAcceptsState0", test_optional_accepts_state0),
        // Not a JUnit 3 test name (capital `T`): the provider skips it, as
        // Java's does.
        test(
            "TestOptionalOptionalAcceptsState0",
            test_optional_optional_accepts_state0,
        ),
        test("testDuelOptional", test_duel_optional),
    ];
    class("org.apache.lucene.util.automaton.TestOperations", methods)
}

/// `assertSame(expected, actual)` over two automata.
///
/// **Divergence from Lucene 10.5.0.** Java asserts that `Operations.repeat`
/// returned its argument itself (object identity); Rucene's operations take
/// the automaton by reference and return a new value, so identity cannot
/// hold. The port asserts the closest observable equivalent: the same
/// states, accept flags and transitions (`toDot()` equality).
fn assert_same_automaton(expected: &Automaton, actual: &Automaton) {
    assert_equals(&expected.to_dot(), &actual.to_dot());
}

/// `testStringUnion()`: test string union.
fn test_string_union() -> Result<()> {
    let mut strings: Vec<BytesRef> = Vec::new();
    let mut i = RandomNumbers::random_int_between(&*random(), 0, 1000);
    loop {
        i -= 1;
        if i < 0 {
            break;
        }
        strings.push(BytesRef::from_char_sequence(
            &TestUtil::random_unicode_string(&*random()),
        ));
    }
    strings.sort();
    let union = Automata::make_string_union(strings.clone())?;
    Assert::assert_true(union.is_deterministic());
    Assert::assert_false(Operations::has_dead_states_from_initial(&union));

    let naive_union = naive_union(&strings)?;
    Assert::assert_true(naive_union.is_deterministic());
    Assert::assert_false(Operations::has_dead_states_from_initial(&naive_union));

    Assert::assert_true(AutomatonTestUtil::same_language(&union, &naive_union));
    Ok(())
}

/// `naiveUnion(strings)`.
fn naive_union(strings: &[BytesRef]) -> Result<Automaton> {
    let mut each_individual = Vec::with_capacity(strings.len());
    for bref in strings {
        each_individual.push(Automata::make_string(&bref.utf8_to_string()?));
    }
    Ok(det(Operations::determinize(
        &Operations::union(&each_individual),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    )))
}

/// `testEmptyLanguageConcatenate()`: test concatenation with empty language
/// returns empty.
fn test_empty_language_concatenate() -> Result<()> {
    let concat = Operations::concatenate(&[Automata::make_string("a"), Automata::make_empty()]);
    AutomatonTestUtil::assert_minimal_dfa(&concat);
    Assert::assert_true(Operations::is_empty(&concat));
    Ok(())
}

/// `testCycledAutomaton()`: `topoSortStates` detects a cycle.
fn test_cycled_automaton() -> Result<()> {
    let a = generate_random_automaton(true);
    let exc = expect_throws("java.lang.IllegalArgumentException", || {
        Operations::topo_sort_states(&a)
    });
    Assert::assert_true(
        exc.message()
            .unwrap_or("")
            .contains("Input automaton has a cycle"),
    );
    Ok(())
}

/// `testTopoSortStates()`.
fn test_topo_sort_states() -> Result<()> {
    let a = generate_random_automaton(false);
    let sorted = Operations::topo_sort_states(&a)?;
    let mut state_map = vec![-1i32; a.get_num_states() as usize];
    let mut order = 0;
    for &state in &sorted {
        assert_equals(&-1, &state_map[state as usize]);
        state_map[state as usize] = order;
        order += 1;
    }

    let mut transition = Transition::new();
    for &state in &sorted {
        let count = a.init_transition(state, &mut transition);
        for _ in 0..count {
            a.get_next_transition(&mut transition);
            // ensure dest's order is higher than current state
            Assert::assert_true(state_map[transition.dest as usize] > state_map[state as usize]);
        }
    }
    Ok(())
}

/// `testEmptySingletonNFAConcatenate()`: test optimization to
/// `concatenate()` with empty String to an NFA.
fn test_empty_singleton_nfa_concatenate() -> Result<()> {
    let singleton = Automata::make_string("");
    let expanded_singleton = singleton.clone();
    // an NFA (two transitions for 't' from initial state)
    let nfa = Operations::union(&[
        Automata::make_string("this"),
        Automata::make_string("three"),
    ]);
    AutomatonTestUtil::assert_clean_nfa(&nfa);

    let concat1 = Operations::concatenate(&[expanded_singleton, nfa.clone()]);
    AutomatonTestUtil::assert_clean_nfa(&concat1);
    let concat2 = Operations::concatenate(&[singleton, nfa.clone()]);
    AutomatonTestUtil::assert_clean_nfa(&concat2);
    Assert::assert_false(concat2.is_deterministic());
    Assert::assert_true(AutomatonTestUtil::same_language(
        &det(Operations::determinize(&concat1, 100)),
        &det(Operations::determinize(&concat2, 100)),
    ));
    Assert::assert_true(AutomatonTestUtil::same_language(
        &det(Operations::determinize(&nfa, 100)),
        &det(Operations::determinize(&concat1, 100)),
    ));
    Assert::assert_true(AutomatonTestUtil::same_language(
        &det(Operations::determinize(&nfa, 100)),
        &det(Operations::determinize(&concat2, 100)),
    ));
    Ok(())
}

/// `testGetRandomAcceptedString()`.
fn test_get_random_accepted_string() -> Result<()> {
    let iter1 = LuceneTestCase::at_least(100);
    let iter2 = LuceneTestCase::at_least(100);

    for _ in 0..iter1 {
        let text = AutomatonTestUtil::random_regexp(&*random());
        let re = RegExp::with_flags(&text, RegExp::NONE)?;
        // System.out.println("TEST i=" + i + " re=" + re);
        let a = det(Operations::determinize(
            &re.to_automaton()?,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        Assert::assert_false_msg(&format!("empty: {text}"), Operations::is_empty(&a));

        let rx = RandomAcceptedStrings::new(&a);
        for _ in 0..iter2 {
            // System.out.println("TEST: j=" + j);
            let mut acc: Option<Vec<i32>> = None;
            let outcome = try_catch("java.lang.Throwable", || {
                let got = rx.get_random_accepted_string(&*random());
                acc = Some(got.clone());
                let s = unchecked(UnicodeUtil::new_string(&got, 0, got.len()));
                // a.writeDot("adot");
                Assert::assert_true(Operations::run(&a, &s));
            });
            if let Err(t) = outcome {
                println!("regexp: {re}");
                if let Some(acc) = &acc {
                    println!("fail acc re={re} count={}", acc.len());
                    for v in acc {
                        println!("  {v:x}");
                    }
                }
                t.throw();
            }
        }
    }
    Ok(())
}

/// `testIsFiniteEatsStack()`.
fn test_is_finite_eats_stack() -> Result<()> {
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
    let exc = expect_throws_panic("java.lang.IllegalArgumentException", || {
        AutomatonTestUtil::is_finite(&a);
    });
    Assert::assert_true(
        exc.message()
            .unwrap_or("")
            .contains("input automaton is too large"),
    );
    Ok(())
}

/// `testIsTotal()`.
fn test_is_total() -> Result<()> {
    // minimal
    Assert::assert_false(Operations::is_total(&Automata::make_empty()));
    Assert::assert_false(Operations::is_total(&Automata::make_empty_string()));
    Assert::assert_true(Operations::is_total(&Automata::make_any_string()));
    Assert::assert_true(Operations::is_total_range(
        &Automata::make_any_binary(),
        0,
        255,
    ));
    Assert::assert_false(Operations::is_total_range(
        &Automata::make_non_empty_binary(),
        0,
        255,
    ));
    // deterministic, but not minimal
    Assert::assert_true(Operations::is_total(&Operations::repeat(
        &Automata::make_any_char(),
    )));
    let tricky = Operations::repeat(&Operations::union(&[
        Automata::make_char_range(MIN_CODE_POINT, 100),
        Automata::make_char_range(101, MAX_CODE_POINT),
    ]));
    Assert::assert_true(Operations::is_total(&tricky));
    // not total, but close
    let tricky2 = Operations::repeat(&Operations::union(&[
        Automata::make_char_range(MIN_CODE_POINT + 1, 100),
        Automata::make_char_range(101, MAX_CODE_POINT),
    ]));
    Assert::assert_false(Operations::is_total(&tricky2));
    let tricky3 = Operations::repeat(&Operations::union(&[
        Automata::make_char_range(MIN_CODE_POINT, 99),
        Automata::make_char_range(101, MAX_CODE_POINT),
    ]));
    Assert::assert_false(Operations::is_total(&tricky3));
    let tricky4 = Operations::repeat(&Operations::union(&[
        Automata::make_char_range(MIN_CODE_POINT, 100),
        Automata::make_char_range(101, MAX_CODE_POINT - 1),
    ]));
    Assert::assert_false(Operations::is_total(&tricky4));
    Ok(())
}

/// `getFiniteStrings(a)`: the set of all accepted strings.
pub fn get_finite_strings(a: &Automaton) -> JavaHashSet<IntsRef> {
    get_finite_strings_of(|| FiniteStringsIterator::new(a).next_all())
}

/// `getFiniteStrings(a, limit)`: the set of accepted strings, up to at most
/// `limit` strings.
pub fn get_finite_strings_limit(a: &Automaton, limit: i32) -> JavaHashSet<IntsRef> {
    get_finite_strings_of(|| {
        let mut iterator = unchecked(LimitedFiniteStringsIterator::new(a, limit));
        let mut result = Vec::new();
        while let Some(finite_string) = unchecked(iterator.next()) {
            result.push(IntsRef::deep_copy_of(&finite_string));
        }
        result
    })
}

/// `getFiniteStrings(iterator)`: all finite strings of an iterator.
fn get_finite_strings_of(all: impl FnOnce() -> Vec<IntsRef>) -> JavaHashSet<IntsRef> {
    let mut result = JavaHashSet::new();
    for finite_string in all() {
        result.add(finite_string);
    }
    result
}

/// Draining a `FiniteStringsIterator`, each string deep-copied
/// (`IntsRef.deepCopyOf`).
pub trait NextAll {
    /// Every remaining string.
    fn next_all(self) -> Vec<IntsRef>;
}

impl NextAll for FiniteStringsIterator<'_> {
    fn next_all(mut self) -> Vec<IntsRef> {
        let mut result = Vec::new();
        while let Some(finite_string) = unchecked(self.next()) {
            result.push(IntsRef::deep_copy_of(&finite_string));
        }
        result
    }
}

/// `generateRandomAutomaton(hasCycle)`: a random automaton built level by
/// level; with `has_cycle`, the first state of the last level goes back to
/// the initial state.
fn generate_random_automaton(has_cycle: bool) -> Automaton {
    let mut a = Automaton::new();
    let mut last_level_states: Vec<i32> = Vec::new();
    let initial_state = a.create_state();
    let max_level = random().next_int_range(4, 10);
    last_level_states.push(initial_state);

    for _level in 1..max_level {
        let num_states = random().next_int_range(3, 10);
        let mut next_level_states = Vec::new();

        for _ in 0..num_states {
            let next_state = a.create_state();
            next_level_states.push(next_state);
        }

        for &last_state in &last_level_states {
            for &next_state in &next_level_states {
                // if hasCycle is enabled, we will always add a transition, so we could make sure the
                // generated Automaton has a cycle.
                if has_cycle || random().next_int_bound(7) >= 1 {
                    a.add_transition(last_state, next_state, random().next_int_bound(10));
                }
            }
        }
        last_level_states = next_level_states;
    }

    if has_cycle {
        let last_state = last_level_states[0];
        a.add_transition(last_state, initial_state, random().next_int_bound(10));
    }

    a.finish_state();
    a
}

/// `testRepeatEmptyLanguage()`.
fn test_repeat_empty_language() -> Result<()> {
    let expected = Automata::make_empty();
    let actual = Operations::repeat(&expected);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    assert_same_automaton(&expected, &actual);
    Ok(())
}

/// `testRepeatEmptyString()`.
fn test_repeat_empty_string() -> Result<()> {
    let expected = Automata::make_empty_string();
    let actual = Operations::repeat(&expected);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    assert_same_automaton(&expected, &actual);
    Ok(())
}

/// `testRepeatChar()`.
fn test_repeat_char() -> Result<()> {
    let actual = Operations::repeat(&Automata::make_char('a' as i32));
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    let mut expected = Automaton::new();
    expected.create_state();
    expected.set_accept(0, true);
    expected.add_transition(0, 0, 'a' as i32);
    expected.finish_state();
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `testRepeatOptionalChar()`.
fn test_repeat_optional_char() -> Result<()> {
    let mut a_or_empty = Automaton::new();
    a_or_empty.create_state();
    a_or_empty.set_accept(0, true);
    a_or_empty.create_state();
    a_or_empty.set_accept(1, true);
    a_or_empty.add_transition(0, 1, 'a' as i32);
    let actual = Operations::repeat(&a_or_empty);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    let expected = Operations::repeat(&Automata::make_char('a' as i32));
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `testRepeatTwoChar()`.
fn test_repeat_two_char() -> Result<()> {
    let mut expected = Automaton::new();
    expected.create_state();
    expected.create_state();
    expected.set_accept(0, true);
    expected.add_transition(0, 1, 'a' as i32);
    expected.finish_state();
    expected.add_transition(1, 0, 'b' as i32);
    expected.finish_state();
    let actual = Operations::repeat(&Automata::make_string("ab"));
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `testRepeatOptionalTwoChar()`.
fn test_repeat_optional_two_char() -> Result<()> {
    let expected = Operations::repeat(&Automata::make_string("ab"));
    let actual = Operations::repeat(&expected);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `testRepeatConcatenation()`.
fn test_repeat_concatenation() -> Result<()> {
    let mut expected = Automaton::new();
    expected.create_state();
    expected.create_state();
    expected.create_state();
    expected.set_accept(0, true);
    expected.add_transition(0, 1, 'a' as i32);
    expected.add_transition(0, 0, 'c' as i32);
    expected.finish_state();
    expected.add_transition(1, 2, 'b' as i32);
    expected.finish_state();
    expected.add_transition(2, 1, 'a' as i32);
    expected.add_transition(2, 0, 'c' as i32);
    expected.finish_state();
    let abs = Operations::repeat(&Automata::make_string("ab"));
    let abs_then_c = Operations::concatenate(&[abs, Automata::make_char('c' as i32)]);
    let actual = Operations::repeat(&abs_then_c);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `testRepeatOptionalConcatenation()`.
fn test_repeat_optional_concatenation() -> Result<()> {
    let abs = Operations::repeat(&Automata::make_string("ab"));
    let abs_then_c = Operations::concatenate(&[abs, Automata::make_char('c' as i32)]);
    let expected = Operations::repeat(&abs_then_c);
    let actual = Operations::repeat(&expected);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    assert_same_automaton(&expected, &Operations::repeat(&actual));
    Ok(())
}

/// `testRepeatConcatenateOptional()`.
fn test_repeat_concatenate_optional() -> Result<()> {
    let mut expected = Automaton::new();
    expected.create_state();
    expected.create_state();
    expected.set_accept(0, true);
    expected.add_transition(0, 0, 'a' as i32);
    expected.add_transition(0, 1, 'a' as i32);
    expected.finish_state();
    expected.add_transition(1, 0, 'b' as i32);
    expected.finish_state();
    let expected = det(Operations::determinize(&expected, i32::MAX));

    let mut a_or_ab = Automaton::new();
    a_or_ab.create_state();
    a_or_ab.create_state();
    a_or_ab.create_state();
    a_or_ab.set_accept(1, true);
    a_or_ab.set_accept(2, true);
    a_or_ab.add_transition(0, 1, 'a' as i32);
    a_or_ab.finish_state();
    a_or_ab.add_transition(1, 2, 'b' as i32);
    a_or_ab.finish_state();
    let actual = Operations::repeat(&a_or_ab);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `getAcceptStates().cardinality()`.
fn accept_cardinality(a: &Automaton) -> i32 {
    a.get_accept_states().iter().filter(|&&b| b).count() as i32
}

/// `testMergeAcceptStatesWithNoTransition()`.
fn test_merge_accept_states_with_no_transition() -> Result<()> {
    let empty_language = Automata::make_empty();
    assert_same_automaton(
        &empty_language,
        &Operations::merge_accept_states_with_no_transition(&empty_language),
    );

    let a = Automata::make_string("a");
    assert_same_automaton(&a, &Operations::merge_accept_states_with_no_transition(&a));

    // All accept states get combined
    let mut a_or_c = Automaton::new();
    a_or_c.create_state();
    a_or_c.create_state();
    a_or_c.create_state();
    a_or_c.add_transition(0, 1, 'a' as i32);
    a_or_c.set_accept(1, true);
    a_or_c.add_transition(0, 2, 'c' as i32);
    a_or_c.set_accept(2, true);
    let a_or_c_single_accept_state = Operations::merge_accept_states_with_no_transition(&a_or_c);
    assert_equals(&1, &accept_cardinality(&a_or_c_single_accept_state));
    Assert::assert_true(AutomatonTestUtil::same_language(
        &a_or_c,
        &a_or_c_single_accept_state,
    ));

    // Two accept states get combined, but not the 3rd one since it has an outgoing transition
    let mut a_or_c_or_x_star = Automaton::new();
    a_or_c_or_x_star.create_state();
    a_or_c_or_x_star.create_state();
    a_or_c_or_x_star.create_state();
    a_or_c_or_x_star.create_state();
    a_or_c_or_x_star.add_transition(0, 1, 'a' as i32);
    a_or_c_or_x_star.set_accept(1, true);
    a_or_c_or_x_star.add_transition(0, 2, 'c' as i32);
    a_or_c_or_x_star.set_accept(2, true);
    a_or_c_or_x_star.add_transition(0, 3, 'x' as i32);
    a_or_c_or_x_star.add_transition(3, 3, 'x' as i32);
    a_or_c_or_x_star.set_accept(3, true);
    let a_or_c_or_x_star_single_accept_state =
        Operations::merge_accept_states_with_no_transition(&a_or_c_or_x_star);
    assert_equals(
        &2,
        &accept_cardinality(&a_or_c_or_x_star_single_accept_state),
    );
    Assert::assert_true(AutomatonTestUtil::same_language(
        &a_or_c_or_x_star,
        &a_or_c_or_x_star_single_accept_state,
    ));

    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        // sameLangage requires a deterministic automaton
        let expected = det(Operations::determinize(
            &AutomatonTestUtil::random_automaton(&*random()),
            i32::MAX,
        ));
        let actual = Operations::merge_accept_states_with_no_transition(&expected);
        Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    }
    Ok(())
}

/// `testDuelRepeat()`.
fn test_duel_repeat() -> Result<()> {
    let iters = LuceneTestCase::at_least(1_000);
    for _ in 0..iters {
        let a = AutomatonTestUtil::random_automaton(&*random());
        let repeat1 = det(Operations::determinize(&Operations::repeat(&a), i32::MAX));
        let repeat2 = det(Operations::determinize(&naive_repeat(&a), i32::MAX));
        Assert::assert_true(AutomatonTestUtil::same_language(&repeat1, &repeat2));
    }
    Ok(())
}

/// `naiveRepeat(a)`: the original implementation of `Operations#repeat`,
/// before it was improved to generate simpler automata in some common cases.
fn naive_repeat(a: &Automaton) -> Automaton {
    if a.get_num_states() == 0 {
        return a.clone();
    }
    let mut builder = AutomatonBuilder::new();
    // Create the initial state, which is accepted
    builder.create_state();
    builder.set_accept(0, true);
    builder.copy(a);

    let mut t = Transition::new();
    let mut count = a.init_transition(0, &mut t);
    for _ in 0..count {
        a.get_next_transition(&mut t);
        builder.add_transition_range(0, t.dest + 1, t.min, t.max);
    }

    let num_states = a.get_num_states();
    for s in 0..num_states {
        if a.is_accept(s) {
            count = a.init_transition(0, &mut t);
            for _ in 0..count {
                a.get_next_transition(&mut t);
                builder.add_transition_range(s + 1, t.dest + 1, t.min, t.max);
            }
        }
    }

    builder.finish()
}

/// `testOptional()`.
fn test_optional() -> Result<()> {
    let mut expected = Automaton::new();
    expected.create_state();
    expected.set_accept(0, true);
    expected.finish_state();
    expected.create_state();
    expected.set_accept(1, true);
    expected.add_transition(0, 1, 'a' as i32);
    expected.finish_state();
    let actual = Operations::optional(&Automata::make_char('a' as i32));
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `testOptionalOptional()`.
fn test_optional_optional() -> Result<()> {
    let expected = Operations::optional(&Automata::make_char('a' as i32));
    let actual = Operations::optional(&expected);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `testOptionalAcceptsState0()`: an automaton that has a transition to
/// state 0, `a(ba)*`.
fn test_optional_accepts_state0() -> Result<()> {
    let mut expected = Automaton::new();
    expected.create_state();
    expected.set_accept(0, true);
    expected.create_state();
    expected.create_state();
    expected.set_accept(2, true);
    expected.add_transition(0, 2, 'a' as i32);
    expected.finish_state();
    expected.add_transition(1, 2, 'a' as i32);
    expected.finish_state();
    expected.add_transition(2, 1, 'b' as i32);
    expected.finish_state();

    let mut a = Automaton::new();
    a.create_state();
    a.create_state();
    a.set_accept(1, true);
    a.add_transition(0, 1, 'a' as i32);
    a.finish_state();
    a.add_transition(1, 0, 'b' as i32);
    a.finish_state();

    let actual = Operations::optional(&a);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `TestOptionalOptionalAcceptsState0()`.
fn test_optional_optional_accepts_state0() -> Result<()> {
    let mut expected = Automaton::new();
    expected.create_state();
    expected.create_state();
    expected.set_accept(1, true);
    expected.add_transition(0, 1, 'a' as i32);
    expected.finish_state();
    expected.add_transition(1, 0, 'b' as i32);
    expected.finish_state();
    let expected = Operations::optional(&expected);
    let actual = Operations::optional(&expected);
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

/// `testDuelOptional()`.
fn test_duel_optional() -> Result<()> {
    let iters = LuceneTestCase::at_least(1_000);
    for _ in 0..iters {
        let a = AutomatonTestUtil::random_automaton(&*random());
        let repeat1 = det(Operations::determinize(&Operations::optional(&a), i32::MAX));
        let repeat2 = det(Operations::determinize(&naive_optional(&a), i32::MAX));
        Assert::assert_true(AutomatonTestUtil::same_language(&repeat1, &repeat2));
    }
    Ok(())
}

/// `naiveOptional(a)`: the original implementation of
/// `Operations#optional`.
fn naive_optional(a: &Automaton) -> Automaton {
    let mut result = Automaton::new();
    result.create_state();
    result.set_accept(0, true);
    if a.get_num_states() > 0 {
        result.copy(a);
        result.add_epsilon(0, 1);
    }
    result.finish_state();
    result
}
