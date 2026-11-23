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

//! Port of `IntAssociationFacetField.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use rucene::error::Result;
use rucene::util::BytesRef;

use crate::taxonomy::association_facet_field::{AssociationFacetField, AssociationKind};

/// Add an instance of this to your `Document` to add a facet label
/// associated with an int.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.IntAssociationFacetField`,
/// a subclass of `AssociationFacetField`: its constructor builds an
/// [`AssociationFacetField`] of [`AssociationKind::Int`], which is what
/// `FacetsConfig.build`'s `instanceof IntAssociationFacetField` tests.
pub struct IntAssociationFacetField;

impl IntAssociationFacetField {
    /// Creates this from `assoc` which represents the association, a
    /// dimension and a path.
    ///
    /// Equivalent to `IntAssociationFacetField(int, String, String...)`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(assoc: i32, dim: &str, path: &[&str]) -> Result<AssociationFacetField> {
        AssociationFacetField::with_kind(int_to_bytes_ref(assoc), dim, path, AssociationKind::Int)
    }

    /// Equivalent to `intToBytesRef(int)`.
    pub fn int_to_bytes_ref(v: i32) -> BytesRef {
        int_to_bytes_ref(v)
    }

    /// Equivalent to `bytesRefToInt(BytesRef)`.
    pub fn bytes_ref_to_int(b: &BytesRef) -> i32 {
        bytes_ref_to_int(b)
    }
}

/// Encodes an `int` as a 4-byte `BytesRef`, big-endian.
///
/// Equivalent to `IntAssociationFacetField.intToBytesRef(int)`.
pub fn int_to_bytes_ref(v: i32) -> BytesRef {
    BytesRef::new(v.to_be_bytes().to_vec())
}

/// Decodes a previously encoded `int`.
///
/// Equivalent to `IntAssociationFacetField.bytesRefToInt(BytesRef)`.
pub fn bytes_ref_to_int(b: &BytesRef) -> i32 {
    let o = b.offset;
    i32::from_be_bytes([b.bytes[o], b.bytes[o + 1], b.bytes[o + 2], b.bytes[o + 3]])
}
