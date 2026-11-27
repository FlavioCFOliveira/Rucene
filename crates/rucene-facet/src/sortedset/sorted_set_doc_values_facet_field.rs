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

//! Port of `SortedSetDocValuesFacetField.java`, of the Java package
//! `org.apache.lucene.facet.sortedset`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::document::{Field, FieldType};
use rucene::error::{LuceneError, Result};
use rucene::index::IndexOptions;

use crate::facet_field::{delegate_indexable_field, verify_label_nullable};
use crate::facets_config::FacetsConfig;

/// Indexed `FieldType`.
///
/// Equivalent to `SortedSetDocValuesFacetField.TYPE`.
pub static TYPE: LazyLock<Arc<FieldType>> = LazyLock::new(|| {
    let field_type = FieldType::new();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .expect("INVARIANT: a new FieldType is not frozen");
    field_type.freeze();
    Arc::new(field_type)
});

/// Add an instance of this to your Document for every facet label to be
/// indexed via SortedSetDocValues.
///
/// Equivalent to `org.apache.lucene.facet.sortedset.SortedSetDocValuesFacetField`.
#[derive(Debug)]
pub struct SortedSetDocValuesFacetField {
    field: Field,
    /// Dimension.
    pub dim: String,
    /// Path.
    pub path: Vec<String>,
}

impl SortedSetDocValuesFacetField {
    /// Sole constructor.
    ///
    /// Equivalent to `SortedSetDocValuesFacetField(String, String...)`.
    pub fn new(dim: &str, path: &[&str]) -> Result<Self> {
        let path: Vec<Option<&str>> = path.iter().copied().map(Some).collect();
        Self::new_nullable(Some(dim), &path)
    }

    /// Sole constructor, with Java's nullable `String` (`None` is `null`).
    ///
    /// Equivalent to `SortedSetDocValuesFacetField(String, String...)`: the
    /// path labels are verified before the dimension, as in Java.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a `null` or empty label or an empty
    /// path.
    pub fn new_nullable(dim: Option<&str>, path: &[Option<&str>]) -> Result<Self> {
        let field = Field::new_with_type("dummy", Arc::clone(&TYPE))?;
        let path = path
            .iter()
            .map(|label| verify_label_nullable(*label).map(str::to_string))
            .collect::<Result<Vec<String>>>()?;
        let dim = verify_label_nullable(dim)?;
        if path.is_empty() {
            return Err(LuceneError::illegal_argument(
                "path must have at least one element",
            ));
        }
        Ok(Self {
            field,
            dim: dim.to_string(),
            path,
        })
    }
}

impl fmt::Display for SortedSetDocValuesFacetField {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = FacetsConfig::path_to_string_len(&self.path, self.path.len())
            .map_err(|_| fmt::Error)?;
        write!(
            f,
            "SortedSetDocValuesFacetField(dim={} path={})",
            self.dim, path
        )
    }
}

delegate_indexable_field!(SortedSetDocValuesFacetField, field);
