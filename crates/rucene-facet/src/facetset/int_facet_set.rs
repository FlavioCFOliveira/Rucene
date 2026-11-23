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

//! Port of `IntFacetSet.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use rucene::document::IntPoint;
use rucene::error::Result;

use crate::facetset::facet_set::{validate_values_and_get_num_dims, FacetSet};

/// A `FacetSet` which encodes integer dimension values.
///
/// Equivalent to `org.apache.lucene.facet.facetset.IntFacetSet`.
#[derive(Debug, Clone)]
pub struct IntFacetSet {
    dims: i32,
    /// The raw dimension values.
    pub values: Vec<i32>,
}

impl IntFacetSet {
    /// Constructs a new instance of a facet set which stores int dimension
    /// values.
    ///
    /// Equivalent to `IntFacetSet(int...)`.
    pub fn new(values: Vec<i32>) -> Result<Self> {
        let dims = validate_values_and_get_num_dims(values.len())?;
        Ok(Self { dims, values })
    }
}

impl FacetSet for IntFacetSet {
    fn dims(&self) -> i32 {
        self.dims
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.facet.facetset.IntFacetSet"
    }

    fn get_comparable_values(&self) -> Vec<i64> {
        self.values.iter().map(|v| *v as i64).collect()
    }

    fn pack_values(&self, buf: &mut [u8], start: usize) -> usize {
        let mut offset = start;
        for v in &self.values {
            IntPoint::encode_dimension(*v, buf, offset);
            offset += 4;
        }
        self.values.len() * 4
    }

    fn size_packed_bytes(&self) -> usize {
        self.dims as usize * 4
    }
}
