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

//! Port of `TestBreakIteratorBoundaryScanner.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_analysis_common::jdk::break_iterator::{
    get_line_instance, get_sentence_instance, get_word_instance,
};
use rucene_analysis_common::jdk::locale::Locale;
use rucene_highlighter::vectorhighlight::boundary_scanner::BoundaryScanner;
use rucene_highlighter::vectorhighlight::break_iterator_boundary_scanner::BreakIteratorBoundaryScanner;

use crate::highlighter_support::core_support::{class, test};

const TEXT: &str = "Apache Lucene(TM) is a high-performance, full-featured text search engine library written entirely in Java.\nIt is a technology suitable for nearly any application that requires\nfull-text search, especially cross-platform. \nApache Lucene is an open source project available for free download.";

/// `TestBreakIteratorBoundaryScanner`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.vectorhighlight.TestBreakIteratorBoundaryScanner",
        vec![
            test("testOutOfRange", test_out_of_range),
            test("testWordBoundary", test_word_boundary),
            test("testSentenceBoundary", test_sentence_boundary),
            test("testLineBoundary", test_line_boundary),
        ],
    )
}

/// `TEXT.indexOf(s)` (ASCII text: bytes are UTF-16 units).
fn index_of(s: &str) -> i32 {
    TEXT.find(s).map_or(-1, |i| i as i32)
}

/// `Locale.ROOT`.
fn root() -> Locale {
    Locale::of_parts("", "", "", "")
}

/// `Locale.getDefault()`, which `LuceneTestCase` randomizes.
fn default_locale() -> Locale {
    match rucene_test_framework::jdk::locale::get_default() {
        Some(l) => Locale::of_parts(
            l.get_language(),
            l.get_script(),
            l.get_country(),
            l.get_variant(),
        ),
        None => root(),
    }
}

/// `new StringBuilder(TEXT)`.
fn text() -> Vec<u16> {
    TEXT.encode_utf16().collect()
}

fn test_out_of_range() -> Result<()> {
    let text = text();
    let bi = get_word_instance(&root());
    let scanner = BreakIteratorBoundaryScanner::new(bi);

    let mut start = TEXT.len() as i32 + 1;
    Assert::assert_equals(start, scanner.find_start_offset(&text, start));
    Assert::assert_equals(start, scanner.find_end_offset(&text, start));
    start = 0;
    Assert::assert_equals(start, scanner.find_start_offset(&text, start));
    start = -1;
    Assert::assert_equals(start, scanner.find_end_offset(&text, start));
    Ok(())
}

fn test_word_boundary() -> Result<()> {
    let text = text();
    let bi = get_word_instance(&root());
    let scanner = BreakIteratorBoundaryScanner::new(bi);

    let start = index_of("formance");
    let mut expected = index_of("high-performance");
    test_find_start_offset(&text, start, expected, &scanner);

    expected = index_of(", full");
    test_find_end_offset(&text, start, expected, &scanner);
    Ok(())
}

fn test_sentence_boundary() -> Result<()> {
    let text = text();
    // we test this with default locale, it's randomized by LuceneTestCase
    let bi = get_sentence_instance(&default_locale());
    let scanner = BreakIteratorBoundaryScanner::new(bi);

    let start = index_of("any application");
    let mut expected = index_of("It is a");
    test_find_start_offset(&text, start, expected, &scanner);

    expected = index_of("Apache Lucene is an open source");
    test_find_end_offset(&text, start, expected, &scanner);
    Ok(())
}

fn test_line_boundary() -> Result<()> {
    let text = text();
    // we test this with default locale, it's randomized by LuceneTestCase
    let bi = get_line_instance(&default_locale());
    let scanner = BreakIteratorBoundaryScanner::new(bi);

    let start = index_of("any application");
    let mut expected = index_of("nearly");
    test_find_start_offset(&text, start, expected, &scanner);

    expected = index_of("application that requires");
    test_find_end_offset(&text, start, expected, &scanner);
    Ok(())
}

fn test_find_start_offset(text: &[u16], start: i32, expected: i32, scanner: &dyn BoundaryScanner) {
    Assert::assert_equals(expected, scanner.find_start_offset(text, start));
}

fn test_find_end_offset(text: &[u16], start: i32, expected: i32, scanner: &dyn BoundaryScanner) {
    Assert::assert_equals(expected, scanner.find_end_offset(text, start));
}
