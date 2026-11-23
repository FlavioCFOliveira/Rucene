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

//! Port of `FloatAssociationFacetField.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use rucene::error::Result;
use rucene::util::BytesRef;

use crate::taxonomy::association_facet_field::{AssociationFacetField, AssociationKind};
use crate::taxonomy::int_association_facet_field::{bytes_ref_to_int, int_to_bytes_ref};

/// Add an instance of this to your `Document` to add a facet label
/// associated with a float.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.FloatAssociationFacetField`,
/// a subclass of `AssociationFacetField`: its constructor builds an
/// [`AssociationFacetField`] of [`AssociationKind::Float`].
pub struct FloatAssociationFacetField;

impl FloatAssociationFacetField {
    /// Creates this from `assoc` which represents the association, a
    /// dimension and a path.
    ///
    /// Equivalent to `FloatAssociationFacetField(float, String, String...)`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(assoc: f32, dim: &str, path: &[&str]) -> Result<AssociationFacetField> {
        AssociationFacetField::with_kind(
            float_to_bytes_ref(assoc),
            dim,
            path,
            AssociationKind::Float,
        )
    }

    /// Equivalent to `floatToBytesRef(float)`.
    pub fn float_to_bytes_ref(v: f32) -> BytesRef {
        float_to_bytes_ref(v)
    }

    /// Equivalent to `bytesRefToFloat(BytesRef)`.
    pub fn bytes_ref_to_float(b: &BytesRef) -> f32 {
        bytes_ref_to_float(b)
    }
}

/// Encodes a `float` as a 4-byte `BytesRef` (`Float.floatToIntBits`, which
/// canonicalizes NaN).
///
/// Equivalent to `FloatAssociationFacetField.floatToBytesRef(float)`.
pub fn float_to_bytes_ref(v: f32) -> BytesRef {
    let bits = if v.is_nan() {
        0x7fc0_0000
    } else {
        v.to_bits() as i32
    };
    int_to_bytes_ref(bits)
}

/// Decodes a previously encoded `float`.
///
/// Equivalent to `FloatAssociationFacetField.bytesRefToFloat(BytesRef)`.
pub fn bytes_ref_to_float(b: &BytesRef) -> f32 {
    f32::from_bits(bytes_ref_to_int(b) as u32)
}
