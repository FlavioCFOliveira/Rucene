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

//! Port of `OffsetsFromValues.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::sync::Arc;

use rucene::analysis::tokenattributes::OffsetAttribute;
use rucene::analysis::{Analyzer, AttributeHandle};
use rucene::error::{LuceneError, Result};
use rucene::search::matches::MatchesIterator;

use crate::matchhighlight::match_region_retriever::FieldValueProvider;
use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::offsets_retrieval_strategy::OffsetsRetrievalStrategy;

/// This strategy works for fields where we know the match occurred but
/// there are no known positions or offsets.
///
/// We re-analyze field values and return offset ranges for entire values
/// (not individual tokens). Re-analysis is required because analyzer may
/// return an unknown offset gap.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.OffsetsFromValues`.
pub struct OffsetsFromValues {
    field: String,
    analyzer: Arc<dyn Analyzer>,
}

impl OffsetsFromValues {
    /// Equivalent to `new OffsetsFromValues(String, Analyzer)`.
    pub fn new(field: &str, analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            field: field.to_string(),
            analyzer,
        }
    }
}

impl OffsetsRetrievalStrategy for OffsetsFromValues {
    fn get(
        &self,
        _matches_iterator: &mut dyn MatchesIterator,
        doc: &dyn FieldValueProvider,
    ) -> Result<Vec<OffsetRange>> {
        let values = doc.get_values(&self.field);

        let mut ranges: Vec<OffsetRange> = Vec::new();
        let mut value_offset: i32 = 0;
        for value in &values {
            let ts = self.analyzer.token_stream_from_str(&self.field, value)?;
            let mut ts = ts.borrow_mut();
            let offset_attr: AttributeHandle<dyn OffsetAttribute> =
                AttributeHandle::get(ts.attribute_source()).ok_or_else(LuceneError::null_pointer)?;
            ts.reset()?;
            let start_offset = value_offset;
            while ts.increment_token()? {
                // Go through all tokens to increment offset attribute
                // properly.
            }
            ts.end()?;
            value_offset += offset_attr.borrow().end_offset();
            ranges.push(OffsetRange::new(start_offset, value_offset));
            value_offset += self.analyzer.get_offset_gap(Some(&self.field));
            ts.close()?;
        }
        Ok(ranges)
    }
}