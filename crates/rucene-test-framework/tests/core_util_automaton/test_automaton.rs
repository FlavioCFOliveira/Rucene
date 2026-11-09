//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestAutomaton.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{
    Automata, Automaton, AutomatonBuilder, Operations, RegExp, Transition, TransitionAccessor,
    UTF32ToUTF8, DEFAULT_DETERMINIZE_WORK_LIMIT, MAX_CODE_POINT, MAX_STRING_UNION_TERM_LENGTH,
    MIN_CODE_POINT,
};
use rucene::util::fst::Util;
use rucene::util::{BytesRef, BytesRefBuilder, IntsRef, IntsRefBuilder, UnicodeUtil};
use rucene_test_framework::util::automaton::automaton_test_util::{
    AutomatonTestUtil, RandomAcceptedStrings,
};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_null, class, det, expect_throws_panic, random, test, try_catch,
    unchecked, verbose, JavaHashSet, TOO_COMPLEX_TO_DETERMINIZE,
};
use crate::minimization_operations::MinimizationOperations;
use crate::test_operations::{get_finite_strings, get_finite_strings_limit};

/// `TestAutomaton`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testBasic", test_basic),
        test("testReduceBasic", test_reduce_basic),
        test("testSameLanguage", test_same_language),
        test("testCommonPrefixString", test_common_prefix_string),
        test("testCommonPrefixEmpty", test_common_prefix_empty),
        test(
            "testCommonPrefixEmptyString",
            test_common_prefix_empty_string,
        ),
        test("testCommonPrefixAny", test_common_prefix_any),
        test("testCommonPrefixRange", test_common_prefix_range),
        test("testAlternatives", test_alternatives),
        test(
            "testCommonPrefixLeadingWildcard",
            test_common_prefix_leading_wildcard,
        ),
        test(
            "testCommonPrefixTrailingWildcard",
            test_common_prefix_trailing_wildcard,
        ),
        test(
            "testCommonPrefixLeadingKleenStar",
            test_common_prefix_leading_kleen_star,
        ),
        test(
            "testCommonPrefixTrailingKleenStar",
            test_common_prefix_trailing_kleen_star,
        ),
        test("testCommonPrefixOptional", test_common_prefix_optional),
        test("testCommonPrefixNFA", test_common_prefix_nfa),
        test(
            "testCommonPrefixNFAInfinite",
            test_common_prefix_nfa_infinite,
        ),
        test("testCommonPrefixUnicode", test_common_prefix_unicode),
        test("testConcatenate1", test_concatenate1),
        test("testConcatenate2", test_concatenate2),
        test("testUnion1", test_union1),
        test("testUnion2", test_union2),
        test("testMinimizeSimple", test_minimize_simple),
        test("testMinimize2", test_minimize2),
        test("testReverse", test_reverse),
        test("testOptional", test_optional),
        test("testRepeatAny", test_repeat_any),
        test("testRepeatMin", test_repeat_min),
        test("testRepeatMinMax1", test_repeat_min_max1),
        test("testRepeatMinMax2", test_repeat_min_max2),
        test("testComplement", test_complement),
        test("testInterval", test_interval),
        test("testCommonSuffix", test_common_suffix),
        test("testCommonSuffixEmpty", test_common_suffix_empty),
        test(
            "testCommonSuffixEmptyString",
            test_common_suffix_empty_string,
        ),
        test(
            "testCommonSuffixTrailingWildcard",
            test_common_suffix_trailing_wildcard,
        ),
        test(
            "testCommonSuffixLeadingKleenStar",
            test_common_suffix_leading_kleen_star,
        ),
        test(
            "testCommonSuffixTrailingKleenStar",
            test_common_suffix_trailing_kleen_star,
        ),
        test("testCommonSuffixUnicode", test_common_suffix_unicode),
        test("testReverseRandom1", test_reverse_random1),
        test("testReverseRandom2", test_reverse_random2),
        test("testAnyStringEmptyString", test_any_string_empty_string),
        test("testBasicIsEmpty", test_basic_is_empty),
        test(
            "testRemoveDeadTransitionsEmpty",
            test_remove_dead_transitions_empty,
        ),
        test("testInvalidAddTransition", test_invalid_add_transition),
        test("testBuilderRandom", test_builder_random),
        test("testIsTotal", test_is_total),
        test("testMinimizeEmpty", test_minimize_empty),
        test("testMinus", test_minus),
        test("testOneInterval", test_one_interval),
        test("testAnotherInterval", test_another_interval),
        test("testIntervalRandom", test_interval_random),
        test(
            "testConcatenatePreservesDet",
            test_concatenate_preserves_det,
        ),
        test("testRemoveDeadStates", test_remove_dead_states),
        test("testRemoveDeadStatesEmpty1", test_remove_dead_states_empty1),
        test("testRemoveDeadStatesEmpty2", test_remove_dead_states_empty2),
        test("testRemoveDeadStatesEmpty3", test_remove_dead_states_empty3),
        test("testConcatEmpty", test_concat_empty),
        test(
            "testSeemsNonEmptyButIsNot1",
            test_seems_non_empty_but_is_not1,
        ),
        test(
            "testSeemsNonEmptyButIsNot2",
            test_seems_non_empty_but_is_not2,
        ),
        test("testSameLanguage1", test_same_language1),
        test("testRandomFinite", test_random_finite),
        test(
            "testMakeBinaryIntervalFiniteCasesBasic",
            test_make_binary_interval_finite_cases_basic,
        ),
        test(
            "testMakeBinaryIntervalFiniteCasesRandom",
            test_make_binary_interval_finite_cases_random,
        ),
        test(
            "testMakeBinaryIntervalRandom",
            test_make_binary_interval_random,
        ),
        test(
            "testMakeBinaryIntervalBasic",
            test_make_binary_interval_basic,
        ),
        test(
            "testMakeBinaryIntervalLowerBoundEmptyString",
            test_make_binary_interval_lower_bound_empty_string,
        ),
        test(
            "testMakeBinaryIntervalEqual",
            test_make_binary_interval_equal,
        ),
        test(
            "testMakeBinaryIntervalCommonPrefix",
            test_make_binary_interval_common_prefix,
        ),
        test("testMakeBinaryExceptEmpty", test_make_binary_except_empty),
        test(
            "testMakeBinaryIntervalOpenMax",
            test_make_binary_interval_open_max,
        ),
        test(
            "testMakeBinaryIntervalOpenMaxZeroLengthMin",
            test_make_binary_interval_open_max_zero_length_min,
        ),
        test(
            "testMakeBinaryIntervalOpenMin",
            test_make_binary_interval_open_min,
        ),
        test(
            "testMakeBinaryIntervalOpenBoth",
            test_make_binary_interval_open_both,
        ),
        test(
            "testAcceptAllEmptyStringMin",
            test_accept_all_empty_string_min,
        ),
        test("testGetSingleton", test_get_singleton),
        test(
            "testGetSingletonEmptyString",
            test_get_singleton_empty_string,
        ),
        test("testGetSingletonNothing", test_get_singleton_nothing),
        test("testGetSingletonTwo", test_get_singleton_two),
        test(
            "testDeterminizeTooMuchEffort",
            test_determinize_too_much_effort,
        ),
        test("testMakeCharSetEmpty", test_make_char_set_empty),
        test("testMakeCharSetOne", test_make_char_set_one),
        test("testMakeCharSetTwo", test_make_char_set_two),
        test("testMakeCharSetDups", test_make_char_set_dups),
        test(
            "testMakeCharCaseInsensitive",
            test_make_char_case_insensitive,
        ),
        test(
            "testMakeStringCaseInsensitiveSimple",
            test_make_string_case_insensitive_simple,
        ),
    ];
    class("org.apache.lucene.util.automaton.TestAutomaton", methods)
}

fn c(ch: char) -> i32 {
    ch as i32
}

fn new_bytes_ref_str(s: &str) -> BytesRef {
    LuceneTestCase::new_bytes_ref_str(s)
}

fn new_bytes_ref() -> BytesRef {
    LuceneTestCase::new_bytes_ref()
}

fn utf8(b: &BytesRef) -> String {
    unchecked(b.utf8_to_string())
}

fn test_basic() -> Result<()> {
    let mut a = Automaton::new();
    let start = a.create_state();
    let x = a.create_state();
    let y = a.create_state();
    let end = a.create_state();
    a.set_accept(end, true);

    a.add_transition_range(start, x, c('a'), c('a'));
    a.add_transition_range(start, end, c('d'), c('d'));
    a.add_transition_range(x, y, c('b'), c('b'));
    a.add_transition_range(y, end, c('c'), c('c'));
    a.finish_state();
    Ok(())
}

fn test_reduce_basic() -> Result<()> {
    let mut a = Automaton::new();
    let start = a.create_state();
    let end = a.create_state();
    a.set_accept(end, true);
    // Should collapse to a-b:
    a.add_transition_range(start, end, c('a'), c('a'));
    a.add_transition_range(start, end, c('b'), c('b'));
    a.add_transition_range(start, end, c('m'), c('m'));
    // Should collapse to x-y:
    a.add_transition_range(start, end, c('x'), c('x'));
    a.add_transition_range(start, end, c('y'), c('y'));

    a.finish_state();
    assert_equals(&3, &a.get_num_transitions(start));
    let mut scratch = Transition::new();
    a.init_transition(start, &mut scratch);
    a.get_next_transition(&mut scratch);
    assert_equals(&c('a'), &scratch.min);
    assert_equals(&c('b'), &scratch.max);
    a.get_next_transition(&mut scratch);
    assert_equals(&c('m'), &scratch.min);
    assert_equals(&c('m'), &scratch.max);
    a.get_next_transition(&mut scratch);
    assert_equals(&c('x'), &scratch.min);
    assert_equals(&c('y'), &scratch.max);
    Ok(())
}

fn test_same_language() -> Result<()> {
    let a1 = Automata::make_string("foobar");
    let a2 = Operations::concatenate(&[Automata::make_string("foo"), Automata::make_string("bar")]);
    Assert::assert_true(AutomatonTestUtil::same_language(&a1, &a2));
    Ok(())
}

fn test_common_prefix_string() -> Result<()> {
    let a =
        Operations::concatenate(&[Automata::make_string("foobar"), Automata::make_any_string()]);
    AutomatonTestUtil::assert_clean_dfa(&a); // not minimal
    assert_equals("foobar", Operations::get_common_prefix(&a)?.as_str());
    Ok(())
}

fn test_common_prefix_empty() -> Result<()> {
    assert_equals(
        "",
        Operations::get_common_prefix(&Automata::make_empty())?.as_str(),
    );
    Ok(())
}

fn test_common_prefix_empty_string() -> Result<()> {
    assert_equals(
        "",
        Operations::get_common_prefix(&Automata::make_empty_string())?.as_str(),
    );
    Ok(())
}

fn test_common_prefix_any() -> Result<()> {
    assert_equals(
        "",
        Operations::get_common_prefix(&Automata::make_any_string())?.as_str(),
    );
    Ok(())
}

fn test_common_prefix_range() -> Result<()> {
    assert_equals(
        "",
        Operations::get_common_prefix(&Automata::make_char_range(c('a'), c('b')))?.as_str(),
    );
    Ok(())
}

fn test_alternatives() -> Result<()> {
    let a = Automata::make_char(c('a'));
    let cc = Automata::make_char(c('c'));
    assert_equals(
        "",
        Operations::get_common_prefix(&Operations::union(&[a, cc]))?.as_str(),
    );
    Ok(())
}

fn test_common_prefix_leading_wildcard() -> Result<()> {
    let a = Operations::concatenate(&[Automata::make_any_char(), Automata::make_string("boo")]);
    AutomatonTestUtil::assert_minimal_dfa(&a);
    assert_equals("", Operations::get_common_prefix(&a)?.as_str());
    Ok(())
}

fn test_common_prefix_trailing_wildcard() -> Result<()> {
    let a = Operations::concatenate(&[Automata::make_string("boo"), Automata::make_any_char()]);
    AutomatonTestUtil::assert_minimal_dfa(&a);
    assert_equals("boo", Operations::get_common_prefix(&a)?.as_str());
    Ok(())
}

fn test_common_prefix_leading_kleen_star() -> Result<()> {
    let a = Operations::concatenate(&[Automata::make_any_string(), Automata::make_string("boo")]);
    AutomatonTestUtil::assert_clean_nfa(&a);
    assert_equals("", Operations::get_common_prefix(&a)?.as_str());
    Ok(())
}

fn test_common_prefix_trailing_kleen_star() -> Result<()> {
    let a = Operations::concatenate(&[Automata::make_string("boo"), Automata::make_any_string()]);
    AutomatonTestUtil::assert_clean_dfa(&a); // not minimal
    assert_equals("boo", Operations::get_common_prefix(&a)?.as_str());
    Ok(())
}

fn test_common_prefix_optional() -> Result<()> {
    let mut a = Automaton::new();
    let init = a.create_state();
    let fini = a.create_state();
    a.set_accept(init, true);
    a.set_accept(fini, true);
    a.add_transition(init, fini, c('m'));
    a.add_transition(fini, fini, c('m'));
    a.finish_state();
    assert_equals("", Operations::get_common_prefix(&a)?.as_str());
    Ok(())
}

fn test_common_prefix_nfa() -> Result<()> {
    let mut a = Automaton::new();
    let init = a.create_state();
    let medial = a.create_state();
    let fini = a.create_state();
    a.set_accept(fini, true);
    a.add_transition(init, medial, c('m'));
    a.add_transition(init, fini, c('m'));
    a.add_transition(medial, fini, c('o'));
    a.finish_state();
    assert_equals("m", Operations::get_common_prefix(&a)?.as_str());
    Ok(())
}

fn test_common_prefix_nfa_infinite() -> Result<()> {
    let mut a = Automaton::new();
    let init = a.create_state();
    let medial = a.create_state();
    let fini = a.create_state();
    a.set_accept(fini, true);
    a.add_transition(init, medial, c('m'));
    a.add_transition(init, fini, c('m'));
    a.add_transition(medial, fini, c('m'));
    a.add_transition(fini, fini, c('m'));
    a.finish_state();
    assert_equals("m", Operations::get_common_prefix(&a)?.as_str());
    Ok(())
}

fn test_common_prefix_unicode() -> Result<()> {
    let a = Operations::concatenate(&[
        Automata::make_string("boo\u{1F602}\u{1F602}\u{1F602}"),
        Automata::make_any_char(),
    ]);
    assert_equals(
        "boo\u{1F602}\u{1F602}\u{1F602}",
        Operations::get_common_prefix(&a)?.as_str(),
    );
    Ok(())
}

fn test_concatenate1() -> Result<()> {
    let a = Operations::concatenate(&[Automata::make_string("m"), Automata::make_any_string()]);
    AutomatonTestUtil::assert_clean_dfa(&a); // not minimal
    Assert::assert_true(Operations::run(&a, "m"));
    Assert::assert_true(Operations::run(&a, "me"));
    Assert::assert_true(Operations::run(&a, "me too"));
    Ok(())
}

fn test_concatenate2() -> Result<()> {
    let a = Operations::concatenate(&[
        Automata::make_string("m"),
        Automata::make_any_string(),
        Automata::make_string("n"),
        Automata::make_any_string(),
    ]);
    AutomatonTestUtil::assert_clean_nfa(&a);
    let a = det(Operations::determinize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT));
    Assert::assert_true(Operations::run(&a, "mn"));
    Assert::assert_true(Operations::run(&a, "mone"));
    Assert::assert_false(Operations::run(&a, "m"));
    Assert::assert_false(AutomatonTestUtil::is_finite(&a));
    Ok(())
}

fn test_union1() -> Result<()> {
    let a = Operations::union(&[
        Automata::make_string("foobar"),
        Automata::make_string("barbaz"),
    ]);
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(Operations::run(&a, "foobar"));
    Assert::assert_true(Operations::run(&a, "barbaz"));

    assert_matches(&a, &["foobar", "barbaz"]);
    Ok(())
}

fn test_union2() -> Result<()> {
    let a = Operations::union(&[
        Automata::make_string("foobar"),
        Automata::make_string(""),
        Automata::make_string("barbaz"),
    ]);
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(Operations::run(&a, "foobar"));
    Assert::assert_true(Operations::run(&a, "barbaz"));
    Assert::assert_true(Operations::run(&a, ""));

    assert_matches(&a, &["", "foobar", "barbaz"]);
    Ok(())
}

fn test_minimize_simple() -> Result<()> {
    let a = Automata::make_string("foobar");
    let a_min = MinimizationOperations::minimize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT);

    Assert::assert_true(AutomatonTestUtil::same_language(&a, &a_min));
    Ok(())
}

fn test_minimize2() -> Result<()> {
    let a = Operations::union(&[
        Automata::make_string("foobar"),
        Automata::make_string("boobar"),
    ]);
    let a_min = MinimizationOperations::minimize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT);
    Assert::assert_true(AutomatonTestUtil::same_language(
        &det(Operations::determinize(
            &Operations::remove_dead_states(&a),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        )),
        &a_min,
    ));
    Ok(())
}

fn test_reverse() -> Result<()> {
    let a = Automata::make_string("foobar");
    let ra = Operations::reverse(&a);
    AutomatonTestUtil::assert_minimal_dfa(&a);
    let a2 = Operations::reverse(&ra);
    AutomatonTestUtil::assert_minimal_dfa(&a2);
    Assert::assert_true(AutomatonTestUtil::same_language(&a, &a2));
    Ok(())
}

fn test_optional() -> Result<()> {
    let a = Automata::make_string("foobar");
    let a2 = Operations::optional(&a);
    AutomatonTestUtil::assert_minimal_dfa(&a2);

    Assert::assert_true(Operations::run(&a, "foobar"));
    Assert::assert_false(Operations::run(&a, ""));
    Assert::assert_true(Operations::run(&a2, "foobar"));
    Assert::assert_true(Operations::run(&a2, ""));
    Ok(())
}

fn test_repeat_any() -> Result<()> {
    let a = Automata::make_string("zee");
    let a2 = Operations::repeat(&a);
    AutomatonTestUtil::assert_minimal_dfa(&a2);
    Assert::assert_true(Operations::run(&a2, ""));
    Assert::assert_true(Operations::run(&a2, "zee"));
    Assert::assert_true(Operations::run(&a2, "zeezee"));
    Assert::assert_true(Operations::run(&a2, "zeezeezee"));
    Ok(())
}

fn test_repeat_min() -> Result<()> {
    let a = Automata::make_string("zee");
    let a2 = Operations::repeat_min(&a, 2);
    AutomatonTestUtil::assert_clean_dfa(&a2); // not minimal
    Assert::assert_false(Operations::run(&a2, ""));
    Assert::assert_false(Operations::run(&a2, "zee"));
    Assert::assert_true(Operations::run(&a2, "zeezee"));
    Assert::assert_true(Operations::run(&a2, "zeezeezee"));
    Ok(())
}

fn test_repeat_min_max1() -> Result<()> {
    let a = Automata::make_string("zee");
    let a2 = Operations::repeat_min_max(&a, 0, 2);
    AutomatonTestUtil::assert_minimal_dfa(&a2);
    Assert::assert_true(Operations::run(&a2, ""));
    Assert::assert_true(Operations::run(&a2, "zee"));
    Assert::assert_true(Operations::run(&a2, "zeezee"));
    Assert::assert_false(Operations::run(&a2, "zeezeezee"));
    Ok(())
}

fn test_repeat_min_max2() -> Result<()> {
    let a = Automata::make_string("zee");
    let a2 = Operations::repeat_min_max(&a, 2, 4);
    AutomatonTestUtil::assert_minimal_dfa(&a2);
    Assert::assert_false(Operations::run(&a2, ""));
    Assert::assert_false(Operations::run(&a2, "zee"));
    Assert::assert_true(Operations::run(&a2, "zeezee"));
    Assert::assert_true(Operations::run(&a2, "zeezeezee"));
    Assert::assert_true(Operations::run(&a2, "zeezeezeezee"));
    Assert::assert_false(Operations::run(&a2, "zeezeezeezeezee"));
    Ok(())
}

fn test_complement() -> Result<()> {
    let a = Automata::make_string("zee");
    let a2 = det(Operations::complement(&a, DEFAULT_DETERMINIZE_WORK_LIMIT));
    AutomatonTestUtil::assert_minimal_dfa(&a2);
    Assert::assert_true(Operations::run(&a2, ""));
    Assert::assert_false(Operations::run(&a2, "zee"));
    Assert::assert_true(Operations::run(&a2, "zeezee"));
    Assert::assert_true(Operations::run(&a2, "zeezeezee"));
    Ok(())
}

fn test_interval() -> Result<()> {
    let a = Automata::make_decimal_interval(17, 100, 3)?;
    AutomatonTestUtil::assert_clean_dfa(&a); // not minimal
    Assert::assert_false(Operations::run(&a, ""));
    Assert::assert_true(Operations::run(&a, "017"));
    Assert::assert_true(Operations::run(&a, "100"));
    Assert::assert_true(Operations::run(&a, "073"));
    Ok(())
}

fn test_common_suffix() -> Result<()> {
    let mut a = Automaton::new();
    let init = a.create_state();
    let fini = a.create_state();
    a.set_accept(init, true);
    a.set_accept(fini, true);
    a.add_transition(init, fini, c('m'));
    a.add_transition(fini, fini, c('m'));
    a.finish_state();
    assert_equals(&0, &Operations::get_common_suffix_bytes_ref(&a)?.length);
    Ok(())
}

fn test_common_suffix_empty() -> Result<()> {
    assert_equals(
        &new_bytes_ref(),
        &Operations::get_common_suffix_bytes_ref(&Automata::make_empty())?,
    );
    Ok(())
}

fn test_common_suffix_empty_string() -> Result<()> {
    assert_equals(
        &new_bytes_ref(),
        &Operations::get_common_suffix_bytes_ref(&Automata::make_empty_string())?,
    );
    Ok(())
}

fn test_common_suffix_trailing_wildcard() -> Result<()> {
    let a = Operations::concatenate(&[Automata::make_string("boo"), Automata::make_any_char()]);
    AutomatonTestUtil::assert_minimal_dfa(&a);
    assert_equals(
        &new_bytes_ref(),
        &Operations::get_common_suffix_bytes_ref(&a)?,
    );
    Ok(())
}

fn test_common_suffix_leading_kleen_star() -> Result<()> {
    let a = Operations::concatenate(&[Automata::make_any_string(), Automata::make_string("boo")]);
    AutomatonTestUtil::assert_clean_nfa(&a);
    assert_equals(
        &new_bytes_ref_str("boo"),
        &Operations::get_common_suffix_bytes_ref(&a)?,
    );
    Ok(())
}

fn test_common_suffix_trailing_kleen_star() -> Result<()> {
    let a = Operations::concatenate(&[Automata::make_string("boo"), Automata::make_any_string()]);
    AutomatonTestUtil::assert_clean_dfa(&a); // not minimal
    assert_equals(
        &new_bytes_ref(),
        &Operations::get_common_suffix_bytes_ref(&a)?,
    );
    Ok(())
}

fn test_common_suffix_unicode() -> Result<()> {
    let a = Operations::concatenate(&[
        Automata::make_any_string(),
        Automata::make_string("boo\u{1F602}\u{1F602}\u{1F602}"),
    ]);
    AutomatonTestUtil::assert_clean_nfa(&a);
    let binary = UTF32ToUTF8::new().convert(&a);
    assert_equals(
        &new_bytes_ref_str("boo\u{1F602}\u{1F602}\u{1F602}"),
        &Operations::get_common_suffix_bytes_ref(&binary)?,
    );
    Ok(())
}

fn test_reverse_random1() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let a = AutomatonTestUtil::random_automaton(&*random());
        let ra = Operations::reverse(&a);
        let rra = Operations::reverse(&ra);
        Assert::assert_true(AutomatonTestUtil::same_language(
            &det(Operations::determinize(
                &Operations::remove_dead_states(&a),
                i32::MAX,
            )),
            &det(Operations::determinize(
                &Operations::remove_dead_states(&rra),
                i32::MAX,
            )),
        ));
    }
    Ok(())
}

fn test_reverse_random2() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _iter in 0..iters {
        // System.out.println("TEST: iter=" + iter);
        let mut a = AutomatonTestUtil::random_automaton(&*random());
        if random().next_boolean() {
            a = Operations::remove_dead_states(&a);
        }
        let ra = Operations::reverse(&a);
        let rda = det(Operations::determinize(&ra, i32::MAX));

        if Operations::is_empty(&a) {
            Assert::assert_true(Operations::is_empty(&rda));
            continue;
        }

        let ras = RandomAcceptedStrings::new(&a);

        for _iter2 in 0..20 {
            // Find string accepted by original automaton
            let mut s = ras.get_random_accepted_string(&*random());

            // Reverse it
            let len = s.len();
            for j in 0..len / 2 {
                s.swap(j, len - j - 1);
            }
            // System.out.println("TEST:   iter2=" + iter2 + " s=" + Arrays.toString(s));

            // Make sure reversed automaton accepts it
            Assert::assert_true(Operations::run_ints(
                &rda,
                &IntsRef::with_offset_length(s, 0, len),
            ));
        }
    }
    Ok(())
}

fn test_any_string_empty_string() -> Result<()> {
    let a = Automata::make_any_string();
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(Operations::run(&a, ""));
    Ok(())
}

fn test_basic_is_empty() -> Result<()> {
    let mut a = Automaton::new();
    a.create_state();
    Assert::assert_true(Operations::is_empty(&a));
    Ok(())
}

fn test_remove_dead_transitions_empty() -> Result<()> {
    let a = Automata::make_empty();
    let a2 = Operations::remove_dead_states(&a);
    Assert::assert_true(Operations::is_empty(&a2));
    Ok(())
}

fn test_invalid_add_transition() -> Result<()> {
    let mut a = Automaton::new();
    let s1 = a.create_state();
    let s2 = a.create_state();
    a.add_transition(s1, s2, c('a'));
    a.add_transition(s2, s2, c('a'));
    expect_throws_panic("java.lang.IllegalStateException", || {
        a.add_transition(s1, s2, c('b'));
    });
    Ok(())
}

fn test_builder_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let a = AutomatonTestUtil::random_automaton(&*random());

        // Just get all transitions, shuffle, and build a new automaton with the same transitions:
        let mut all_trans: Vec<Transition> = Vec::new();
        let num_states = a.get_num_states();
        for s in 0..num_states {
            let count = a.get_num_transitions(s);
            for i in 0..count {
                let mut t = Transition::new();
                a.get_transition(s, i, &mut t);
                all_trans.push(t);
            }
        }

        let mut builder = AutomatonBuilder::new();
        for _ in 0..num_states {
            let s = builder.create_state();
            builder.set_accept(s, a.is_accept(s));
        }

        crate::core_support::shuffle(&mut all_trans, &*random());
        for t in &all_trans {
            builder.add_transition_range(t.source, t.dest, t.min, t.max);
        }

        Assert::assert_true(AutomatonTestUtil::same_language(
            &det(Operations::determinize(
                &Operations::remove_dead_states(&a),
                i32::MAX,
            )),
            &det(Operations::determinize(
                &Operations::remove_dead_states(&builder.finish()),
                i32::MAX,
            )),
        ));
    }
    Ok(())
}

fn test_is_total() -> Result<()> {
    Assert::assert_false(Operations::is_total(&Automaton::new()));
    let mut a = Automaton::new();
    let init = a.create_state();
    let fini = a.create_state();
    a.set_accept(fini, true);
    a.add_transition_range(init, fini, MIN_CODE_POINT, MAX_CODE_POINT);
    a.finish_state();
    Assert::assert_false(Operations::is_total(&a));
    a.add_transition_range(fini, fini, MIN_CODE_POINT, MAX_CODE_POINT);
    a.finish_state();
    Assert::assert_false(Operations::is_total(&a));
    a.set_accept(init, true);
    Assert::assert_true(Operations::is_total(&MinimizationOperations::minimize(
        &a,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    )));
    Ok(())
}

fn test_minimize_empty() -> Result<()> {
    let mut a = Automaton::new();
    let init = a.create_state();
    let fini = a.create_state();
    a.add_transition(init, fini, c('a'));
    a.finish_state();
    let a = MinimizationOperations::minimize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT);
    assert_equals(&0, &a.get_num_states());
    Ok(())
}

fn test_minus() -> Result<()> {
    let a1 = Automata::make_string("foobar");
    let a2 = Automata::make_string("boobar");
    let a3 = Automata::make_string("beebar");
    let a = Operations::union(&[a1.clone(), a2.clone(), a3.clone()]);
    AutomatonTestUtil::assert_clean_nfa(&a); // this is why we have makeStringUnion()
    assert_matches(&a, &["foobar", "beebar", "boobar"]);

    let mut a4 = det(Operations::minus(&a, &a2, DEFAULT_DETERMINIZE_WORK_LIMIT));
    AutomatonTestUtil::assert_clean_dfa(&a4);

    Assert::assert_true(Operations::run(&a4, "foobar"));
    Assert::assert_false(Operations::run(&a4, "boobar"));
    Assert::assert_true(Operations::run(&a4, "beebar"));
    assert_matches(&a4, &["foobar", "beebar"]);

    a4 = det(Operations::minus(&a4, &a1, DEFAULT_DETERMINIZE_WORK_LIMIT));
    AutomatonTestUtil::assert_clean_dfa(&a4);
    Assert::assert_false(Operations::run(&a4, "foobar"));
    Assert::assert_false(Operations::run(&a4, "boobar"));
    Assert::assert_true(Operations::run(&a4, "beebar"));
    assert_matches(&a4, &["beebar"]);

    a4 = det(Operations::minus(&a4, &a3, DEFAULT_DETERMINIZE_WORK_LIMIT));
    AutomatonTestUtil::assert_clean_dfa(&a4);
    Assert::assert_false(Operations::run(&a4, "foobar"));
    Assert::assert_false(Operations::run(&a4, "boobar"));
    Assert::assert_false(Operations::run(&a4, "beebar"));
    assert_matches(&a4, &[]);
    Ok(())
}

fn test_one_interval() -> Result<()> {
    let a = Automata::make_decimal_interval(999, 1032, 0)?;
    AutomatonTestUtil::assert_clean_nfa(&a);
    let a = det(Operations::determinize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT));
    Assert::assert_true(Operations::run(&a, "0999"));
    Assert::assert_true(Operations::run(&a, "00999"));
    Assert::assert_true(Operations::run(&a, "000999"));
    Ok(())
}

fn test_another_interval() -> Result<()> {
    let a = Automata::make_decimal_interval(1, 2, 0)?;
    AutomatonTestUtil::assert_clean_dfa(&a);
    Assert::assert_true(Operations::run(&a, "01"));
    Ok(())
}

fn test_interval_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let min = TestUtil::next_int(&*random(), 0, 100000);
        let max = TestUtil::next_int(&*random(), min, min + 100000);
        let digits = if random().next_boolean() {
            0
        } else {
            let s = max.to_string();
            TestUtil::next_int(&*random(), s.len() as i32, 2 * s.len() as i32)
        };
        let mut b = String::new();
        for _ in 0..digits {
            b.push('0');
        }
        let prefix = b.clone();
        let mut a = det(Operations::determinize(
            &Automata::make_decimal_interval(min, max, digits)?,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        if random().next_boolean() {
            a = MinimizationOperations::minimize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT);
        }
        let mut mins = min.to_string();
        let mut maxs = max.to_string();
        if digits > 0 {
            mins = format!("{}{}", &prefix[mins.len()..], mins);
            maxs = format!("{}{}", &prefix[maxs.len()..], maxs);
        }
        Assert::assert_true(Operations::run(&a, &mins));
        Assert::assert_true(Operations::run(&a, &maxs));

        for _iter2 in 0..100 {
            let x = random().next_int_bound(2 * max);
            let expected = x >= min && x <= max;
            let mut sx = x.to_string();
            if (sx.len() as i32) < digits {
                // Left-fill with 0s
                sx = format!("{}{}", &b[sx.len()..], sx);
            } else if digits == 0 {
                // Left-fill with random number of 0s:
                let num_zeros = random().next_int_bound(10);
                let mut sb = String::new();
                for _ in 0..num_zeros {
                    sb.push('0');
                }
                sb.push_str(&sx);
                sx = sb;
            }
            assert_equals(&expected, &Operations::run(&a, &sx));
        }
    }
    Ok(())
}

/// `assertMatches(a, strings...)`.
fn assert_matches(a: &Automaton, strings: &[&str]) {
    let mut expected = JavaHashSet::new();
    for s in strings {
        let mut ints = IntsRefBuilder::new();
        expected.add(Util::to_utf32(s, &mut ints));
    }

    assert_equals(&expected, &get_finite_strings(a));
}

fn test_concatenate_preserves_det() -> Result<()> {
    let a1 = Automata::make_string("foobar");
    AutomatonTestUtil::assert_minimal_dfa(&a1);
    let a2 = Automata::make_string("baz");
    AutomatonTestUtil::assert_minimal_dfa(&a2);
    let a3 = Operations::concatenate(&[a1, a2]);
    AutomatonTestUtil::assert_minimal_dfa(&a3);
    Ok(())
}

fn test_remove_dead_states() -> Result<()> {
    let mut a = Automaton::new();
    let s1 = a.create_state();
    a.create_state(); // create dead state
    a.set_accept(s1, true);
    a.finish_state();
    assert_equals(&2, &a.get_num_states());
    let a = Operations::remove_dead_states(&a);
    assert_equals(&1, &a.get_num_states());
    Ok(())
}

fn test_remove_dead_states_empty1() -> Result<()> {
    let mut a = Automaton::new();
    a.finish_state();
    Assert::assert_true(Operations::is_empty(&a));
    Assert::assert_true(Operations::is_empty(&Operations::remove_dead_states(&a)));
    Ok(())
}

fn test_remove_dead_states_empty2() -> Result<()> {
    let mut a = Automaton::new();
    a.finish_state();
    Assert::assert_true(Operations::is_empty(&a));
    Assert::assert_true(Operations::is_empty(&Operations::remove_dead_states(&a)));
    Ok(())
}

fn test_remove_dead_states_empty3() -> Result<()> {
    let mut a = Automaton::new();
    let init = a.create_state();
    let fini = a.create_state();
    a.add_transition(init, fini, c('a'));
    let a2 = Operations::remove_dead_states(&a);
    assert_equals(&0, &a2.get_num_states());
    Ok(())
}

fn test_concat_empty() -> Result<()> {
    // If you concat empty automaton to anything the result should still be empty:
    let a = Operations::concatenate(&[Automata::make_empty(), Automata::make_string("foo")]);
    AutomatonTestUtil::assert_minimal_dfa(&a);
    assert_equals(&JavaHashSet::<IntsRef>::new(), &get_finite_strings(&a));

    let a = Operations::concatenate(&[Automata::make_string("foo"), Automata::make_empty()]);
    AutomatonTestUtil::assert_minimal_dfa(&a);
    assert_equals(&JavaHashSet::<IntsRef>::new(), &get_finite_strings(&a));
    Ok(())
}

fn test_seems_non_empty_but_is_not1() -> Result<()> {
    let mut a = Automaton::new();
    // Init state has a transition but doesn't lead to accept
    let init = a.create_state();
    let s = a.create_state();
    a.add_transition(init, s, c('a'));
    a.finish_state();
    Assert::assert_true(Operations::is_empty(&a));
    Ok(())
}

fn test_seems_non_empty_but_is_not2() -> Result<()> {
    let mut a = Automaton::new();
    let init = a.create_state();
    let mut s = a.create_state();
    a.add_transition(init, s, c('a'));
    // An orphan'd accept state
    s = a.create_state();
    a.set_accept(s, true);
    a.finish_state();
    Assert::assert_true(Operations::is_empty(&a));
    Ok(())
}

fn test_same_language1() -> Result<()> {
    let a = Automata::make_empty_string();
    let mut a2 = Automata::make_empty_string();
    let state = a2.create_state();
    a2.add_transition(0, state, c('a'));
    a2.finish_state();
    Assert::assert_true(AutomatonTestUtil::same_language(
        &a,
        &Operations::remove_dead_states(&a2),
    ));
    Ok(())
}

/// `randomNoOp(a)`.
fn random_no_op(a: Automaton) -> Automaton {
    match random().next_int_bound(7) {
        0 => {
            if verbose() {
                println!("  randomNoOp: determinize");
            }
            det(Operations::determinize(&a, i32::MAX))
        }
        1 => {
            if a.get_num_states() < 100 {
                if verbose() {
                    println!("  randomNoOp: minimize");
                }
                MinimizationOperations::minimize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT)
            } else {
                if verbose() {
                    println!(
                        "  randomNoOp: skip op=minimize: too many states ({})",
                        a.get_num_states()
                    );
                }
                a
            }
        }
        2 => {
            if verbose() {
                println!("  randomNoOp: removeDeadStates");
            }
            Operations::remove_dead_states(&a)
        }
        3 => {
            if verbose() {
                println!("  randomNoOp: reverse reverse");
            }
            let a = Operations::reverse(&a);
            let a = random_no_op(a);
            Operations::reverse(&a)
        }
        4 => {
            if verbose() {
                println!("  randomNoOp: concat empty string");
            }
            Operations::concatenate(&[a, Automata::make_empty_string()])
        }
        5 => {
            if verbose() {
                println!("  randomNoOp: union empty automaton");
            }
            Operations::union(&[a, Automata::make_empty()])
        }
        6 => {
            if verbose() {
                println!("  randomNoOp: do nothing!");
            }
            a
        }
        _ => unreachable!("assert false"),
    }
}

/// `hasMassiveTerm(terms)`.
fn has_massive_term<'a>(terms: impl IntoIterator<Item = &'a BytesRef>) -> bool {
    terms
        .into_iter()
        .any(|term| term.length > MAX_STRING_UNION_TERM_LENGTH)
}

/// `unionTerms(terms)`.
fn union_terms(terms: &[BytesRef]) -> Result<Automaton> {
    let a = if random().next_boolean() || has_massive_term(terms) {
        if verbose() {
            println!("TEST: unionTerms: use union");
        }
        let mut as_ = Vec::new();
        for term in terms {
            as_.push(Automata::make_string(&term.utf8_to_string()?));
        }
        Operations::union(&as_)
    } else {
        if verbose() {
            println!("TEST: unionTerms: use makeStringUnion");
        }
        let mut terms_list: Vec<BytesRef> = terms.to_vec();
        terms_list.sort();
        Automata::make_string_union(terms_list)?
    };

    Ok(random_no_op(a))
}

/// `getRandomString()`.
fn get_random_string() -> String {
    // return TestUtil.randomSimpleString(random());
    TestUtil::random_realistic_unicode_string(&*random())
}

/// `newBytesRef(new StringBuilder(s).reverse().toString())`: the reverse
/// of a string reverses its code points (surrogate pairs kept).
fn reverse_string(s: &str) -> String {
    s.chars().rev().collect()
}

fn test_random_finite() -> Result<()> {
    let num_terms = LuceneTestCase::at_least(10);
    let iters = LuceneTestCase::at_least(100);

    if verbose() {
        println!("TEST: numTerms={num_terms} iters={iters}");
    }

    let mut terms: JavaHashSet<BytesRef> = JavaHashSet::new();
    while terms.size() < num_terms as usize {
        terms.add(new_bytes_ref_str(&get_random_string()));
    }

    let mut a = union_terms(&terms.to_vec())?;
    assert_same(&terms, &a)?;

    for iter in 0..iters {
        if verbose() {
            println!(
                "TEST: iter={iter} numTerms={} a.numStates={}",
                terms.size(),
                a.get_num_states()
            );
        }
        match random().next_int_bound(15) {
            0 => {
                // concatenate prefix
                if verbose() {
                    println!("  op=concat prefix");
                }
                let mut new_terms = JavaHashSet::new();
                let prefix = new_bytes_ref_str(&get_random_string());
                let mut new_term = BytesRefBuilder::new();
                for term in terms.iter() {
                    new_term.copy_ref(&prefix);
                    new_term.append_ref(term);
                    new_terms.add(new_term.to_bytes_ref());
                }
                terms = new_terms;
                let was_deterministic = a.is_deterministic();
                a = Operations::concatenate(&[Automata::make_string(&utf8(&prefix)), a]);
                if was_deterministic {
                    assert_equals(&was_deterministic, &a.is_deterministic());
                }
            }
            1 => {
                // concatenate suffix
                let suffix = new_bytes_ref_str(&get_random_string());
                if verbose() {
                    println!("  op=concat suffix {suffix}");
                }
                let mut new_terms = JavaHashSet::new();
                let mut new_term = BytesRefBuilder::new();
                for term in terms.iter() {
                    new_term.copy_ref(term);
                    new_term.append_ref(&suffix);
                    new_terms.add(new_term.to_bytes_ref());
                }
                terms = new_terms;
                a = Operations::concatenate(&[a, Automata::make_string(&utf8(&suffix))]);
            }
            2 => {
                // determinize
                if verbose() {
                    println!("  op=determinize");
                }
                a = det(Operations::determinize(&a, i32::MAX));
                Assert::assert_true(a.is_deterministic());
            }
            3 => {
                if a.get_num_states() < 100 {
                    if verbose() {
                        println!("  op=minimize");
                    }
                    // minimize
                    a = MinimizationOperations::minimize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT);
                } else if verbose() {
                    println!(
                        "  skip op=minimize: too many states ({})",
                        a.get_num_states()
                    );
                }
            }
            4 => {
                // union
                if verbose() {
                    println!("  op=union");
                }
                let mut new_terms = JavaHashSet::new();
                let num_new_terms = random().next_int_bound(5);
                while new_terms.size() < num_new_terms as usize {
                    new_terms.add(new_bytes_ref_str(&get_random_string()));
                }
                terms.add_all(new_terms.to_vec());
                let new_a = union_terms(&new_terms.to_vec())?;
                a = Operations::union(&[a, new_a]);
            }
            5 => {
                // optional
                if verbose() {
                    println!("  op=optional");
                }
                // NOTE: This can add a dead state:
                a = Operations::optional(&a);
                terms.add(new_bytes_ref());
            }
            6 => {
                // minus finite
                if verbose() {
                    println!("  op=minus finite");
                }
                if terms.size() > 0 {
                    let live = Operations::remove_dead_states(&a);
                    let rasl = RandomAcceptedStrings::new(&live);
                    let mut to_remove = JavaHashSet::new();
                    let num_to_remove =
                        TestUtil::next_int(&*random(), 1, (terms.size() as i32 + 1) / 2);
                    while to_remove.size() < num_to_remove as usize {
                        let ints = rasl.get_random_accepted_string(&*random());
                        let term =
                            new_bytes_ref_str(&UnicodeUtil::new_string(&ints, 0, ints.len())?);
                        if !to_remove.contains(&term) {
                            to_remove.add(term);
                        }
                    }
                    for term in to_remove.iter() {
                        let removed = terms.remove(term);
                        Assert::assert_true(removed);
                    }
                    let a2 = union_terms(&to_remove.to_vec())?;
                    a = det(Operations::minus(&a, &a2, i32::MAX));
                }
            }
            7 => {
                // minus infinite
                let mut as_ = Vec::new();
                let count = TestUtil::next_int(&*random(), 1, 5);
                let mut prefixes: JavaHashSet<i32> = JavaHashSet::new();
                while prefixes.size() < count as usize {
                    // prefix is a leading ascii byte; we remove <prefix>* from a
                    let prefix = random().next_int_bound(128);
                    prefixes.add(prefix);
                }

                if verbose() {
                    println!("  op=minus infinite prefixes={:?}", prefixes.to_vec());
                }

                for &prefix in prefixes.to_vec().iter() {
                    // prefix is a leading ascii byte; we remove <prefix>* from a
                    let mut a2 = Automaton::new();
                    let init = a2.create_state();
                    let state = a2.create_state();
                    a2.add_transition(init, state, prefix);
                    a2.set_accept(state, true);
                    a2.add_transition_range(state, state, MIN_CODE_POINT, MAX_CODE_POINT);
                    a2.finish_state();
                    as_.push(a2);
                    terms.retain(|term| {
                        !(term.length > 0 && (term.bytes[term.offset] as i32) == prefix)
                    });
                }
                let a2 = random_no_op(Operations::union(&as_));
                a = det(Operations::minus(&a, &a2, DEFAULT_DETERMINIZE_WORK_LIMIT));
            }
            8 => {
                let count = TestUtil::next_int(&*random(), 10, 20);
                if verbose() {
                    println!("  op=intersect infinite count={count}");
                }
                // intersect infinite
                let mut as_ = Vec::new();

                let mut prefixes: JavaHashSet<i32> = JavaHashSet::new();
                while prefixes.size() < count as usize {
                    let prefix = random().next_int_bound(128);
                    prefixes.add(prefix);
                }
                if verbose() {
                    println!("  prefixes={:?}", prefixes.to_vec());
                }

                for prefix in prefixes.to_vec() {
                    // prefix is a leading ascii byte; we retain <prefix>* in a
                    let mut a2 = Automaton::new();
                    let init = a2.create_state();
                    let state = a2.create_state();
                    a2.add_transition(init, state, prefix);
                    a2.set_accept(state, true);
                    a2.add_transition_range(state, state, MIN_CODE_POINT, MAX_CODE_POINT);
                    a2.finish_state();
                    as_.push(a2);
                    prefixes.add(prefix);
                }

                let mut a2 = Operations::union(&as_);
                if random().next_boolean() {
                    a2 = det(Operations::determinize(&a2, DEFAULT_DETERMINIZE_WORK_LIMIT));
                } else if random().next_boolean() {
                    a2 = MinimizationOperations::minimize(&a2, DEFAULT_DETERMINIZE_WORK_LIMIT);
                }
                a = Operations::intersection(&a, &a2);

                terms.retain(|term| {
                    if term.length == 0 || !prefixes.contains(&(term.bytes[term.offset] as i32)) {
                        if verbose() {
                            println!("  drop term={term}");
                        }
                        false
                    } else {
                        if verbose() {
                            println!("  keep term={term}");
                        }
                        true
                    }
                });
            }
            9 => {
                // reverse
                if verbose() {
                    println!("  op=reverse");
                }
                a = Operations::reverse(&a);
                let mut new_terms = JavaHashSet::new();
                for term in terms.iter() {
                    new_terms.add(new_bytes_ref_str(&reverse_string(&utf8(term))));
                }
                terms = new_terms;
            }
            10 => {
                if verbose() {
                    println!("  op=randomNoOp");
                }
                a = random_no_op(a);
            }
            11 => {
                // interval
                let min = random().next_int_bound(1000);
                let max = min + random().next_int_bound(50);
                // digits must be non-zero else we make cycle
                let digits = max.to_string().len() as i32;
                if verbose() {
                    println!("  op=union interval min={min} max={max} digits={digits}");
                }
                a = Operations::union(&[a, Automata::make_decimal_interval(min, max, digits)?]);
                let mut b = String::new();
                for _ in 0..digits {
                    b.push('0');
                }
                let prefix = b;
                for i in min..=max {
                    let mut s = i.to_string();
                    if (s.len() as i32) < digits {
                        // Left-fill with 0s
                        s = format!("{}{}", &prefix[s.len()..], s);
                    }
                    terms.add(new_bytes_ref_str(&s));
                }
            }
            12 => {
                if verbose() {
                    println!("  op=remove the empty string");
                }
                a = det(Operations::minus(
                    &a,
                    &Automata::make_empty_string(),
                    DEFAULT_DETERMINIZE_WORK_LIMIT,
                ));
                terms.remove(&new_bytes_ref());
            }
            13 => {
                if verbose() {
                    println!("  op=add the empty string");
                }
                a = Operations::union(&[a, Automata::make_empty_string()]);
                terms.add(new_bytes_ref());
            }
            14 => {
                // Safety in case we are really unlucky w/ the dice:
                if terms.size() <= (num_terms * 3) as usize {
                    if verbose() {
                        println!("  op=concat finite automaton");
                    }
                    let count = if random().next_boolean() { 2 } else { 3 };
                    let mut add_terms = JavaHashSet::new();
                    while add_terms.size() < count {
                        add_terms.add(new_bytes_ref_str(&get_random_string()));
                    }
                    if verbose() {
                        for term in add_terms.iter() {
                            println!("    term={term}");
                        }
                    }
                    let a2 = union_terms(&add_terms.to_vec())?;
                    let mut new_terms = JavaHashSet::new();
                    if random().next_boolean() {
                        // suffix
                        if verbose() {
                            println!("  do suffix");
                        }
                        a = Operations::concatenate(&[a, random_no_op(a2)]);
                        let mut new_term = BytesRefBuilder::new();
                        for term in terms.iter() {
                            for suffix in add_terms.iter() {
                                new_term.copy_ref(term);
                                new_term.append_ref(suffix);
                                new_terms.add(new_term.to_bytes_ref());
                            }
                        }
                    } else {
                        // prefix
                        if verbose() {
                            println!("  do prefix");
                        }
                        a = Operations::concatenate(&[random_no_op(a2), a]);
                        let mut new_term = BytesRefBuilder::new();
                        for term in terms.iter() {
                            for prefix in add_terms.iter() {
                                new_term.copy_ref(prefix);
                                new_term.append_ref(term);
                                new_terms.add(new_term.to_bytes_ref());
                            }
                        }
                    }

                    terms = new_terms;
                }
            }
            _ => unreachable!("throw new AssertionError()"),
        }

        assert_same(&terms, &a)?;
        assert_equals(
            &AutomatonTestUtil::is_deterministic_slow(&a),
            &a.is_deterministic(),
        );

        if random().next_int_bound(10) == 7 {
            a = verify_topo_sort(&a)?;
        }
    }

    assert_same(&terms, &a)?;
    Ok(())
}

/// `verifyTopoSort(a)`: runs topo sort, verifies transitions then only "go
/// forwards", and builds and returns new automaton with those remapped
/// toposorted states.
fn verify_topo_sort(a: &Automaton) -> Result<Automaton> {
    let sorted = Operations::topo_sort_states(a)?;
    // This can be < if we removed dead states:
    Assert::assert_true(sorted.len() as i32 <= a.get_num_states());
    let mut a2 = Automaton::new();
    let mut state_map = vec![-1i32; a.get_num_states() as usize];
    let mut transition = Transition::new();
    for &state in &sorted {
        let new_state = a2.create_state();
        a2.set_accept(new_state, a.is_accept(state));

        // Each state should only appear once in the sort:
        assert_equals(&-1, &state_map[state as usize]);
        state_map[state as usize] = new_state;
    }

    // 2nd pass: add new transitions
    for &state in &sorted {
        let count = a.init_transition(state, &mut transition);
        for _ in 0..count {
            a.get_next_transition(&mut transition);
            assert!(state_map[transition.dest as usize] > state_map[state as usize]);
            a2.add_transition_range(
                state_map[state as usize],
                state_map[transition.dest as usize],
                transition.min,
                transition.max,
            );
        }
    }

    a2.finish_state();
    Ok(a2)
}

/// `assertSame(terms, a)`.
fn assert_same(terms: &JavaHashSet<BytesRef>, a: &Automaton) -> Result<()> {
    let outcome = try_catch("java.lang.AssertionError", || -> Result<()> {
        Assert::assert_true(AutomatonTestUtil::is_finite(a));
        Assert::assert_false(Operations::is_total(a));

        let det_a = det(Operations::determinize(a, DEFAULT_DETERMINIZE_WORK_LIMIT));

        // Make sure all terms are accepted:
        let mut scratch = IntsRefBuilder::new();
        for term in terms.iter() {
            Util::to_ints_ref(term, &mut scratch);
            Assert::assert_true_msg(
                &format!("failed to accept term={}", utf8(term)),
                Operations::run(&det_a, &utf8(term)),
            );
        }

        // Use getFiniteStrings:
        let mut expected = JavaHashSet::new();
        for term in terms.iter() {
            let mut ints_ref = IntsRefBuilder::new();
            Util::to_utf32(&utf8(term), &mut ints_ref);
            expected.add(ints_ref.to_ints_ref());
        }
        let actual = get_finite_strings(a);

        if expected != actual {
            println!("FAILED:");
            for term in expected.iter() {
                if !actual.contains(term) {
                    println!("  term={term} should be accepted but isn't");
                }
            }
            for term in actual.iter() {
                if !expected.contains(term) {
                    println!("  term={term} is accepted but should not be");
                }
            }
            Assert::fail("mismatch");
        }

        // Use sameLanguage:
        let a2 = Operations::remove_dead_states(&det(Operations::determinize(
            &union_terms(&terms.to_vec())?,
            i32::MAX,
        )));
        Assert::assert_true(AutomatonTestUtil::same_language(
            &a2,
            &Operations::remove_dead_states(&det(Operations::determinize(a, i32::MAX))),
        ));

        // Do same check, in UTF8 space
        let utf8_a = random_no_op(UTF32ToUTF8::new().convert(a));

        let mut expected2 = JavaHashSet::new();
        for term in terms.iter() {
            let mut ints_ref = IntsRefBuilder::new();
            Util::to_ints_ref(term, &mut ints_ref);
            expected2.add(ints_ref.to_ints_ref());
        }
        assert_equals(&expected2, &get_finite_strings(&utf8_a));
        Ok(())
    });
    match outcome {
        Ok(r) => r,
        Err(ae) => {
            println!("TEST: FAILED: not same");
            println!("  terms (count={}):", terms.size());
            for term in terms.iter() {
                println!("    {term}");
            }
            println!("  automaton:");
            println!("{}", a.to_dot());
            // a.writeDot("fail");
            ae.throw()
        }
    }
}

/// `accepts(a, b)`.
fn accepts(a: &Automaton, b: &BytesRef) -> bool {
    let mut ints_builder = IntsRefBuilder::new();
    Util::to_ints_ref(b, &mut ints_builder);
    Operations::run_ints(a, &ints_builder.to_ints_ref())
}

/// `makeBinaryInterval(minTerm, minInclusive, maxTerm, maxInclusive)`.
fn make_binary_interval(
    min_term: Option<&BytesRef>,
    min_inclusive: bool,
    max_term: Option<&BytesRef>,
    max_inclusive: bool,
) -> Result<Automaton> {
    let show = |t: Option<&BytesRef>| t.map_or("null".to_string(), |t| t.to_string());
    if verbose() {
        println!(
            "TEST: minTerm={} minInclusive={min_inclusive} maxTerm={} maxInclusive={max_inclusive}",
            show(min_term),
            show(max_term)
        );
    }

    let a = Automata::make_binary_interval(min_term, min_inclusive, max_term, max_inclusive)?;

    let min_a = MinimizationOperations::minimize(&a, i32::MAX);
    if min_a.get_num_states() != a.get_num_states() {
        Assert::assert_true(min_a.get_num_states() < a.get_num_states());
        println!("Original was not minimal:");
        println!("Original:\n{}", a.to_dot());
        println!("Minimized:\n{}", min_a.to_dot());
        println!("minTerm={} minInclusive={min_inclusive}", show(min_term));
        println!("maxTerm={} maxInclusive={max_inclusive}", show(max_term));
        Assert::fail("automaton was not minimal");
    }

    if verbose() {
        println!("{}", a.to_dot());
    }

    Ok(a)
}

fn test_make_binary_interval_finite_cases_basic() -> Result<()> {
    // 0 (incl) - 00 (incl)
    let zeros = [0u8; 3];
    let nbr = |len: usize| LuceneTestCase::new_bytes_ref_slice(&zeros, 0, len);
    let mut a = make_binary_interval(Some(&nbr(1)), true, Some(&nbr(2)), true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(AutomatonTestUtil::is_finite(&a));
    Assert::assert_false(accepts(&a, &new_bytes_ref()));
    Assert::assert_true(accepts(&a, &nbr(1)));
    Assert::assert_true(accepts(&a, &nbr(2)));
    Assert::assert_false(accepts(&a, &nbr(3)));

    // '' (incl) - 00 (incl)
    a = make_binary_interval(Some(&new_bytes_ref()), true, Some(&nbr(2)), true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(AutomatonTestUtil::is_finite(&a));
    Assert::assert_true(accepts(&a, &new_bytes_ref()));
    Assert::assert_true(accepts(&a, &nbr(1)));
    Assert::assert_true(accepts(&a, &nbr(2)));
    Assert::assert_false(accepts(&a, &nbr(3)));

    // '' (excl) - 00 (incl)
    a = make_binary_interval(Some(&new_bytes_ref()), false, Some(&nbr(2)), true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(AutomatonTestUtil::is_finite(&a));
    Assert::assert_false(accepts(&a, &new_bytes_ref()));
    Assert::assert_true(accepts(&a, &nbr(1)));
    Assert::assert_true(accepts(&a, &nbr(2)));
    Assert::assert_false(accepts(&a, &nbr(3)));

    // 0 (excl) - 00 (incl)
    a = make_binary_interval(Some(&nbr(1)), false, Some(&nbr(2)), true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(AutomatonTestUtil::is_finite(&a));
    Assert::assert_false(accepts(&a, &new_bytes_ref()));
    Assert::assert_false(accepts(&a, &nbr(1)));
    Assert::assert_true(accepts(&a, &nbr(2)));
    Assert::assert_false(accepts(&a, &nbr(3)));

    // 0 (excl) - 00 (excl)
    a = make_binary_interval(Some(&nbr(1)), false, Some(&nbr(2)), false)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(AutomatonTestUtil::is_finite(&a));
    Assert::assert_false(accepts(&a, &new_bytes_ref()));
    Assert::assert_false(accepts(&a, &nbr(1)));
    Assert::assert_false(accepts(&a, &nbr(2)));
    Assert::assert_false(accepts(&a, &nbr(3)));
    Ok(())
}

fn test_make_binary_interval_finite_cases_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let prefix = new_bytes_ref_str(&TestUtil::random_realistic_unicode_string(&*random()));

        let mut b = BytesRefBuilder::new();
        b.append_ref(&prefix);
        let mut num_zeros = random().next_int_bound(10);
        for _ in 0..num_zeros {
            b.append(0);
        }
        let min_term = b.get();

        b = BytesRefBuilder::new();
        b.append_ref(&min_term);
        num_zeros = random().next_int_bound(10);
        for _ in 0..num_zeros {
            b.append(0);
        }
        let max_term = b.get();

        let min_inclusive = random().next_boolean();
        let max_inclusive = random().next_boolean();
        let a = make_binary_interval(
            Some(&min_term),
            min_inclusive,
            Some(&max_term),
            max_inclusive,
        )?;
        Assert::assert_true(AutomatonTestUtil::is_finite(&a));
        let mut expected_count = max_term.length as i32 - min_term.length as i32 + 1;
        if !min_inclusive {
            expected_count -= 1;
        }
        if !max_inclusive {
            expected_count -= 1;
        }

        if expected_count <= 0 {
            Assert::assert_true(Operations::is_empty(&a));
            continue;
        } else {
            // Enumerate all finite strings and verify the count matches what we expect:
            assert_equals(
                &(expected_count as usize),
                &get_finite_strings_limit(&a, expected_count).size(),
            );
        }

        b = BytesRefBuilder::new();
        b.append_ref(&min_term);
        if !min_inclusive {
            Assert::assert_false(accepts(&a, &b.get()));
            b.append(0);
        }
        while b.length() < max_term.length {
            b.append(0);

            let expected = if b.length() == max_term.length {
                max_inclusive
            } else {
                true
            };
            assert_equals(&expected, &accepts(&a, &b.get()));
        }
    }
    Ok(())
}

fn test_make_binary_interval_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let min_term = TestUtil::random_binary_term(&*random());
        let min_inclusive = random().next_boolean();
        let max_term = TestUtil::random_binary_term(&*random());
        let max_inclusive = random().next_boolean();

        let a = make_binary_interval(
            Some(&min_term),
            min_inclusive,
            Some(&max_term),
            max_inclusive,
        )?;

        for _iter2 in 0..500 {
            let term = TestUtil::random_binary_term(&*random());
            let min_cmp = min_term.cmp(&term) as i32;
            let max_cmp = max_term.cmp(&term) as i32;

            let expected = if min_cmp > 0 || max_cmp < 0 {
                false
            } else if min_cmp == 0 && max_cmp == 0 {
                min_inclusive && max_inclusive
            } else if min_cmp == 0 {
                min_inclusive
            } else if max_cmp == 0 {
                max_inclusive
            } else {
                true
            };

            if verbose() {
                println!("  check term={term} expected={expected}");
            }
            let mut ints_builder = IntsRefBuilder::new();
            Util::to_ints_ref(&term, &mut ints_builder);
            assert_equals(
                &expected,
                &Operations::run_ints(&a, &ints_builder.to_ints_ref()),
            );
        }
    }
    Ok(())
}

/// `intsRef(s)`.
fn ints_ref(s: &str) -> IntsRef {
    let mut ints_builder = IntsRefBuilder::new();
    Util::to_ints_ref(&new_bytes_ref_str(s), &mut ints_builder);
    ints_builder.to_ints_ref()
}

fn test_make_binary_interval_basic() -> Result<()> {
    let a = Automata::make_binary_interval(
        Some(&new_bytes_ref_str("bar")),
        true,
        Some(&new_bytes_ref_str("foo")),
        true,
    )?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bar")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("foo")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("beep")));
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("baq")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bara")));
    Ok(())
}

fn test_make_binary_interval_lower_bound_empty_string() -> Result<()> {
    let mut a = Automata::make_binary_interval(
        Some(&new_bytes_ref_str("")),
        true,
        Some(&new_bytes_ref_str("bar")),
        true,
    )?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("a")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bar")));
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("bara")));
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("baz")));

    a = Automata::make_binary_interval(
        Some(&new_bytes_ref_str("")),
        false,
        Some(&new_bytes_ref_str("bar")),
        true,
    )?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("a")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bar")));
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("bara")));
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("baz")));
    Ok(())
}

fn test_make_binary_interval_equal() -> Result<()> {
    let a = Automata::make_binary_interval(
        Some(&new_bytes_ref_str("bar")),
        true,
        Some(&new_bytes_ref_str("bar")),
        true,
    )?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bar")));
    Assert::assert_true(AutomatonTestUtil::is_finite(&a));
    assert_equals(&1, &get_finite_strings(&a).size());
    Ok(())
}

fn test_make_binary_interval_common_prefix() -> Result<()> {
    let a = Automata::make_binary_interval(
        Some(&new_bytes_ref_str("bar")),
        true,
        Some(&new_bytes_ref_str("barfoo")),
        true,
    )?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("bam")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bar")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bara")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barf")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barfo")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barfoo")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barfonz")));
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("barfop")));
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("barfoop")));
    Ok(())
}

fn test_make_binary_except_empty() -> Result<()> {
    let a = Automata::make_non_empty_binary();
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("")));
    Assert::assert_true(Operations::run_ints(
        &a,
        &ints_ref(&TestUtil::random_realistic_unicode_string_between(
            &*random(),
            1,
            10,
        )),
    ));
    Ok(())
}

fn test_make_binary_interval_open_max() -> Result<()> {
    let a = Automata::make_binary_interval(Some(&new_bytes_ref_str("bar")), true, None, true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("bam")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bar")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bara")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barf")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barfo")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barfoo")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barfonz")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barfop")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("barfoop")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("zzz")));
    Ok(())
}

fn test_make_binary_interval_open_max_zero_length_min() -> Result<()> {
    // when including min, automaton should accept "a"
    let mut a = Automata::make_binary_interval(Some(&new_bytes_ref_str("")), true, None, true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("a")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("aaaaaa")));
    // excluding min should still accept "a"
    a = Automata::make_binary_interval(Some(&new_bytes_ref_str("")), false, None, true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("a")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("aaaaaa")));
    Ok(())
}

fn test_make_binary_interval_open_min() -> Result<()> {
    let a = Automata::make_binary_interval(None, true, Some(&new_bytes_ref_str("foo")), true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("foz")));
    Assert::assert_false(Operations::run_ints(&a, &ints_ref("zzz")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("foo")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("a")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("aaa")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bz")));
    Ok(())
}

fn test_make_binary_interval_open_both() -> Result<()> {
    let a = Automata::make_binary_interval(None, true, None, true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("foz")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("zzz")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("foo")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("a")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("aaa")));
    Assert::assert_true(Operations::run_ints(&a, &ints_ref("bz")));
    Ok(())
}

fn test_accept_all_empty_string_min() -> Result<()> {
    let a = Automata::make_binary_interval(Some(&new_bytes_ref()), true, None, true)?;
    AutomatonTestUtil::assert_minimal_dfa(&a);
    Assert::assert_true(AutomatonTestUtil::same_language(
        &Automata::make_any_binary(),
        &a,
    ));
    Ok(())
}

/// `toIntsRef(s)`: the code points of `s`.
fn to_ints_ref(s: &str) -> IntsRef {
    let mut b = IntsRefBuilder::new();
    for cp in s.chars() {
        b.append(cp as i32);
    }
    b.get()
}

fn test_get_singleton() -> Result<()> {
    let iters = LuceneTestCase::at_least(10000);
    for _ in 0..iters {
        let s = TestUtil::random_realistic_unicode_string(&*random());
        let a = Automata::make_string(&s);
        assert_equals(&Some(to_ints_ref(&s)), &Operations::get_singleton(&a)?);
    }
    Ok(())
}

fn test_get_singleton_empty_string() -> Result<()> {
    let mut a = Automaton::new();
    let s = a.create_state();
    a.set_accept(s, true);
    a.finish_state();
    assert_equals(&Some(IntsRef::default()), &Operations::get_singleton(&a)?);
    Ok(())
}

fn test_get_singleton_nothing() -> Result<()> {
    let mut a = Automaton::new();
    a.create_state();
    a.finish_state();
    assert_null(&Operations::get_singleton(&a)?);
    Ok(())
}

fn test_get_singleton_two() -> Result<()> {
    let mut a = Automaton::new();
    let s = a.create_state();
    let x = a.create_state();
    a.set_accept(x, true);
    a.add_transition(s, x, 55);
    let y = a.create_state();
    a.set_accept(y, true);
    a.add_transition(s, y, 58);
    a.finish_state();
    assert_null(&Operations::get_singleton(&a)?);
    Ok(())
}

/// `testDeterminizeTooMuchEffort()` (LUCENE-9981).
fn test_determinize_too_much_effort() -> Result<()> {
    // make sure determinize properly aborts, relatively quickly, for this regexp:
    expect_throws_panic(TOO_COMPLEX_TO_DETERMINIZE, || {
        let a = unchecked(unchecked(RegExp::new("(.*a){2000}")).to_automaton());
        det(Operations::determinize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT));
    });
    // ... and for its reversed form too:
    expect_throws_panic(TOO_COMPLEX_TO_DETERMINIZE, || {
        let a = unchecked(unchecked(RegExp::new("(.*a){2000}")).to_automaton());
        let a = Operations::reverse(&a);
        det(Operations::determinize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT));
    });
    Ok(())
}

fn test_make_char_set_empty() -> Result<()> {
    let expected = Automata::make_empty();
    let actual = Automata::make_char_set(&[]);

    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

fn test_make_char_set_one() -> Result<()> {
    let expected = Automata::make_char(c('a'));
    let actual = Automata::make_char_set(&[c('a')]);

    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

fn test_make_char_set_two() -> Result<()> {
    let expected = Operations::union(&[Automata::make_char(c('a')), Automata::make_char(c('A'))]);
    let actual = Automata::make_char_set(&[c('a'), c('A')]);

    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

fn test_make_char_set_dups() -> Result<()> {
    let expected = Automata::make_char(c('a'));
    let actual = Automata::make_char_set(&[c('a'), c('a'), c('a')]);

    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}

fn test_make_char_case_insensitive() -> Result<()> {
    let mut a = Automata::make_case_insensitive_char(c('a'));
    Assert::assert_true(Operations::run(&a, "a"));
    Assert::assert_true(Operations::run(&a, "A"));
    Assert::assert_false(Operations::run(&a, "b"));

    a = Automata::make_case_insensitive_char(c('A'));
    Assert::assert_true(Operations::run(&a, "a"));
    Assert::assert_true(Operations::run(&a, "A"));
    Assert::assert_false(Operations::run(&a, "b"));

    a = Automata::make_case_insensitive_char(c('Σ'));
    Assert::assert_true(Operations::run(&a, "Σ"));
    Assert::assert_true(Operations::run(&a, "σ"));
    Assert::assert_true(Operations::run(&a, "ς"));

    // German sharp S: 'ß' (U+00DF) and 'ẞ' (U+1E9E)
    a = Automata::make_case_insensitive_char(223);
    Assert::assert_true(Operations::run(&a, "\u{DF}"));
    Assert::assert_true(Operations::run(&a, "\u{1E9E}"));

    AutomatonTestUtil::assert_minimal_dfa(&a);
    AutomatonTestUtil::assert_no_detached_states(&a);
    Assert::assert_true(AutomatonTestUtil::is_finite(&a));
    Ok(())
}

fn test_make_string_case_insensitive_simple() -> Result<()> {
    let mut a = Automata::make_case_insensitive_string("abc");
    Assert::assert_true(Operations::run(&a, "abc"));
    Assert::assert_true(Operations::run(&a, "AbC"));
    Assert::assert_false(Operations::run(&a, "ab"));
    Assert::assert_false(Operations::run(&a, "xyz"));

    a = Automata::make_case_insensitive_string("Σίγμα");
    Assert::assert_true(Operations::run(&a, "Σίγμα"));
    Assert::assert_true(Operations::run(&a, "σίγμα"));
    Assert::assert_true(Operations::run(&a, "ςίγμα"));

    AutomatonTestUtil::assert_minimal_dfa(&a);
    AutomatonTestUtil::assert_no_detached_states(&a);
    Assert::assert_true(AutomatonTestUtil::is_finite(&a));
    Ok(())
}
