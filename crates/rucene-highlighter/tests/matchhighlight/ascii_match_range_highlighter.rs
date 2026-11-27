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

//! Port of `AsciiMatchRangeHighlighter.java`: a simple ASCII match range
//! highlighter for tests.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::document::Document;
use rucene::error::Result;
use rucene_highlighter::matchhighlight::offset_range::OffsetRange;
use rucene_highlighter::matchhighlight::passage_formatter::PassageFormatter;
use rucene_highlighter::matchhighlight::passage_selector::PassageSelector;

/// `final class AsciiMatchRangeHighlighter`.
pub struct AsciiMatchRangeHighlighter {
    analyzer: Arc<dyn Analyzer>,
    passage_formatter: PassageFormatter,
    selector: PassageSelector,
    max_passage_window: i32,
    max_passages: i32,
}

fn utf16_len(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

impl AsciiMatchRangeHighlighter {
    /// `new AsciiMatchRangeHighlighter(analyzer)`.
    pub fn new(analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            passage_formatter: PassageFormatter::new("...", ">", "<"),
            selector: PassageSelector::new(),
            analyzer,
            max_passage_window: 160,
            max_passages: 10,
        }
    }

    /// `apply(document, fieldHighlights)`: `field_highlights` iterates in
    /// the order of the caller's map; the result keeps it (a
    /// `LinkedHashMap`).
    pub fn apply<'a>(
        &self,
        document: &Document,
        field_highlights: impl IntoIterator<Item = (&'a String, &'a Vec<OffsetRange>)>,
    ) -> Result<Vec<(String, Vec<String>)>> {
        let mut value_ranges: Vec<OffsetRange> = Vec::new();
        let mut field_snippets: Vec<(String, Vec<String>)> = Vec::new();
        for (field, match_ranges) in field_highlights {
            let offset_gap = self.analyzer.get_offset_gap(Some(field));

            let values = document.get_values(field);
            let value = if values.len() == 1 {
                values[0].clone()
            } else {
                // This can be inefficient if offset gap is large but recomputing
                // offsets in a smart way doesn't make sense for tests.
                let field_gap_padding = " ".repeat(offset_gap.max(0) as usize);
                values.join(&field_gap_padding)
            };

            // Create permitted range windows for passages so that they don't cross
            // multi-value boundary.
            value_ranges.clear();
            let mut offset = 0;
            for v in &values {
                value_ranges.push(OffsetRange::new(offset, offset + utf16_len(v)));
                offset += utf16_len(v);
                offset += offset_gap;
            }

            let passages = self.selector.pick_best_in_ranges(
                &value,
                match_ranges,
                self.max_passage_window,
                self.max_passages,
                &value_ranges,
            )?;

            let formatted = self
                .passage_formatter
                .format(&value, &passages, &value_ranges)?;
            match field_snippets.iter_mut().find(|(f, _)| f == field) {
                Some(entry) => entry.1 = formatted,
                None => field_snippets.push((field.clone(), formatted)),
            }
        }
        Ok(field_snippets)
    }
}
