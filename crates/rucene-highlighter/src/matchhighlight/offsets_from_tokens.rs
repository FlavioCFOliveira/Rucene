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

//! Port of `OffsetsFromTokens.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::collections::HashSet;
use std::sync::Arc;

use rucene::analysis::tokenattributes::{OffsetAttribute, TermToBytesRefAttribute};
use rucene::analysis::{Analyzer, AttributeHandle};
use rucene::error::{LuceneError, Result};
use rucene::index::Term;
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::util::BytesRef;

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
/// Equivalent to `org.apache.lucene.search.matchhighlight.OffsetsFromTokens`.
pub struct OffsetsFromTokens {
    field: String,
    analyzer: Arc<dyn Analyzer>,
}

impl OffsetsFromTokens {
    /// Equivalent to `new OffsetsFromTokens(String, Analyzer)`.
    pub fn new(field: &str, analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            field: field.to_string(),
            analyzer,
        }
    }
}

/// The anonymous visitor of `get`.
struct MatchTermsVisitor<'a> {
    field: &'a str,
    match_terms: &'a mut HashSet<BytesRef>,
}

impl QueryVisitor for MatchTermsVisitor<'_> {
    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        for t in terms {
            if t.field_opt() == Some(self.field) {
                self.match_terms.insert(t.bytes().clone());
            }
        }
    }
}

impl OffsetsRetrievalStrategy for OffsetsFromTokens {
    fn get(
        &self,
        matches_iterator: &mut dyn MatchesIterator,
        doc: &dyn FieldValueProvider,
    ) -> Result<Vec<OffsetRange>> {
        let values = doc.get_values(&self.field);

        let mut match_terms: HashSet<BytesRef> = HashSet::new();
        while matches_iterator.next()? {
            let q = matches_iterator
                .get_query()
                .ok_or_else(LuceneError::null_pointer)?;
            q.visit(&mut MatchTermsVisitor {
                field: &self.field,
                match_terms: &mut match_terms,
            });
        }

        let mut ranges: Vec<OffsetRange> = Vec::new();
        let mut value_offset: i32 = 0;
        for value in &values {
            let ts = self.analyzer.token_stream_from_str(&self.field, value)?;
            let mut ts = ts.borrow_mut();
            let offset_attr: AttributeHandle<dyn OffsetAttribute> =
                AttributeHandle::get(ts.attribute_source())
                    .ok_or_else(LuceneError::null_pointer)?;
            let term_attr: AttributeHandle<dyn TermToBytesRefAttribute> =
                AttributeHandle::get(ts.attribute_source())
                    .ok_or_else(LuceneError::null_pointer)?;
            ts.reset()?;
            while ts.increment_token()? {
                if match_terms.contains(&term_attr.borrow().get_bytes_ref()) {
                    let (start, end) = {
                        let att = offset_attr.borrow();
                        (att.start_offset(), att.end_offset())
                    };
                    ranges.push(OffsetRange::new(value_offset + start, value_offset + end));
                }
            }
            ts.end()?;
            value_offset +=
                offset_attr.borrow().end_offset() + self.analyzer.get_offset_gap(Some(&self.field));
            ts.close()?;
        }
        Ok(ranges)
    }
}
