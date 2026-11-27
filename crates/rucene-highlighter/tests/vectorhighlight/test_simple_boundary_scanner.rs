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

//! Port of `TestSimpleBoundaryScanner.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_highlighter::vectorhighlight::boundary_scanner::BoundaryScanner;
use rucene_highlighter::vectorhighlight::simple_boundary_scanner::SimpleBoundaryScanner;

use crate::highlighter_support::core_support::{class, test};

const TEXT: &str =
    "Apache Lucene(TM) is a high-performance, full-featured\ntext search engine library written entirely in Java.";

/// `TestSimpleBoundaryScanner`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.vectorhighlight.TestSimpleBoundaryScanner",
        vec![
            test("testFindStartOffset", test_find_start_offset),
            test("testFindEndOffset", test_find_end_offset),
        ],
    )
}

/// `TEXT.indexOf(s)` (ASCII text: bytes are UTF-16 units).
fn index_of(s: &str) -> i32 {
    TEXT.find(s).map_or(-1, |i| i as i32)
}

fn test_find_start_offset() -> Result<()> {
    let text: Vec<u16> = TEXT.encode_utf16().collect();
    let scanner = SimpleBoundaryScanner::new();

    // test out of range
    let mut start = TEXT.len() as i32 + 1;
    Assert::assert_equals(start, scanner.find_start_offset(&text, start));
    start = 0;
    Assert::assert_equals(start, scanner.find_start_offset(&text, start));

    start = index_of("formance");
    let mut expected = index_of("high-performance");
    Assert::assert_equals(expected, scanner.find_start_offset(&text, start));

    start = index_of("che");
    expected = index_of("Apache");
    Assert::assert_equals(expected, scanner.find_start_offset(&text, start));
    Ok(())
}

fn test_find_end_offset() -> Result<()> {
    let text: Vec<u16> = TEXT.encode_utf16().collect();
    let scanner = SimpleBoundaryScanner::new();

    // test out of range
    let mut start = TEXT.len() as i32 + 1;
    Assert::assert_equals(start, scanner.find_end_offset(&text, start));
    start = -1;
    Assert::assert_equals(start, scanner.find_end_offset(&text, start));

    start = index_of("full-");
    let expected = index_of("\ntext");
    Assert::assert_equals(expected, scanner.find_end_offset(&text, start));
    Ok(())
}
