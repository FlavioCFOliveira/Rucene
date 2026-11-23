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

//! Port of `FacetSet.java`, of the Java package
//! `org.apache.lucene.facet.facetset`.

use rucene::document::LongPoint;

/// Holds a set of facet dimension values.
///
/// Equivalent to `org.apache.lucene.facet.facetset.FacetSet`; its public
/// `dims` field is [`dims`](Self::dims).
pub trait FacetSet: Send + Sync {
    /// The number of dimension values in this set.
    fn dims(&self) -> i32;

    /// `getClass().getName()` of the Java class, for the type checks of
    /// `FacetSetsField`.
    fn java_class_name(&self) -> &'static str;

    /// Returns the dimension values in this facet set as comparable longs.
    ///
    /// Equivalent to `getComparableValues()`.
    fn get_comparable_values(&self) -> Vec<i64>;

    /// Packs the dimension values into the given `buf` and returns the
    /// number of packed bytes.
    ///
    /// Equivalent to `packValues(byte[], int)`.
    fn pack_values(&self, buf: &mut [u8], start: usize) -> usize {
        let comparable_values = self.get_comparable_values();
        let mut offset = start;
        for v in &comparable_values {
            LongPoint::encode_dimension(*v, buf, offset);
            offset += 8;
        }
        comparable_values.len() * 8
    }

    /// Returns the size of the packed values in this facet set.
    ///
    /// Equivalent to `sizePackedBytes()`.
    fn size_packed_bytes(&self) -> usize {
        self.dims() as usize * 8
    }
}

/// `validateValuesAndGetNumDims(...)` of the subclasses.
pub(crate) fn validate_values_and_get_num_dims(len: usize) -> rucene::error::Result<i32> {
    if len == 0 {
        return Err(rucene::error::LuceneError::illegal_argument(
            "values cannot be null or empty",
        ));
    }
    Ok(len as i32)
}
