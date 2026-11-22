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

//! Port of `FacetResult.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::fmt;

use rucene::document::NumericValue;

use crate::label_and_value::{number_equals, number_hash_code, LabelAndValue};
use crate::taxonomy::facet_label::arrays_to_string;

/// Counts or aggregates for a single dimension.
///
/// Equivalent to `org.apache.lucene.facet.FacetResult`.
#[derive(Debug, Clone)]
pub struct FacetResult {
    /// Dimension that was requested.
    pub dim: String,
    /// Path whose children were requested.
    pub path: Vec<String>,
    /// Total number of documents containing a value for this path, or the
    /// aggregated value; `-1` when unknown.
    pub value: NumericValue,
    /// How many child labels were encountered.
    pub child_count: i32,
    /// Child counts.
    pub label_values: Vec<LabelAndValue>,
}

impl FacetResult {
    /// Sole constructor.
    ///
    /// Equivalent to `FacetResult(String, String[], Number, LabelAndValue[],
    /// int)`.
    pub fn new(
        dim: impl Into<String>,
        path: Vec<String>,
        value: NumericValue,
        label_values: Vec<LabelAndValue>,
        child_count: i32,
    ) -> Self {
        Self {
            dim: dim.into(),
            path,
            value,
            child_count,
            label_values,
        }
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut hash_code = number_hash_code(&self.value).wrapping_add(31i32.wrapping_mul(self.child_count));
        for label_value in &self.label_values {
            hash_code = label_value.hash_code().wrapping_add(31i32.wrapping_mul(hash_code));
        }
        hash_code
    }
}

impl fmt::Display for FacetResult {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dim={} path={} value={} childCount={}\n",
            self.dim,
            arrays_to_string(&self.path),
            self.value,
            self.child_count
        )?;
        for label_value in &self.label_values {
            writeln!(f, "  {label_value}")?;
        }
        Ok(())
    }
}

impl PartialEq for FacetResult {
    /// Equivalent to `equals(Object)`: the value, the child count and the
    /// label values; not the dim nor the path.
    fn eq(&self, other: &Self) -> bool {
        number_equals(&self.value, &other.value)
            && self.child_count == other.child_count
            && self.label_values == other.label_values
    }
}