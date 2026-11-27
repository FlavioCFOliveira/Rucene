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

//! Port of `TestWholeBreakIterator.java`, whose static `assertSameBreaks`
//! helpers `TestCustomSeparatorBreakIterator` reuses.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_analysis_common::jdk::break_iterator::{get_sentence_instance, BreakIterator, DONE};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, StringCharacterIterator};
use rucene_analysis_common::jdk::locale::Locale;
use rucene_highlighter::uhighlight::whole_break_iterator::WholeBreakIterator;

use crate::highlighter_support::core_support::{class, test};

/// `TestWholeBreakIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.uhighlight.TestWholeBreakIterator",
        vec![
            test("testSingleSentences", test_single_sentences),
            test("testSliceEnd", test_slice_end),
            test("testSliceStart", test_slice_start),
            test("testSliceMiddle", test_slice_middle),
            test("testFirstPosition", test_first_position),
        ],
    )
}

/// `Locale.ROOT`.
pub fn root() -> Locale {
    Locale::of_parts("", "", "", "")
}

fn whole() -> Box<dyn BreakIterator> {
    Box::new(WholeBreakIterator::new())
}

/// For single sentences, we know WholeBreakIterator should break the same as
/// a sentence iterator.
fn test_single_sentences() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = whole();
    assert_same_breaks("a", &mut *expected, &mut *actual)?;
    assert_same_breaks("ab", &mut *expected, &mut *actual)?;
    assert_same_breaks("abc", &mut *expected, &mut *actual)?;
    assert_same_breaks("", &mut *expected, &mut *actual)
}

fn test_slice_end() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = whole();
    assert_same_breaks_range("a000", 0, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("ab000", 0, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("abc000", 0, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000", 0, 0, &mut *expected, &mut *actual)
}

fn test_slice_start() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = whole();
    assert_same_breaks_range("000a", 3, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000ab", 3, 2, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000abc", 3, 3, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000", 3, 0, &mut *expected, &mut *actual)
}

fn test_slice_middle() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = whole();
    assert_same_breaks_range("000a000", 3, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000ab000", 3, 2, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000abc000", 3, 3, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000000", 3, 0, &mut *expected, &mut *actual)
}

/// The current position must be ignored, initial position is always
/// `first()`.
fn test_first_position() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = whole();
    assert_same_breaks_at("000ab000", 3, 2, 4, &mut *expected, &mut *actual)
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// `assertSameBreaks(String, BreakIterator, BreakIterator)`.
pub fn assert_same_breaks(
    text: &str,
    expected: &mut dyn BreakIterator,
    actual: &mut dyn BreakIterator,
) -> Result<()> {
    assert_same_breaks_iter(
        Box::new(StringCharacterIterator::new(units(text))),
        Box::new(StringCharacterIterator::new(units(text))),
        expected,
        actual,
    )
}

/// `assertSameBreaks(String, int, int, BreakIterator, BreakIterator)`.
pub fn assert_same_breaks_range(
    text: &str,
    offset: i32,
    length: i32,
    expected: &mut dyn BreakIterator,
    actual: &mut dyn BreakIterator,
) -> Result<()> {
    assert_same_breaks_at(text, offset, length, offset, expected, actual)
}

/// `assertSameBreaks(String, int, int, int, BreakIterator, BreakIterator)`.
pub fn assert_same_breaks_at(
    text: &str,
    offset: i32,
    length: i32,
    current: i32,
    expected: &mut dyn BreakIterator,
    actual: &mut dyn BreakIterator,
) -> Result<()> {
    assert_same_breaks_iter(
        Box::new(StringCharacterIterator::with_range(
            units(text),
            offset,
            offset + length,
            current,
        )?),
        Box::new(StringCharacterIterator::with_range(
            units(text),
            offset,
            offset + length,
            current,
        )?),
        expected,
        actual,
    )
}

/// Asserts that two breakiterators break the text the same way:
/// `assertSameBreaks(CharacterIterator, CharacterIterator, BreakIterator,
/// BreakIterator)`.
pub fn assert_same_breaks_iter(
    one: Box<dyn CharacterIterator>,
    two: Box<dyn CharacterIterator>,
    expected: &mut dyn BreakIterator,
    actual: &mut dyn BreakIterator,
) -> Result<()> {
    let begin = one.get_begin_index();
    let end = one.get_end_index();
    expected.set_text(one);
    actual.set_text(two);

    Assert::assert_equals(expected.current(), actual.current());

    // next()
    let mut v = expected.current();
    while v != DONE {
        v = expected.next();
        Assert::assert_equals(v, actual.next());
        Assert::assert_equals(expected.current(), actual.current());
    }

    // first()
    Assert::assert_equals(expected.first(), actual.first());
    Assert::assert_equals(expected.current(), actual.current());
    // last()
    Assert::assert_equals(expected.last(), actual.last());
    Assert::assert_equals(expected.current(), actual.current());

    // previous()
    v = expected.current();
    while v != DONE {
        v = expected.previous();
        Assert::assert_equals(v, actual.previous());
        Assert::assert_equals(expected.current(), actual.current());
    }

    // following()
    for i in begin..=end {
        expected.first();
        actual.first();
        Assert::assert_equals(expected.following(i)?, actual.following(i)?);
        Assert::assert_equals(expected.current(), actual.current());
    }

    // preceding()
    for i in begin..=end {
        expected.last();
        actual.last();
        Assert::assert_equals(expected.preceding(i)?, actual.preceding(i)?);
        Assert::assert_equals(expected.current(), actual.current());
    }
    Ok(())
}
