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

//! Port of `OffsetsFromPositions.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::fmt;
use std::sync::Arc;

use rucene::analysis::tokenattributes::{OffsetAttribute, PositionIncrementAttribute};
use rucene::analysis::{Analyzer, AttributeHandle};
use rucene::error::{LuceneError, Result};
use rucene::search::matches::MatchesIterator;

use crate::matchhighlight::match_region_retriever::FieldValueProvider;
use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::offsets_retrieval_strategy::OffsetsRetrievalStrategy;

/// This strategy applies to fields with stored positions but no offsets.
/// We re-analyze the field's value to find out offsets of match positions.
///
/// Note that this may fail if index data (positions stored in the index)
/// is out of sync with the field values or the analyzer. This strategy
/// assumes it'll never happen.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.OffsetsFromPositions`.
pub struct OffsetsFromPositions {
    field: String,
    analyzer: Arc<dyn Analyzer>,
}

/// The local class `PositionSpan` of `convertPositionsToOffsets`.
#[derive(Clone, Copy)]
struct PositionSpan {
    from: i32,
    to: i32,
    left_offset: i32,
    right_offset: i32,
}

impl fmt::Display for PositionSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[from={}, to={}, L: {}, R: {}]",
            self.from, self.to, self.left_offset, self.right_offset
        )
    }
}

impl OffsetsFromPositions {
    /// Equivalent to `new OffsetsFromPositions(String, Analyzer)`.
    pub fn new(field: &str, analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            field: field.to_string(),
            analyzer,
        }
    }

    /// Equivalent to the package-private
    /// `convertPositionsToOffsets(ArrayList, List)`.
    ///
    /// # Errors
    ///
    /// What the analyzer raises; Java's `RuntimeException` when an offset is
    /// missing for a position range.
    pub(crate) fn convert_positions_to_offsets(
        &self,
        position_ranges: Vec<OffsetRange>,
        values: &[String],
    ) -> Result<Vec<OffsetRange>> {
        if position_ranges.is_empty() {
            return Ok(position_ranges);
        }

        let mut spans: Vec<PositionSpan> = Vec::with_capacity(position_ranges.len());
        let mut min_position = i32::MAX;
        let mut max_position = i32::MIN;
        for range in &position_ranges {
            spans.push(PositionSpan {
                from: range.from,
                to: range.to,
                left_offset: i32::MAX,
                right_offset: i32::MIN,
            });
            min_position = min_position.min(range.from);
            max_position = max_position.max(range.to);
        }

        // Iterate over the field's value tokens, computing the offsets of
        // the spans. Indices into `spans` stand for Java's shared span
        // objects.
        let mut spans_table: Vec<usize> = (0..spans.len()).collect();
        let mut span_count = spans_table.len();
        let mut position: i32 = -1;
        let mut value_offset: i32 = 0;
        let max = values.len();
        for (value_index, value) in values.iter().enumerate() {
            let last_value = value_index + 1 == max;

            let ts = self.analyzer.token_stream_from_str(&self.field, value)?;
            let mut ts = ts.borrow_mut();
            let offset_attr: AttributeHandle<dyn OffsetAttribute> =
                AttributeHandle::get(ts.attribute_source())
                    .ok_or_else(LuceneError::null_pointer)?;
            let pos_attr: AttributeHandle<dyn PositionIncrementAttribute> =
                AttributeHandle::get(ts.attribute_source())
                    .ok_or_else(LuceneError::null_pointer)?;
            ts.reset()?;
            while ts.increment_token()? {
                position += pos_attr.borrow().get_position_increment();

                // Only consider positions that fall within the range of
                // matched positions.
                if position >= min_position {
                    let (start, end) = {
                        let att = offset_attr.borrow();
                        (att.start_offset(), att.end_offset())
                    };
                    let start_offset = value_offset + start;
                    let end_offset = value_offset + end;

                    let mut j = 0;
                    for i in 0..span_count {
                        spans_table[j] = spans_table[i];
                        let span = &mut spans[spans_table[i]];
                        if position >= span.from {
                            if position <= span.to {
                                span.left_offset = span.left_offset.min(start_offset);
                                span.right_offset = span.right_offset.max(end_offset);
                            } else {
                                // This span can't intersect with any
                                // following position, remove it.
                                continue;
                            }
                        }
                        j += 1;
                    }
                    span_count = j;

                    // Only exit the loop if there are no more values to
                    // process.
                    if position > max_position && last_value {
                        break;
                    }
                }
            }
            ts.end()?;
            position += pos_attr.borrow().get_position_increment()
                + self.analyzer.get_position_increment_gap(Some(&self.field));
            value_offset +=
                offset_attr.borrow().end_offset() + self.analyzer.get_offset_gap(Some(&self.field));
            ts.close()?;
        }

        let mut converted: Vec<OffsetRange> = Vec::with_capacity(spans.len());
        for span in &spans {
            if span.left_offset == i32::MAX || span.right_offset == i32::MIN {
                return Err(LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    format!("One of the offsets missing for position range: {span}"),
                ));
            }
            converted.push(OffsetRange::new(span.left_offset, span.right_offset));
        }

        Ok(converted)
    }
}

impl OffsetsRetrievalStrategy for OffsetsFromPositions {
    fn get(
        &self,
        matches_iterator: &mut dyn MatchesIterator,
        doc: &dyn FieldValueProvider,
    ) -> Result<Vec<OffsetRange>> {
        let mut position_ranges: Vec<OffsetRange> = Vec::new();
        while matches_iterator.next()? {
            let from = matches_iterator.start_position();
            let to = matches_iterator.end_position();
            if from < 0 || to < 0 {
                return Err(LuceneError::from(std::io::Error::other(format!(
                    "Matches API returned negative positions for field: {}",
                    self.field
                ))));
            }
            position_ranges.push(OffsetRange::new(from, to));
        }

        self.convert_positions_to_offsets(position_ranges, &doc.get_values(&self.field))
    }
}
