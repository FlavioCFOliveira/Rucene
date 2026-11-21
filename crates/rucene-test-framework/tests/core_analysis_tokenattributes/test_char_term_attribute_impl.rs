//! Port of `org.apache.lucene.analysis.tokenattributes.TestCharTermAttributeImpl`.
//!
//! Rucene's `CharTermAttribute` holds `char`s (Unicode scalar values) where
//! Java's holds UTF-16 code units, and takes `usize` indices where Java takes
//! `int`s (the divergence `rucene::analysis::tokenattributes` declares). Every
//! term here is ASCII, so the two count alike; a negative Java index is
//! passed as its two's-complement `usize`, which the bounds checks reject as
//! Java's do. Java's `CharSequence` arguments (`String`, `StringBuilder`,
//! `StringBuffer`, `CharBuffer`, a custom `CharSequence`) are the Rust strings
//! they hold, appended through `append_string` and `append_string_range`;
//! a `CharTermAttribute` argument goes through `append_char_term_attribute`
//! and `append_char_term_attribute_range`. A Java `null` argument is `None`
//! (user decision, sprint 9).

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::format::format;
use randomizedtesting::jdk::reflect::{Class, Value};
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::AbstractAnalysisFactory;
use rucene::analysis::tokenattributes::{CharTermAttribute, CharTermAttributeImpl};
use rucene::error::Result;
use rucene::util::attribute::AttributeImpl;
use rucene::util::BytesRef;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::lucene_core_shims::{lucene_class, t};

/// `IndexOutOfBoundsException.class`.
const IOOBE: &str = "java.lang.IndexOutOfBoundsException";

/// Java's `-1` passed where Rucene takes a `usize`.
const MINUS_ONE: usize = -1i64 as usize;

/// `s.toCharArray()`.
fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

/// `CharBuffer.wrap(array, offset, length)` as the `CharSequence` it is:
/// the characters of its window, which `toString()`, `length()` and
/// `subSequence` read.
fn char_buffer_wrap(array: &str, offset: usize, length: usize) -> String {
    array.chars().skip(offset).take(length).collect()
}

/// `Pattern.matches(regex, input)`: `Pattern.compile(regex).matcher(input).matches()`.
///
/// **Divergence from Lucene 10.5.0.** Rust has no `java.util.regex`. The
/// pattern is compiled by Rucene's port of Java's pattern dialect (the one
/// `AbstractAnalysisFactory.getPattern` uses), and `matches()`, a match of the
/// whole input, is a search of `\A(?:regex)\z`.
fn pattern_matches(regex: &str, input: &str) -> Result<bool> {
    let mut args = HashMap::from([("pattern".to_string(), format!("\\A(?:{regex})\\z"))]);
    let pattern = AbstractAnalysisFactory::get_pattern(&mut args, "pattern", "Pattern")?;
    pattern
        .is_match(input)
        .map_err(|e| rucene::error::LuceneError::illegal_state(e.to_string()))
}

fn test_resize() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    let content = chars("hello");
    t.copy_buffer(&content, 0, content.len());
    for i in 0..2000 {
        t.resize_buffer(i);
        Assert::assert_true(i <= t.buffer().len());
        Assert::assert_equals_str(None, Some("hello"), Some(&t.to_string()));
    }
    Ok(())
}

fn test_set_length() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    let content = chars("hello");
    t.copy_buffer(&content, 0, content.len());
    LuceneTestCase::expect_throws(IOOBE, || {
        t.set_length(MINUS_ONE);
        Ok(())
    });
    Ok(())
}

fn test_grow() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    let mut buf = String::from("ab");
    for _ in 0..20 {
        let content = chars(&buf);
        t.copy_buffer(&content, 0, content.len());
        Assert::assert_equals(buf.chars().count(), t.length());
        Assert::assert_equals_str(None, Some(&buf), Some(&t.to_string()));
        let copy = buf.clone();
        buf.push_str(&copy);
    }
    Assert::assert_equals(1048576usize, t.length());

    // now as a StringBuilder, first variant
    t = CharTermAttributeImpl::new();
    buf = String::from("ab");
    for _ in 0..20 {
        t.set_empty();
        t.append_string(Some(&buf));
        Assert::assert_equals(buf.chars().count(), t.length());
        Assert::assert_equals_str(None, Some(&buf), Some(&t.to_string()));
        // `buf.append(t)`: `StringBuilder.append(CharSequence)`.
        buf.push_str(&t.to_string());
    }
    Assert::assert_equals(1048576usize, t.length());

    // Test for slow growth to a long term
    t = CharTermAttributeImpl::new();
    buf = String::from("a");
    for _ in 0..20000 {
        t.set_empty();
        t.append_string(Some(&buf));
        Assert::assert_equals(buf.chars().count(), t.length());
        Assert::assert_equals_str(None, Some(&buf), Some(&t.to_string()));
        buf.push('a');
    }
    Assert::assert_equals(20000usize, t.length());
    Ok(())
}

fn test_to_string() -> Result<()> {
    let b = ['a', 'l', 'o', 'h', 'a'];
    let mut t = CharTermAttributeImpl::new();
    t.copy_buffer(&b, 0, 5);
    Assert::assert_equals_str(None, Some("aloha"), Some(&t.to_string()));

    t.set_empty();
    t.append_string(Some("hi there"));
    Assert::assert_equals_str(None, Some("hi there"), Some(&t.to_string()));
    Ok(())
}

fn test_clone() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    let content = chars("hello");
    t.copy_buffer(&content, 0, 5);
    let buf = t.buffer().as_ptr();
    let copy = assert_clone_is_equal(&t);
    Assert::assert_equals_str(None, Some(&t.to_string()), Some(&copy.to_string()));
    // `assertNotSame(buf, copy.buffer())`.
    Assert::assert_true(buf != copy.buffer().as_ptr());
    Ok(())
}

fn test_equals() -> Result<()> {
    let mut t1a = CharTermAttributeImpl::new();
    let content1a = chars("hello");
    t1a.copy_buffer(&content1a, 0, 5);
    let mut t1b = CharTermAttributeImpl::new();
    let content1b = chars("hello");
    t1b.copy_buffer(&content1b, 0, 5);
    let mut t2 = CharTermAttributeImpl::new();
    let content2 = chars("hello2");
    t2.copy_buffer(&content2, 0, 6);
    Assert::assert_true(t1a.equals(&t1b));
    Assert::assert_false(t1a.equals(&t2));
    Assert::assert_false(t2.equals(&t1b));
    Ok(())
}

fn test_copy_to() -> Result<()> {
    let t = CharTermAttributeImpl::new();
    let copy = assert_copy_is_equal(&t);
    Assert::assert_equals_str(None, Some(""), Some(&t.to_string()));
    Assert::assert_equals_str(None, Some(""), Some(&copy.to_string()));

    let mut t = CharTermAttributeImpl::new();
    let content = chars("hello");
    t.copy_buffer(&content, 0, 5);
    let buf = t.buffer().as_ptr();
    let copy = assert_copy_is_equal(&t);
    Assert::assert_equals_str(None, Some(&t.to_string()), Some(&copy.to_string()));
    // `assertNotSame(buf, copy.buffer())`.
    Assert::assert_true(buf != copy.buffer().as_ptr());
    Ok(())
}

/// The reflected map of `TestUtil.assertAttributeReflection`, keyed and
/// valued as Rucene's `AttributeReflector` reports them (see that method's
/// divergence): `CharTermAttribute.class.getName()` and
/// `TermToBytesRefAttribute.class.getName()` both stand for the
/// implementation's type name.
fn test_attribute_reflection() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    t.append_string(Some("foobar"));
    let term = std::any::type_name::<CharTermAttributeImpl>();
    let mut expected: HashMap<String, String> = HashMap::new();
    expected.insert(format!("{term}#term"), format!("{:?}", "foobar"));
    expected.insert(
        format!("{term}#bytes"),
        format!("{:?}", BytesRef::new(b"foobar".to_vec())),
    );
    TestUtil::assert_attribute_reflection(&t, &expected);
    Ok(())
}

fn test_char_sequence_interface() -> Result<()> {
    let s = "0123456789";
    let mut t = CharTermAttributeImpl::new();
    t.append_string(Some(s));

    Assert::assert_equals(s.chars().count(), t.length());
    Assert::assert_equals_str(None, Some("12"), Some(&t.sub_sequence(1, 3)));
    Assert::assert_equals_str(None, Some(s), Some(&t.sub_sequence(0, s.len())));

    Assert::assert_true(pattern_matches("01\\d+", &t.to_string())?);
    Assert::assert_true(pattern_matches("34", &t.sub_sequence(3, 5))?);

    Assert::assert_equals_str(None, Some(&s[3..7]), Some(&t.sub_sequence(3, 7)));

    for (i, c) in s.chars().enumerate() {
        Assert::assert_true(t.char_at(i) == c);
    }
    Ok(())
}

/// `Formatter.format(format, args)` on a `Formatter` over `t`: the formatted
/// string is appended to `t` through `Appendable.append(CharSequence)`.
fn formatter_format(t: &mut CharTermAttributeImpl, template: &str, args: &[Value]) {
    t.append_string(Some(&format(template, args)));
}

fn test_appendable_interface() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    // `Formatter formatter = new Formatter(t, Locale.ROOT);`
    formatter_format(&mut t, "%d", &[Value::Int(1234)]);
    Assert::assert_equals_str(None, Some("1234"), Some(&t.to_string()));
    formatter_format(&mut t, "%d", &[Value::Int(5678)]);
    Assert::assert_equals_str(None, Some("12345678"), Some(&t.to_string()));
    t.append_char('9');
    Assert::assert_equals_str(None, Some("123456789"), Some(&t.to_string()));
    t.append_string(Some("0"));
    Assert::assert_equals_str(None, Some("1234567890"), Some(&t.to_string()));
    t.append_string_range(Some("0123456789"), 1, 3);
    Assert::assert_equals_str(None, Some("123456789012"), Some(&t.to_string()));
    // `t.append((CharSequence) CharBuffer.wrap("0123456789".toCharArray()), 3, 5)`.
    t.append_string_range(Some(&char_buffer_wrap("0123456789", 0, 10)), 3, 5);
    Assert::assert_equals_str(None, Some("12345678901234"), Some(&t.to_string()));
    // `t.append((CharSequence) t)`. **Divergence from Lucene 10.5.0.** Rust
    // cannot borrow `t` as the argument while appending to it; Java copies
    // out of `t.buffer()` after `resizeBuffer` has preserved its content, so
    // appending a copy of `t` appends the same characters.
    let snapshot = t.clone();
    t.append_char_term_attribute(Some(&snapshot));
    Assert::assert_equals_str(
        None,
        Some("1234567890123412345678901234"),
        Some(&t.to_string()),
    );
    // `t.append((CharSequence) new StringBuilder("0123456789"), 5, 7)`.
    t.append_string_range(Some("0123456789"), 5, 7);
    Assert::assert_equals_str(
        None,
        Some("123456789012341234567890123456"),
        Some(&t.to_string()),
    );
    // `t.append((CharSequence) new StringBuffer(t))`.
    let buffer = t.to_string();
    t.append_string(Some(&buffer));
    Assert::assert_equals_str(
        None,
        Some("123456789012341234567890123456123456789012341234567890123456"),
        Some(&t.to_string()),
    );
    // very wierd, to test if a subSlice is wrapped correct :)
    let buf = char_buffer_wrap("0123456789", 3, 5);
    Assert::assert_equals_str(None, Some("34567"), Some(&buf));
    t.set_empty();
    t.append_string_range(Some(&buf), 1, 2);
    Assert::assert_equals_str(None, Some("4"), Some(&t.to_string()));
    let mut t2 = CharTermAttributeImpl::new();
    t2.append_string(Some("test"));
    t.append_char_term_attribute(Some(&t2));
    Assert::assert_equals_str(None, Some("4test"), Some(&t.to_string()));
    t.append_char_term_attribute_range(&t2, 1, 2);
    Assert::assert_equals_str(None, Some("4teste"), Some(&t.to_string()));

    LuceneTestCase::expect_throws(IOOBE, || {
        t.append_char_term_attribute_range(&t2, 1, 5);
        Ok(())
    });

    LuceneTestCase::expect_throws(IOOBE, || {
        t.append_char_term_attribute_range(&t2, 1, 0);
        Ok(())
    });

    // `t.append((CharSequence) null)`
    t.append_string(None);
    Assert::assert_equals_str(None, Some("4testenull"), Some(&t.to_string()));
    Ok(())
}

fn test_appendable_interface_with_long_sequences() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    t.append_string(Some("01234567890123456789012345678901234567890123456789"));
    // `t.append((CharSequence) CharBuffer.wrap("…".toCharArray()), 3, 50)`.
    t.append_string_range(
        Some(&char_buffer_wrap(
            "01234567890123456789012345678901234567890123456789",
            0,
            50,
        )),
        3,
        50,
    );
    Assert::assert_equals_str(
        None,
        Some(
            "0123456789012345678901234567890123456789012345678934567890123456789012345678901234567890123456789",
        ),
        Some(&t.to_string()),
    );
    // `t.setEmpty().append((CharSequence) new StringBuilder("01234567890123456789"), 5, 17)`.
    t.set_empty();
    t.append_string_range(Some("01234567890123456789"), 5, 17);
    Assert::assert_equals_str(None, Some("567890123456"), Some(&t.to_string()));
    // `t.append(new StringBuffer(t))`.
    let buffer = t.to_string();
    t.append_string(Some(&buffer));
    Assert::assert_equals_str(None, Some("567890123456567890123456"), Some(&t.to_string()));
    // very wierd, to test if a subSlice is wrapped correct :)
    let buf = char_buffer_wrap("012345678901234567890123456789", 3, 15);
    Assert::assert_equals_str(None, Some("345678901234567"), Some(&buf));
    t.set_empty();
    t.append_string_range(Some(&buf), 1, 14);
    Assert::assert_equals_str(None, Some("4567890123456"), Some(&t.to_string()));

    // finally use a completely custom CharSequence that is not catched by instanceof checks
    let long_test_string = "012345678901234567890123456789";
    // `t.append(new CharSequence() { … })`: the anonymous `CharSequence`
    // reads `longTestString`, so its characters are those of the string.
    t.append_string(Some(long_test_string));
    Assert::assert_equals_str(
        None,
        Some(&format!("4567890123456{long_test_string}")),
        Some(&t.to_string()),
    );
    Ok(())
}

fn test_non_char_sequence_append() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    t.append_string(Some("0123456789"));
    t.append_string(Some("0123456789"));
    Assert::assert_equals_str(None, Some("01234567890123456789"), Some(&t.to_string()));
    // `t.append(new StringBuilder("0123456789"))`.
    t.append_string(Some("0123456789"));
    Assert::assert_equals_str(
        None,
        Some("012345678901234567890123456789"),
        Some(&t.to_string()),
    );
    let mut t2 = CharTermAttributeImpl::new();
    t2.append_string(Some("test"));
    t.append_char_term_attribute(Some(&t2));
    Assert::assert_equals_str(
        None,
        Some("012345678901234567890123456789test"),
        Some(&t.to_string()),
    );
    // `t.append((String) null)`
    t.append_string(None);
    // `t.append((StringBuilder) null)`
    t.append_string(None);
    // `t.append((CharTermAttribute) null)`
    t.append_char_term_attribute(None);
    Assert::assert_equals_str(
        None,
        Some("012345678901234567890123456789testnullnullnull"),
        Some(&t.to_string()),
    );
    Ok(())
}

fn test_exceptions() -> Result<()> {
    let mut t = CharTermAttributeImpl::new();
    t.append_string(Some("test"));
    Assert::assert_equals_str(None, Some("test"), Some(&t.to_string()));

    LuceneTestCase::expect_throws(IOOBE, || {
        t.char_at(MINUS_ONE);
        Ok(())
    });

    LuceneTestCase::expect_throws(IOOBE, || {
        t.char_at(4);
        Ok(())
    });

    LuceneTestCase::expect_throws(IOOBE, || {
        t.sub_sequence(0, 5);
        Ok(())
    });

    LuceneTestCase::expect_throws(IOOBE, || {
        t.sub_sequence(5, 0);
        Ok(())
    });
    Ok(())
}

/// `assertCloneIsEqual(T att)` (`public static`).
pub fn assert_clone_is_equal<T: AttributeImpl + Clone>(att: &T) -> T {
    let clone = att.clone();
    Assert::assert_true_msg("Clone must be equal", att.equals(&clone));
    Assert::assert_equals_msg(
        Some("Clone's hashcode must be equal"),
        att.hash_code(),
        clone.hash_code(),
    );
    clone
}

/// `assertCopyIsEqual(T att)` (`public static`): the copy is a new instance
/// of the attribute's class, `att.getClass().getConstructor().newInstance()`.
pub fn assert_copy_is_equal<T: AttributeImpl + Default>(att: &T) -> T {
    let mut copy = T::default();
    att.copy_to(&mut copy);
    Assert::assert_true_msg("Copied instance must be equal", att.equals(&copy));
    Assert::assert_equals_msg(
        Some("Copied instance's hashcode must be equal"),
        att.hash_code(),
        copy.hash_code(),
    );
    copy
}

/// The class `org.apache.lucene.analysis.tokenattributes.TestCharTermAttributeImpl`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.tokenattributes.TestCharTermAttributeImpl")
        .method(t("testResize", test_resize))
        .method(t("testSetLength", test_set_length))
        .method(t("testGrow", test_grow))
        .method(t("testToString", test_to_string))
        .method(t("testClone", test_clone))
        .method(t("testEquals", test_equals))
        .method(t("testCopyTo", test_copy_to))
        .method(t("testAttributeReflection", test_attribute_reflection))
        .method(t("testCharSequenceInterface", test_char_sequence_interface))
        .method(t("testAppendableInterface", test_appendable_interface))
        .method(t(
            "testAppendableInterfaceWithLongSequences",
            test_appendable_interface_with_long_sequences,
        ))
        .method(t(
            "testNonCharSequenceAppend",
            test_non_char_sequence_append,
        ))
        .method(t("testExceptions", test_exceptions))
        .build()
}
