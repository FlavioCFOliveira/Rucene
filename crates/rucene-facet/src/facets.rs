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

//! Port of `Facets.java`, of the Java package `org.apache.lucene.facet`.

use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};

use crate::facet_result::FacetResult;

/// Common base class for all facets implementations.
///
/// Equivalent to `org.apache.lucene.facet.Facets`. Java's `null`
/// `FacetResult` is `None`.
pub trait Facets {
    /// Returns all child labels with non-zero counts under the specified path.
    ///
    /// Equivalent to `getAllChildren(String, String...)`.
    fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>>;

    /// Returns the topN child labels under the specified path.
    ///
    /// Equivalent to `getTopChildren(int, String, String...)`.
    fn get_top_children(&self, top_n: i32, dim: &str, path: &[&str])
        -> Result<Option<FacetResult>>;

    /// Return the count or value for a specific path; `-1` if this path
    /// doesn't exist.
    ///
    /// Equivalent to `getSpecificValue(String, String...)`.
    fn get_specific_value(&self, dim: &str, path: &[&str]) -> Result<NumericValue>;

    /// Returns topN labels for any dimension that had hits, sorted by the
    /// number of hits that dimension matched.
    ///
    /// Equivalent to `getAllDims(int)`.
    fn get_all_dims(&self, top_n: i32) -> Result<Vec<FacetResult>>;

    /// Returns labels for topN dimensions and their topNChildren sorted by the
    /// number of hits/aggregated values that dimension matched.
    ///
    /// Equivalent to `getTopDims(int, int)`.
    fn get_top_dims(&self, top_n_dims: i32, top_n_children: i32) -> Result<Vec<FacetResult>> {
        let mut all_results = self.get_all_dims(top_n_children)?;
        let n = (top_n_dims.max(0) as usize).min(all_results.len());
        all_results.truncate(n);
        Ok(all_results)
    }

    /// This instance as `Any`, for Java's `instanceof` tests and casts on a
    /// `Facets` (`facets instanceof MultiFacets`); every concrete facets
    /// class of the crate returns itself. `None` for an implementor that
    /// does not answer.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
}

/// This helper method checks if topN is valid.
///
/// Equivalent to `Facets.validateTopN(int)`.
pub fn validate_top_n(top_n: i32) -> Result<()> {
    if top_n <= 0 {
        return Err(LuceneError::illegal_argument(format!(
            "topN must be > 0 (got: {top_n})"
        )));
    }
    Ok(())
}
