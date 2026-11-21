//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestRegExp.java`.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::AbstractAnalysisFactory;
use rucene::error::Result;
use rucene::util::automaton::case_folding;
use rucene::util::automaton::{
    Automata, ByteRunAutomaton, ByteRunnable, CharacterRunAutomaton, Operations, RegExp,
    DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, det, hamcrest, random, test, unchecked};

/// `TestRegExp`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testSmoke", test_smoke),
        test(
            "testUnicodeAsciiInsensitiveFlags",
            test_unicode_ascii_insensitive_flags,
        ),
        test("testRepeatWithEmptyString", test_repeat_with_empty_string),
        test(
            "testRepeatWithEmptyLanguage",
            test_repeat_with_empty_language,
        ),
        test("testCoreJavaParity", test_core_java_parity),
        test("testIllegalBackslashChars", test_illegal_backslash_chars),
        test("testLegalBackslashChars", test_legal_backslash_chars),
        test("testParseIllegalRepeatExp", test_parse_illegal_repeat_exp),
        test("testRegExpNoStackOverflow", test_reg_exp_no_stack_overflow),
        test("testDeprecatedComplement", test_deprecated_complement),
    ];
    class("org.apache.lucene.util.automaton.TestRegExp", methods)
}

const IAE: &str = "java.lang.IllegalArgumentException";

/// `new CharacterRunAutomaton(r.toAutomaton()).run(s)`.
fn runs(r: &RegExp, s: &str) -> bool {
    unchecked(CharacterRunAutomaton::new(unchecked(r.to_automaton()))).run(s)
}

/// `testSmoke()`: simple smoke test for regular expression.
fn test_smoke() -> Result<()> {
    let r = RegExp::new("a(b+|c+)d")?;
    let a = r.to_automaton()?;
    Assert::assert_true(a.is_deterministic());
    let run = CharacterRunAutomaton::new(a)?;
    Assert::assert_true(run.run("abbbbbd"));
    Assert::assert_true(run.run("acd"));
    Assert::assert_false(run.run("ad"));
    Ok(())
}

fn test_unicode_ascii_insensitive_flags() -> Result<()> {
    let mut r;
    // ASCII behaves appropriately with different flags
    r = RegExp::new("A")?;
    Assert::assert_false(runs(&r, "a"));

    r = RegExp::with_match_flags("A", RegExp::ALL, RegExp::CASE_INSENSITIVE)?;
    Assert::assert_true(runs(&r, "a"));

    r = RegExp::with_match_flags("A", RegExp::ALL, RegExp::ASCII_CASE_INSENSITIVE)?;
    Assert::assert_true(runs(&r, "a"));

    r = RegExp::with_match_flags(
        "A",
        RegExp::ALL,
        RegExp::ASCII_CASE_INSENSITIVE | RegExp::CASE_INSENSITIVE,
    )?;
    Assert::assert_true(runs(&r, "a"));

    // class 1 Unicode characters behaves appropriately with different flags
    r = RegExp::new("Σ")?;
    Assert::assert_false(runs(&r, "σ"));
    Assert::assert_false(runs(&r, "ς"));

    r = RegExp::new("σ")?;
    Assert::assert_false(runs(&r, "ς"));

    r = RegExp::with_match_flags("Σ", RegExp::ALL, RegExp::ASCII_CASE_INSENSITIVE)?;
    Assert::assert_true(runs(&r, "σ"));
    Assert::assert_true(runs(&r, "ς"));

    r = RegExp::with_match_flags("σ", RegExp::ALL, RegExp::ASCII_CASE_INSENSITIVE)?;
    Assert::assert_true(runs(&r, "ς"));

    r = RegExp::with_match_flags("Σ", RegExp::ALL, RegExp::CASE_INSENSITIVE)?;
    Assert::assert_true(runs(&r, "σ"));
    Assert::assert_true(runs(&r, "ς"));

    r = RegExp::with_match_flags("σ", RegExp::ALL, RegExp::CASE_INSENSITIVE)?;
    Assert::assert_true(runs(&r, "ς"));

    r = RegExp::with_match_flags(
        "Σ",
        RegExp::ALL,
        RegExp::ASCII_CASE_INSENSITIVE | RegExp::CASE_INSENSITIVE,
    )?;
    Assert::assert_true(runs(&r, "σ"));
    Assert::assert_true(runs(&r, "ς"));

    r = RegExp::with_match_flags(
        "σ",
        RegExp::ALL,
        RegExp::ASCII_CASE_INSENSITIVE | RegExp::CASE_INSENSITIVE,
    )?;
    Assert::assert_true(runs(&r, "ς"));

    // class 2 Unicode characters behaves appropriately with different flags
    r = RegExp::with_match_flags("ῼ", RegExp::ALL, RegExp::CASE_INSENSITIVE)?;
    Assert::assert_true(runs(&r, "ῳ"));

    r = RegExp::with_match_flags("ῼ", RegExp::ALL, RegExp::CASE_INSENSITIVE)?;
    // "ΩΙ": `String.toUpperCase(Locale.ROOT)` is the full mapping.
    Assert::assert_false(runs(&r, &"ῼ".to_uppercase()));

    // class 3 Unicode characters behaves appropriately with different flags
    r = RegExp::with_match_flags("ﬗ", RegExp::ALL, RegExp::CASE_INSENSITIVE)?;
    Assert::assert_false(runs(&r, &"ﬗ".to_uppercase()));
    Ok(())
}

/// `Object.toString()` of an `Automaton` (which does not override it):
/// `getClass().getName() + "@" + Integer.toHexString(hashCode())`, here the
/// class name and the value's address.
fn object_to_string(a: &rucene::util::automaton::Automaton) -> String {
    format!(
        "org.apache.lucene.util.automaton.Automaton@{:x}",
        a as *const _ as usize as u32
    )
}

/// `testRepeatWithEmptyString()` (LUCENE-6046).
fn test_repeat_with_empty_string() -> Result<()> {
    let a = RegExp::new("[^y]*{1,2}")?.to_automaton()?;
    // paranoia:
    hamcrest::assert_has_non_empty_to_string(&object_to_string(&a));
    Ok(())
}

fn test_repeat_with_empty_language() -> Result<()> {
    let mut a = RegExp::new("#*")?.to_automaton()?;
    // paranoia:
    hamcrest::assert_has_non_empty_to_string(&object_to_string(&a));
    a = RegExp::new("#+")?.to_automaton()?;
    hamcrest::assert_has_non_empty_to_string(&object_to_string(&a));
    a = RegExp::new("#{2,10}")?.to_automaton()?;
    hamcrest::assert_has_non_empty_to_string(&object_to_string(&a));
    a = RegExp::new("#?")?.to_automaton()?;
    hamcrest::assert_has_non_empty_to_string(&object_to_string(&a));
    Ok(())
}

/// The instance fields `caseSensitiveQuery` and `unicodeCaseQuery`.
struct Fields {
    case_sensitive_query: Cell<bool>,
    unicode_case_query: Cell<bool>,
}

fn test_core_java_parity() -> Result<()> {
    let this = Fields {
        case_sensitive_query: Cell::new(true),
        unicode_case_query: Cell::new(true),
    };
    // Generate random doc values and random regular expressions
    // and check for same matching behaviour as Java's Pattern class.
    for _ in 0..1000 {
        this.case_sensitive_query.set(true);
        check_random_expression(
            &this,
            &random_doc_value(1 + random().next_int_bound(30), false),
        );
    }
    for _ in 0..1000 {
        this.case_sensitive_query.set(true);
        this.unicode_case_query.set(true);
        check_random_expression(
            &this,
            &random_doc_value(1 + random().next_int_bound(30), true),
        );
    }
    Ok(())
}

fn test_illegal_backslash_chars() -> Result<()> {
    let illegal_chars = "abcefghijklmnopqrtuvxyzABCEFGHIJKLMNOPQRTUVXYZ";
    for ch in illegal_chars.chars() {
        let illegal_expression = format!("\\{ch}");
        let expected = expect_throws(IAE, || RegExp::new(&illegal_expression));
        hamcrest::assert_contains_string(expected.message(), "invalid character class");
    }
    Ok(())
}

fn test_legal_backslash_chars() -> Result<()> {
    let legal_chars = "dDsSWw0123456789[]*&^$@!{}\\/";
    for ch in legal_chars.chars() {
        let legal_expression = format!("\\{ch}");
        RegExp::new(&legal_expression)?;
    }
    Ok(())
}

fn test_parse_illegal_repeat_exp() -> Result<()> {
    // out of order
    let expected = expect_throws(IAE, || RegExp::new("a{99,11}"));
    hamcrest::assert_contains_string(expected.message(), "out of order");
    Ok(())
}

/// `randomDocValue(minLength, includeUnicode)`. The palette's characters are
/// all in the BMP, so a `char` index is a UTF-16 index.
fn random_doc_value(min_length: i32, include_unicode: bool) -> String {
    let mut char_palette = String::from("AAAaaaBbbCccc123456 \t");
    if include_unicode {
        char_palette.push_str("Σσςῼῳ");
    }
    let palette: Vec<char> = char_palette.chars().collect();
    let mut sb = String::new();
    for _ in 0..min_length {
        sb.push(palette[random_int(palette.len() as i32 - 1) as usize]);
    }
    sb
}

/// `randomInt(bound)`.
fn random_int(bound: i32) -> i32 {
    if bound == 0 {
        0
    } else {
        random().next_int_bound(bound)
    }
}

/// `String.replaceAll("\\d", replacement)` / `replaceAll("\\s", …)`: Java's
/// `\d` is `[0-9]` and `\s` is `[ \t\n\x0B\f\r]` without
/// `UNICODE_CHARACTER_CLASS`.
fn replace_all_class(s: &str, class: fn(char) -> bool, replacement: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if class(ch) {
            out.push_str(replacement);
        } else {
            out.push(ch);
        }
    }
    out
}

fn java_digit(ch: char) -> bool {
    ch.is_ascii_digit()
}

fn java_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

/// `Character.isLowerCase(int)`.
fn is_lower_case(cp: i32) -> bool {
    char::from_u32(cp as u32).is_some_and(|c| c.is_lowercase())
}

/// `checkRandomExpression(docValue)`: generate and test a random regular
/// expression which should match the given docValue. The doc values are
/// built from BMP characters, so `char` indices stand for Java's UTF-16
/// indices.
fn check_random_expression(this: &Fields, doc_value: &str) -> String {
    let doc: Vec<char> = doc_value.chars().collect();
    let sub = |from: usize, to: usize| -> String { doc[from..to].iter().collect() };
    // Generate and test a random regular expression which should match the given docValue
    let mut result = String::new();
    // Pick a part of the string to change
    let substitution_point = random_int(doc.len() as i32 - 1) as usize;
    let substitution_length =
        1 + random_int(10.min(doc.len() as i32 - substitution_point as i32)) as usize;

    // Add any head to the result, unchanged
    if substitution_point > 0 {
        result.push_str(&sub(0, substitution_point));
    }

    // Modify the middle...
    let replacement_part = sub(substitution_point, substitution_point + substitution_length);
    let replacement_len = replacement_part.chars().count() as i32;
    let mutation = random().next_int_bound(15);
    match mutation {
        0 => {
            // OR with random alpha of same length
            result.push_str(&format!(
                "({replacement_part}|d{})",
                random_doc_value(replacement_len, false)
            ));
        }
        1 => {
            // OR with non-existant value
            result.push_str(&format!("({replacement_part}|doesnotexist)"));
        }
        2 => {
            // OR with another randomised regex (used to create nested levels of expression).
            result.push_str(&format!(
                "({}|doesnotexist)",
                check_random_expression(this, &replacement_part)
            ));
        }
        3 => {
            // Star-replace all ab sequences.
            result.push_str(&replacement_part.replace("ab", ".*"));
        }
        4 => {
            // .-replace all b chars
            result.push_str(&replacement_part.replace('b', "."));
        }
        5 => {
            // length-limited stars {1,2}
            result.push_str(&format!(".{{1,{replacement_len}}}"));
        }
        6 => {
            // replace all chars with .
            result.push_str(&".".repeat(replacement_len as usize));
        }
        7 => {
            // OR with uppercase chars eg [aA] (many of these sorts of expression in the wild..
            for ch in replacement_part.chars() {
                let upper = char::from_u32(case_folding::to_upper_case(ch as i32) as u32)
                    .expect("a BMP character");
                result.push_str(&format!("[{ch}{upper}]"));
            }
        }
        8 => {
            // NOT a character - replace all b's with "not a"
            result.push_str(&replacement_part.replace('b', "[^a]"));
        }
        9 => {
            // Make whole part repeatable 1 or more times
            result.push_str(&format!("({replacement_part})+"));
        }
        10 => {
            // Make whole part repeatable 0 or more times
            result.push_str(&format!("({replacement_part})?"));
        }
        11 => {
            // Make any digits replaced by character class
            result.push_str(&replace_all_class(&replacement_part, java_digit, "\\d"));
        }
        12 => {
            // Make any whitespace chars replaced by not word class
            result.push_str(&replace_all_class(&replacement_part, java_space, "\\W"));
        }
        13 => {
            // Make any whitespace chars replace by whitespace class
            result.push_str(&replace_all_class(&replacement_part, java_space, "\\s"));
        }
        14 => {
            // Switch case of characters
            let mut switched_case = String::new();
            for ch in replacement_part.chars() {
                let p = ch as i32;
                let switched_p = if is_lower_case(p) {
                    case_folding::to_upper_case(p)
                } else {
                    case_folding::to_lower_case(p)
                };
                switched_case.push(char::from_u32(switched_p as u32).expect("a code point"));
                if p != switched_p {
                    this.case_sensitive_query.set(false);
                }
            }
            result.push_str(&switched_case);
        }
        _ => {}
    }
    // add any remaining tail, unchanged
    if substitution_point + substitution_length < doc.len() {
        result.push_str(&sub(substitution_point + substitution_length, doc.len()));
    }

    let regex_pattern = result;
    // Assert our randomly generated regex actually matches the provided raw input using java's
    // expression matcher
    assert_matches_java_pattern(doc_value, &regex_pattern, this.case_sensitive_query.get());

    let match_flags = if this.case_sensitive_query.get() {
        0
    } else {
        RegExp::ASCII_CASE_INSENSITIVE | RegExp::CASE_INSENSITIVE
    };
    let regex = unchecked(RegExp::with_match_flags(
        &regex_pattern,
        RegExp::ALL,
        match_flags,
    ));
    let automaton = det(Operations::determinize(
        &unchecked(regex.to_automaton()),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    let bytes_matcher = unchecked(ByteRunAutomaton::from_utf32(automaton));
    let br = LuceneTestCase::new_bytes_ref_str(doc_value);
    Assert::assert_true_msg(
        &format!(
            "[{regex_pattern}]should match [{doc_value}]{substitution_point}-{substitution_length}/{}",
            doc.len()
        ),
        bytes_matcher.run_range(&br.bytes, br.offset, br.length),
    );
    if !this.case_sensitive_query.get() {
        let case_sensitive_regex = unchecked(RegExp::new(&regex_pattern));
        let cs_automaton = unchecked(case_sensitive_regex.to_automaton());
        let cs_automaton = det(Operations::determinize(
            &cs_automaton,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        let cs_bytes_matcher = unchecked(ByteRunAutomaton::from_utf32(cs_automaton));
        Assert::assert_false_msg(
            &format!(
                "[{regex_pattern}] with case sensitive setting should not match [{doc_value}]"
            ),
            cs_bytes_matcher.run_range(&br.bytes, br.offset, br.length),
        );
    }
    regex_pattern
}

/// `assertThat(docValue, matchesRegex(pattern))`, `pattern` being
/// `Pattern.compile(regexPattern)` or, when not `caseSensitive`,
/// `Pattern.compile(regexPattern, Pattern.CASE_INSENSITIVE |
/// Pattern.UNICODE_CASE)`: `pattern.matcher(docValue).matches()`.
///
/// The pattern is compiled by Rucene's port of `java.util.regex.Pattern`
/// (`AbstractAnalysisFactory.getPattern`). The flags are spelled as the
/// leading embedded flags `(?iu)`, which `Pattern` documents as
/// equivalent, and `matches()` (the whole input) as the anchors
/// `\A(?:…)\z` around the pattern.
fn assert_matches_java_pattern(doc_value: &str, regex_pattern: &str, case_sensitive: bool) {
    let flags = if case_sensitive { "" } else { "(?iu)" };
    let mut args = HashMap::from([(
        "pattern".to_string(),
        format!("{flags}\\A(?:{regex_pattern})\\z"),
    )]);
    let pattern = unchecked(AbstractAnalysisFactory::get_pattern(
        &mut args,
        "pattern",
        "TestRegExp",
    ));
    let matches = pattern.is_match(doc_value).unwrap_or(false);
    if !matches {
        Assert::fail(&format!(
            "\nExpected: a string matching the pattern '{regex_pattern}'\n     but: was \"{doc_value}\""
        ));
    }
}

fn test_reg_exp_no_stack_overflow() -> Result<()> {
    RegExp::new(&format!("{}(a)", "(a)|".repeat(50000)))?;
    Ok(())
}

/// `testDeprecatedComplement()`: the deprecated complement flag. Keep the
/// simple test only, no random tests to let it cause us pain.
fn test_deprecated_complement() -> Result<()> {
    let expected = det(Operations::complement(
        &Automata::make_string("abcd"),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    let actual = RegExp::with_flags("~(abcd)", RegExp::DEPRECATED_COMPLEMENT)?.to_automaton()?;
    Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    Ok(())
}
