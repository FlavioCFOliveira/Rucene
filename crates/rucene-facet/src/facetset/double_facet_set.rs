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

//! Port of `DoubleFacetSet.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use rucene::error::Result;
use rucene::util::NumericUtils;

use crate::facetset::facet_set::{validate_values_and_get_num_dims, FacetSet};

/// A `FacetSet` which encodes double dimension values.
///
/// Equivalent to `org.apache.lucene.facet.facetset.DoubleFacetSet`.
#[derive(Debug, Clone)]
pub struct DoubleFacetSet {
    dims: i32,
    /// The raw dimension values.
    pub values: Vec<f64>,
}

impl DoubleFacetSet {
    /// Constructs a new instance of a facet set which stores double dimension
    /// values.
    ///
    /// Equivalent to `DoubleFacetSet(double...)`.
    pub fn new(values: Vec<f64>) -> Result<Self> {
        let dims = validate_values_and_get_num_dims(values.len())?;
        Ok(Self { dims, values })
    }
}

impl FacetSet for DoubleFacetSet {
    fn dims(&self) -> i32 {
        self.dims
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.facetset.DoubleFacetSet"
    }

    fn get_comparable_values(&self) -> Vec<i64> {
        self.values
            .iter()
            .map(|v| NumericUtils::double_to_sortable_long(*v))
            .collect()
    }
}