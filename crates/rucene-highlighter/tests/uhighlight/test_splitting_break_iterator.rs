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

//! Port of `TestSplittingBreakIterator.java`.
//!
//! Java's static `LINE_BI` and `SPLIT_BI` (which wraps that same `LINE_BI`)
//! are shared by the methods; every check starts with `setText`, so each
//! method here builds them afresh, with the same breaks.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_analysis_common::jdk::break_iterator::{get_line_instance, BreakIterator, DONE};
use rucene_highlighter::uhighlight::splitting_break_iterator::SplittingBreakIterator;

use crate::highlighter_support::core_support::{class, random, shuffle, test};
use crate::uhighlight::test_whole_break_iterator::root;

/// `TestSplittingBreakIterator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.uhighlight.TestSplittingBreakIterator",
        vec![
            test("testLineBreakIterator", test_line_break_iterator),
            test("testWithoutSplits", test_without_splits),
            test("testOnlySingleSplitChar", test_only_single_split_char),
            test("testSplitThenValue", test_split_then_value),
            test("testValueThenSplit", test_value_then_split),
            test(
                "testValueThenSplitThenValue",
                test_value_then_split_then_value,
            ),
            test(
                "testValueThenDoubleSplitThenValue",
                test_value_then_double_split_then_value,
            ),
            test(
                "testValueThenSplitThenDoubleValueThenSplitThenValue",
                test_value_then_split_then_double_value_then_split_then_value,
            ),
        ],
    )
}

/// `LINE_BI`: `BreakIterator.getLineInstance(Locale.ROOT)`.
fn line_bi() -> Box<dyn BreakIterator> {
    get_line_instance(&root())
}

/// `SPLIT_BI`: `new SplittingBreakIterator(LINE_BI, '|')`.
fn split_bi() -> Box<dyn BreakIterator> {
    Box::new(SplittingBreakIterator::new(line_bi(), b'|' as u16))
}

fn test_line_break_iterator() -> Result<()> {
    without_splits(&mut *line_bi())
}

fn without_splits(bi: &mut dyn BreakIterator) -> Result<()> {
    break_iterator(bi, " a", "^^^")?;
    break_iterator(bi, "aa", "^ ^")?;
    break_iterator(bi, "aa a", "^  ^^")
}

fn test_without_splits() -> Result<()> {
    without_splits(&mut *split_bi())
}

fn test_only_single_split_char() -> Result<()> {
    break_iterator(&mut *split_bi(), "|", "^^")
}

fn test_split_then_value() -> Result<()> {
    break_iterator(&mut *split_bi(), "|a", "^^^")
}

fn test_value_then_split() -> Result<()> {
    break_iterator(&mut *split_bi(), "a|", "^^^")
}

fn test_value_then_split_then_value() -> Result<()> {
    break_iterator(&mut *split_bi(), "aa|aa", "^ ^^ ^")
}

fn test_value_then_double_split_then_value() -> Result<()> {
    break_iterator(&mut *split_bi(), "aa||aa", "^ ^^^ ^")
}

fn test_value_then_split_then_double_value_then_split_then_value() -> Result<()> {
    break_iterator(&mut *split_bi(), "a|bb cc|d", "^^^  ^ ^^^")
}

/// `String.indexOf('^', from)` over an ASCII marker string.
fn index_of(boundaries: &str, from: i32) -> i32 {
    let from = from.max(0) as usize;
    if from >= boundaries.len() {
        return -1;
    }
    boundaries[from..]
        .find('^')
        .map_or(-1, |i| (i + from) as i32)
}

/// `String.lastIndexOf('^', from)` over an ASCII marker string.
fn last_index_of(boundaries: &str, from: i32) -> i32 {
    if from < 0 {
        return -1;
    }
    let end = (from as usize + 1).min(boundaries.len());
    boundaries[..end].rfind('^').map_or(-1, |i| i as i32)
}

fn text_len(text: &str) -> i32 {
    text.encode_utf16().count() as i32
}

fn break_iterator(bi: &mut dyn BreakIterator, text: &str, boundaries: &str) -> Result<()> {
    bi.set_text_str(text);

    // Test first & last
    first_and_last(bi, text, boundaries);

    // Test if expected boundaries are consistent with reading them from next() in a loop:
    Assert::assert_equals(boundaries.to_string(), read_boundaries_to_string(bi, text));

    // Test following() and preceding():
    // get each index, randomized in case their is a sequencing bug:
    let indexes = random_ints_between_inclusive(text_len(text) + 1);
    following(bi, text, boundaries, &indexes)?;
    preceding(bi, text, boundaries, &indexes)?;

    // Test previous():
    previous(bi, text, boundaries);
    Ok(())
}

fn first_and_last(bi: &mut dyn BreakIterator, text: &str, boundaries: &str) {
    let message = format!("Text: {text}");
    let mut current = bi.current();
    Assert::assert_equals_msg(Some(&message), index_of(boundaries, 0), current);
    Assert::assert_equals_msg(Some(&message), current, bi.first());
    Assert::assert_equals_msg(Some(&message), current, bi.current());
    current = bi.last();
    Assert::assert_equals(last_index_of(boundaries, boundaries.len() as i32), current);
    Assert::assert_equals_msg(Some(&message), current, bi.current());
}

fn following(
    bi: &mut dyn BreakIterator,
    text: &str,
    boundaries: &str,
    indexes: &[i32],
) -> Result<()> {
    let message = format!("Text: {text}");
    for &index in indexes {
        let got = bi.following(index)?;
        if index == boundaries.len() as i32 {
            Assert::assert_equals_msg(Some(&message), DONE, got);
            Assert::assert_equals(
                last_index_of(boundaries, boundaries.len() as i32),
                bi.current(),
            );
            continue;
        }
        Assert::assert_equals_msg(
            Some(&format!("{message} index:{index}")),
            index_of(boundaries, index + 1),
            got,
        );
    }
    Ok(())
}

fn preceding(
    bi: &mut dyn BreakIterator,
    text: &str,
    boundaries: &str,
    indexes: &[i32],
) -> Result<()> {
    let message = format!("Text: {text}");
    for &index in indexes {
        let got = bi.preceding(index)?;
        if index == 0 {
            Assert::assert_equals_msg(Some(&message), DONE, got);
            Assert::assert_equals(index_of(boundaries, 0), bi.current());
            continue;
        }
        Assert::assert_equals_msg(
            Some(&format!("{message} index:{index}")),
            last_index_of(boundaries, index - 1),
            got,
        );
    }
    Ok(())
}

fn random_ints_between_inclusive(end: i32) -> Vec<i32> {
    let mut indexes: Vec<i32> = (0..end).collect();
    shuffle(&mut indexes, &*random());
    indexes
}

fn previous(bi: &mut dyn BreakIterator, text: &str, boundaries: &str) {
    let message = format!("Text: {text}");

    bi.set_text_str(text);
    let mut idx = bi.last(); // position at the end
    loop {
        idx = last_index_of(boundaries, idx - 1);
        if idx == -1 {
            Assert::assert_equals_msg(Some(&message), DONE, bi.previous());
            break;
        }
        Assert::assert_equals_msg(Some(&message), idx, bi.previous());
    }
    Assert::assert_equals_msg(Some(&message), index_of(boundaries, 0), bi.current());
    // finishes at first
}

/// Returns a string comprised of spaces and '^' only at the boundaries.
fn read_boundaries_to_string(bi: &mut dyn BreakIterator, text: &str) -> String {
    // init markers to spaces
    let mut markers = vec![b' '; text_len(text) as usize + 1];

    bi.set_text_str(text);
    let mut boundary = bi.current();
    while boundary != DONE {
        markers[boundary as usize] = b'^';
        boundary = bi.next();
    }
    String::from_utf8(markers).expect("ASCII markers")
}
