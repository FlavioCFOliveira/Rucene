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

//! Port of `TestDefaultPassageFormatter.java`.
//!
//! `Passage.addMatch` takes a `BytesRef`; the Java `null` term of
//! `testBasic` is an empty `BytesRef`, which the formatter never reads.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::BytesRef;
use rucene_highlighter::uhighlight::default_passage_formatter::DefaultPassageFormatter;
use rucene_highlighter::uhighlight::passage::Passage;
use rucene_highlighter::uhighlight::passage_formatter::PassageFormatter;

use crate::highlighter_support::core_support::{class, test};

/// `TestDefaultPassageFormatter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.uhighlight.TestDefaultPassageFormatter",
        vec![
            test("testBasic", test_basic),
            test("testOverlappingPassages", test_overlapping_passages),
            test("testReversedStartOffsetOrder", test_reversed_start_offset_order),
        ],
    )
}

/// `String.indexOf(str, from)` in UTF-16 units over an ASCII text.
fn index_of(text: &str, needle: &str, from: i32) -> i32 {
    let from = from.max(0) as usize;
    if from > text.len() {
        return -1;
    }
    text[from..].find(needle).map_or(-1, |i| (i + from) as i32)
}

fn format(formatter: &DefaultPassageFormatter, passages: &[Passage], text: &str) -> Result<String> {
    Ok(formatter.format(passages, text)?.to_string())
}

fn test_basic() -> Result<()> {
    let text = "Test customization & <div class=\"xy\">&quot;escaping&quot;</div> of this very formatter. Unrelated part. It's not very N/A!";
    // fabricate passages with matches to format
    let mut passages = vec![Passage::new(), Passage::new()];
    passages[0].set_start_offset(0);
    passages[0].set_end_offset(index_of(text, ".", 0) + 1);
    passages[0].add_match(
        index_of(text, "very", 0),
        index_of(text, "very", 0) + 4,
        BytesRef::default(),
        2,
    );
    let start = index_of(text, ".", passages[0].get_end_offset() + 1) + 2;
    passages[1].set_start_offset(start);
    passages[1].set_end_offset(text.len() as i32);
    let end0 = passages[0].get_end_offset();
    passages[1].add_match(
        index_of(text, "very", end0),
        index_of(text, "very", end0) + 4,
        BytesRef::default(),
        2,
    );

    // test default
    let mut formatter = DefaultPassageFormatter::new();
    Assert::assert_equals(
        "Test customization & <div class=\"xy\">&quot;escaping&quot;</div> of this <b>very</b> formatter.... It's not <b>very</b> N/A!".to_string(),
        format(&formatter, &passages, text)?,
    );

    // test customization and encoding
    formatter = DefaultPassageFormatter::with_tags("<u>", "</u>", "\u{2026} ", true);
    Assert::assert_equals(
        "Test customization &amp; &lt;div class=&quot;xy&quot;&gt;&amp;quot;escaping&amp;quot;&lt;&#x2F;div&gt; of this <u>very</u> formatter.\u{2026} It&#x27;s not <u>very</u> N&#x2F;A!".to_string(),
        format(&formatter, &passages, text)?,
    );
    Ok(())
}

fn test_overlapping_passages() -> Result<()> {
    let content = "Yin yang loooooooooong, yin gap yang yong";
    let mut passages = vec![Passage::new()];
    passages[0].set_start_offset(0);
    passages[0].set_end_offset(41);
    passages[0].set_score(5.93812f32);
    passages[0].set_score(5.93812f32);
    passages[0].add_match(0, 3, BytesRef::from_char_sequence("yin"), 1);
    passages[0].add_match(0, 22, BytesRef::from_char_sequence("yin yang loooooooooooong"), 1);
    passages[0].add_match(4, 8, BytesRef::from_char_sequence("yang"), 1);
    passages[0].add_match(9, 22, BytesRef::from_char_sequence("loooooooooong"), 1);
    passages[0].add_match(24, 27, BytesRef::from_char_sequence("yin"), 1);
    passages[0].add_match(32, 36, BytesRef::from_char_sequence("yang"), 1);

    // test default
    let formatter = DefaultPassageFormatter::new();
    Assert::assert_equals(
        "<b>Yin yang loooooooooong</b>, <b>yin</b> gap <b>yang</b> yong".to_string(),
        format(&formatter, &passages, content)?,
    );
    Ok(())
}

fn test_reversed_start_offset_order() -> Result<()> {
    let content = concat!(
        "When indexing data in Solr, each document is composed of various fields. ",
        "A document essentially represents a single record, and each document typically contains a unique ID field."
    );

    let mut passages = vec![Passage::new(), Passage::new()];
    passages[0].set_start_offset(73);
    passages[0].set_end_offset(179);
    passages[0].set_score(1.8846991f32);
    passages[0].add_match(75, 83, BytesRef::from_char_sequence("document"), 1);
    passages[0].add_match(133, 141, BytesRef::from_char_sequence("document"), 1);

    passages[1].set_start_offset(0);
    passages[1].set_end_offset(73);
    passages[1].set_score(1.5923802f32);
    passages[1].add_match(33, 41, BytesRef::from_char_sequence("document"), 1);

    let formatter = DefaultPassageFormatter::with_tags("<b>", "</b>", "\n", false);
    Assert::assert_equals(
        concat!(
            "A <b>document</b> essentially represents a single record, and each <b>document</b> typically contains a unique ID field.\n",
            "When indexing data in Solr, each <b>document</b> is composed of various fields. "
        )
        .to_string(),
        format(&formatter, &passages, content)?,
    );
    Ok(())
}
