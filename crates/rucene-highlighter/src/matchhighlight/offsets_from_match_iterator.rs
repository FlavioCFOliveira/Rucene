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

//! Port of `OffsetsFromMatchIterator.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use rucene::error::{LuceneError, Result};
use rucene::search::matches::MatchesIterator;

use crate::matchhighlight::match_region_retriever::FieldValueProvider;
use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::offsets_from_positions::OffsetsFromPositions;
use crate::matchhighlight::offsets_retrieval_strategy::OffsetsRetrievalStrategy;

/// This strategy retrieves offsets directly from `MatchesIterator`, if
/// they are available, otherwise it falls back to using
/// `OffsetsFromPositions`.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.OffsetsFromMatchIterator`.
pub struct OffsetsFromMatchIterator {
    field: String,
    no_offsets_fallback: OffsetsFromPositions,
}

impl OffsetsFromMatchIterator {
    /// Equivalent to `new OffsetsFromMatchIterator(String,
    /// OffsetsFromPositions)`.
    pub fn new(field: &str, no_offsets_fallback: OffsetsFromPositions) -> Self {
        Self {
            field: field.to_string(),
            no_offsets_fallback,
        }
    }
}

impl OffsetsRetrievalStrategy for OffsetsFromMatchIterator {
    fn get(
        &self,
        matches_iterator: &mut dyn MatchesIterator,
        doc: &dyn FieldValueProvider,
    ) -> Result<Vec<OffsetRange>> {
        let mut position_ranges: Vec<OffsetRange> = Vec::new();
        let mut offset_ranges: Option<Vec<OffsetRange>> = Some(Vec::new());
        while matches_iterator.next()? {
            let from_position = matches_iterator.start_position();
            let to_position = matches_iterator.end_position();
            if from_position < 0 || to_position < 0 {
                return Err(LuceneError::from(std::io::Error::other(format!(
                    "Matches API returned negative positions for field: {}",
                    self.field
                ))));
            }
            position_ranges.push(OffsetRange::new(from_position, to_position));

            if let Some(ranges) = offset_ranges.as_mut() {
                let from = matches_iterator.start_offset()?;
                let to = matches_iterator.end_offset()?;
                if from < 0 || to < 0 {
                    // At least one offset isn't available. Fallback to just
                    // positions.
                    offset_ranges = None;
                } else {
                    ranges.push(OffsetRange::new(from, to));
                }
            }
        }

        // Convert positions to offsets if needed.
        match offset_ranges {
            None => self
                .no_offsets_fallback
                .convert_positions_to_offsets(position_ranges, &doc.get_values(&self.field)),
            Some(offset_ranges) => Ok(offset_ranges),
        }
    }
}
