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

//! Port of `AssociationFacetField.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::document::{Field, FieldType};
use rucene::error::{LuceneError, Result};
use rucene::index::IndexOptions;
use rucene::util::BytesRef;

use crate::facet_field::{delegate_indexable_field, verify_label_nullable};
use crate::taxonomy::facet_label::arrays_to_string;

/// Indexed `FieldType`.
///
/// Equivalent to `AssociationFacetField.TYPE`.
pub static TYPE: LazyLock<Arc<FieldType>> = LazyLock::new(|| {
    let field_type = FieldType::new();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .expect("INVARIANT: a new FieldType is not frozen");
    field_type.freeze();
    Arc::new(field_type)
});

/// The Java class of an association field: `AssociationFacetField` or one of
/// its two subclasses, which `FacetsConfig.build` tells apart with
/// `instanceof`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationKind {
    /// `AssociationFacetField` itself: `"bytes"`.
    Bytes,
    /// `IntAssociationFacetField`: `"int"`.
    Int,
    /// `FloatAssociationFacetField`: `"float"`.
    Float,
}

/// Add an instance of this to your `Document` to add a facet label
/// associated with an arbitrary `byte[]`.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.AssociationFacetField`.
#[derive(Debug)]
pub struct AssociationFacetField {
    field: Field,
    /// Dimension for this field.
    pub dim: String,
    /// Facet path for this field.
    pub path: Vec<String>,
    /// Associated value.
    pub assoc: BytesRef,
    kind: AssociationKind,
}

impl AssociationFacetField {
    /// Creates this from `bytes` which represents the association, a
    /// dimension and a path.
    ///
    /// Equivalent to `AssociationFacetField(BytesRef, String, String...)`.
    pub fn new(assoc: BytesRef, dim: &str, path: &[&str]) -> Result<Self> {
        let path: Vec<Option<&str>> = path.iter().copied().map(Some).collect();
        Self::new_nullable(assoc, Some(dim), &path)
    }

    /// Creates this from `bytes` which represents the association, a
    /// nullable dimension and nullable path labels.
    ///
    /// Equivalent to `AssociationFacetField(BytesRef, String, String...)`
    /// with Java's nullable `String` (`None` is `null`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a `null` or empty label or an empty
    /// path.
    pub fn new_nullable(assoc: BytesRef, dim: Option<&str>, path: &[Option<&str>]) -> Result<Self> {
        Self::with_kind(assoc, dim, path, AssociationKind::Bytes)
    }

    pub(crate) fn with_kind(
        assoc: BytesRef,
        dim: Option<&str>,
        path: &[Option<&str>],
        kind: AssociationKind,
    ) -> Result<Self> {
        let field = Field::new_with_type("dummy", Arc::clone(&TYPE))?;
        let dim = verify_label_nullable(dim)?;
        let path = path
            .iter()
            .map(|label| verify_label_nullable(*label).map(str::to_string))
            .collect::<Result<Vec<String>>>()?;
        if path.is_empty() {
            return Err(LuceneError::illegal_argument(
                "path must have at least one element",
            ));
        }
        Ok(Self {
            field,
            dim: dim.to_string(),
            path,
            assoc,
            kind,
        })
    }

    /// Which Java class this field is.
    pub fn kind(&self) -> AssociationKind {
        self.kind
    }
}

impl fmt::Display for AssociationFacetField {
    /// Equivalent to `toString()` of the field's Java class.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            AssociationKind::Bytes => write!(
                f,
                "AssociationFacetField(dim={} path={} bytes={})",
                self.dim,
                arrays_to_string(&self.path),
                self.assoc
            ),
            AssociationKind::Int => write!(
                f,
                "IntAssociationFacetField(dim={} path={} value={})",
                self.dim,
                arrays_to_string(&self.path),
                crate::taxonomy::int_association_facet_field::bytes_ref_to_int(&self.assoc)
            ),
            AssociationKind::Float => write!(
                f,
                "FloatAssociationFacetField(dim={} path={} value={})",
                self.dim,
                arrays_to_string(&self.path),
                rucene::util::java_fmt::float_to_string(
                    crate::taxonomy::float_association_facet_field::bytes_ref_to_float(&self.assoc)
                )
            ),
        }
    }
}

delegate_indexable_field!(AssociationFacetField, field);
