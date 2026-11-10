//! Port of `org.apache.lucene.analysis.TestCharacterUtils`: "TestCase for the
//! `CharacterUtils` class."
//!
//! `CharacterBuffer`, `toCodePoints`, `toChars` and `CharReader` hold UTF-16
//! code units (`u16`), as Java's `char[]` and `Reader` do (user decision,
//! sprint 9). A Java string with a lone surrogate is compared by its units.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::character_utils::{
    fill_buffer, new_character_buffer, to_chars, to_code_points, to_lower_case_utf16,
    to_upper_case_utf16, CharacterBuffer,
};
use rucene::analysis::StringCharReader;
use rucene::error::Result;
use rucene_test_framework::index::base_index_file_format_test_case::assert_array_equals;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::lucene_core_shims::{lucene_class, t};

/// `new String(char[])`.
fn string(chars: &[u16]) -> String {
    String::from_utf16_lossy(chars)
}

/// `new String(buffer.getBuffer(), buffer.getOffset(), buffer.getLength())`.
fn content(buffer: &CharacterBuffer) -> String {
    string(&buffer.buffer()[buffer.offset()..buffer.offset() + buffer.length()])
}

fn test_lower_upper() -> Result<()> {
    let mut reader = StringCharReader::new("ABc");
    let mut buffer = new_character_buffer(3)?;
    Assert::assert_true(fill_buffer(&mut buffer, &mut reader)?);
    Assert::assert_equals(3usize, buffer.length());
    // `CharacterUtils.toLowerCase(buffer.getBuffer(), 1, 3)`: the buffer is
    // borrowed by the call, so it is copied out and back.
    let mut chars: Vec<u16> = buffer.buffer().to_vec();
    to_lower_case_utf16(&mut chars, 1, 3);
    Assert::assert_equals_str(None, Some("Abc"), Some(&string(&chars)));
    to_upper_case_utf16(&mut chars, 1, 3);
    Assert::assert_equals_str(None, Some("ABC"), Some(&string(&chars)));
    Ok(())
}

fn test_conversions() -> Result<()> {
    let orig: Vec<u16> = TestUtil::random_unicode_string_max(&*LuceneTestCase::random(), 100)
        .encode_utf16()
        .collect();
    let mut buf = vec![0i32; orig.len()];
    let mut restored = vec![0u16; buf.len()];
    let o1 = TestUtil::next_int(&*LuceneTestCase::random(), 0, 5.min(orig.len() as i32)) as usize;
    let o2 = TestUtil::next_int(&*LuceneTestCase::random(), 0, o1 as i32) as usize;
    let o3 = TestUtil::next_int(&*LuceneTestCase::random(), 0, o1 as i32) as usize;
    let code_point_count = to_code_points(&orig, o1, orig.len() - o1, &mut buf, o2)?;
    let char_count = to_chars(&buf, o2, code_point_count, &mut restored, o3)?;
    Assert::assert_equals(orig.len() - o1, char_count);
    assert_array_equals(&orig[o1..o1 + char_count], &restored[o3..o3 + char_count]);
    Ok(())
}

fn test_new_character_buffer() -> Result<()> {
    let mut new_character_buffer_ = new_character_buffer(1024)?;
    Assert::assert_equals(1024usize, new_character_buffer_.buffer().len());
    Assert::assert_equals(0usize, new_character_buffer_.offset());
    Assert::assert_equals(0usize, new_character_buffer_.length());

    new_character_buffer_ = new_character_buffer(2)?;
    Assert::assert_equals(2usize, new_character_buffer_.buffer().len());
    Assert::assert_equals(0usize, new_character_buffer_.offset());
    Assert::assert_equals(0usize, new_character_buffer_.length());

    // length must be >= 2
    expect_throws("java.lang.IllegalArgumentException", || {
        new_character_buffer(1)
    });
    Ok(())
}

fn test_fill_no_high_surrogate() -> Result<()> {
    let mut reader = StringCharReader::new("helloworld");
    let mut buffer = new_character_buffer(6)?;
    Assert::assert_true(fill_buffer(&mut buffer, &mut reader)?);
    Assert::assert_equals(0usize, buffer.offset());
    Assert::assert_equals(6usize, buffer.length());
    Assert::assert_equals_str(None, Some("hellow"), Some(&string(buffer.buffer())));
    Assert::assert_false(fill_buffer(&mut buffer, &mut reader)?);
    Assert::assert_equals(4usize, buffer.length());
    Assert::assert_equals(0usize, buffer.offset());

    Assert::assert_equals_str(None, Some("orld"), Some(&content(&buffer)));
    Assert::assert_false(fill_buffer(&mut buffer, &mut reader)?);
    Ok(())
}

/// The UTF-16 units of `new String(buffer.getBuffer(), buffer.getOffset(),
/// buffer.getLength())`.
fn content_units(buffer: &CharacterBuffer) -> Vec<u16> {
    buffer.buffer()[buffer.offset()..buffer.offset() + buffer.length()].to_vec()
}

fn test_fill() -> Result<()> {
    // "1234\ud801\udc1c789123\ud801\ud801\udc1c\ud801": a Java string with
    // lone high surrogates, as UTF-16 units.
    let mut input: Vec<u16> = "1234".encode_utf16().collect();
    input.extend_from_slice(&[0xD801, 0xDC1C]);
    input.extend("789123".encode_utf16());
    input.extend_from_slice(&[0xD801, 0xD801, 0xDC1C, 0xD801]);
    let mut reader = StringCharReader::from_utf16(input);
    let mut buffer = new_character_buffer(5)?;
    Assert::assert_true(fill_buffer(&mut buffer, &mut reader)?);
    Assert::assert_equals(4usize, buffer.length());
    Assert::assert_equals_str(None, Some("1234"), Some(&content(&buffer)));
    Assert::assert_true(fill_buffer(&mut buffer, &mut reader)?);
    Assert::assert_equals(5usize, buffer.length());
    Assert::assert_equals_str(None, Some("\u{1041C}789"), Some(&string(buffer.buffer())));
    Assert::assert_true(fill_buffer(&mut buffer, &mut reader)?);
    Assert::assert_equals(4usize, buffer.length());
    // "123\ud801"
    assert_array_equals(&[0x31u16, 0x32, 0x33, 0xD801], &content_units(&buffer));
    Assert::assert_false(fill_buffer(&mut buffer, &mut reader)?);
    Assert::assert_equals(3usize, buffer.length());
    // "\ud801\udc1c\ud801"
    assert_array_equals(&[0xD801u16, 0xDC1C, 0xD801], &content_units(&buffer));
    Assert::assert_false(fill_buffer(&mut buffer, &mut reader)?);
    Assert::assert_equals(0usize, buffer.length());
    Ok(())
}

/// The class `org.apache.lucene.analysis.TestCharacterUtils`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestCharacterUtils")
        .method(t("testLowerUpper", test_lower_upper))
        .method(t("testConversions", test_conversions))
        .method(t("testNewCharacterBuffer", test_new_character_buffer))
        .method(t("testFillNoHighSurrogate", test_fill_no_high_surrogate))
        .method(t("testFill", test_fill))
        .build()
}
