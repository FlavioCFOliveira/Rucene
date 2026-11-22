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

//! Port of `LongRangeOnRangeFacetCounts.java`, of the Java package
//! `org.apache.lucene.facet.rangeonrange`.

use std::sync::Arc;

use rucene::document::range_fields::{LongRange as LongRangeField, RangeQueryType};
use rucene::error::Result;
use rucene::search::Query;

use crate::facets_collector::FacetsCollector;
use crate::rangeonrange::long_range::LongRange;
use crate::rangeonrange::range::Range;
use crate::rangeonrange::range_on_range_facet_counts::{
    range_on_range_facets_delegate, RangeOnRangeFacetCounts,
};

/// Represents counts for long range on range faceting.
///
/// Equivalent to `org.apache.lucene.facet.rangeonrange.LongRangeOnRangeFacetCounts`.
pub struct LongRangeOnRangeFacetCounts {
    base: RangeOnRangeFacetCounts,
}

impl LongRangeOnRangeFacetCounts {
    /// Represents counts for long range on range faceting.
    ///
    /// Equivalent to `LongRangeOnRangeFacetCounts(String, FacetsCollector,
    /// RangeFieldQuery.QueryType, LongRange...)`.
    pub fn new(field: &str, hits: &FacetsCollector, query_type: RangeQueryType, ranges: &[LongRange]) -> Result<Self> {
        Self::with_fast_match(field, hits, query_type, None, ranges)
    }

    /// Represents counts for long range on range faceting with a fast-match
    /// query.
    ///
    /// Equivalent to `LongRangeOnRangeFacetCounts(String, FacetsCollector,
    /// RangeFieldQuery.QueryType, Query, LongRange...)`.
    pub fn with_fast_match(
        field: &str,
        hits: &FacetsCollector,
        query_type: RangeQueryType,
        fast_match_query: Option<Arc<dyn Query>>,
        ranges: &[LongRange],
    ) -> Result<Self> {
        Ok(Self {
            base: RangeOnRangeFacetCounts::new(
                field,
                hits,
                query_type,
                fast_match_query,
                8,
                Self::get_encoded_ranges(ranges)?,
                Range::get_labels_from_ranges(ranges.iter().map(|r| &r.range)),
            )?,
        })
    }

    /// Equivalent to `getEncodedRanges(LongRange...)`.
    fn get_encoded_ranges(ranges: &[LongRange]) -> Result<Vec<Vec<u8>>> {
        let dims = ranges[0].range.dims as usize;
        let mut result = vec![vec![0u8; 2 * 8 * dims]; ranges.len()];
        for (i, range) in ranges.iter().enumerate() {
            LongRangeField::verify_and_encode(&range.min, &range.max, &mut result[i])?;
        }
        Ok(result)
    }
}

range_on_range_facets_delegate!(LongRangeOnRangeFacetCounts, base);