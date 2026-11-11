//! Port of `lucene/core/src/test/org/apache/lucene/util/TestCharsRefBuilder.java`.
//!
//! Java `String` lengths, indices and `CharSequence`s are UTF-16 units, as
//! `CharsRefBuilder` stores them.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::CharsRefBuilder;
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};

/// `TestCharsRefBuilder`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestCharsRefBuilder",
        vec![
            test("testAppend", test_append),
            test("testAppendNull", test_append_null),
        ],
    )
}

fn test_append() -> Result<()> {
    let s = TestUtil::random_unicode_string_max(&*random(), 100);
    let s_chars: Vec<u16> = s.encode_utf16().collect();
    let mut builder = CharsRefBuilder::new();
    while builder.length() < s_chars.len() {
        if random().next_boolean() {
            builder.append(s_chars[builder.length()]);
        } else {
            let start = builder.length();
            let end = TestUtil::next_int(&*random(), start as i32, s_chars.len() as i32) as usize;
            if random().next_boolean() {
                let sub: Vec<u16> = s_chars[start..end].to_vec();
                builder.append_str(Some(&sub));
            } else {
                builder.append_str_range(Some(&s_chars), start, end);
            }
        }
    }
    assert_equals(&s, &builder.to_string());
    Ok(())
}

fn test_append_null() -> Result<()> {
    let mut builder = CharsRefBuilder::new();
    builder.append_str(None);
    builder.append_str_range(None, 1, 3);
    assert_equals("nullnull", builder.to_string().as_str());
    Ok(())
}
