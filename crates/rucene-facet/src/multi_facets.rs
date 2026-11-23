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

//! Port of `MultiFacets.java`, of the Java package `org.apache.lucene.facet`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::document::NumericValue;
use rucene::error::{LuceneError, Result};

use crate::facet_result::FacetResult;
use crate::facets::{validate_top_n, Facets};

/// Maps specified dims to provided Facets impls; else, uses the default
/// Facets impl.
///
/// Equivalent to `org.apache.lucene.facet.MultiFacets`. `dim_to_facets`
/// keeps the caller's iteration order, which `getAllDims` follows as Java
/// follows the given map's.
pub struct MultiFacets {
    dim_to_facets: Vec<(String, Arc<dyn Facets>)>,
    default_facets: Option<Arc<dyn Facets>>,
}

impl MultiFacets {
    /// Create this, with no default `Facets`.
    ///
    /// Equivalent to `MultiFacets(Map)`.
    pub fn new(dim_to_facets: Vec<(String, Arc<dyn Facets>)>) -> Self {
        Self::with_default(dim_to_facets, None)
    }

    /// Create this, with the specified default `Facets` for fields not
    /// included in `dim_to_facets`.
    ///
    /// Equivalent to `MultiFacets(Map, Facets)`.
    pub fn with_default(
        dim_to_facets: Vec<(String, Arc<dyn Facets>)>,
        default_facets: Option<Arc<dyn Facets>>,
    ) -> Self {
        Self {
            dim_to_facets,
            default_facets,
        }
    }

    /// Builds the instance from a `HashMap`.
    pub fn from_map(
        dim_to_facets: HashMap<String, Arc<dyn Facets>>,
        default_facets: Option<Arc<dyn Facets>>,
    ) -> Self {
        Self::with_default(dim_to_facets.into_iter().collect(), default_facets)
    }

    fn facets_for(&self, dim: &str) -> Result<&Arc<dyn Facets>> {
        match self.dim_to_facets.iter().find(|(d, _)| d == dim) {
            Some((_, facets)) => Ok(facets),
            None => self
                .default_facets
                .as_ref()
                .ok_or_else(|| LuceneError::illegal_argument(format!("invalid dim \"{dim}\""))),
        }
    }
}

impl Facets for MultiFacets {
    fn get_all_children(&self, dim: &str, path: &[&str]) -> Result<Option<FacetResult>> {
        self.facets_for(dim)?.get_all_children(dim, path)
    }

    fn get_top_children(
        &self,
        top_n: i32,
        dim: &str,
        path: &[&str],
    ) -> Result<Option<FacetResult>> {
        validate_top_n(top_n)?;
        self.facets_for(dim)?.get_top_children(top_n, dim, path)
    }

    fn get_specific_value(&self, dim: &str, path: &[&str]) -> Result<NumericValue> {
        self.facets_for(dim)?.get_specific_value(dim, path)
    }

    fn get_all_dims(&self, top_n: i32) -> Result<Vec<FacetResult>> {
        validate_top_n(top_n)?;
        let mut results = Vec::new();

        // First add the specific dim's facets:
        for (dim, facets) in &self.dim_to_facets {
            if let Some(result) = facets.get_top_children(top_n, dim, &[])? {
                results.push(result);
            }
        }

        if let Some(default_facets) = &self.default_facets {
            // Then add all default facets as long as we didn't already add
            // that dim:
            for result in default_facets.get_all_dims(top_n)? {
                if !self.dim_to_facets.iter().any(|(d, _)| *d == result.dim) {
                    results.push(result);
                }
            }
        }

        Ok(results)
    }
}
