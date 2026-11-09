//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestRegExpParsing.java`:
//! simple unit tests for `RegExp` parsing. For each type of node, test the
//! `toString()` and parse tree, test the resulting automaton's language, and
//! test properties such as minimal, deterministic, no dead states.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::util::automaton::{
    Automata, Automaton, AutomatonProvider, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, det, test};

/// `TestRegExpParsing`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testAnyChar", test_any_char),
        test("testAnyString", test_any_string),
        test("testChar", test_char),
        test("testCaseInsensitiveChar", test_case_insensitive_char),
        test(
            "testCaseInsensitiveClassChar",
            test_case_insensitive_class_char,
        ),
        test(
            "testCaseInsensitiveClassRangeDisabled",
            test_case_insensitive_class_range_disabled,
        ),
        test(
            "testCaseInsensitiveClassRange",
            test_case_insensitive_class_range,
        ),
        test(
            "testCaseInsensitiveClassRangeCompression",
            test_case_insensitive_class_range_compression,
        ),
        test(
            "testCaseInsensitiveCharUpper",
            test_case_insensitive_char_upper,
        ),
        test(
            "testCaseInsensitiveCharNotSensitive",
            test_case_insensitive_char_not_sensitive,
        ),
        test(
            "testCaseInsensitiveCharNonAscii",
            test_case_insensitive_char_non_ascii,
        ),
        test(
            "testCaseInsensitiveCharUnicode",
            test_case_insensitive_char_unicode,
        ),
        test(
            "testCaseInsensitiveCharUnicodeSigma",
            test_case_insensitive_char_unicode_sigma,
        ),
        test("testNegatedChar", test_negated_char),
        test("testNegatedClass", test_negated_class),
        test("testCharRange", test_char_range),
        test("testNegatedCharRange", test_negated_char_range),
        test("testIllegalCharRange", test_illegal_char_range),
        test("testCharClassDigit", test_char_class_digit),
        test("testCharClassNonDigit", test_char_class_non_digit),
        test("testCharClassWhitespace", test_char_class_whitespace),
        test("testCharClassNonWhitespace", test_char_class_non_whitespace),
        test("testCharClassWord", test_char_class_word),
        test("testCharClassNonWord", test_char_class_non_word),
        test("testJumboCharClass", test_jumbo_char_class),
        test("testTruncatedCharClass", test_truncated_char_class),
        test("testBogusCharClass", test_bogus_char_class),
        test("testExcapedNotCharClass", test_excaped_not_char_class),
        test(
            "testExcapedSlashNotCharClass",
            test_excaped_slash_not_char_class,
        ),
        test("testEscapedDashCharClass", test_escaped_dash_char_class),
        test("testEmpty", test_empty),
        test("testEmptyClass", test_empty_class),
        test("testEscapedInvalidClass", test_escaped_invalid_class),
        test("testInterval", test_interval),
        test("testBackwardsInterval", test_backwards_interval),
        test("testTruncatedInterval", test_truncated_interval),
        test("testTruncatedInterval2", test_truncated_interval2),
        test("testEmptyInterval", test_empty_interval),
        test("testOptional", test_optional),
        test("testRepeat0", test_repeat0),
        test("testRepeat1", test_repeat1),
        test("testRepeatN", test_repeat_n),
        test("testRepeatNPlus", test_repeat_n_plus),
        test("testRepeatMN", test_repeat_mn),
        test("testTruncatedRepeat", test_truncated_repeat),
        test("testBogusRepeat", test_bogus_repeat),
        test("testString", test_string),
        test("testCaseInsensitiveString", test_case_insensitive_string),
        test("testExplicitString", test_explicit_string),
        test("testNotTerminatedString", test_not_terminated_string),
        test("testConcatenation", test_concatenation),
        test("testIntersection", test_intersection),
        test("testTruncatedIntersection", test_truncated_intersection),
        test(
            "testTruncatedIntersectionParens",
            test_truncated_intersection_parens,
        ),
        test("testUnion", test_union),
        test("testTruncatedUnion", test_truncated_union),
        test("testTruncatedUnionParens", test_truncated_union_parens),
        test("testAutomaton", test_automaton),
        test("testAutomatonMap", test_automaton_map),
        test("testAutomatonIOException", test_automaton_io_exception),
        test("testAutomatonNotFound", test_automaton_not_found),
        test("testIllegalSyntaxFlags", test_illegal_syntax_flags),
        test("testIllegalMatchFlags", test_illegal_match_flags),
    ];
    class(
        "org.apache.lucene.util.automaton.TestRegExpParsing",
        methods,
    )
}

const IAE: &str = "java.lang.IllegalArgumentException";

fn c(ch: char) -> i32 {
    ch as i32
}

/// `String.join("\n", lines...)`.
fn join(lines: &[&str]) -> String {
    lines.join("\n")
}

fn test_any_char() -> Result<()> {
    let re = RegExp::new(".")?;
    assert_equals(".", re.to_string().as_str());
    assert_equals("REGEXP_ANYCHAR\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_any_char();
    assert_same_language(expected, actual);
    Ok(())
}

fn test_any_string() -> Result<()> {
    let re = RegExp::with_flags("@", RegExp::ANYSTRING)?;
    assert_equals("@", re.to_string().as_str());
    assert_equals("REGEXP_ANYSTRING\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_any_string();
    assert_same_language(expected, actual);
    Ok(())
}

fn test_char() -> Result<()> {
    let re = RegExp::new("c")?;
    assert_equals("\\c", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=c\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char(c('c'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_case_insensitive_char() -> Result<()> {
    let re = RegExp::with_match_flags("c", RegExp::NONE, RegExp::ASCII_CASE_INSENSITIVE)?;
    assert_equals("\\c", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=c\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char_set(&[c('c'), c('C')]);
    assert_same_language(expected, actual);
    Ok(())
}

/// Individual characters (only) inside a class are treated as case
/// insensitive.
fn test_case_insensitive_class_char() -> Result<()> {
    let re = RegExp::with_match_flags("[c]", RegExp::NONE, RegExp::ASCII_CASE_INSENSITIVE)?;
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0043 U+0063] ends=[U+0043 U+0063]\n",
        re.to_string_tree().as_str(),
    );
    AutomatonTestUtil::assert_minimal_dfa(&re.to_automaton()?);
    Ok(())
}

/// Ranges aren't treated as case-insensitive, unless special flag is
/// provided.
fn test_case_insensitive_class_range_disabled() -> Result<()> {
    let re = RegExp::with_match_flags("[c-d]", RegExp::NONE, RegExp::ASCII_CASE_INSENSITIVE)?;
    assert_equals(
        "REGEXP_CHAR_RANGE from=c to=d\n",
        re.to_string_tree().as_str(),
    );
    AutomatonTestUtil::assert_minimal_dfa(&re.to_automaton()?);
    Ok(())
}

/// Case insensitive range.
fn test_case_insensitive_class_range() -> Result<()> {
    let re = RegExp::with_match_flags("[c-d]", RegExp::NONE, RegExp::CASE_INSENSITIVE_RANGE)?;
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0043 U+0063] ends=[U+0044 U+0064]\n",
        re.to_string_tree().as_str(),
    );
    AutomatonTestUtil::assert_minimal_dfa(&re.to_automaton()?);
    Ok(())
}

/// Common enough that we need to ensure this one is efficient.
fn test_case_insensitive_class_range_compression() -> Result<()> {
    let re = RegExp::with_match_flags("[a-z]", RegExp::NONE, RegExp::CASE_INSENSITIVE_RANGE)?;
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0041 U+0061 U+017F U+212A] ends=[U+005A U+007A U+017F U+212A]\n",
        re.to_string_tree().as_str(),
    );
    AutomatonTestUtil::assert_minimal_dfa(&re.to_automaton()?);
    Ok(())
}

fn test_case_insensitive_char_upper() -> Result<()> {
    let re = RegExp::with_match_flags("C", RegExp::NONE, RegExp::ASCII_CASE_INSENSITIVE)?;
    assert_equals("\\C", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=C\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char_set(&[c('c'), c('C')]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_case_insensitive_char_not_sensitive() -> Result<()> {
    let re = RegExp::with_match_flags("4", RegExp::NONE, RegExp::ASCII_CASE_INSENSITIVE)?;
    assert_equals("\\4", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=4\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char(c('4'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_case_insensitive_char_non_ascii() -> Result<()> {
    let re = RegExp::with_match_flags("Ж", RegExp::NONE, RegExp::ASCII_CASE_INSENSITIVE)?;
    assert_equals("\\Ж", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=Ж\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char_set(&[c('Ж'), c('ж')]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_case_insensitive_char_unicode() -> Result<()> {
    let re = RegExp::with_match_flags("Ж", RegExp::NONE, RegExp::CASE_INSENSITIVE)?;
    assert_equals("\\Ж", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=Ж\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char_set(&[c('Ж'), c('ж')]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_case_insensitive_char_unicode_sigma() -> Result<()> {
    let re = RegExp::with_match_flags("σ", RegExp::NONE, RegExp::CASE_INSENSITIVE)?;
    assert_equals("\\σ", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=σ\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char_set(&[c('Σ'), c('σ'), c('ς')]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_negated_char() -> Result<()> {
    let re = RegExp::new("[^c]")?;
    // TODO: would be nice to emit negated class rather than this
    assert_equals("(.&~(\\c))", re.to_string().as_str());
    assert_equals(
        join(&[
            "REGEXP_INTERSECTION",
            "  REGEXP_ANYCHAR",
            "  REGEXP_COMPLEMENT",
            "    REGEXP_CHAR char=c\n",
        ])
        .as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::union(&[
        Automata::make_char_range(0, c('b')),
        Automata::make_char_range(c('d'), i32::MAX),
    ]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_negated_class() -> Result<()> {
    let re = RegExp::new("[^c-da]")?;
    assert_equals(
        join(&[
            "REGEXP_INTERSECTION",
            "  REGEXP_ANYCHAR",
            "  REGEXP_COMPLEMENT",
            "    REGEXP_CHAR_CLASS starts=[U+0063 U+0061] ends=[U+0064 U+0061]\n",
        ])
        .as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Ok(())
}

fn test_char_range() -> Result<()> {
    let re = RegExp::new("[b-d]")?;
    assert_equals("[\\b-\\d]", re.to_string().as_str());
    assert_equals(
        "REGEXP_CHAR_RANGE from=b to=d\n",
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char_range(c('b'), c('d'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_negated_char_range() -> Result<()> {
    let re = RegExp::new("[^b-d]")?;
    // TODO: would be nice to emit negated class rather than this
    assert_equals("(.&~([\\b-\\d]))", re.to_string().as_str());
    assert_equals(
        join(&[
            "REGEXP_INTERSECTION",
            "  REGEXP_ANYCHAR",
            "  REGEXP_COMPLEMENT",
            "    REGEXP_CHAR_RANGE from=b to=d\n",
        ])
        .as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::union(&[
        Automata::make_char_range(0, c('a')),
        Automata::make_char_range(c('e'), i32::MAX),
    ]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_illegal_char_range() -> Result<()> {
    expect_throws(IAE, || RegExp::new("[z-a]"));
    Ok(())
}

fn test_char_class_digit() -> Result<()> {
    let re = RegExp::new("[\\d]")?;
    assert_equals("[\\0-\\9]", re.to_string().as_str());
    assert_equals(
        "REGEXP_CHAR_RANGE from=0 to=9\n",
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char_range(c('0'), c('9'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_char_class_non_digit() -> Result<()> {
    let re = RegExp::new("[\\D]")?;
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0000 U+003A] ends=[U+002F U+10FFFF]\n",
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = det(Operations::minus(
        &Automata::make_any_char(),
        &Automata::make_char_range(c('0'), c('9')),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_char_class_whitespace() -> Result<()> {
    let re = RegExp::new("[\\s]")?;
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0009 U+000D U+0020] ends=[U+000A U+000D U+0020]\n",
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::union(&[
        Automata::make_char(c(' ')),
        Automata::make_char(c('\n')),
        Automata::make_char(c('\r')),
        Automata::make_char(c('\t')),
    ]);
    assert_same_language(expected, actual);
    Ok(())
}

/// `Operations.minus(expected, x, DEFAULT_DETERMINIZE_WORK_LIMIT)`.
fn minus(expected: &Automaton, x: Automaton) -> Automaton {
    det(Operations::minus(
        expected,
        &x,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ))
}

fn test_char_class_non_whitespace() -> Result<()> {
    let re = RegExp::new("[\\S]")?;
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0000 U+000B U+000E U+0021] ends=[U+0008 U+000C U+001F U+10FFFF]\n",
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let mut expected = Automata::make_any_char();
    expected = minus(&expected, Automata::make_char(c(' ')));
    expected = minus(&expected, Automata::make_char(c('\n')));
    expected = minus(&expected, Automata::make_char(c('\r')));
    expected = minus(&expected, Automata::make_char(c('\t')));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_char_class_word() -> Result<()> {
    let re = RegExp::new("[\\w]")?;
    assert_equals("[\\0-\\9\\A-\\Z\\_\\a-\\z]", re.to_string().as_str());
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0030 U+0041 U+005F U+0061] ends=[U+0039 U+005A U+005F U+007A]\n",
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::union(&[
        Automata::make_char_range(c('a'), c('z')),
        Automata::make_char_range(c('A'), c('Z')),
        Automata::make_char_range(c('0'), c('9')),
        Automata::make_char(c('_')),
    ]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_char_class_non_word() -> Result<()> {
    let re = RegExp::new("[\\W]")?;
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0000 U+003A U+005B U+0060 U+007B] ends=[U+002F U+0040 U+005E U+0060 U+10FFFF]\n",
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let mut expected = Automata::make_any_char();
    expected = minus(&expected, Automata::make_char_range(c('a'), c('z')));
    expected = minus(&expected, Automata::make_char_range(c('A'), c('Z')));
    expected = minus(&expected, Automata::make_char_range(c('0'), c('9')));
    expected = minus(&expected, Automata::make_char(c('_')));
    assert_same_language(expected, actual);
    Ok(())
}

/// Char class with a couple of ranges, predefined, and individual chars.
fn test_jumbo_char_class() -> Result<()> {
    let re = RegExp::new("[0-5a\\sbc-d]")?;
    assert_equals(
        "REGEXP_CHAR_CLASS starts=[U+0030 U+0061 U+0009 U+000D U+0020 U+0062 U+0063] ends=[U+0035 U+0061 U+000A U+000D U+0020 U+0062 U+0064]\n",
        re.to_string_tree().as_str(),
    );
    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);
    Ok(())
}

fn test_truncated_char_class() -> Result<()> {
    expect_throws(IAE, || RegExp::new("[b-d"));
    Ok(())
}

fn test_bogus_char_class() -> Result<()> {
    expect_throws(IAE, || RegExp::new("[\\q]"));
    Ok(())
}

fn test_excaped_not_char_class() -> Result<()> {
    let re = RegExp::new("[\\?]")?;
    assert_equals("\\?", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=?\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char(c('?'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_excaped_slash_not_char_class() -> Result<()> {
    let re = RegExp::new("[\\\\]")?;
    assert_equals("\\\\", re.to_string().as_str());
    assert_equals("REGEXP_CHAR char=\\\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char(c('\\'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_escaped_dash_char_class() -> Result<()> {
    let re = RegExp::new("[\\-]")?;
    assert_equals("REGEXP_CHAR char=-\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char(c('-'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_empty() -> Result<()> {
    let re = RegExp::with_flags("#", RegExp::EMPTY)?;
    assert_equals("#", re.to_string().as_str());
    assert_equals("REGEXP_EMPTY\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_empty();
    assert_same_language(expected, actual);
    Ok(())
}

fn test_empty_class() -> Result<()> {
    let expected = expect_throws(IAE, || RegExp::new("[]"));
    assert_equals(&Some("expected ']' at position 2"), &expected.message());
    Ok(())
}

fn test_escaped_invalid_class() -> Result<()> {
    let expected = expect_throws(IAE, || RegExp::new("[\\]"));
    assert_equals(&Some("expected ']' at position 3"), &expected.message());
    Ok(())
}

fn test_interval() -> Result<()> {
    let re = RegExp::new("<5-40>")?;
    assert_equals("<5-40>", re.to_string().as_str());
    assert_equals("REGEXP_INTERVAL<5-40>\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    // TODO: numeric intervals are NFAs
    AutomatonTestUtil::assert_clean_nfa(&actual);

    let expected = Automata::make_decimal_interval(5, 40, 0)?;
    assert_same_language(expected, actual);
    Ok(())
}

fn test_backwards_interval() -> Result<()> {
    let re = RegExp::new("<40-5>")?;
    assert_equals("<5-40>", re.to_string().as_str());
    assert_equals("REGEXP_INTERVAL<5-40>\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    // TODO: numeric intervals are NFAs
    AutomatonTestUtil::assert_clean_nfa(&actual);

    let expected = Automata::make_decimal_interval(5, 40, 0)?;
    assert_same_language(expected, actual);
    Ok(())
}

fn test_truncated_interval() -> Result<()> {
    expect_throws(IAE, || RegExp::new("<1-"));
    Ok(())
}

fn test_truncated_interval2() -> Result<()> {
    expect_throws(IAE, || RegExp::new("<1"));
    Ok(())
}

fn test_empty_interval() -> Result<()> {
    expect_throws(IAE, || RegExp::new("<->"));
    Ok(())
}

fn test_optional() -> Result<()> {
    let re = RegExp::new("a?")?;
    assert_equals("(\\a)?", re.to_string().as_str());
    assert_equals(
        join(&["REGEXP_OPTIONAL", "  REGEXP_CHAR char=a\n"]).as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::optional(&Automata::make_char(c('a')));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_repeat0() -> Result<()> {
    let re = RegExp::new("a*")?;
    assert_equals("(\\a)*", re.to_string().as_str());
    assert_equals(
        join(&["REGEXP_REPEAT", "  REGEXP_CHAR char=a\n"]).as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::repeat(&Automata::make_char(c('a')));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_repeat1() -> Result<()> {
    let re = RegExp::new("a+")?;
    assert_equals("(\\a){1,}", re.to_string().as_str());
    assert_equals(
        join(&["REGEXP_REPEAT_MIN min=1", "  REGEXP_CHAR char=a\n"]).as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    // not minimal, but close: minimal + 1
    assert_equals(&3, &actual.get_num_states());
    AutomatonTestUtil::assert_clean_dfa(&actual);

    let expected = Operations::repeat_min(&Automata::make_char(c('a')), 1);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_repeat_n() -> Result<()> {
    let re = RegExp::new("a{5}")?;
    assert_equals("(\\a){5,5}", re.to_string().as_str());
    assert_equals(
        join(&["REGEXP_REPEAT_MINMAX min=5 max=5", "  REGEXP_CHAR char=a\n"]).as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::repeat_min_max(&Automata::make_char(c('a')), 5, 5);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_repeat_n_plus() -> Result<()> {
    let re = RegExp::new("a{5,}")?;
    assert_equals("(\\a){5,}", re.to_string().as_str());
    assert_equals(
        join(&["REGEXP_REPEAT_MIN min=5", "  REGEXP_CHAR char=a\n"]).as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    // not minimal, but close: minimal + 1
    assert_equals(&7, &actual.get_num_states());
    AutomatonTestUtil::assert_clean_dfa(&actual);

    let expected = Operations::repeat_min(&Automata::make_char(c('a')), 5);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_repeat_mn() -> Result<()> {
    let re = RegExp::new("a{5,8}")?;
    assert_equals("(\\a){5,8}", re.to_string().as_str());
    assert_equals(
        join(&["REGEXP_REPEAT_MINMAX min=5 max=8", "  REGEXP_CHAR char=a\n"]).as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::repeat_min_max(&Automata::make_char(c('a')), 5, 8);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_truncated_repeat() -> Result<()> {
    expect_throws(IAE, || RegExp::new("a{5,8"));
    Ok(())
}

fn test_bogus_repeat() -> Result<()> {
    expect_throws(IAE, || RegExp::new("a{Z}"));
    Ok(())
}

fn test_string() -> Result<()> {
    let re = RegExp::new("boo")?;
    assert_equals("\"boo\"", re.to_string().as_str());
    assert_equals("REGEXP_STRING string=boo\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_string("boo");
    assert_same_language(expected, actual);
    Ok(())
}

fn test_case_insensitive_string() -> Result<()> {
    let re = RegExp::with_match_flags("boo", RegExp::NONE, RegExp::ASCII_CASE_INSENSITIVE)?;
    assert_equals("\"boo\"", re.to_string().as_str());
    assert_equals("REGEXP_STRING string=boo\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let c1 = Operations::union(&[Automata::make_char(c('b')), Automata::make_char(c('B'))]);
    let c2 = Operations::union(&[Automata::make_char(c('o')), Automata::make_char(c('O'))]);

    let expected = Operations::concatenate(&[c1, c2.clone(), c2]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_explicit_string() -> Result<()> {
    let re = RegExp::new("\"boo\"")?;
    assert_equals("\"boo\"", re.to_string().as_str());
    assert_equals("REGEXP_STRING string=boo\n", re.to_string_tree().as_str());

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_string("boo");
    assert_same_language(expected, actual);
    Ok(())
}

fn test_not_terminated_string() -> Result<()> {
    expect_throws(IAE, || RegExp::new("\"boo"));
    Ok(())
}

fn test_concatenation() -> Result<()> {
    let re = RegExp::new("[b-c][e-f]")?;
    assert_equals("[\\b-\\c][\\e-\\f]", re.to_string().as_str());
    assert_equals(
        join(&[
            "REGEXP_CONCATENATION",
            "  REGEXP_CHAR_RANGE from=b to=c",
            "  REGEXP_CHAR_RANGE from=e to=f\n",
        ])
        .as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::concatenate(&[
        Automata::make_char_range(c('b'), c('c')),
        Automata::make_char_range(c('e'), c('f')),
    ]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_intersection() -> Result<()> {
    let re = RegExp::new("[b-f]&[e-f]")?;
    assert_equals("([\\b-\\f]&[\\e-\\f])", re.to_string().as_str());
    assert_equals(
        join(&[
            "REGEXP_INTERSECTION",
            "  REGEXP_CHAR_RANGE from=b to=f",
            "  REGEXP_CHAR_RANGE from=e to=f\n",
        ])
        .as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::intersection(
        &Automata::make_char_range(c('b'), c('f')),
        &Automata::make_char_range(c('e'), c('f')),
    );
    assert_same_language(expected, actual);
    Ok(())
}

fn test_truncated_intersection() -> Result<()> {
    expect_throws(IAE, || RegExp::new("a&"));
    Ok(())
}

fn test_truncated_intersection_parens() -> Result<()> {
    expect_throws(IAE, || RegExp::new("(a)&("));
    Ok(())
}

fn test_union() -> Result<()> {
    let re = RegExp::new("[b-c]|[e-f]")?;
    assert_equals("([\\b-\\c]|[\\e-\\f])", re.to_string().as_str());
    assert_equals(
        join(&[
            "REGEXP_UNION",
            "  REGEXP_CHAR_RANGE from=b to=c",
            "  REGEXP_CHAR_RANGE from=e to=f\n",
        ])
        .as_str(),
        re.to_string_tree().as_str(),
    );

    let actual = re.to_automaton()?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Operations::union(&[
        Automata::make_char_range(c('b'), c('c')),
        Automata::make_char_range(c('e'), c('f')),
    ]);
    assert_same_language(expected, actual);
    Ok(())
}

fn test_truncated_union() -> Result<()> {
    expect_throws(IAE, || RegExp::new("a|"));
    Ok(())
}

fn test_truncated_union_parens() -> Result<()> {
    expect_throws(IAE, || RegExp::new("(a)|("));
    Ok(())
}

/// The anonymous `AutomatonProvider` of `testAutomaton`.
struct MyLetterProvider;

impl AutomatonProvider for MyLetterProvider {
    fn get_automaton(&self, _name: &str) -> Result<Automaton> {
        Ok(Automata::make_char(c('z')))
    }
}

/// The anonymous `AutomatonProvider` of `testAutomatonIOException`.
struct IOExceptionProvider;

impl AutomatonProvider for IOExceptionProvider {
    fn get_automaton(&self, _name: &str) -> Result<Automaton> {
        Err(LuceneError::Io(std::io::Error::other("fake ioexception")))
    }
}

/// `Set.of("myletter")`.
fn myletter_set() -> HashSet<String> {
    HashSet::from(["myletter".to_string()])
}

fn test_automaton() -> Result<()> {
    let my_provider = MyLetterProvider;
    let re = RegExp::with_flags("<myletter>", RegExp::ALL)?;
    assert_equals("<myletter>", re.to_string().as_str());
    assert_equals("REGEXP_AUTOMATON\n", re.to_string_tree().as_str());
    assert_equals(&myletter_set(), &re.get_identifiers());

    let actual = re.to_automaton_with(None, Some(&my_provider))?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char(c('z'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_automaton_map() -> Result<()> {
    let re = RegExp::with_flags("<myletter>", RegExp::ALL)?;
    assert_equals("<myletter>", re.to_string().as_str());
    assert_equals("REGEXP_AUTOMATON\n", re.to_string_tree().as_str());
    assert_equals(&myletter_set(), &re.get_identifiers());

    let map = HashMap::from([("myletter".to_string(), Automata::make_char(c('z')))]);
    let actual = re.to_automaton_with(Some(&map), None)?;
    AutomatonTestUtil::assert_minimal_dfa(&actual);

    let expected = Automata::make_char(c('z'));
    assert_same_language(expected, actual);
    Ok(())
}

fn test_automaton_io_exception() -> Result<()> {
    let my_provider = IOExceptionProvider;
    let re = RegExp::with_flags("<myletter>", RegExp::ALL)?;
    assert_equals("<myletter>", re.to_string().as_str());
    assert_equals("REGEXP_AUTOMATON\n", re.to_string_tree().as_str());
    assert_equals(&myletter_set(), &re.get_identifiers());

    expect_throws(IAE, || re.to_automaton_with(None, Some(&my_provider)));
    Ok(())
}

fn test_automaton_not_found() -> Result<()> {
    let re = RegExp::with_flags("<bogus>", RegExp::ALL)?;
    assert_equals("<bogus>", re.to_string().as_str());
    assert_equals("REGEXP_AUTOMATON\n", re.to_string_tree().as_str());

    let map = HashMap::from([("myletter".to_string(), Automata::make_char(c('z')))]);
    expect_throws(IAE, || re.to_automaton_with(Some(&map), None));
    Ok(())
}

fn test_illegal_syntax_flags() -> Result<()> {
    expect_throws(IAE, || RegExp::with_flags("bogus", i32::MAX));
    Ok(())
}

fn test_illegal_match_flags() -> Result<()> {
    expect_throws(IAE, || RegExp::with_match_flags("bogus", RegExp::ALL, 1));
    Ok(())
}

/// `assertSameLanguage(expected, actual)`.
fn assert_same_language(expected: Automaton, actual: Automaton) {
    let expected = det(Operations::determinize(
        &expected,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    let actual = det(Operations::determinize(
        &actual,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    let result = AutomatonTestUtil::same_language(&expected, &actual);
    if !result {
        println!("{}", expected.to_dot());
        println!("{}", actual.to_dot());
    }
    Assert::assert_true(result);
}
