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

//! Port of `FieldValueHighlighters.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::collections::HashSet;
use std::sync::Arc;

use rucene::error::Result;

use crate::matchhighlight::break_iterator_shrinking_adjuster::BreakIteratorShrinkingAdjuster;
use crate::matchhighlight::match_highlighter::{FieldValueHighlighter, QueryOffsetRange};
use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::passage_formatter::PassageFormatter;
use crate::matchhighlight::passage_selector::{default_scorer, PassageSelector};

/// `BiPredicate<String, Boolean>`.
type TestPredicate = Arc<dyn Fn(&str, bool) -> bool + Send + Sync>;

/// The `format` body of an [`AbstractFieldValueHighlighter`].
type FormatFn = Arc<
    dyn Fn(
            &str,
            &[String],
            &str,
            &[OffsetRange],
            Option<&[QueryOffsetRange]>,
        ) -> Result<Option<Vec<String>>>
        + Send
        + Sync,
>;

/// A field highlighter whose applicability is a predicate.
///
/// Equivalent to the private nested abstract class
/// `FieldValueHighlighters.AbstractFieldValueHighlighter`, with its
/// anonymous subclasses' `format` and `alwaysFetchedFields`.
pub(crate) struct AbstractFieldValueHighlighter {
    test_predicate: TestPredicate,
    format: FormatFn,
    always_fetched_fields: Vec<String>,
}

impl FieldValueHighlighter for AbstractFieldValueHighlighter {
    fn is_applicable(&self, field: &str, has_matches: bool) -> bool {
        (self.test_predicate)(field, has_matches)
    }

    fn format(
        &self,
        field: &str,
        values: &[String],
        contiguous_value: &str,
        value_ranges: &[OffsetRange],
        match_offsets: Option<&[QueryOffsetRange]>,
    ) -> Result<Option<Vec<String>>> {
        (self.format)(field, values, contiguous_value, value_ranges, match_offsets)
    }

    fn always_fetched_fields(&self) -> Vec<String> {
        self.always_fetched_fields.clone()
    }
}

/// A collection of commonly used field value highlighters.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.FieldValueHighlighters`.
#[derive(Debug)]
pub struct FieldValueHighlighters {
    _private: (),
}

impl FieldValueHighlighters {
    /// Displays up to `max_leading_characters` of the field's value,
    /// regardless of whether it contained highlights or not.
    ///
    /// Equivalent to the static `maxLeadingCharacters(int, String, Set)`.
    ///
    pub fn max_leading_characters(
        max_leading_characters: i32,
        ellipsis: &str,
        fields: HashSet<String>,
    ) -> Arc<dyn FieldValueHighlighter> {
        let passage_selector = Self::default_passage_selector();
        let passage_formatter = PassageFormatter::new(ellipsis, "", "");
        let always: Vec<String> = fields.iter().cloned().collect();
        Arc::new(AbstractFieldValueHighlighter {
            test_predicate: Arc::new(move |field, _has_matches| fields.contains(field)),
            format: Arc::new(
                move |_field, _values, contiguous_value, value_ranges, _match_offsets| {
                    let best_passages = passage_selector.pick_best_in_ranges(
                        contiguous_value,
                        &[],
                        max_leading_characters,
                        1,
                        value_ranges,
                    )?;
                    Ok(Some(passage_formatter.format(
                        contiguous_value,
                        &best_passages,
                        value_ranges,
                    )?))
                },
            ),
            always_fetched_fields: always,
        })
    }

    /// Default preconfigured `PassageSelector`.
    ///
    /// Equivalent to the static `defaultPassageSelector()`.
    pub fn default_passage_selector() -> PassageSelector {
        PassageSelector::with_scorer(
            default_scorer(),
            Some(Arc::new(BreakIteratorShrinkingAdjuster::new())),
        )
    }

    /// Highlights fields matching predicate `match_fields` only if they
    /// contained query matches.
    ///
    /// Equivalent to the static `highlighted(int, int, PassageFormatter,
    /// Predicate)`.
    ///
    pub fn highlighted(
        max_passage_window: i32,
        max_passages: i32,
        passage_formatter: PassageFormatter,
        match_fields: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    ) -> Arc<dyn FieldValueHighlighter> {
        let passage_selector = Self::default_passage_selector();
        Arc::new(AbstractFieldValueHighlighter {
            test_predicate: Arc::new(move |field, has_matches| match_fields(field) && has_matches),
            format: Arc::new(
                move |_field, _values, contiguous_value, value_ranges, match_offsets| {
                    let match_offsets =
                        match_offsets.ok_or_else(rucene::error::LuceneError::null_pointer)?;
                    let markers: Vec<OffsetRange> = match_offsets
                        .iter()
                        .map(QueryOffsetRange::as_offset_range)
                        .collect();
                    let best_passages = passage_selector.pick_best_in_ranges(
                        contiguous_value,
                        &markers,
                        max_passage_window,
                        max_passages,
                        value_ranges,
                    )?;
                    Ok(Some(passage_formatter.format(
                        contiguous_value,
                        &best_passages,
                        value_ranges,
                    )?))
                },
            ),
            always_fetched_fields: Vec::new(),
        })
    }

    /// Always returns raw field values, no highlighting or value
    /// truncation is applied.
    ///
    /// Equivalent to the static `verbatimValue(String, String...)`.
    pub fn verbatim_value(field: &str, more_fields: &[&str]) -> Arc<dyn FieldValueHighlighter> {
        let mut match_fields: HashSet<String> =
            more_fields.iter().map(|f| (*f).to_string()).collect();
        match_fields.insert(field.to_string());
        let always: Vec<String> = match_fields.iter().cloned().collect();
        Arc::new(AbstractFieldValueHighlighter {
            test_predicate: Arc::new(move |fld, _has_matches| match_fields.contains(fld)),
            format: Arc::new(
                |_field, values, _contiguous_value, _value_ranges, _match_offsets| {
                    Ok(Some(values.to_vec()))
                },
            ),
            always_fetched_fields: always,
        })
    }

    /// Matches all fields and omits their value in the output (so that no
    /// highlight or value is emitted).
    ///
    /// Equivalent to the static `skipRemaining()`.
    pub fn skip_remaining() -> Arc<dyn FieldValueHighlighter> {
        Arc::new(AbstractFieldValueHighlighter {
            test_predicate: Arc::new(|_field, _has_matches| true),
            format: Arc::new(
                |_field, _values, _contiguous_value, _value_ranges, _match_offsets| Ok(None),
            ),
            always_fetched_fields: Vec::new(),
        })
    }
}
