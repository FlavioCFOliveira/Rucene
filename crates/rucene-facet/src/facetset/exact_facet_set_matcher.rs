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

//! Port of `ExactFacetSetMatcher.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use rucene::error::Result;

use crate::facetset::facet_set::FacetSet;
use crate::facetset::facet_set_matcher::{validate_matcher, FacetSetMatcher};

/// A `FacetSetMatcher` which considers a set as a match only if all
/// dimension values are equal to the given one.
///
/// Equivalent to `org.apache.lucene.facet.facetset.ExactFacetSetMatcher`.
#[derive(Debug, Clone)]
pub struct ExactFacetSetMatcher {
    label: String,
    dims: i32,
    values: Vec<i64>,
}

impl ExactFacetSetMatcher {
    /// Constructs an instance to match the given facet set.
    ///
    /// Equivalent to `ExactFacetSetMatcher(String, FacetSet)`.
    pub fn new(label: &str, facet_set: &dyn FacetSet) -> Result<Self> {
        validate_matcher(facet_set.dims())?;
        Ok(Self {
            label: label.to_string(),
            dims: facet_set.dims(),
            values: facet_set.get_comparable_values(),
        })
    }
}

impl FacetSetMatcher for ExactFacetSetMatcher {
    fn label(&self) -> &str {
        &self.label
    }

    fn dims(&self) -> i32 {
        self.dims
    }

    fn matches(&self, dim_values: &[i64]) -> bool {
        debug_assert!(
            dim_values.len() as i32 == self.dims,
            "Encoded dimensions (dims={}) is incompatible with FacetSet dimensions (dims={})",
            dim_values.len(),
            self.dims
        );
        dim_values == self.values.as_slice()
    }
}
