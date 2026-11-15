/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `TestUnescapedCharSequence.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_queryparser::flexible::core::util::unescaped_char_sequence::UnescapedCharSequence;
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_queryparser::jdk::locale::Locale;

use crate::qp_support::core_support::{class, test};
use crate::qp_support::{assert_str_eq, jdk_default_hooks};

/// `TestUnescapedCharSequence`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        test("testToStringEscaped", test_to_string_escaped),
        test(
            "testToStringEscapedWithEnabledChars",
            test_to_string_escaped_with_enabled_chars,
        ),
        test("testSubSequence", test_sub_sequence),
        test("testToLowerCase", test_to_lower_case),
    ]);
    class(
        "org.apache.lucene.queryparser.flexible.core.util.TestUnescapedCharSequence",
        methods,
    )
}

/// `private static final char[] wildcardChars = {'*', '?'}`.
const WILDCARD_CHARS: [u16; 2] = [b'*' as u16, b'?' as u16];

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn test_to_string_escaped() -> Result<()> {
    let chars = units("abc\\e");
    let was_escaped = [false, true, true, false, false];
    let sequence = UnescapedCharSequence::new(&chars, &was_escaped, 0, chars.len())?;
    assert_str_eq("a\\b\\c\\\\e", &sequence.to_string_escaped());
    Assert::assert_false(sequence.was_escaped(0)?);
    Assert::assert_true(sequence.was_escaped(1)?);
    Ok(())
}

fn test_to_string_escaped_with_enabled_chars() -> Result<()> {
    let chars = units("abc?*");
    let was_escaped = [true, true, true, true, true];
    let sequence = UnescapedCharSequence::new(&chars, &was_escaped, 0, chars.len())?;
    assert_str_eq(
        "abc\\?\\*",
        &sequence.to_string_escaped_with(&WILDCARD_CHARS),
    );
    Ok(())
}

fn test_sub_sequence() -> Result<()> {
    let sequence = UnescapedCharSequence::from_text(&CharSequence::from("abcdef"));
    assert_str_eq("bc", &sequence.sub_sequence(1, 3)?.to_string());
    Ok(())
}

fn test_to_lower_case() -> Result<()> {
    let sequence = UnescapedCharSequence::from_text(&CharSequence::from("ABC"));
    // the same object, referenced as a `CharSequence`
    let as_char_sequence = sequence.clone().into_char_sequence();
    assert_str_eq(
        "abc",
        &UnescapedCharSequence::to_lower_case(&as_char_sequence, &Locale::get_default())?
            .to_string(),
    );
    Assert::assert_false(sequence.was_escaped(0)?);
    Assert::assert_false(sequence.was_escaped(1)?);
    Assert::assert_false(sequence.was_escaped(2)?);
    Ok(())
}
