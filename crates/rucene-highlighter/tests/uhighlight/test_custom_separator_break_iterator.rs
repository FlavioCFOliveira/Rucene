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

//! Port of `TestCustomSeparatorBreakIterator.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_analysis_common::jdk::break_iterator::{get_sentence_instance, BreakIterator, DONE};
use rucene_highlighter::uhighlight::custom_separator_break_iterator::CustomSeparatorBreakIterator;

use crate::highlighter_support::core_support::{class, random, test};
use crate::uhighlight::test_whole_break_iterator::{
    assert_same_breaks, assert_same_breaks_at, assert_same_breaks_range, root,
};

/// `SEPARATORS`: `' '`, `'\u0000'`, `8233`.
const SEPARATORS: [u16; 3] = [b' ' as u16, 0, 8233];

/// `TestCustomSeparatorBreakIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.uhighlight.TestCustomSeparatorBreakIterator",
        vec![
            test("testBreakOnCustomSeparator", test_break_on_custom_separator),
            test("testSingleSentences", test_single_sentences),
            test("testSliceEnd", test_slice_end),
            test("testSliceStart", test_slice_start),
            test("testSliceMiddle", test_slice_middle),
            test("testFirstPosition", test_first_position),
        ],
    )
}

/// `String.substring(begin, end)` over UTF-16 units.
fn substring(source: &[u16], begin: i32, end: i32) -> String {
    assert!(
        begin >= 0 && begin <= end && end as usize <= source.len(),
        "java.lang.StringIndexOutOfBoundsException: begin {begin}, end {end}, length {}",
        source.len()
    );
    String::from_utf16(&source[begin as usize..end as usize]).expect("BMP text")
}

fn test_break_on_custom_separator() -> Result<()> {
    let separator = random_separator();
    let mut bi = CustomSeparatorBreakIterator::new(separator);
    let sep = String::from_utf16(&[separator]).expect("BMP separator");
    let source = format!("this{sep}is{sep}the{sep}first{sep}sentence");
    let src: Vec<u16> = source.encode_utf16().collect();
    bi.set_text_str(&source);

    Assert::assert_equals(0, bi.current());
    Assert::assert_equals(0, bi.first());
    let forward = |expected: String, bi: &mut CustomSeparatorBreakIterator| {
        let current = bi.current();
        let next = bi.next();
        Assert::assert_equals(expected, substring(&src, current, next));
    };
    forward(format!("this{sep}"), &mut bi);
    forward(format!("is{sep}"), &mut bi);
    forward(format!("the{sep}"), &mut bi);
    forward(format!("first{sep}"), &mut bi);
    forward("sentence".to_string(), &mut bi);
    Assert::assert_equals(DONE, bi.next());

    Assert::assert_equals(src.len() as i32, bi.last());
    let backward = |expected: String, bi: &mut CustomSeparatorBreakIterator| {
        let current = bi.current();
        let previous = bi.previous();
        Assert::assert_equals(expected, substring(&src, previous, current));
    };
    backward("sentence".to_string(), &mut bi);
    backward(format!("first{sep}"), &mut bi);
    backward(format!("the{sep}"), &mut bi);
    backward(format!("is{sep}"), &mut bi);
    backward(format!("this{sep}"), &mut bi);
    Assert::assert_equals(DONE, bi.previous());
    Assert::assert_equals(0, bi.current());

    Assert::assert_equals(
        format!("this{sep}is{sep}the{sep}"),
        substring(&src, 0, bi.following(9)?),
    );

    Assert::assert_equals(
        format!("this{sep}is{sep}"),
        substring(&src, 0, bi.preceding(9)?),
    );

    Assert::assert_equals(0, bi.first());
    Assert::assert_equals(
        format!("this{sep}is{sep}the{sep}"),
        substring(&src, 0, bi.next_n(3)),
    );
    Ok(())
}

fn custom() -> Box<dyn BreakIterator> {
    Box::new(CustomSeparatorBreakIterator::new(random_separator()))
}

fn test_single_sentences() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = custom();
    assert_same_breaks("a", &mut *expected, &mut *actual)?;
    assert_same_breaks("ab", &mut *expected, &mut *actual)?;
    assert_same_breaks("abc", &mut *expected, &mut *actual)?;
    assert_same_breaks("", &mut *expected, &mut *actual)
}

fn test_slice_end() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = custom();
    assert_same_breaks_range("a000", 0, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("ab000", 0, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("abc000", 0, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000", 0, 0, &mut *expected, &mut *actual)
}

fn test_slice_start() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = custom();
    assert_same_breaks_range("000a", 3, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000ab", 3, 2, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000abc", 3, 3, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000", 3, 0, &mut *expected, &mut *actual)
}

fn test_slice_middle() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = custom();
    assert_same_breaks_range("000a000", 3, 1, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000ab000", 3, 2, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000abc000", 3, 3, &mut *expected, &mut *actual)?;
    assert_same_breaks_range("000000", 3, 0, &mut *expected, &mut *actual)
}

/// The current position must be ignored, initial position is always
/// `first()`.
fn test_first_position() -> Result<()> {
    let mut expected = get_sentence_instance(&root());
    let mut actual = custom();
    assert_same_breaks_at("000ab000", 3, 2, 4, &mut *expected, &mut *actual)
}

fn random_separator() -> u16 {
    *RandomPicks::random_from(&*random(), &SEPARATORS)
}
