//! Port of `lucene/core/src/test/org/apache/lucene/util/TestUnicodeUtil.java`.
//!
//! Java `String`s are sequences of UTF-16 code units; the strings the test
//! draws are valid Unicode, so each is a Rust `String` whose UTF-16 form
//! (`encode_utf16`) is Java's `char[]`. `testNewString` needs lone surrogates,
//! which only the UTF-16 form can hold.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{AutomatonBuilder, CompiledAutomaton, FiniteStringsIterator};
use rucene::util::{CharsRefBuilder, UnicodeUtil};
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestUnicodeUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestUnicodeUtil",
        vec![
            test("testCodePointCount", test_code_point_count),
            test("testUTF8toUTF32", test_utf8_to_utf32),
            test("testUTF8CodePointAt", test_utf8_code_point_at),
            test("testUTF8SpanMultipleBytes", test_utf8_span_multiple_bytes),
            test("testNewString", test_new_string),
            test("testUTF8UTF16CharsRef", test_utf8_utf16_chars_ref),
            test("testCalcUTF16toUTF8Length", test_calc_utf16_to_utf8_length),
        ],
    )
}

use LuceneTestCase as Ltc;

/// `asByteArray(int... ints)`.
fn as_byte_array(ints: &[i32]) -> Vec<u8> {
    ints.iter().map(|&i| i as u8).collect()
}

/// `assertcodePointCountThrowsAssertionOn(byte... bytes)`.
fn assert_code_point_count_throws_assertion_on(bytes: &[u8]) {
    expect_throws("java.lang.IllegalArgumentException", || {
        UnicodeUtil::code_point_count(&Ltc::new_bytes_ref_bytes(bytes))
    });
}

/// `UnicodeUtil.UTF16toUTF8(s, 0, s.length(), utf8)` into a buffer of
/// `maxUTF8Length(s.length())`: the buffer and the encoded length.
fn utf16_to_utf8(s: &str) -> (Vec<u16>, Vec<u8>, usize) {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut utf8 = vec![0u8; UnicodeUtil::max_utf8_length(units.len())];
    let utf8_len = UnicodeUtil::utf16_to_utf8(&units, 0, units.len(), &mut utf8);
    (units, utf8, utf8_len)
}

fn test_code_point_count() -> Result<()> {
    // Check invalid codepoints.
    assert_code_point_count_throws_assertion_on(&as_byte_array(&[
        'z' as i32, 0x80, 'z' as i32, 'z' as i32, 'z' as i32,
    ]));
    assert_code_point_count_throws_assertion_on(&as_byte_array(&[
        'z' as i32,
        0xc0 - 1,
        'z' as i32,
        'z' as i32,
        'z' as i32,
    ]));
    // Check 5-byte and longer sequences.
    assert_code_point_count_throws_assertion_on(&as_byte_array(&[
        'z' as i32, 0xf8, 'z' as i32, 'z' as i32, 'z' as i32,
    ]));
    assert_code_point_count_throws_assertion_on(&as_byte_array(&[
        'z' as i32, 0xfc, 'z' as i32, 'z' as i32, 'z' as i32,
    ]));
    // Check improperly terminated codepoints.
    assert_code_point_count_throws_assertion_on(&as_byte_array(&['z' as i32, 0xc2]));
    assert_code_point_count_throws_assertion_on(&as_byte_array(&['z' as i32, 0xe2]));
    assert_code_point_count_throws_assertion_on(&as_byte_array(&['z' as i32, 0xe2, 0x82]));
    assert_code_point_count_throws_assertion_on(&as_byte_array(&['z' as i32, 0xf0]));
    assert_code_point_count_throws_assertion_on(&as_byte_array(&['z' as i32, 0xf0, 0xa4]));
    assert_code_point_count_throws_assertion_on(&as_byte_array(&['z' as i32, 0xf0, 0xa4, 0xad]));

    // Check some typical examples (multibyte).
    assert_equals(
        &0,
        &UnicodeUtil::code_point_count(&Ltc::new_bytes_ref_bytes(&as_byte_array(&[])))?,
    );
    assert_equals(
        &3,
        &UnicodeUtil::code_point_count(&Ltc::new_bytes_ref_bytes(&as_byte_array(&[
            'z' as i32, 'z' as i32, 'z' as i32,
        ])))?,
    );
    assert_equals(
        &2,
        &UnicodeUtil::code_point_count(&Ltc::new_bytes_ref_bytes(&as_byte_array(&[
            'z' as i32, 0xc2, 0xa2,
        ])))?,
    );
    assert_equals(
        &2,
        &UnicodeUtil::code_point_count(&Ltc::new_bytes_ref_bytes(&as_byte_array(&[
            'z' as i32, 0xe2, 0x82, 0xac,
        ])))?,
    );
    assert_equals(
        &2,
        &UnicodeUtil::code_point_count(&Ltc::new_bytes_ref_bytes(&as_byte_array(&[
            'z' as i32, 0xf0, 0xa4, 0xad, 0xa2,
        ])))?,
    );

    // And do some random stuff.
    let num = Ltc::at_least(50000);
    for _ in 0..num {
        let s = TestUtil::random_unicode_string(&*random());
        let (_, utf8, utf8_len) = utf16_to_utf8(&s);
        assert_equals(
            &s.chars().count(),
            &UnicodeUtil::code_point_count(&Ltc::new_bytes_ref_slice(&utf8, 0, utf8_len))?,
        );
    }
    Ok(())
}

fn test_utf8_to_utf32() -> Result<()> {
    let mut utf32: Vec<i32> = Vec::new();

    let num = Ltc::at_least(50000);
    for _ in 0..num {
        let s = TestUtil::random_unicode_string(&*random());
        let (units, utf8, utf8_len) = utf16_to_utf8(&s);
        // `ArrayUtil.grow(utf32, utf8Len)`.
        if utf32.len() < utf8_len {
            utf32.resize(utf8_len, 0);
        }
        let utf32_len =
            UnicodeUtil::utf8_to_utf32(&Ltc::new_bytes_ref_slice(&utf8, 0, utf8_len), &mut utf32)?;

        let code_points: Vec<i32> = s.chars().map(|c| c as i32).collect();
        if code_points[..] != utf32[0..code_points.len()] {
            println!("FAILED");
            for (j, u) in units.iter().enumerate() {
                println!("  char[{j}]={u:x}");
            }
            println!();
            assert_equals(&code_points.len(), &utf32_len);
            for j in 0..code_points.len() {
                println!("  {:x} vs {:x}", utf32[j], code_points[j]);
            }
            Assert::fail("mismatch");
        }
    }
    Ok(())
}

fn test_utf8_code_point_at() -> Result<()> {
    let num = Ltc::at_least(50000);
    // Java's `reuse` instance: Rucene returns a fresh value per call.
    for _ in 0..num {
        let s = TestUtil::random_unicode_string(&*random());
        let (_, utf8, utf8_len) = utf16_to_utf8(&s);

        let expected: Vec<i32> = s.chars().map(|c| c as i32).collect();
        let mut pos = 0usize;
        let mut expected_upto = 0usize;
        while pos < utf8_len {
            let reuse = UnicodeUtil::code_point_at(&utf8, pos)?;
            assert_equals(&expected[expected_upto], &(reuse.code_point as i32));
            expected_upto += 1;
            pos += reuse.num_bytes;
        }
        assert_equals(&utf8_len, &pos);
        assert_equals(&expected.len(), &expected_upto);
    }
    Ok(())
}

fn test_utf8_span_multiple_bytes() -> Result<()> {
    let mut b = AutomatonBuilder::new();
    // start state:
    let s1 = b.create_state();
    // single end accept state:
    let s2 = b.create_state();
    b.set_accept(s2, true);
    // utf8 codepoint length range from [1,2]
    b.add_transition_range(s1, s2, 0x7F, 0x80);
    // utf8 codepoint length range from [2,3]
    b.add_transition_range(s1, s2, 0x7FF, 0x800);
    // utf8 codepoint length range from [3,4]
    b.add_transition_range(s1, s2, 0xFFFF, 0x10000);
    let a = b.finish();

    // `new CompiledAutomaton(a)`: finite unknown, simplify, not binary.
    let c = CompiledAutomaton::new(a, false, true, false)?;
    let automaton = c.automaton.as_ref().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.NullPointerException",
            "c.automaton".to_string(),
        )
        .throw()
    });
    let mut it = FiniteStringsIterator::new(automaton);
    let mut term_count = 0;
    while it.next()?.is_some() {
        term_count += 1;
    }
    /*
    CompiledAutomaton.automaton contains six terms in UTF-8 byte form:
    [0x7F], [0xC2, 0x80], [0xDF, 0xBF], [0xE0, 0xA0, 0x80], [0xEF, 0xBF, 0xBF], [0xF0, 0x90, 0x80, 0x80]
    */
    assert_equals(&6, &term_count);
    Ok(())
}

const MIN_HIGH_SURROGATE: u16 = 0xD800;
const MAX_HIGH_SURROGATE: u16 = 0xDBFF;
const MIN_LOW_SURROGATE: u16 = 0xDC00;
const MAX_LOW_SURROGATE: u16 = 0xDFFF;

/// `Character.toCodePoint(high, low)`.
fn to_code_point(high: u16, low: u16) -> i32 {
    ((((high as i32) << 10) + low as i32) + (0x010000 - (0xD800 << 10) - 0xDC00)) as i32
}

fn test_new_string() -> Result<()> {
    let code_points: [i32; 5] = [
        to_code_point(MIN_HIGH_SURROGATE, MAX_LOW_SURROGATE),
        to_code_point(MAX_HIGH_SURROGATE, MIN_LOW_SURROGATE),
        MAX_HIGH_SURROGATE as i32,
        'A' as i32,
        -1,
    ];

    // Java's `String`, as UTF-16 code units (it holds lone surrogates).
    let cp_string: [u16; 6] = [
        MIN_HIGH_SURROGATE,
        MAX_LOW_SURROGATE,
        MAX_HIGH_SURROGATE,
        MIN_LOW_SURROGATE,
        MAX_HIGH_SURROGATE,
        'A' as u16,
    ];

    let tests: [[i32; 4]; 9] = [
        [0, 1, 0, 2],
        [0, 2, 0, 4],
        [1, 1, 2, 2],
        [1, 2, 2, 3],
        [1, 3, 2, 4],
        [2, 2, 4, 2],
        [2, 3, 0, -1],
        [4, 5, 0, -1],
        [3, -1, 0, -1],
    ];

    for t in tests.iter() {
        let s = t[0];
        let c = t[1];
        let rs = t[2];
        let rc = t[3];

        // try { ... } catch (IndexOutOfBoundsException | IllegalArgumentException e1)
        // `newString` returns a Java `String`, whose UTF-16 code units may hold
        // lone surrogates.
        match UnicodeUtil::new_string_utf16(&code_points, s, c) {
            Ok(actual) => {
                Assert::assert_false(rc == -1);
                let expected = &cp_string[rs as usize..(rs + rc) as usize];
                assert_equals(&expected.to_vec(), &actual);
                continue;
            }
            Err(e) => {
                if !is_instance(&e, "java.lang.IndexOutOfBoundsException")
                    && !is_instance(&e, "java.lang.IllegalArgumentException")
                {
                    return Err(e);
                }
                // Ignored.
            }
        }
        Assert::assert_true(rc == -1);
    }
    Ok(())
}

fn test_utf8_utf16_chars_ref() -> Result<()> {
    let num = Ltc::at_least(3989);
    for _ in 0..num {
        let unicode = TestUtil::random_realistic_unicode_string(&*random());
        let r = Ltc::new_bytes_ref_str(&unicode);
        let mut c_ref = CharsRefBuilder::new();
        c_ref.copy_utf8_bytes_ref(&r);
        assert_equals(&c_ref.to_string(), &unicode);
    }
    Ok(())
}

fn test_calc_utf16_to_utf8_length() -> Result<()> {
    let num = Ltc::at_least(5000);
    for _ in 0..num {
        let unicode = TestUtil::random_unicode_string(&*random());
        let (units, _, len) = utf16_to_utf8(&unicode);
        assert_equals(
            &len,
            &UnicodeUtil::calc_utf16_to_utf8_length(&unicode, 0, units.len()),
        );
    }
    Ok(())
}
