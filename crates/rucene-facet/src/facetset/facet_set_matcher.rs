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

//! Port of `FacetSetMatcher.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use rucene::error::{LuceneError, Result};

/// Matches the encoded `FacetSet` that was indexed in `FacetSetsField`.
///
/// Equivalent to `org.apache.lucene.facet.facetset.FacetSetMatcher`; its
/// public `label` and `dims` fields are [`label`](Self::label) and
/// [`dims`](Self::dims).
pub trait FacetSetMatcher: Send + Sync {
    /// The label to associate to this matcher's aggregated value.
    fn label(&self) -> &str;

    /// The number of dimensions that are matched by this matcher.
    fn dims(&self) -> i32;

    /// Returns true if the facet set encoded in the given `long[]` is
    /// matched by this matcher.
    ///
    /// Equivalent to `matches(long[])`.
    fn matches(&self, dim_values: &[i64]) -> bool;
}

/// The checks of the `FacetSetMatcher(String, int)` constructor.
pub(crate) fn validate_matcher(dims: i32) -> Result<()> {
    if dims <= 0 {
        return Err(LuceneError::illegal_argument(format!(
            "dims must be greater than 0: {dims}"
        )));
    }
    Ok(())
}
