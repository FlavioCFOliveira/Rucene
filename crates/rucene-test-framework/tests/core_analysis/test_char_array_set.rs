//! Port of `org.apache.lucene.analysis.TestCharArraySet`.
//!
//! Java's `add(Object)` and `contains(Object)` read a non-`char[]` element by
//! its `toString()` (`CharArrayMap.put(Object, V)`,
//! `CharArrayMap.containsKey(Object)`); the port passes that string, and a
//! `char[]` element through the `char[]` overloads.
//!
//! A Java `null` argument is `None` (user decision, sprint 9).
//!
//! A Java string that a Rust `str` cannot hold (one with a lone surrogate)
//! is its UTF-16 code units, `Vec<u16>`.

use std::sync::Arc;

use randomizedtesting::jdk::format::format;
use randomizedtesting::jdk::reflect::{Class, Value};
use randomizedtesting::junit::Assert;
use rucene::analysis::CharArraySet;
use rucene::error::Result;
use rucene::util::java_collections::JavaHashSet;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::lucene_core_shims::{lucene_class, t};

/// `UnsupportedOperationException.class`.
const UOE: &str = "java.lang.UnsupportedOperationException";

/// `TEST_STOP_WORDS` (`static final`).
pub const TEST_STOP_WORDS: [&str; 33] = [
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "into", "is", "it",
    "no", "not", "of", "on", "or", "such", "that", "the", "their", "then", "there", "these",
    "they", "this", "to", "was", "will", "with",
];

/// `String.format(Locale.ROOT, format, arg)`.
fn format1(template: &str, arg: &str) -> String {
    format(template, &[Value::Str(arg.to_string())])
}

/// `s.toCharArray()`.
fn chars(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `new String(char[], int, int)` for a well-formed range.
fn string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

fn test_rehash() -> Result<()> {
    let mut cas = CharArraySet::new(0, true);
    for stop_word in TEST_STOP_WORDS {
        cas.add(stop_word);
    }
    Assert::assert_equals(TEST_STOP_WORDS.len(), cas.size());
    for test_stop_word in TEST_STOP_WORDS {
        Assert::assert_true(cas.contains(Some(test_stop_word)));
    }
    Ok(())
}

fn test_non_zero_offset() -> Result<()> {
    let words = ["Hello", "World", "this", "is", "a", "test"];
    let findme = chars("xthisy");
    let mut set = CharArraySet::new(10, true);
    set.add_all(words);
    Assert::assert_true(set.contains_chars(Some(&findme), 1, 4));
    Assert::assert_true(set.contains(Some(string(&findme[1..5]))));

    // test unmodifiable
    set = CharArraySet::unmodifiable_set(Some(set));
    Assert::assert_true(set.contains_chars(Some(&findme), 1, 4));
    Assert::assert_true(set.contains(Some(string(&findme[1..5]))));
    Ok(())
}

fn test_object_contains() -> Result<()> {
    let mut set = CharArraySet::new(10, true);
    let val: i32 = 1;
    // `set.add(val)`: `add(Object)` stores `val.toString()`.
    set.add(val.to_string());
    // `set.contains(val)`: `contains(Object)` reads `val.toString()`.
    Assert::assert_true(set.contains(Some(val.to_string())));
    Assert::assert_true(set.contains(Some("1")));
    Assert::assert_true(set.contains_chars(Some(&[u16::from(b'1')]), 0, 1));
    // test unmodifiable
    set = CharArraySet::unmodifiable_set(Some(set));
    Assert::assert_true(set.contains(Some(val.to_string())));
    Assert::assert_true(set.contains(Some("1")));
    Assert::assert_true(set.contains_chars(Some(&[u16::from(b'1')]), 0, 1));
    Ok(())
}

fn test_clear() -> Result<()> {
    let mut set = CharArraySet::new(10, true);
    set.add_all(TEST_STOP_WORDS);
    Assert::assert_equals_msg(
        Some("Not all words added"),
        TEST_STOP_WORDS.len(),
        set.size(),
    );
    set.clear();
    Assert::assert_equals_msg(Some("not empty"), 0usize, set.size());
    for test_stop_word in TEST_STOP_WORDS {
        Assert::assert_false(set.contains(Some(test_stop_word)));
    }
    set.add_all(TEST_STOP_WORDS);
    Assert::assert_equals_msg(
        Some("Not all words added"),
        TEST_STOP_WORDS.len(),
        set.size(),
    );
    for test_stop_word in TEST_STOP_WORDS {
        Assert::assert_true(set.contains(Some(test_stop_word)));
    }
    Ok(())
}

// TODO: break this up into simpler test methods, vs "telling a story"
fn test_modify_on_unmodifiable() -> Result<()> {
    let mut set = CharArraySet::new(10, true);
    set.add_all(TEST_STOP_WORDS);
    let size = set.size();
    // **Divergence from Lucene 10.5.0.** Java's unmodifiable set shares the
    // arrays of `set`; `unmodifiable_set` takes its set by value, so it gets
    // a copy. Neither set is modified below, so both hold the same keys
    // throughout, as Java's do.
    let mut unmodifiable_set = CharArraySet::unmodifiable_set(Some(set.clone()));
    Assert::assert_equals_msg(
        Some("Set size changed due to unmodifiableSet call"),
        size,
        unmodifiable_set.size(),
    );
    let not_in_set = "SirGallahad";
    Assert::assert_false_msg(
        "Test String already exists in set",
        unmodifiable_set.contains(Some(not_in_set)),
    );

    expect_throws(UOE, || {
        let mut text = chars(not_in_set);
        let len = text.len();
        unmodifiable_set.add_chars(&mut text, 0, len);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable set",
        unmodifiable_set.contains(Some(not_in_set)),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable set has changed"),
        size,
        unmodifiable_set.size(),
    );

    expect_throws(UOE, || {
        unmodifiable_set.add(not_in_set);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable set",
        unmodifiable_set.contains(Some(not_in_set)),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable set has changed"),
        size,
        unmodifiable_set.size(),
    );

    // `add(new StringBuilder(NOT_IN_SET))`: `add(CharSequence)`.
    expect_throws(UOE, || {
        unmodifiable_set.add(String::from(not_in_set));
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable set",
        unmodifiable_set.contains(Some(not_in_set)),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable set has changed"),
        size,
        unmodifiable_set.size(),
    );

    expect_throws(UOE, || {
        unmodifiable_set.clear();
        Ok(())
    });
    Assert::assert_false_msg(
        "Changed unmodifiable set",
        unmodifiable_set.contains(Some(not_in_set)),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable set has changed"),
        size,
        unmodifiable_set.size(),
    );

    // `add((Object) NOT_IN_SET)`: `add(Object)` stores `toString()`.
    expect_throws(UOE, || {
        unmodifiable_set.add(not_in_set);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable set",
        unmodifiable_set.contains(Some(not_in_set)),
    );
    Assert::assert_equals_msg(
        Some("Size of unmodifiable set has changed"),
        size,
        unmodifiable_set.size(),
    );

    // This test was changed in 3.1, as a contains() call on the given Collection using the
    // "correct" iterator's
    // current key (now a char[]) on a Set<String> would not hit any element of the CAS and therefor
    // never call
    // remove() on the iterator
    expect_throws(UOE, || {
        unmodifiable_set.remove_all(&CharArraySet::from_iter(TEST_STOP_WORDS, true))
    });
    Assert::assert_equals_msg(
        Some("Size of unmodifiable set has changed"),
        size,
        unmodifiable_set.size(),
    );

    expect_throws(UOE, || {
        unmodifiable_set.retain_all(&CharArraySet::from_iter([not_in_set], true))
    });
    Assert::assert_equals_msg(
        Some("Size of unmodifiable set has changed"),
        size,
        unmodifiable_set.size(),
    );

    expect_throws(UOE, || {
        unmodifiable_set.add_all([not_in_set]);
        Ok(())
    });
    Assert::assert_false_msg(
        "Test String has been added to unmodifiable set",
        unmodifiable_set.contains(Some(not_in_set)),
    );

    for test_stop_word in TEST_STOP_WORDS {
        Assert::assert_true(set.contains(Some(test_stop_word)));
        Assert::assert_true(unmodifiable_set.contains(Some(test_stop_word)));
    }
    Ok(())
}

fn test_unmodifiable_set() -> Result<()> {
    let mut set = CharArraySet::new(10, true);
    set.add_all(TEST_STOP_WORDS);
    // `set.add(1)`: `add(Object)` stores `toString()`.
    set.add(1.to_string());
    let size = set.size();
    set = CharArraySet::unmodifiable_set(Some(set));
    Assert::assert_equals_msg(
        Some("Set size changed due to unmodifiableSet call"),
        size,
        set.size(),
    );
    for stopword in TEST_STOP_WORDS {
        Assert::assert_true(set.contains(Some(stopword)));
    }
    // `set.contains(1)`: `contains(Object)` reads `toString()`.
    Assert::assert_true(set.contains(Some(1.to_string())));
    Assert::assert_true(set.contains(Some("1")));
    Assert::assert_true(set.contains_chars(Some(&[u16::from(b'1')]), 0, 1));

    LuceneTestCase::expect_throws("java.lang.NullPointerException", || {
        CharArraySet::unmodifiable_set(None);
        Ok(())
    });
    Ok(())
}

fn test_supplementary_chars() -> Result<()> {
    let missing = "Term %s is missing in the set";
    let false_pos = "Term %s is in the set but shouldn't";
    // for reference see
    // http://unicode.org/cldr/utility/list-unicodeset.jsp?a=[[%3ACase_Sensitive%3DTrue%3A]%26[^[\u0000-￿]]]&esc=on
    let upper_arr = ["Abc\u{1041c}", "\u{1041c}\u{1041c}CDE", "A\u{1041c}B"];
    let lower_arr = ["abc\u{10444}", "\u{10444}\u{10444}cde", "a\u{10444}b"];
    let mut set = CharArraySet::from_iter(TEST_STOP_WORDS, true);
    // `Collections.addAll(set, upperArr)`.
    for e in upper_arr {
        set.add(e);
    }
    for i in 0..upper_arr.len() {
        Assert::assert_true_msg(
            &format1(missing, upper_arr[i]),
            set.contains(Some(upper_arr[i])),
        );
        Assert::assert_true_msg(
            &format1(missing, lower_arr[i]),
            set.contains(Some(lower_arr[i])),
        );
    }
    set = CharArraySet::from_iter(TEST_STOP_WORDS, false);
    for e in upper_arr {
        set.add(e);
    }
    for i in 0..upper_arr.len() {
        Assert::assert_true_msg(
            &format1(missing, upper_arr[i]),
            set.contains(Some(upper_arr[i])),
        );
        Assert::assert_false_msg(
            &format1(false_pos, lower_arr[i]),
            set.contains(Some(lower_arr[i])),
        );
    }
    Ok(())
}

fn test_single_high_surrogate() -> Result<()> {
    let missing = "Term %s is missing in the set";
    let false_pos = "Term %s is in the set but shouldn't";
    // Java strings with a lone high surrogate: their UTF-16 units.
    let upper_arr: [Vec<u16>; 4] = [
        vec![0x41, 0x42, 0x43, 0xD800],
        vec![0x41, 0x42, 0x43, 0xD800, 0x45, 0x66, 0x47],
        vec![0xD800, 0x45, 0x66, 0x47],
        vec![0xD800, 0xD801, 0xDC1C, 0x42],
    ];

    let lower_arr: [Vec<u16>; 4] = [
        vec![0x61, 0x62, 0x63, 0xD800],
        vec![0x61, 0x62, 0x63, 0xD800, 0x65, 0x66, 0x67],
        vec![0xD800, 0x65, 0x66, 0x67],
        vec![0xD800, 0xD801, 0xDC44, 0x62],
    ];
    // `String.format(Locale.ROOT, fmt, s)`: the message renders a lone
    // surrogate as U+FFFD, which a Rust string holds in its place.
    let message = |template: &str, units: &[u16]| format1(template, &string(units));
    let mut set = CharArraySet::from_iter(TEST_STOP_WORDS, true);
    // `Collections.addAll(set, upperArr)`: `add(Object)` stores
    // `toString().toCharArray()`, a copy of the string's units.
    for e in &upper_arr {
        let mut text = e.clone();
        let len = text.len();
        set.add_chars(&mut text, 0, len);
    }
    // `set.contains(String)`: `contains(Object)` looks the string's units up.
    for i in 0..upper_arr.len() {
        Assert::assert_true_msg(
            &message(missing, &upper_arr[i]),
            set.contains_chars(Some(&upper_arr[i]), 0, upper_arr[i].len()),
        );
        Assert::assert_true_msg(
            &message(missing, &lower_arr[i]),
            set.contains_chars(Some(&lower_arr[i]), 0, lower_arr[i].len()),
        );
    }
    set = CharArraySet::from_iter(TEST_STOP_WORDS, false);
    for e in &upper_arr {
        let mut text = e.clone();
        let len = text.len();
        set.add_chars(&mut text, 0, len);
    }
    for i in 0..upper_arr.len() {
        Assert::assert_true_msg(
            &message(missing, &upper_arr[i]),
            set.contains_chars(Some(&upper_arr[i]), 0, upper_arr[i].len()),
        );
        Assert::assert_false_msg(
            &message(false_pos, &upper_arr[i]),
            set.contains_chars(Some(&lower_arr[i]), 0, lower_arr[i].len()),
        );
    }
    Ok(())
}

fn test_copy_char_array_set_bw_compat() -> Result<()> {
    let mut set_ignore_case = CharArraySet::new(10, true);
    let mut set_case_sensitive = CharArraySet::new(10, false);

    let stopwords: Vec<&str> = TEST_STOP_WORDS.to_vec();
    let mut stopwords_upper: Vec<String> = Vec::new();
    for string in &stopwords {
        stopwords_upper.push(string.to_uppercase());
    }
    set_ignore_case.add_all(TEST_STOP_WORDS);
    set_ignore_case.add(1.to_string());
    set_case_sensitive.add_all(TEST_STOP_WORDS);
    set_case_sensitive.add(1.to_string());

    let mut copy = set_ignore_case.copy();
    let copy_case_sens = set_case_sensitive.copy();

    Assert::assert_equals(set_ignore_case.size(), copy.size());
    Assert::assert_equals(set_case_sensitive.size(), copy.size());

    Assert::assert_true(copy.contains_all(&stopwords));
    Assert::assert_true(copy.contains_all(&stopwords_upper));
    Assert::assert_true(copy_case_sens.contains_all(&stopwords));
    for string in &stopwords_upper {
        Assert::assert_false(copy_case_sens.contains(Some(string)));
    }
    // test adding terms to the copy
    let mut new_words: Vec<String> = Vec::new();
    for string in &stopwords {
        new_words.push(format!("{string}_1"));
    }
    copy.add_all(&new_words);

    Assert::assert_true(copy.contains_all(&stopwords));
    Assert::assert_true(copy.contains_all(&stopwords_upper));
    Assert::assert_true(copy.contains_all(&new_words));
    // new added terms are not in the source set
    for string in &new_words {
        Assert::assert_false(set_ignore_case.contains(Some(string)));
        Assert::assert_false(set_case_sensitive.contains(Some(string)));
    }
    Ok(())
}

/// Test the static #copy() function with a CharArraySet as a source
fn test_copy_char_array_set() -> Result<()> {
    let mut set_ignore_case = CharArraySet::new(10, true);
    let mut set_case_sensitive = CharArraySet::new(10, false);

    let stopwords: Vec<&str> = TEST_STOP_WORDS.to_vec();
    let mut stopwords_upper: Vec<String> = Vec::new();
    for string in &stopwords {
        stopwords_upper.push(string.to_uppercase());
    }
    set_ignore_case.add_all(TEST_STOP_WORDS);
    set_ignore_case.add(1.to_string());
    set_case_sensitive.add_all(TEST_STOP_WORDS);
    set_case_sensitive.add(1.to_string());

    let mut copy = set_ignore_case.copy();
    let copy_case_sens = set_case_sensitive.copy();

    Assert::assert_equals(set_ignore_case.size(), copy.size());
    Assert::assert_equals(set_case_sensitive.size(), copy.size());

    Assert::assert_true(copy.contains_all(&stopwords));
    Assert::assert_true(copy.contains_all(&stopwords_upper));
    Assert::assert_true(copy_case_sens.contains_all(&stopwords));
    for string in &stopwords_upper {
        Assert::assert_false(copy_case_sens.contains(Some(string)));
    }
    // test adding terms to the copy
    let mut new_words: Vec<String> = Vec::new();
    for string in &stopwords {
        new_words.push(format!("{string}_1"));
    }
    copy.add_all(&new_words);

    Assert::assert_true(copy.contains_all(&stopwords));
    Assert::assert_true(copy.contains_all(&stopwords_upper));
    Assert::assert_true(copy.contains_all(&new_words));
    // new added terms are not in the source set
    for string in &new_words {
        Assert::assert_false(set_ignore_case.contains(Some(string)));
        Assert::assert_false(set_case_sensitive.contains(Some(string)));
    }
    Ok(())
}

/// Test the static #copy() function with a JDK {@link Set} as a source
fn test_copy_jdk_set() -> Result<()> {
    let stopwords: Vec<&str> = TEST_STOP_WORDS.to_vec();
    let mut stopwords_upper: Vec<String> = Vec::new();
    for string in &stopwords {
        stopwords_upper.push(string.to_uppercase());
    }
    let set = JavaHashSet::from_collection(TEST_STOP_WORDS.iter().map(|s| s.to_string()));

    // `CharArraySet.copy(set)` of a set that is not a `CharArraySet`:
    // `new CharArraySet(set, false)`, in the set's iteration order.
    let mut copy = CharArraySet::from_iter(set.iter(), false);

    Assert::assert_equals(set.len(), copy.size());
    Assert::assert_equals(set.len(), copy.size());

    Assert::assert_true(copy.contains_all(&stopwords));
    for string in &stopwords_upper {
        Assert::assert_false(copy.contains(Some(string)));
    }

    let mut new_words: Vec<String> = Vec::new();
    for string in &stopwords {
        new_words.push(format!("{string}_1"));
    }
    copy.add_all(&new_words);

    Assert::assert_true(copy.contains_all(&stopwords));
    Assert::assert_true(copy.contains_all(&new_words));
    // new added terms are not in the source set
    for string in &new_words {
        Assert::assert_false(set.contains(string));
    }
    Ok(())
}

/// Tests a special case of {@link CharArraySet#copy(Set)} where the set to copy is the {@link
/// CharArraySet#EMPTY_SET}
///
/// **Divergence from Lucene 10.5.0.** `assertSame(EMPTY_SET, copy(EMPTY_SET))`
/// compares identities, which Rust values lack; `EMPTY_SET` is any empty,
/// unmodifiable set (see `CharArraySet::empty_set`), so the assertion is that
/// the copy is one.
fn test_copy_empty_set() -> Result<()> {
    let copy = CharArraySet::empty_set().copy();
    Assert::assert_true(copy.is_empty() && copy.is_unmodifiable());
    Ok(())
}

/// Smoketests the static empty set
fn test_empty_set() -> Result<()> {
    let empty_set = CharArraySet::empty_set();
    Assert::assert_equals(0usize, empty_set.size());

    Assert::assert_true(empty_set.is_empty());
    for stopword in TEST_STOP_WORDS {
        Assert::assert_false(empty_set.contains(Some(stopword)));
    }
    Assert::assert_false(empty_set.contains(Some("foo")));
    // `contains((Object) "foo")`: `contains(Object)` reads `toString()`.
    Assert::assert_false(empty_set.contains(Some("foo".to_string())));
    // `contains("foo".toCharArray())`: `contains(Object)` with a `char[]`.
    Assert::assert_false(empty_set.contains_chars(Some(&chars("foo")), 0, 3));
    Assert::assert_false(empty_set.contains_chars(Some(&chars("foo")), 0, 3));
    Ok(())
}

fn test_contains_with_null() -> Result<()> {
    let set = CharArraySet::new(1, true);

    LuceneTestCase::expect_throws("java.lang.NullPointerException", || {
        set.contains_chars(None, 0, 10);
        Ok(())
    });

    // `set.contains((CharSequence) null)`
    LuceneTestCase::expect_throws("java.lang.NullPointerException", || {
        set.contains(None::<&str>);
        Ok(())
    });

    // `set.contains((Object) null)`: `CharArrayMap.containsKey(Object)` calls
    // `o.toString()`, which throws the `NullPointerException`; the port's
    // `contains(Object)` is `contains` with the element's string.
    LuceneTestCase::expect_throws("java.lang.NullPointerException", || {
        set.contains(None::<String>);
        Ok(())
    });
    Ok(())
}

fn test_to_string() -> Result<()> {
    // `CharArraySet.copy(Collections.singleton("test"))`: `new
    // CharArraySet(set, false)`.
    let mut set = CharArraySet::from_iter(["test"], false);
    Assert::assert_equals_str(None, Some("[test]"), Some(&set.to_string()));
    set.add("test2");
    Assert::assert_true(set.to_string().contains(", "));
    Ok(())
}

/// The class `org.apache.lucene.analysis.TestCharArraySet`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestCharArraySet")
        .method(t("testRehash", test_rehash))
        .method(t("testNonZeroOffset", test_non_zero_offset))
        .method(t("testObjectContains", test_object_contains))
        .method(t("testClear", test_clear))
        .method(t("testModifyOnUnmodifiable", test_modify_on_unmodifiable))
        .method(t("testUnmodifiableSet", test_unmodifiable_set))
        .method(t("testSupplementaryChars", test_supplementary_chars))
        .method(t("testSingleHighSurrogate", test_single_high_surrogate))
        .method(t(
            "testCopyCharArraySetBWCompat",
            test_copy_char_array_set_bw_compat,
        ))
        .method(t("testCopyCharArraySet", test_copy_char_array_set))
        .method(t("testCopyJDKSet", test_copy_jdk_set))
        .method(t("testCopyEmptySet", test_copy_empty_set))
        .method(t("testEmptySet", test_empty_set))
        .method(t("testContainsWithNull", test_contains_with_null))
        .method(t("testToString", test_to_string))
        .build()
}
