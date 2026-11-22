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

//! Port of `FacetSetsField.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use std::fmt;

use rucene::document::{BinaryDocValuesField, IntPoint};
use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;

use crate::facet_field::delegate_indexable_field;
use crate::facetset::facet_set::FacetSet;

/// A `BinaryDocValuesField` which encodes a list of `FacetSet`.
///
/// Equivalent to `org.apache.lucene.facet.facetset.FacetSetsField`, a
/// `BinaryDocValuesField` subclass.
pub struct FacetSetsField {
    field: BinaryDocValuesField,
}

impl FacetSetsField {
    /// Create a new `FacetSetsField` with the given `FacetSet`s.
    ///
    /// Equivalent to `create(String, FacetSet...)`.
    pub fn create(name: &str, facet_sets: &[&dyn FacetSet]) -> Result<Self> {
        if facet_sets.is_empty() {
            return Err(LuceneError::illegal_argument("FacetSets cannot be null or empty!"));
        }

        Ok(Self {
            field: BinaryDocValuesField::new(name.to_string(), Self::to_packed_values(facet_sets)?),
        })
    }

    /// Equivalent to `toPackedValues(FacetSet...)`.
    fn to_packed_values(facet_sets: &[&dyn FacetSet]) -> Result<BytesRef> {
        let num_dims = facet_sets[0].dims();
        let expected_class = facet_sets[0].java_class_name();
        let mut buf = vec![0u8; 4 + facet_sets[0].size_packed_bytes() * facet_sets.len()];
        IntPoint::encode_dimension(num_dims, &mut buf, 0);
        let mut offset = 4usize;
        for facet_set in facet_sets {
            if facet_set.dims() != num_dims {
                return Err(LuceneError::illegal_argument(format!(
                    "All FacetSets must have the same number of dimensions. Expected {} found {}",
                    num_dims,
                    facet_set.dims()
                )));
            }
            // Note that we only check the classes since the first FacetSet
            // could be a subclass of the others.
            if facet_set.java_class_name() != expected_class {
                return Err(LuceneError::illegal_argument(format!(
                    "All FacetSets must be the same type. Expected class {} found class {}",
                    expected_class,
                    facet_set.java_class_name()
                )));
            }
            offset += facet_set.pack_values(&mut buf, offset);
        }
        buf.truncate(offset);
        Ok(BytesRef::new(buf))
    }
}

impl fmt::Display for FacetSetsField {
    /// Inherited `Field.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.field, f)
    }
}

delegate_indexable_field!(FacetSetsField, field);