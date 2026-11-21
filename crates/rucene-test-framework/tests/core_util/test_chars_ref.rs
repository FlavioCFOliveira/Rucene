//! Port of `lucene/core/src/test/org/apache/lucene/util/TestCharsRef.java`.
//!
//! Rucene's `CharsRef` stores UTF-16 units, as Java's `char[]`, so Java's
//! `toCharArray()` is `encode_utf16().collect()` here; negative Java indices are passed as their
//! two's-complement `usize`, which the bounds checks reject as Java's do.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::{BytesRef, CharsRef, CharsRefBuilder};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, expect_throws_panic, random, test, unchecked};

const IOOBE: &str = "java.lang.IndexOutOfBoundsException";

/// Java's `-1` passed where Rucene takes a `usize`.
const MINUS_ONE: usize = -1i64 as usize;

/// `TestCharsRef`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestCharsRef",
        vec![
            test("testUTF16InUTF8Order", test_utf16_in_utf8_order),
            test("testAppend", test_append),
            test("testCopy", test_copy),
            test("testCharSequenceCharAt", test_char_sequence_char_at),
            test(
                "testCharSequenceSubSequence",
                test_char_sequence_sub_sequence,
            ),
            test("testInvalidDeepCopy", test_invalid_deep_copy),
        ],
    )
}

fn test_utf16_in_utf8_order() -> Result<()> {
    let num_strings = LuceneTestCase::at_least(1000);
    let mut utf8: Vec<BytesRef> = Vec::with_capacity(num_strings as usize);
    let mut utf16: Vec<CharsRef> = Vec::with_capacity(num_strings as usize);

    for _ in 0..num_strings {
        let s = TestUtil::random_unicode_string(&*random());
        utf8.push(BytesRef::from_char_sequence(&s));
        utf16.push(CharsRef::from_string(&s));
    }

    utf8.sort();
    utf16.sort_by(CharsRef::get_utf16_sorted_as_utf8_comparator());

    for i in 0..num_strings as usize {
        assert_equals(&unchecked(utf8[i].utf8_to_string()), &utf16[i].to_string());
    }
    Ok(())
}

fn test_append() -> Result<()> {
    let mut r#ref = CharsRefBuilder::new();
    // Java's `StringBuilder`, a sequence of UTF-16 units.
    let mut builder: Vec<u16> = Vec::new();
    let num_strings = LuceneTestCase::at_least(10);
    for _ in 0..num_strings {
        let char_array: Vec<u16> =
            TestUtil::random_realistic_unicode_string_between(&*random(), 1, 100)
                .encode_utf16()
                .collect();
        let offset = random().next_int_bound(char_array.len() as i32) as usize;
        let length = char_array.len() - offset;
        builder.extend_from_slice(&char_array[offset..offset + length]);
        r#ref.append_chars(&char_array, offset, length);
    }

    assert_equals(builder.as_slice(), r#ref.get().chars());
    Ok(())
}

fn test_copy() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(10);
    for _ in 0..num_iters {
        let mut r#ref = CharsRefBuilder::new();
        let char_array: Vec<u16> =
            TestUtil::random_realistic_unicode_string_between(&*random(), 1, 100)
                .encode_utf16()
                .collect();
        let offset = random().next_int_bound(char_array.len() as i32) as usize;
        let length = char_array.len() - offset;
        // `new String(charArray, offset, length)`: the same UTF-16 units.
        let str: &[u16] = &char_array[offset..offset + length];
        r#ref.copy_chars(&char_array, offset, length);
        assert_equals(str, r#ref.get().chars());
    }
    Ok(())
}

// LUCENE-3590: fix charsequence to fully obey interface
fn test_char_sequence_char_at() -> Result<()> {
    let c = CharsRef::from_string("abc");

    assert_equals(&u16::from(b'b'), &c.char_at(1));

    expect_throws_panic(IOOBE, || {
        c.char_at(MINUS_ONE);
    });

    expect_throws_panic(IOOBE, || {
        c.char_at(3);
    });
    Ok(())
}

// LUCENE-3590: fix off-by-one in subsequence, and fully obey interface
// LUCENE-4671: fix subSequence
fn test_char_sequence_sub_sequence() -> Result<()> {
    let sequences: Vec<CharsRef> = vec![
        CharsRef::from_string("abc"),
        CharsRef::from_chars_offset("0abc".encode_utf16().collect(), 1, 3),
        CharsRef::from_chars_offset("abc0".encode_utf16().collect(), 0, 3),
        CharsRef::from_chars_offset("0abc0".encode_utf16().collect(), 1, 3),
    ];

    for c in &sequences {
        do_test_sequence(c);
    }
    Ok(())
}

/// `doTestSequence(CharSequence c)`.
fn do_test_sequence(c: &CharsRef) {
    // slice
    assert_equals("a", c.sub_sequence(0, 1).to_string().as_str());
    // mid subsequence
    assert_equals("b", c.sub_sequence(1, 2).to_string().as_str());
    // end subsequence
    assert_equals("bc", c.sub_sequence(1, 3).to_string().as_str());
    // empty subsequence
    assert_equals("", c.sub_sequence(0, 0).to_string().as_str());

    expect_throws_panic(IOOBE, || {
        c.sub_sequence(MINUS_ONE, 1);
    });

    expect_throws_panic(IOOBE, || {
        c.sub_sequence(0, MINUS_ONE);
    });

    expect_throws_panic(IOOBE, || {
        c.sub_sequence(0, 4);
    });

    expect_throws_panic(IOOBE, || {
        c.sub_sequence(2, 1);
    });
}

fn test_invalid_deep_copy() -> Result<()> {
    let mut from = CharsRef::from_chars_offset(vec![u16::from(b'a'), u16::from(b'b')], 0, 2);
    from.offset += 1; // now invalid
    expect_throws_panic(IOOBE, || {
        CharsRef::deep_copy_of(&from);
    });
    Ok(())
}
