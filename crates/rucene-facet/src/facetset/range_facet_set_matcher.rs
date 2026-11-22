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

//! Port of `RangeFacetSetMatcher.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use rucene::error::{LuceneError, Result};

use crate::facetset::dim_range::DimRange;
use crate::facetset::facet_set_matcher::{validate_matcher, FacetSetMatcher};

/// A `FacetSetMatcher` which considers a set as a match if all dimensions
/// fall within the given corresponding range.
///
/// Equivalent to `org.apache.lucene.facet.facetset.RangeFacetSetMatcher`.
#[derive(Debug, Clone)]
pub struct RangeFacetSetMatcher {
    label: String,
    dims: i32,
    lower_ranges: Vec<i64>,
    upper_ranges: Vec<i64>,
}

impl RangeFacetSetMatcher {
    /// Constructs an instance to match facet sets with dimensions that fall
    /// within the given ranges.
    ///
    /// Equivalent to `RangeFacetSetMatcher(String, DimRange...)`.
    pub fn new(label: &str, dim_ranges: &[DimRange]) -> Result<Self> {
        let dims = Self::get_dims(dim_ranges)?;
        validate_matcher(dims)?;
        Ok(Self {
            label: label.to_string(),
            dims,
            lower_ranges: dim_ranges.iter().map(|r| r.min).collect(),
            upper_ranges: dim_ranges.iter().map(|r| r.max).collect(),
        })
    }

    /// Equivalent to `getDims(DimRange...)`.
    fn get_dims(dim_ranges: &[DimRange]) -> Result<i32> {
        if dim_ranges.is_empty() {
            return Err(LuceneError::illegal_argument("dimRanges cannot be null or empty"));
        }
        Ok(dim_ranges.len() as i32)
    }
}

impl FacetSetMatcher for RangeFacetSetMatcher {
    fn label(&self) -> &str {
        &self.label
    }

    fn dims(&self) -> i32 {
        self.dims
    }

    fn matches(&self, dim_values: &[i64]) -> bool {
        debug_assert!(
            dim_values.len() as i32 == self.dims,
            "Encoded dimensions (dims={}) is incompatible with range dimensions (dims={})",
            dim_values.len(),
            self.dims
        );

        for i in 0..dim_values.len() {
            if dim_values[i] < self.lower_ranges[i] {
                return false;
            }
            if dim_values[i] > self.upper_ranges[i] {
                return false;
            }
        }
        true
    }
}