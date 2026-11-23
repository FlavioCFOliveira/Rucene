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

//! Port of `FacetField.java`, of the Java package `org.apache.lucene.facet`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::document::{Field, FieldType};
use rucene::error::{LuceneError, Result};

use crate::taxonomy::facet_label::arrays_to_string;

/// Delegates `IndexableField` to the wrapped `Field` a Java subclass of
/// `Field` extends, with `as_any` answering the subclass itself.
macro_rules! delegate_indexable_field {
    ($ty:ty, $field:ident) => {
        impl rucene::index::IndexableField for $ty {
            fn name(&self) -> &str {
                rucene::index::IndexableField::name(&self.$field)
            }

            fn field_type(&self) -> &dyn rucene::index::IndexableFieldType {
                rucene::index::IndexableField::field_type(&self.$field)
            }

            fn token_stream(
                &self,
                analyzer: Option<&dyn rucene::analysis::Analyzer>,
                reuse: &mut Option<Box<dyn rucene::analysis::TokenStream>>,
            ) -> rucene::error::Result<Box<dyn rucene::analysis::TokenStream>> {
                rucene::index::IndexableField::token_stream(&self.$field, analyzer, reuse)
            }

            fn binary_value(&self) -> Option<rucene::util::BytesRef> {
                rucene::index::IndexableField::binary_value(&self.$field)
            }

            fn string_value(&self) -> Option<String> {
                rucene::index::IndexableField::string_value(&self.$field)
            }

            fn reader_value(&self) -> Option<rucene::document::SharedReader> {
                rucene::index::IndexableField::reader_value(&self.$field)
            }

            fn numeric_value(&self) -> Option<rucene::document::NumericValue> {
                rucene::index::IndexableField::numeric_value(&self.$field)
            }

            fn stored_value(&self) -> rucene::error::Result<Option<rucene::document::StoredValue>> {
                rucene::index::IndexableField::stored_value(&self.$field)
            }

            fn invertable_type(&self) -> Option<rucene::document::InvertableType> {
                rucene::index::IndexableField::invertable_type(&self.$field)
            }

            fn as_any(&self) -> Option<&dyn std::any::Any> {
                Some(self)
            }
        }
    };
}
pub(crate) use delegate_indexable_field;

/// Field type used for storing facet values.
///
/// Equivalent to `FacetField.TYPE`.
pub static TYPE: LazyLock<Arc<FieldType>> = LazyLock::new(|| Arc::new(FieldType::new()));

/// Add an instance of this to your `Document` for every facet label.
///
/// Equivalent to `org.apache.lucene.facet.FacetField`, a `Field` named
/// `"dummy"` of [`TYPE`].
#[derive(Debug)]
pub struct FacetField {
    field: Field,
    /// Dimension for this field.
    pub dim: String,
    /// Path for this field.
    pub path: Vec<String>,
}

impl FacetField {
    /// Creates the this from `dim` and `path`.
    ///
    /// Equivalent to `FacetField(String, String...)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty label or an empty path.
    pub fn new(dim: &str, path: &[&str]) -> Result<Self> {
        let field = Field::new_with_type("dummy", Arc::clone(&TYPE))?;
        verify_label(dim)?;
        for label in path {
            verify_label(label)?;
        }
        if path.is_empty() {
            return Err(LuceneError::illegal_argument(
                "path must have at least one element",
            ));
        }
        Ok(Self {
            field,
            dim: dim.to_string(),
            path: path.iter().map(|p| p.to_string()).collect(),
        })
    }
}

impl fmt::Display for FacetField {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FacetField(dim={} path={})",
            self.dim,
            arrays_to_string(&self.path)
        )
    }
}

delegate_indexable_field!(FacetField, field);

/// Verifies the label is not null or empty string.
///
/// Equivalent to `FacetField.verifyLabel(String)`.
pub fn verify_label(label: &str) -> Result<()> {
    if label.is_empty() {
        return Err(LuceneError::illegal_argument(format!(
            "empty or null components not allowed; got: {label}"
        )));
    }
    Ok(())
}
