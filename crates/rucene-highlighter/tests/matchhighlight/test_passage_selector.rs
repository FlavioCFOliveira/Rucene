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

//! Port of `TestPassageSelector.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::{Assert, Test};
use randomizedtesting::RandomizedTest;
use rucene::error::Result;
use rucene_highlighter::matchhighlight::break_iterator_shrinking_adjuster::BreakIteratorShrinkingAdjuster;
use rucene_highlighter::matchhighlight::offset_range::OffsetRange;
use rucene_highlighter::matchhighlight::passage_formatter::PassageFormatter;
use rucene_highlighter::matchhighlight::passage_selector::{default_scorer, PassageSelector};

use crate::highlighter_support::core_support::{class, test};

fn t(name: &str, body: fn() -> Result<()>) -> Method {
    test(name, body).annotate(Test::new())
}

/// `TestPassageSelector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.matchhighlight.TestPassageSelector",
        vec![
            t("checkEmptyExtra", check_empty_extra),
            t("checkOddOverlaps", check_odd_overlaps),
            t("oneMarker", one_marker),
            t("noHighlights", no_highlights),
            t("oneMarkerTruncated", one_marker_truncated),
            t("highlightLargerThanWindow", highlight_larger_than_window),
            t(
                "overlappingResultsDontPushOutLowerScoringResults",
                overlapping_results_dont_push_out_lower_scoring_results,
            ),
            t(
                "highlightLargerThanWindowWithSubranges",
                highlight_larger_than_window_with_subranges,
            ),
            t("twoMarkers", two_markers),
            t("noMarkers", no_markers),
            t("markersOutsideValue", markers_outside_value),
            t("largeWindow", large_window),
            t("twoPassages", two_passages),
            t("emptyRanges", empty_ranges),
            t("defaultPassages", default_passages),
            t("whitespaceBoundaries", whitespace_boundaries),
            t("passageScoring", passage_scoring),
            t("rangeWindows", range_windows),
            t(
                "testHighlightAcrossAllowedValueRange",
                test_highlight_across_allowed_value_range,
            ),
            t("randomizedSanityCheck", randomized_sanity_check),
        ],
    )
}

fn r(from: i32, to: i32) -> OffsetRange {
    OffsetRange::new(from, to)
}

fn len(value: &str) -> i32 {
    value.encode_utf16().count() as i32
}

fn check_empty_extra() -> Result<()> {
    check(
        "foo >>bar<< baz abc",
        "foo bar baz abc",
        300,
        100,
        &[r(4, 7), r(4, 7)],
    )?;
    check(
        ">foo >bar< >baz<< abc",
        "foo bar baz abc",
        300,
        100,
        &[r(0, 11), r(4, 7), r(8, 11)],
    )?;
    check(
        ">>foo< bar >baz<< abc",
        "foo bar baz abc",
        300,
        100,
        &[r(0, 11), r(0, 3), r(8, 11)],
    )
}

fn check_odd_overlaps() -> Result<()> {
    check(
        "foo >bar >baz<<> abc< xyz",
        "foo bar baz abc xyz",
        300,
        100,
        &[r(4, 11), r(8, 15)],
    )
}

fn one_marker() -> Result<()> {
    check(">0<123456789a", "0123456789a", 300, 1, &[r(0, 1)])?;
    check("0123456789>a<", "0123456789a", 300, 1, &[r(10, 11)])?;
    check(">0123456789a<", "0123456789a", 300, 1, &[r(0, 11)])
}

fn no_highlights() -> Result<()> {
    check("0123456789a", "0123456789a", 300, 1, &[])?;
    check("01234...", "0123456789a", 5, 1, &[])?;
    check_in_ranges("0123|45678", "0123456789a", 15, 2, &[], &[r(0, 4), r(4, 9)])
}

fn one_marker_truncated() -> Result<()> {
    check(">0<12...", "0123456789a", 4, 1, &[r(0, 1)])?;
    check("...789>a<", "0123456789a", 4, 1, &[r(10, 11)])?;
    check("...>3456<...", "0123456789a", 4, 1, &[r(3, 7)])?;
    check("...3>45<6...", "0123456789a", 4, 1, &[r(4, 6)])
}

fn highlight_larger_than_window() -> Result<()> {
    let value = "0123456789a";
    check(">0123<...", value, 4, 1, &[r(0, len(value))])?;
    check("...>123456<...", value, 6, 1, &[r(1, len(value))])
}

fn overlapping_results_dont_push_out_lower_scoring_results() -> Result<()> {
    let value =
        "a fair amount of space and then two matches here and then two big long spaces for matches";
    check(
        "...and then >two< >matches<...|...and then >two< big lon...",
        value,
        20,
        2,
        &[r(32, 35), r(36, 43), r(58, 61)],
    )
}

fn highlight_larger_than_window_with_subranges() -> Result<()> {
    let value = "0123456789a";
    check_in_ranges(
        "0>12<|>456789<...",
        value,
        6,
        2,
        &[r(1, len(value))],
        &[r(0, 3), r(4, len(value))],
    )?;

    check_in_ranges(
        ">01<...|>45<...",
        value,
        2,
        2,
        &[r(0, len(value))],
        &[r(0, 3), r(4, len(value))],
    )
}

fn two_markers() -> Result<()> {
    check(
        "0>12<3>45<6789a",
        "0123456789a",
        300,
        1,
        &[r(1, 3), r(4, 6)],
    )?;
    check(
        "0>123<>45<6789a",
        "0123456789a",
        300,
        1,
        &[r(1, 4), r(4, 6)],
    )
}

fn no_markers() -> Result<()> {
    check("0123456789a", "0123456789a", 300, 1, &[])?;
    check("0123...", "0123456789a", 4, 1, &[])
}

fn markers_outside_value() -> Result<()> {
    check("0123456789a", "0123456789a", 300, 1, &[r(100, 200)])
}

fn large_window() -> Result<()> {
    check("01234>567<89a", "0123456789a", i32::MAX, 1, &[r(5, 8)])
}

fn two_passages() -> Result<()> {
    check(
        "0>12<3...|...6>78<9...",
        "0123456789a",
        4,
        2,
        &[r(1, 3), r(7, 9)],
    )
}

fn empty_ranges() -> Result<()> {
    // Empty ranges cover the highlight, so it is omitted.
    // Instead, the first non-empty range is taken as the default.
    check_in_ranges(
        "6789...",
        "0123456789a",
        4,
        2,
        &[r(0, 1)],
        &[r(0, 0), r(2, 2), r(6, 11)],
    )
}

fn default_passages() -> Result<()> {
    // No highlights, multiple value ranges.
    check_in_ranges(
        "01|23|4567",
        "0123456789",
        100,
        100,
        &[],
        &[r(0, 2), r(2, 4), r(4, 8)],
    )?;

    // No highlights, multiple value ranges, maxPassages = 1
    check_in_ranges(
        "01",
        "0123456789",
        100,
        1,
        &[],
        &[r(0, 2), r(2, 4), r(4, 8)],
    )?;

    // No highlights, multiple value ranges, maxWindows size too short.
    check_in_ranges(
        "0123...|5678...",
        "0123456789",
        4,
        2,
        &[],
        &[r(0, 5), r(5, 10)],
    )
}

fn whitespace_boundaries() -> Result<()> {
    let selector = PassageSelector::with_scorer(
        default_scorer(),
        Some(Arc::new(BreakIteratorShrinkingAdjuster::new())),
    );

    check_with_selector(
        "...>  value  <...",
        "x          value          y",
        9,
        1,
        &selector,
        &[r(9, 18)],
    )
}

fn passage_scoring() -> Result<()> {
    // More highlights per passage -> better passage
    check(
        ">01<>23<...",
        "0123456789a",
        4,
        1,
        &[r(0, 2), r(2, 4), r(8, 10)],
    )?;

    check(
        "...>01<23>45<67>89<...",
        "__________0123456789a__________",
        10,
        1,
        &[r(10, 12), r(14, 16), r(18, 20)],
    )?;

    // ...if tied, the one with longer highlight length overall.
    check("...6>789<...", "0123456789a", 4, 1, &[r(0, 2), r(7, 10)])?;

    // ...if tied, the first one in order.
    check(">01<23...", "0123456789a", 4, 1, &[r(0, 2), r(8, 10)])
}

fn range_windows() -> Result<()> {
    // Add constraint windows to split the three highlights.
    check_in_ranges(
        "..._______>01<2",
        "__________0123456789a__________",
        10,
        3,
        &[r(10, 12), r(14, 16), r(18, 20)],
        &[r(0, 13)],
    )?;

    check_in_ranges(
        ">89<a_______...",
        "__________0123456789a__________",
        10,
        3,
        &[r(10, 12), r(14, 16), r(18, 20)],
        &[r(18, i32::MAX)],
    )?;

    check_in_ranges(
        "...________>01<|23>45<67|>89<a_______...",
        "__________0123456789a__________",
        10,
        3,
        &[r(10, 12), r(14, 16), r(18, 20)],
        &[r(0, 12), r(12, 18), r(18, i32::MAX)],
    )
}

fn test_highlight_across_allowed_value_range() -> Result<()> {
    check_in_ranges(
        "012>34<|>56<789",
        "0123456789",
        100,
        10,
        &[r(3, 7)],
        &[r(0, 5), r(5, 10)],
    )
}

fn randomized_sanity_check() -> Result<()> {
    let selector = PassageSelector::new();
    let formatter = PassageFormatter::new("...", ">", "<");
    let mut highlights: Vec<OffsetRange> = Vec::new();
    let mut permitted_ranges: Vec<OffsetRange> = Vec::new();
    for _i in 0..100 {
        let value = if RandomizedTest::random_boolean() {
            RandomizedTest::random_ascii_letters_of_length_between(0, 100)
        } else {
            RandomizedTest::random_realistic_unicode_of_codepoint_length_between(0, 1000)
        };

        let max_length = len(&value);
        permitted_ranges.clear();
        highlights.clear();
        let mut j = RandomizedTest::random_int_between(0, 10);
        loop {
            j -= 1;
            if j < 0 {
                break;
            }
            let from = RandomizedTest::random_int_between(0, len(&value));
            let to = (from + RandomizedTest::random_int_between(1, 10)).min(max_length);
            if from < to {
                highlights.push(OffsetRange::new(from, to));
            }
        }

        let char_window = RandomizedTest::random_int_between(1, 100);
        let max_passages = RandomizedTest::random_int_between(1, 10);

        if RandomizedTest::random_int_between(0, 5) == 0 {
            let increment = len(&value) / 10;
            let mut c = RandomizedTest::random_int_between(0, 20);
            let mut start = 0;
            loop {
                c -= 1;
                if c < 0 {
                    break;
                }
                let step = RandomizedTest::random_int_between(0, increment);
                let to = (start + step).min(max_length);
                if start < to {
                    permitted_ranges.push(OffsetRange::new(start, to));
                }
                start += step + RandomizedTest::random_int_between(0, 3);
            }
        } else {
            permitted_ranges.push(OffsetRange::new(0, len(&value)));
        }

        // Just make sure there are no exceptions.
        let passages = selector.pick_best_in_ranges(
            &value,
            &highlights,
            char_window,
            max_passages,
            &permitted_ranges,
        )?;

        // Make sure no empty ranges are returned.
        for p in &passages {
            Assert::assert_true(p.from != p.to);
            for _r in &p.markers {
                Assert::assert_true(p.from != p.to);
            }
        }

        let format = formatter.format(&value, &passages, &permitted_ranges)?;

        // Check that the formatted value doesn't have an 'empty range' marker.
        // If the original value contained such a sequence of characters, omit the check
        // (can happen on random inputs, see https://github.com/apache/lucene/issues/12562)
        if !value.contains("><") && format.iter().any(|f| f == "><") {
            Assert::fail(&format!(
                "\nExpected: not a collection containing \"><\"\n     but: was {format:?}"
            ));
        }
    }
    Ok(())
}

/// `checkPassages(expected, value, charWindow, maxPassages, highlights...)`.
fn check(
    expected: &str,
    value: &str,
    char_window: i32,
    max_passages: i32,
    highlights: &[OffsetRange],
) -> Result<()> {
    check_with_selector(
        expected,
        value,
        char_window,
        max_passages,
        &PassageSelector::new(),
        highlights,
    )
}

/// `checkPassages(expected, value, charWindow, maxPassages, selector,
/// highlights...)`.
fn check_with_selector(
    expected: &str,
    value: &str,
    char_window: i32,
    max_passages: i32,
    selector: &PassageSelector,
    highlights: &[OffsetRange],
) -> Result<()> {
    check_full(
        expected,
        value,
        char_window,
        max_passages,
        highlights,
        &[OffsetRange::new(0, len(value))],
        selector,
    )
}

/// `checkPassages(expected, value, charWindow, maxPassages, highlights,
/// ranges)`.
fn check_in_ranges(
    expected: &str,
    value: &str,
    char_window: i32,
    max_passages: i32,
    highlights: &[OffsetRange],
    ranges: &[OffsetRange],
) -> Result<()> {
    check_full(
        expected,
        value,
        char_window,
        max_passages,
        highlights,
        ranges,
        &PassageSelector::new(),
    )
}

fn check_full(
    expected: &str,
    value: &str,
    char_window: i32,
    max_passages: i32,
    highlights: &[OffsetRange],
    ranges: &[OffsetRange],
    selector: &PassageSelector,
) -> Result<()> {
    let result = get_passages(
        value,
        char_window,
        max_passages,
        highlights,
        ranges,
        selector,
    )?;
    if result != expected {
        println!("Value:  {value}");
        println!("Result: {result}");
        println!("Expect: {expected}");
    }
    if result != expected {
        Assert::fail(&format!(
            "\nExpected: \"{expected}\"\n     but: was \"{result}\""
        ));
    }
    Ok(())
}

/// `getPassages(value, charWindow, maxPassages, highlights, ranges,
/// selector)`.
fn get_passages(
    value: &str,
    char_window: i32,
    max_passages: i32,
    highlights: &[OffsetRange],
    ranges: &[OffsetRange],
    selector: &PassageSelector,
) -> Result<String> {
    let passage_formatter = PassageFormatter::new("...", ">", "<");
    let passages =
        selector.pick_best_in_ranges(value, highlights, char_window, max_passages, ranges)?;
    Ok(passage_formatter
        .format(value, &passages, ranges)?
        .join("|"))
}
